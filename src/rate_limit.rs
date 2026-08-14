use actix_web::{
    body::EitherBody,
    dev::{forward_ready, Service, ServiceRequest, ServiceResponse, Transform},
    Error, HttpResponse,
};
use futures_util::future::{ready, LocalBoxFuture, Ready};
use std::{
    collections::HashMap,
    net::{IpAddr, Ipv4Addr},
    sync::{Arc, Mutex},
    time::Instant,
};

fn is_private_or_local(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_loopback() || v4.is_private() || v4.is_link_local(),
        IpAddr::V6(v6) => v6.is_loopback(),
    }
}

/// Resolve the effective client IP for rate limiting.
///
/// Proxy-supplied headers are only trusted when the immediate peer is a
/// loopback/private/link-local address (i.e. the request arrived via the local
/// reverse proxy, which is the deployment's shape). Public peers are treated as
/// direct connections, so their self-supplied `X-Forwarded-For`/`X-Real-IP`
/// headers are ignored — previously any client could rotate its IP header to
/// obtain a fresh rate-limit bucket and bypass e.g. the login brute-force limit.
///
/// When trusted: `X-Real-IP` wins (nginx sets it from `$remote_addr`,
/// overwriting client input), else the *last* `X-Forwarded-For` entry (nginx
/// appends `$remote_addr`, so the last value is the proxy's view, whereas the
/// first value is attacker-controlled).
fn parse_client_ip(peer: Option<IpAddr>, x_real_ip: Option<&str>, x_forwarded_for: Option<&str>) -> IpAddr {
    let peer = peer.unwrap_or_else(|| IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)));
    let trusted_proxy = is_private_or_local(peer);
    if !trusted_proxy {
        return peer;
    }
    if let Some(v) = x_real_ip {
        if let Ok(p) = v.trim().parse::<IpAddr>() {
            return p;
        }
    }
    if let Some(xff) = x_forwarded_for {
        if let Some(last) = xff.rsplit(',').next() {
            if let Ok(p) = last.trim().parse::<IpAddr>() {
                return p;
            }
        }
    }
    peer
}

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
    // Sharded state: each IP is pinned to one shard so concurrent requests from
    // different IPs do not contend on a single global mutex.
    shards: Arc<Vec<Mutex<LimiterState>>>,
}

const SHARD_COUNT: usize = 16;

fn shard_index(ip: IpAddr) -> usize {
    // FNV-1a over the IP octets, folded into the shard space.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let octets: [u8; 16] = match ip {
        IpAddr::V4(v4) => {
            let mut o = [0u8; 16];
            o[12..16].copy_from_slice(&v4.octets());
            o
        }
        IpAddr::V6(v6) => v6.octets(),
    };
    for b in octets {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    (hash as usize) % SHARD_COUNT
}

impl RateLimiter {
    pub fn new() -> Self {
        Self {
            shards: Arc::new(
                (0..SHARD_COUNT)
                    .map(|_| Mutex::new(LimiterState {
                        buckets: HashMap::new(),
                        last_cleanup: Instant::now(),
                    }))
                    .collect(),
            ),
        }
    }
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new()
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
            limiter: self.clone(),
        }))
    }
}

pub struct RateLimitMiddleware<S> {
    service: S,
    limiter: RateLimiter,
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
        // Resolve the effective client IP without trusting peer-supplied headers
        // for direct (public) connections (see parse_client_ip docs).
        let peer_ip = req.peer_addr().map(|addr| addr.ip());
        let x_real_ip = req
            .headers()
            .get("x-real-ip")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());
        let xff = req
            .headers()
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());
        let ip = parse_client_ip(peer_ip, x_real_ip.as_deref(), xff.as_deref());

        // Determine limits based on target path
        let path = req.path();

        // Exclude system, health, metrics, setup-status, documentation, and static media/thumbnail endpoints from rate limiting
        if path == "/ping"
            || path == "/health"
            || path == "/metrics"
            || path == "/api-doc/openapi.json"
            || path.starts_with("/swagger-ui/")
            || path == "/swagger-ui"
            || path == "/api/auth/setup-status"
            || path.starts_with("/api/thumbnail/")
            || path.starts_with("/api/face/")
            || path.starts_with("/api/image/")
            || path.starts_with("/api/images/")
            || path.starts_with("/api/video/")
            || path.starts_with("/api/videos/")
        {
            let fut = self.service.call(req);
            return Box::pin(async move {
                let res = fut.await?;
                Ok(res.map_into_left_body())
            });
        }

        let is_login = path.contains("/user-login") || path.contains("/login") || path.contains("/register");

        // General limits: max 2000 tokens, refills 100 tokens/second (supports large gallery loads & parallel requests)
        // Stricter login limits: max 50 tokens, refills 2 tokens/second
        let (max_tokens, refill_rate) = if is_login {
            (50.0, 2.0)
        } else {
            (2000.0, 100.0)
        };

        let mut state = self
            .limiter
            .shards[shard_index(ip)]
            .lock()
            .unwrap_or_else(|e| e.into_inner());

        const MAX_BUCKETS: usize = 10000;
        let now = Instant::now();

        // Periodic or capacity-triggered cleanup of stale entries (last update > 10 mins ago)
        if now.duration_since(state.last_cleanup).as_secs() > 60 || state.buckets.len() >= MAX_BUCKETS {
            state.buckets.retain(|_, bucket| {
                now.duration_since(bucket.last_update).as_secs() < 600
            });
            state.last_cleanup = now;
        }

        // Emergency eviction if flooded with distinct IPs beyond MAX_BUCKETS
        if state.buckets.len() >= MAX_BUCKETS && !state.buckets.contains_key(&ip) {
            if let Some(oldest_ip) = state.buckets.iter()
                .min_by_key(|(_, b)| b.last_update)
                .map(|(ip, _)| *ip)
            {
                state.buckets.remove(&oldest_ip);
            }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn rate_limit_ignores_spoofed_headers_on_public_peers() {
        // A public (direct) peer cannot regenerate its bucket by lying in headers.
        let peer = ip("203.0.113.7");
        assert_eq!(
            parse_client_ip(Some(peer), Some("1.2.3.4"), Some("1.2.3.4, 5.6.7.8")),
            peer,
            "public peer headers must be ignored",
        );
    }

    #[test]
    fn rate_limit_trusts_x_real_ip_behind_proxy() {
        // Loopback peer (nginx -> backend on host network): X-Real-IP is the real
        // client as set by nginx and must be used.
        assert_eq!(
            parse_client_ip(Some(ip("127.0.0.1")), Some("198.51.100.9"), Some("5.6.7.8")),
            ip("198.51.100.9"),
        );
    }

    #[test]
    fn rate_limit_uses_last_forwarded_entry_not_the_spoofed_first() {
        // $proxy_add_x_forwarded_for = "$remote_addr, $http_x_forwarded_for",
        // so the LAST entry is the proxy's view and the FIRST is attacker input.
        assert_eq!(
            parse_client_ip(Some(ip("127.0.0.1")), None, Some("1.2.3.4, 198.51.100.9")),
            ip("198.51.100.9"),
        );
    }

    #[test]
    fn rate_limit_falls_back_to_peer_on_garbage_headers() {
        assert_eq!(
            parse_client_ip(Some(ip("10.0.0.5")), Some("not-an-ip"), Some("also-bad")),
            ip("10.0.0.5"),
        );
        assert_eq!(parse_client_ip(None, None, None), ip("127.0.0.1"));
    }

    #[test]
    fn token_bucket_allows_full_capacity_and_denies_when_empty() {
        let mut tb = TokenBucket::new(10.0);
        assert!(tb.consume(10.0, 1.0, 10.0), "a full burst must be allowed");
        assert!(!tb.consume(10.0, 1.0, 1.0), "an empty bucket must deny immediately");
    }

    #[test]
    fn token_bucket_refills_over_time() {
        let mut tb = TokenBucket::new(10.0);
        assert!(tb.consume(10.0, 10.0, 10.0));
        assert!(!tb.consume(10.0, 10.0, 1.0), "drained bucket denies");
        // ~200ms at 10 tokens/sec refills ~2 tokens -> a 1-token request passes.
        std::thread::sleep(Duration::from_millis(200));
        assert!(tb.consume(10.0, 10.0, 1.0), "bucket refilled enough for one token");
    }

    #[test]
    fn token_bucket_caps_at_max_capacity() {
        let mut tb = TokenBucket::new(10.0);
        assert!(tb.consume(10.0, 10.0, 1.0));
        std::thread::sleep(Duration::from_millis(500));
        // 500ms * 10/s = 5 refilled; tokens are capped at max (never above).
        assert!(tb.consume(10.0, 10.0, 10.0), "refill is capped at max_tokens");
        assert!(!tb.consume(10.0, 10.0, 1.0), "drained again after full draw");
    }

    #[test]
    fn shard_index_is_stable_and_in_range() {
        let ips: [IpAddr; 4] = [
            "1.2.3.4".parse().unwrap(),
            "192.168.1.1".parse().unwrap(),
            "::1".parse().unwrap(),
            "2001:db8::1".parse().unwrap(),
        ];
        for ip in ips {
            let a = shard_index(ip);
            assert!(a < SHARD_COUNT, "shard index out of range: {}", a);
            assert_eq!(a, shard_index(ip), "same IP must map to the same shard");
        }
    }

    #[test]
    fn test_is_private_or_local_branches() {
        assert!(is_private_or_local("127.0.0.1".parse().unwrap()));
        assert!(is_private_or_local("10.0.0.1".parse().unwrap()));
        assert!(is_private_or_local("192.168.1.50".parse().unwrap()));
        assert!(is_private_or_local("169.254.1.1".parse().unwrap()));
        assert!(!is_private_or_local("8.8.8.8".parse().unwrap()));
        assert!(is_private_or_local("::1".parse().unwrap()));
        assert!(!is_private_or_local("2607:f8b0:4005:805::200e".parse().unwrap()));
    }

    #[actix_web::test]
    async fn test_rate_limiter_middleware_actix_integration() {
        use actix_web::{test, web, App, HttpResponse};

        let app = test::init_service(
            App::new()
                .wrap(RateLimiter::new())
                .route("/ping", web::get().to(|| async { HttpResponse::Ok().body("pong") }))
                .route("/api/auth/login", web::post().to(|| async { HttpResponse::Ok().body("logged_in") }))
                .route("/api/data", web::get().to(|| async { HttpResponse::Ok().body("data") }))
        ).await;

        // 1. Exempt route always succeeds
        let req = test::TestRequest::get().uri("/ping").to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), actix_web::http::StatusCode::OK);

        // 2. Normal data route succeeds
        let req = test::TestRequest::get().uri("/api/data").to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), actix_web::http::StatusCode::OK);

        // 3. Login route succeeds within 50 burst
        for _ in 0..50 {
            let req = test::TestRequest::post()
                .uri("/api/auth/login")
                .insert_header(("x-real-ip", "198.51.100.55"))
                .to_request();
            let _ = test::call_service(&app, req).await;
        }

        // 51st request should be throttled (429 Too Many Requests)
        let req_over = test::TestRequest::post()
            .uri("/api/auth/login")
            .insert_header(("x-real-ip", "198.51.100.55"))
            .to_request();
        let resp_over = test::call_service(&app, req_over).await;
        assert_eq!(resp_over.status(), actix_web::http::StatusCode::TOO_MANY_REQUESTS);
    }
}

