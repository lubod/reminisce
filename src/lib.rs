use utoipa::OpenApi;
use std::sync::Arc;


pub mod config;
pub mod constants;
pub mod db;
pub mod db_instrumentation;
pub mod query_builder;
pub mod utils;
pub mod auth_utils;
pub mod system_utils;
pub mod geo_utils;
pub mod media_utils;
pub mod verification_worker;
pub mod p2p_audit_worker;
pub mod media_replication_worker;
pub mod shard_rebalance_worker;
pub mod db_backup_worker;
pub mod p2p_restore;
pub mod db_restore;
pub mod p2p_upload;
pub mod ai_worker;
pub mod ai_client;
pub mod telemetry;
pub mod metrics;
pub mod logtail;
pub mod alerts;
pub mod openapi;
pub mod p2p_error;

pub mod duplicate_worker;
pub mod test_utils;
pub mod metrics_collector;
pub mod rate_limit;

pub mod services {
    pub mod auth;
    pub mod health;
    pub mod existence_check;
    pub mod upload;
    pub mod thumbnail;
    pub mod media;
    pub mod embedding;
    pub mod text_search;
    pub mod stats;
    pub mod pool_stats;
    pub mod geodb_stats;
    pub mod geocoding;
    pub mod map;
    pub mod ai_settings;
    pub mod face_detection;
    pub mod person;
    pub mod system_stats;
    pub mod label;
    pub mod ingest;
    pub mod import_dir;
    pub mod p2p_status;
pub mod p2p_restore;
    pub mod proxy_manager;
    pub mod duplicates;
    pub mod quality;
    pub mod user_management;
    pub mod observability;
}

use crate::config::Config;
use actix_web::{ App, HttpServer, web, HttpResponse };
use prometheus::{Encoder, TextEncoder};

use tracing::{error, info, warn, Span};
use tracing_actix_web::{TracingLogger, RootSpanBuilder};
use actix_web_prom::PrometheusMetricsBuilder;
use actix_web::dev::ServiceRequest;
use actix_web::http::header;
use tracing::field::Empty;

// Swagger UI is a development convenience: compiled out of release builds so
// production never advertises the full route surface unauthenticated.
#[cfg(debug_assertions)]
fn configure_swagger(cfg: &mut web::ServiceConfig) {
    use utoipa_swagger_ui::SwaggerUi;
    cfg.service(
        SwaggerUi::new("/swagger-ui/{_:.*}").url("/api-doc/openapi.json", crate::openapi::ApiDoc::openapi())
    );
}

#[cfg(not(debug_assertions))]
fn configure_swagger(_cfg: &mut web::ServiceConfig) {}

async fn metrics_handler(
    req: actix_web::HttpRequest,
    config: web::Data<Config>,
) -> Result<HttpResponse, actix_web::Error> {
    let claims = match crate::auth_utils::authenticate_request(&req, "metrics", config.get_api_key()).await {
        Ok(c) => c,
        Err(response) => return Ok(response),
    };
    if claims.role != "admin" {
        return Ok(HttpResponse::Forbidden().json(serde_json::json!({
            "error": "Forbidden: Admin role required"
        })));
    }

    let encoder = TextEncoder::new();
    let mut buffer = vec![];
    let metric_families = prometheus::gather();
    match encoder.encode(&metric_families, &mut buffer) {
        Ok(_) => {
            Ok(HttpResponse::Ok()
                .content_type(encoder.format_type())
                .body(buffer))
        }
        Err(e) => {
            error!("Could not encode metrics: {}", e);
            Ok(HttpResponse::InternalServerError().finish())
        }
    }
}

pub use crate::services::auth::{register_user, user_login, user_login_form, user_logout, get_me, setup_status, setup_admin, Claims};
pub use crate::services::user_management::{list_users, create_user, update_user, delete_user};
pub use crate::services::health::{ping, health_check, HealthCheckResponse};
pub use crate::services::existence_check::{check_image_exists, check_video_exists};
pub use crate::services::upload::{upload_image, upload_video, upload_image_metadata, upload_video_metadata, batch_upload_image, check_images_exist_batch, check_videos_exist_batch, batch_check_images, batch_check_videos};
pub use crate::services::thumbnail::{list_image_thumbnails, list_video_thumbnails, list_all_media_thumbnails, get_thumbnail, get_face_thumbnail};
pub use crate::services::media::{get_image, get_video, get_image_metadata, toggle_image_star, toggle_video_star, delete_image, delete_video, get_device_ids, get_random_image, restore_image, restore_video, get_trash, enhance_image, save_enhanced_image};
pub use crate::services::embedding::search_images;
pub use crate::services::embedding::search_video_keyframes;
pub use crate::services::stats::get_stats;
pub use crate::services::pool_stats::get_pool_stats;
pub use crate::services::geodb_stats::get_geodb_stats;
pub use crate::services::geocoding::search_places;
pub use crate::services::map::get_map_points;
pub use crate::services::ai_settings::{get_ai_settings, update_ai_settings};
pub use crate::services::import_dir::{import_directory, get_import_status};


struct CustomRootSpanBuilder;

impl RootSpanBuilder for CustomRootSpanBuilder {
    fn on_request_start(request: &ServiceRequest) -> Span {
        let path = request.path();
        let method = request.method();
        let version = request.version();
        let scheme = request.connection_info().scheme().to_string();
        let host = request.connection_info().host().to_string();
        let client_ip = request.connection_info().realip_remote_addr().map(|s| s.to_string());
        let user_agent = request.headers().get(header::USER_AGENT).and_then(|h| h.to_str().ok()).unwrap_or("");
        // Path only (no query string): query params must never reach logs/OTLP.
        let target = request.path().to_string();
        let request_id = uuid::Uuid::new_v4().to_string();

        if path == "/pool-stats" || path == "/system-stats" {
            tracing::debug_span!(
                "HTTP request",
                http.method = %method,
                http.route = %path,
                http.flavor = ?version,
                http.scheme = %scheme,
                http.host = %host,
                http.client_ip = ?client_ip,
                http.user_agent = %user_agent,
                http.target = %target,
                otel.kind = "server",
                otel.name = %format!("{} {}", method, path),
                request_id = %request_id,
                http.status_code = Empty,
                otel.status_code = Empty,
                exception.message = Empty,
            )
        } else {
            tracing::info_span!(
                "HTTP request",
                http.method = %method,
                http.route = %path,
                http.flavor = ?version,
                http.scheme = %scheme,
                http.host = %host,
                http.client_ip = ?client_ip,
                http.user_agent = %user_agent,
                http.target = %target,
                otel.kind = "server",
                otel.name = %format!("{} {}", method, path),
                request_id = %request_id,
                http.status_code = Empty,
                otel.status_code = Empty,
                exception.message = Empty,
            )
        }
    }

    fn on_request_end<B>(_span: Span, _outcome: &Result<actix_web::dev::ServiceResponse<B>, actix_web::Error>) {
    }
}

pub async fn run_server(config: Config) -> std::io::Result<()> {
    info!("Server starting up with config file");
    crate::metrics::init_metrics();

    let shutdown_token = tokio_util::sync::CancellationToken::new();

    // Signal listener task to cancel the token on termination signals
    let signal_token = shutdown_token.clone();
    tokio::spawn(async move {
        #[cfg(unix)]
        {
            use tokio::signal::unix::{signal, SignalKind};
            let mut sigint = signal(SignalKind::interrupt()).unwrap();
            let mut sigterm = signal(SignalKind::terminate()).unwrap();
            tokio::select! {
                _ = sigint.recv() => info!("Received SIGINT signal. Starting graceful shutdown..."),
                _ = sigterm.recv() => info!("Received SIGTERM signal. Starting graceful shutdown..."),
            }
        }
        #[cfg(not(unix))]
        {
            let _ = tokio::signal::ctrl_c().await;
            info!("Received Ctrl-C signal. Starting graceful shutdown...");
        }
        signal_token.cancel();
    });

    // Validate API Secret Key strength
    let api_secret = config.api_secret_key.as_deref().unwrap_or("");
    if api_secret.is_empty() {
        error!("❌ api_secret_key is missing or empty in configuration!");
        return Err(std::io::Error::other("api_secret_key is required"));
    }
    if api_secret.len() < 32 {
        error!("❌ api_secret_key is too weak! Must be at least 32 characters.");
        return Err(std::io::Error::other(
            "api_secret_key must be at least 32 characters long",
        ));
    }

    if config.database_url.is_none() {
        error!("❌ Headless P2P storage node mode has been removed!");
        return Err(std::io::Error::other(
            "Headless mode removed - use standalone p2p daemon"
        ));
    }

    let database_url = config.database_url.clone()
        .expect("database_url is required for API mode");
    
    let pool_options = db::DbPoolOptions {
        max_size: config.db_pool_max_size,
        min_size: config.db_pool_min_size,
        timeout_secs: config.db_pool_timeout_secs,
    };

    if !config.db_tls && config.environment.as_deref() == Some("production") {
        log::warn!("SECURITY WARNING: Database TLS is disabled (db_tls: false) in a production environment!");
    }

    let pool = db::create_pool_with_options(&database_url, pool_options.clone(), config.db_tls)
        .expect("Failed to create database pool");
    let geotagging_pool = db::create_pool_with_options(&config.geotagging_database_url, pool_options, config.db_tls)
        .expect("Failed to create geotagging database pool");

    let worker_pool_options = db::DbPoolOptions {
        max_size: 10,
        min_size: 1,
        timeout_secs: config.db_pool_timeout_secs,
    };
    let worker_pool_inner = db::create_pool_with_options(&database_url, worker_pool_options, config.db_tls)
        .expect("Failed to create worker database pool");

    // Apply idempotent schema migrations on every startup. A migration failure
    // is fatal: continuing under an inconsistent schema silently corrupts data,
    // so the server refuses to start instead.
    if let Err(e) = db::run_migrations(&pool).await {
        error!("DB migration failed: {} — refusing to start (schema may be inconsistent)", e);
        std::process::exit(1);
    }

    let main_pool = db::MainDbPool(pool.clone());
    let worker_pool = db::MainDbPool(worker_pool_inner);
    let geo_pool = db::GeotaggingDbPool(geotagging_pool.clone());

    let config_data = web::Data::new(config.clone());
    let import_job_store: web::Data<services::import_dir::ImportJobStore> =
        web::Data::new(std::sync::Mutex::new(std::collections::HashMap::new()));

    let duplicate_status: web::Data<duplicate_worker::SharedDuplicateStatus> =
        web::Data::new(Arc::new(tokio::sync::Mutex::new(
            duplicate_worker::DuplicateWorkerStatus::new()
        )));

    // --- P2P Identity & Service ---
    let p2p_data_path = std::path::Path::new(&config.p2p_data_dir);
    if !p2p_data_path.exists() {
        std::fs::create_dir_all(p2p_data_path).expect("Failed to create P2P data directory");
    }

    let identity_path = p2p_data_path.join("node.key");
    let identity = if config.p2p_deterministic_identity {
        let secret_for_identity = config.get_api_key().expect("api_secret_key required for deterministic P2P identity");
        let id = match config.p2p_identity_kdf.as_deref() {
            Some("argon2id") => {
                info!("P2P identity: Argon2id (hardened) derivation");
                np2p::crypto::NodeIdentity::from_secret_hardened(secret_for_identity)
            }
            _ => {
                warn!("P2P identity uses legacy fast-hash derivation (offline brute-force oracle on the API secret). Set p2p_identity_kdf: argon2id to harden — NOTE this changes the node ID and requires re-pairing storage nodes.");
                np2p::crypto::NodeIdentity::from_secret(secret_for_identity)
            }
        };
        // Persist the derived identity so tooling that reads node.key stays consistent.
        if !identity_path.exists() {
            std::fs::write(&identity_path, id.signing_key.to_bytes()).expect("Failed to save P2P identity file");
        }
        info!("P2P Identity derived from api_secret (deterministic, full-disk-loss recoverable)");
        id
    } else if identity_path.exists() {
        info!("Loading P2P identity from {:?}", identity_path);
        let bytes = std::fs::read(&identity_path).expect("Failed to read P2P identity file");
        np2p::crypto::NodeIdentity::from_secret_bytes(&bytes).expect("Invalid P2P identity file")
    } else {
        info!("Generating new P2P identity...");
        let id = np2p::crypto::NodeIdentity::generate();
        std::fs::write(&identity_path, id.signing_key.to_bytes()).expect("Failed to save P2P identity file");
        info!("P2P Identity saved to {:?}", identity_path);
        id
    };

        info!("P2P Node ID: {}", hex::encode(identity.node_id()));

        let p2p_service = Arc::new(np2p::network::P2PService::with_allowed_nodes(
            "0.0.0.0:0".parse().unwrap(),
            identity,
            config.p2p_allowed_node_ids.clone(),
        ).await.expect("Failed to initialize P2P service"));
        if config.p2p_allowed_node_ids.is_empty() {
            warn!("P2P admission is OPEN — set p2p_allowed_node_ids in config.yaml to restrict which nodes can receive shards");
        } else {
            info!("P2P admission allow-list active with {} node(s)", config.p2p_allowed_node_ids.len());
        }
    
        if let Err(e) = services::ai_settings::load_ai_settings_from_db(&pool, &config).await {
            error!("Failed to load AI settings from database: {}", e);
        }
        let verification_token = shutdown_token.clone();
        tokio::spawn(
            verification_worker::start_verification_worker(
                web::Data::new(worker_pool.clone()),
                config_data.clone(),
                verification_token,
            )
        );
    
        let ai_token = shutdown_token.clone();
        tokio::spawn(
            crate::ai_worker::start_ai_worker(
                web::Data::new(worker_pool.clone()),
                config_data.clone(),
                ai_token,
            )
        );

        let duplicate_token = shutdown_token.clone();
        tokio::spawn(
            duplicate_worker::start_duplicate_worker(
                web::Data::new(worker_pool.clone()),
                duplicate_status.get_ref().clone(),
                config_data.clone(),
                duplicate_token,
            )
        );

        let metrics_token = shutdown_token.clone();
        tokio::spawn(
            metrics_collector::start_metrics_collector(
                web::Data::new(worker_pool.clone()),
                web::Data::new(geo_pool.clone()),
                config_data.clone(),
                metrics_token,
            )
        );
    
        if config.database_url.is_some() {
            let replication_pool = worker_pool.0.clone();
            let replication_config = config.clone();
            let replication_service = p2p_service.clone();
            let replication_token = shutdown_token.clone();
            tokio::spawn(async move {
                media_replication_worker::media_replication_loop(
                    replication_pool,
                    replication_config,
                    replication_service,
                    replication_token,
                ).await;
            });

            let db_backup_pool = worker_pool.0.clone();
            let db_backup_config = config.clone();
            let db_backup_service = p2p_service.clone();
            let db_backup_token = shutdown_token.clone();
            tokio::spawn(async move {
                db_backup_worker::db_backup_loop(
                    db_backup_pool,
                    db_backup_config,
                    db_backup_service,
                    db_backup_token,
                ).await;
            });
        }
    
        // Discovery listener — hears UDP broadcasts from storage nodes on the LAN
        np2p::network::discovery::start_listener(
            p2p_service.registry.clone(),
            config.p2p_discovery_port,
            hex::encode(p2p_service.identity().node_id()),
        );

        // Coordinator client — cross-network peer discovery + reverse tunnel
        if config.p2p_coordinator_addr.is_some() && config.p2p_coordinator_node_id.is_none() {
            error!("p2p_coordinator_addr is set but p2p_coordinator_node_id is missing — cannot verify coordinator identity; coordinator/tunnel disabled (add p2p_coordinator_node_id to config)");
        }
        if let (Some(coord_str), Some(coord_node_id)) =
            (&config.p2p_coordinator_addr, &config.p2p_coordinator_node_id)
        {
            match tokio::net::lookup_host(coord_str.as_str()).await {
                Ok(mut addrs) => match addrs.next() {
                    Some(coord_addr) => {
                        let node_id = hex::encode(p2p_service.identity().node_id());

                        // Register with coordinator and sync peer list
                        np2p::network::coordinator::start_coordinator_client(
                            coord_addr,
                            coord_node_id,
                            p2p_service.node().clone(),
                            node_id.clone(),
                            None, // home server doesn't expose a storage port
                            p2p_service.registry.clone(),
                            config.p2p_namespace.clone(),
                        );

                        // Reverse tunnel — lets Android reach this home server via VPS
                        if let Some(local_port) = config.p2p_tunnel_local_port {
                            np2p::network::tunnel::start_tunnel_client(
                                coord_addr,
                                coord_node_id,
                                p2p_service.node().clone(),
                                (*p2p_service.identity()).clone(),
                                local_port,
                            );
                            info!("Reverse tunnel started → coordinator={} local_port={}", coord_str, local_port);
                        }
                    }
                    None => error!("p2p_coordinator_addr '{}' resolved to no addresses", coord_str),
                },
                Err(e) => error!("Failed to resolve p2p_coordinator_addr '{}': {}", coord_str, e),
            }
        }

        // Registry starts empty on boot — peers register as they connect via discovery

            let audit_token = shutdown_token.clone();
            tokio::spawn(
                crate::p2p_audit_worker::start_audit_worker(
                    worker_pool.0.clone(),
                    config.clone(),
                    p2p_service.clone(),
                    audit_token,
                )
            );

            let rebalance_token = shutdown_token.clone();
            tokio::spawn(
                crate::shard_rebalance_worker::start_rebalance_worker(
                    worker_pool.0.clone(),
                    config.clone(),
                    p2p_service.clone(),
                    rebalance_token,
                )
            );
        
        let p2p_service_data = web::Data::new(p2p_service.clone());

    // --- Start P2P Accept Loop ---
    let accept_service = p2p_service.clone();

    let shard_storage_path = p2p_data_path.join("shards");
    if !shard_storage_path.exists() {
        std::fs::create_dir_all(&shard_storage_path).ok();
    }
    let shard_storage = np2p::storage::DiskStorage::new(shard_storage_path).await
        .expect("Failed to initialize shard storage");

    tokio::spawn(async move {
        info!("P2P Accept Loop started");
        loop {
            if let Some(incoming) = accept_service.node().accept().await {
                let storage = shard_storage.clone();
                let identity = accept_service.identity().clone();

                tokio::spawn(async move {
                    match incoming.await {
                        Ok(conn) => {
                            let handler = np2p::network::ConnectionHandler::new(conn, storage, identity);
                            handler.run().await;
                        }
                        Err(e) => error!("Incoming P2P connection failed: {}", e),
                    }
                });
            }
        }
    });

    let registry = prometheus::default_registry().clone();

    #[cfg(target_os = "linux")]
    {
        use prometheus::process_collector::ProcessCollector;
        let pc = ProcessCollector::for_self();
        let _ = registry.register(Box::new(pc));
    }

    let prom_metrics = PrometheusMetricsBuilder::new("api")
        .registry(registry)
        .build()
        .unwrap();

    let shared_system = web::Data::new(services::system_stats::start_system_monitor());
    let trusted_proxies: Vec<std::net::IpAddr> = config
        .rate_limit_trusted_proxies
        .iter()
        .filter_map(|s| match s.trim().parse::<std::net::IpAddr>() {
            Ok(ip) => Some(ip),
            Err(_) => {
                error!("Ignoring invalid rate_limit_trusted_proxies entry: {:?}", s);
                None
            }
        })
        .collect();
    let rate_limiter = rate_limit::RateLimiter::with_trusted_proxies(trusted_proxies);

    let _server_config = config.clone();
    let cors_allowed_origins = config.cors_allowed_origins.clone();
    HttpServer::new(move || {
        // CORS policy: browsers send Origin on same-origin POSTs too, so a
        // blanket rejection breaks the SPA. Instead, allow an Origin when its
        // authority matches the request's Host (same-origin through any
        // reverse proxy), plus whatever extra origins are explicitly
        // configured. Anything else is refused — no wildcard reflection for a
        // cookie-authenticated API.
        let configured = cors_allowed_origins.clone();
        // Split "host[:port]" (IPv6-safe) into lowercase host + port.
        fn split_authority(auth: &str) -> (String, Option<String>) {
            let auth = auth.trim();
            let (host, port) = match auth.rsplit_once(':') {
                Some((h, p)) if !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()) => {
                    (h, Some(p.to_string()))
                }
                _ => (auth, None),
            };
            (host.trim_matches(['[', ']']).to_ascii_lowercase(), port)
        }
        let cors = actix_cors::Cors::default()
            .allowed_origin_fn(move |origin: &actix_web::http::header::HeaderValue, head: &actix_web::dev::RequestHead| {
                let origin_str = origin.to_str().unwrap_or("");
                let Some((_, authority)) = origin_str.split_once("://") else {
                    return false;
                };
                let origin_authority = authority.split('/').next().unwrap_or("");
                let (o_host, o_port) = split_authority(origin_authority);
                let Some(host_hdr) = head
                    .headers
                    .get(actix_web::http::header::HOST)
                    .and_then(|h| h.to_str().ok())
                else {
                    return false;
                };
                let (h_host, h_port) = split_authority(host_hdr);
                // Reverse proxies often strip the port from Host ($host),
                // while the browser keeps it in Origin — so a missing port on
                // either side matches any port on the other. Same hostname is
                // still required (SameSite-equivalent CSRF posture).
                if o_host == h_host
                    && (o_port == h_port || o_port.is_none() || h_port.is_none())
                {
                    return true;
                }
                configured.iter().any(|c| c == origin_str)
            })
            .allowed_methods(vec!["GET", "POST", "PUT", "DELETE", "OPTIONS"])
            .allowed_headers(vec!["Authorization", "Content-Type", "X-Requested-With"])
            .supports_credentials()
            .max_age(3600);
        if !cors_allowed_origins.is_empty() {
            info!("CORS: extra allowed origins: {:?}", cors_allowed_origins);
        } else {
            info!("CORS: same-origin (via Host match) only");
        }

        App::new()
            .wrap(TracingLogger::<CustomRootSpanBuilder>::new())
            .wrap(prom_metrics.clone())
            .wrap(cors)
            .wrap(rate_limiter.clone())
            .app_data(web::Data::new(main_pool.clone()))
            .app_data(web::Data::new(geo_pool.clone()))
            .app_data(config_data.clone())
            .app_data(p2p_service_data.clone())
            .app_data(import_job_store.clone())
            .app_data(shared_system.clone())
            .app_data(duplicate_status.clone())
            .configure(configure_swagger)
            .route("/metrics", web::get().to(metrics_handler))
            .service(ping)
            .service(health_check)
            .service(
                web::scope("/api")
                    .service(register_user)
                    .service(user_login)
                    .service(user_login_form)
                    .service(user_logout)
                    .service(get_me)
                    .service(setup_status)
                    .service(setup_admin)
                    .service(list_users)
                    .service(create_user)
                    .service(update_user)
                    .service(delete_user)
                    .service(check_image_exists)
                    .service(check_video_exists)
                    .service(upload_image)
                    .service(upload_video)
                    .service(upload_image_metadata)
                    .service(upload_video_metadata)
                    .service(batch_upload_image)
                    .service(check_images_exist_batch)
                    .service(check_videos_exist_batch)
                    .service(batch_check_images)
                    .service(batch_check_videos)
                    .service(list_image_thumbnails)
                    .service(list_video_thumbnails)
                    .service(list_all_media_thumbnails)
                    .service(get_thumbnail)
                    .service(services::thumbnail::get_face_thumbnail)
                    .service(get_random_image)
                    .service(get_image)
                    .service(get_video)
                    .service(get_image_metadata)
                    .service(toggle_image_star)
                    .service(toggle_video_star)
                    .service(delete_image)
                    .service(delete_video)
                    .service(restore_image)
                    .service(restore_video)
                    .service(get_trash)
                    .service(search_images)
                    .service(search_video_keyframes)
                    .service(get_stats)
                    .service(get_pool_stats)
                    .service(get_geodb_stats)
                    .service(get_device_ids)
                    .service(search_places)
                    .service(services::map::get_map_points)
                    .service(get_ai_settings)
                    .service(update_ai_settings)
                    .service(services::person::get_persons)
                    .service(services::person::get_person)
                    .service(services::person::get_person_images)
                    .service(services::person::update_person_name)
                    .service(services::person::set_representative_face)
                    .service(services::person::merge_persons)
                    .service(services::system_stats::get_system_stats)
                    .service(services::system_stats::get_p2p_daemon_status)
                    .service(services::label::get_labels)
                    .service(services::label::create_label)
                    .service(services::label::delete_label)
                    .service(services::label::get_image_labels)
                    .service(services::label::add_image_label)
                    .service(services::label::remove_image_label)
                    .service(services::label::get_video_labels)
                    .service(services::label::add_video_label)
                    .service(services::label::remove_video_label)
                    .service(import_directory)
                    .service(get_import_status)
                    .service(services::p2p_restore::restore_p2p_file)
                    .service(services::p2p_status::get_p2p_backup_status)
                    .service(services::p2p_status::verify_p2p_backup)
                    .service(services::p2p_status::list_p2p_backups)
                    .service(services::p2p_status::list_backup_timestamps)
                    .service(services::p2p_status::get_p2p_connection_info)
                    .service(services::p2p_status::get_discovered_peers)
                    .service(services::p2p_status::get_invite_status)
                    .service(services::p2p_status::remove_p2p_node)
                    .service(services::p2p_status::trigger_rebalance)
                    .service(services::duplicates::get_duplicates)
                    .service(services::duplicates::get_duplicate_status)
                    .service(services::duplicates::trigger_duplicate_scan)
                    .service(enhance_image)
                    .service(save_enhanced_image)
                    .service(services::observability::get_admin_logs)
                    .service(services::observability::get_admin_errors)
                    .service(services::observability::get_admin_alerts)
                    .service(services::observability::get_admin_gpu)
                    .service(services::observability::get_admin_ai_models)
                    .service(services::observability::get_admin_pipeline)
                    .service(services::observability::get_admin_series)
            )
    })
    .bind(format!("0.0.0.0:{}", config.port))?
    .run().await
}
