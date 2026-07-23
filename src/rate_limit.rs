use actix_web::{
    body::EitherBody,
    dev::{forward_ready, Service, ServiceRequest, ServiceResponse, Transform},
    Error, HttpResponse,
};
use futures_util::future::{ready, LocalBoxFuture, Ready};
use std::{
    collections::HashMap,
    net::IpAddr,
    sync::{Arc, Mutex},
    time::Instant,
};

// Token Bucket for tracking rate limits
struct TokenBucket {
    tokens: f64,
    last_update: Instant,
}

impl TokenBucket {
    fn new(max_tokens: f64) -> Self {
        Self {
            tokens: max_tokens,
            last_update: Instant::now(),
        }
    }

    fn consume(&mut self, max_tokens: f64, refill_rate: f64, amount: f64) -> bool {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_update).as_secs_f64();
        self.last_update = now;

        // Refill tokens based on elapsed time
        self.tokens = (self.tokens + elapsed * refill_rate).min(max_tokens);

        if self.tokens >= amount {
            self.tokens -= amount;
            true
        } else {
            false
        }
    }
}

struct LimiterState {
    buckets: HashMap<IpAddr, TokenBucket>,
    last_cleanup: Instant,
}

#[derive(Clone)]
pub struct RateLimiter {
    // Shared state across workers
    state: Arc<Mutex<LimiterState>>,
}

impl RateLimiter {
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(LimiterState {
                buckets: HashMap::new(),
                last_cleanup: Instant::now(),
            })),
        }
    }
}

impl<S, B> Transform<S, ServiceRequest> for RateLimiter
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    S::Future: 'static,
    B: 'static,
{
    type Response = ServiceResponse<EitherBody<B>>;
    type Error = Error;
    type Transform = RateLimitMiddleware<S>;
    type InitError = ();
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ready(Ok(RateLimitMiddleware {
            service,
            state: self.state.clone(),
        }))
    }
}

pub struct RateLimitMiddleware<S> {
    service: S,
    state: Arc<Mutex<LimiterState>>,
}

impl<S, B> Service<ServiceRequest> for RateLimitMiddleware<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    S::Future: 'static,
    B: 'static,
{
    type Response = ServiceResponse<EitherBody<B>>;
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    forward_ready!(service);

    fn call(&self, req: ServiceRequest) -> Self::Future {
        // Extract IP address (supporting X-Forwarded-For / X-Real-IP behind reverse proxy)
        let mut ip = req
            .peer_addr()
            .map(|addr| addr.ip())
            .unwrap_or_else(|| IpAddr::V4(std::net::Ipv4Addr::new(127, 0, 0, 1)));

        if let Some(x_forwarded_for) = req.headers().get("x-forwarded-for") {
            if let Ok(x_forwarded_str) = x_forwarded_for.to_str() {
                if let Some(first_ip_str) = x_forwarded_str.split(',').next() {
                    if let Ok(parsed_ip) = first_ip_str.trim().parse::<IpAddr>() {
                        ip = parsed_ip;
                    }
                }
            }
        } else if let Some(x_real_ip) = req.headers().get("x-real-ip") {
            if let Ok(x_real_str) = x_real_ip.to_str() {
                if let Ok(parsed_ip) = x_real_str.trim().parse::<IpAddr>() {
                    ip = parsed_ip;
                }
            }
        }

        // Determine limits based on target path
        let path = req.path();

        // Exclude system, health, metrics, setup-status, and documentation endpoints from rate limiting
        if path == "/ping"
            || path == "/health"
            || path == "/metrics"
            || path == "/api-doc/openapi.json"
            || path.starts_with("/swagger-ui/")
            || path == "/swagger-ui"
            || path == "/api/auth/setup-status"
        {
            let fut = self.service.call(req);
            return Box::pin(async move {
                let res = fut.await?;
                Ok(res.map_into_left_body())
            });
        }

        let is_login = path.contains("/user-login") || path.contains("/login") || path.contains("/register");

        // General limits: max 100 tokens, refills 5 tokens/second (bursts allowed, fast recovery)
        // Stricter login limits: max 20 tokens, refills 0.5 tokens/second
        let (max_tokens, refill_rate) = if is_login {
            (20.0, 0.5)
        } else {
            (100.0, 5.0)
        };

        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());

        // Periodic cleanup of stale entries (last update > 10 mins ago)
        let now = Instant::now();
        if now.duration_since(state.last_cleanup).as_secs() > 60 {
            state.buckets.retain(|_, bucket| {
                now.duration_since(bucket.last_update).as_secs() < 600
            });
            state.last_cleanup = now;
        }

        let bucket = state.buckets
            .entry(ip)
            .or_insert_with(|| TokenBucket::new(max_tokens));

        let allowed = bucket.consume(max_tokens, refill_rate, 1.0);

        if !allowed {
            log::warn!("Rate limit exceeded for IP: {} on path: {}", ip, path);
            let (request, _pl) = req.into_parts();
            let response = HttpResponse::TooManyRequests()
                .json(serde_json::json!({
                    "status": "error",
                    "message": "Too many requests. Please try again later."
                }))
                .map_into_right_body();
            return Box::pin(ready(Ok(ServiceResponse::new(request, response))));
        }

        // Allow request to proceed
        let fut = self.service.call(req);
        Box::pin(async move {
            let res = fut.await?;
            Ok(res.map_into_left_body())
        })
    }
}
