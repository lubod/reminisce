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

#[derive(Clone)]
pub struct RateLimiter {
    // Shared state across workers
    buckets: Arc<Mutex<HashMap<IpAddr, TokenBucket>>>,
}

impl RateLimiter {
    pub fn new() -> Self {
        Self {
            buckets: Arc::new(Mutex::new(HashMap::new())),
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
            buckets: self.buckets.clone(),
        }))
    }
}

pub struct RateLimitMiddleware<S> {
    service: S,
    buckets: Arc<Mutex<HashMap<IpAddr, TokenBucket>>>,
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
        // Extract IP address
        let ip = req
            .peer_addr()
            .map(|addr| addr.ip())
            .unwrap_or_else(|| IpAddr::V4(std::net::Ipv4Addr::new(127, 0, 0, 1)));

        // Determine limits based on target path
        let path = req.path();
        let is_auth = path.contains("/auth/");

        // General limits: max 100 tokens, refills 5 tokens/second (bursts allowed, fast recovery)
        // Stricter auth limits: max 5 tokens, refills 0.1 tokens/second (max 6 requests per minute)
        let (max_tokens, refill_rate) = if is_auth {
            (5.0, 0.1)
        } else {
            (100.0, 5.0)
        };

        let mut buckets = self.buckets.lock().unwrap();
        let bucket = buckets
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
