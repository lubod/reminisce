use utoipa::{ OpenApi };
use utoipa_swagger_ui::SwaggerUi;
use std::sync::Arc;


pub mod config;
pub mod constants;
pub mod db;
pub mod db_instrumentation;
pub mod query_builder;
pub mod utils;
pub mod auth_utils;
pub mod verification_worker;
pub mod p2p_audit_worker;
pub mod media_replication_worker;
pub mod shard_rebalance_worker;
pub mod ai_worker;
pub mod telemetry;
pub mod metrics;

pub mod test_utils;
pub mod metrics_collector;

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
    pub mod ai_settings;
    pub mod face_detection;
    pub mod person;
    pub mod system_stats;
    pub mod label;
    pub mod ingest;
    pub mod p2p_ingest;
    pub mod import_dir;
    pub mod p2p_status;
    pub mod proxy_manager;
}

use crate::config::Config;
use actix_web::{ App, HttpServer, web, HttpResponse };
use prometheus::{Encoder, TextEncoder};

use tracing::{error, info, Span};
use tracing_actix_web::{TracingLogger, RootSpanBuilder};
use actix_web_prom::PrometheusMetricsBuilder;
use actix_web::dev::ServiceRequest;
use actix_web::http::header;
use tracing::field::Empty;

async fn metrics_handler() -> HttpResponse {
    let encoder = TextEncoder::new();
    let mut buffer = vec![];
    let metric_families = prometheus::gather();
    match encoder.encode(&metric_families, &mut buffer) {
        Ok(_) => {
            HttpResponse::Ok()
                .content_type(encoder.format_type())
                .body(buffer)
        }
        Err(e) => {
            error!("Could not encode metrics: {}", e);
            HttpResponse::InternalServerError().finish()
        }
    }
}

pub use crate::services::auth::{register_user, user_login, Claims};
pub use crate::services::health::{ping, health_check, HealthCheckResponse};
pub use crate::services::existence_check::{check_image_exists, check_video_exists};
pub use crate::services::upload::{upload_image, upload_video, upload_image_metadata, upload_video_metadata, batch_upload_image, check_images_exist_batch, check_videos_exist_batch, batch_check_images, batch_check_videos};
pub use crate::services::thumbnail::{list_image_thumbnails, list_video_thumbnails, list_all_media_thumbnails, get_thumbnail, get_face_thumbnail};
pub use crate::services::media::{get_image, get_video, get_image_metadata, toggle_image_star, toggle_video_star, delete_image, delete_video, get_device_ids, get_random_image};
pub use crate::services::embedding::search_images;
pub use crate::services::stats::get_stats;
pub use crate::services::pool_stats::get_pool_stats;
pub use crate::services::geodb_stats::get_geodb_stats;
pub use crate::services::geocoding::search_places;
pub use crate::services::ai_settings::{get_ai_settings, update_ai_settings};
pub use crate::services::import_dir::import_directory;

#[derive(OpenApi)]
#[openapi(
    paths(
        crate::services::auth::register_user,
        crate::services::auth::user_login,
        crate::services::health::ping,
        crate::services::health::health_check,
        crate::services::existence_check::check_image_exists,
        crate::services::existence_check::check_video_exists,
        crate::services::upload::upload_image,
        crate::services::upload::upload_video,
        crate::services::upload::upload_image_metadata,
        crate::services::upload::upload_video_metadata,
        crate::services::upload::batch_upload_image,
        crate::services::upload::check_images_exist_batch,
        crate::services::upload::check_videos_exist_batch,
        crate::services::upload::batch_check_images,
        crate::services::upload::batch_check_videos,
        crate::services::thumbnail::list_image_thumbnails,
        crate::services::thumbnail::list_video_thumbnails,
        crate::services::thumbnail::list_all_media_thumbnails,
        crate::services::thumbnail::get_thumbnail,
        crate::services::thumbnail::get_face_thumbnail,
        crate::services::media::get_image,
        crate::services::media::get_video,
        crate::services::media::get_image_metadata,
        crate::services::media::toggle_image_star,
        crate::services::media::toggle_video_star,
        crate::services::media::delete_image,
        crate::services::media::delete_video,
        crate::services::media::get_device_ids,
        crate::services::media::get_random_image,
        crate::services::embedding::search_images,
        crate::services::stats::get_stats,
        crate::services::pool_stats::get_pool_stats,
        crate::services::geodb_stats::get_geodb_stats,
        crate::services::geocoding::search_places,
        crate::services::ai_settings::get_ai_settings,
        crate::services::ai_settings::update_ai_settings,
        crate::services::person::get_persons,
        crate::services::person::get_person,
        crate::services::person::get_person_images,
        crate::services::person::update_person_name,
        crate::services::person::merge_persons,
        crate::services::system_stats::get_system_stats,
        crate::services::system_stats::get_p2p_daemon_status,
        crate::services::label::get_labels,
        crate::services::label::create_label,
        crate::services::label::delete_label,
        crate::services::label::get_image_labels,
        crate::services::label::add_image_label,
        crate::services::label::remove_image_label,
        crate::services::import_dir::import_directory,
        crate::services::p2p_status::get_p2p_backup_status,
        crate::services::p2p_status::verify_p2p_backup,
        crate::services::p2p_status::list_p2p_backups,
        crate::services::p2p_status::list_backup_timestamps,
        crate::services::p2p_status::get_p2p_connection_info,
        crate::services::p2p_status::get_discovered_peers,
        crate::services::p2p_status::get_invite_status
    ),
    components(
        schemas(
            crate::services::existence_check::ImageCheckQuery,
            crate::services::existence_check::ExistenceResponse,
            crate::services::existence_check::VideoCheckQuery,
            crate::services::thumbnail::PaginationQuery,
            crate::services::thumbnail::ThumbnailItem,
            crate::services::thumbnail::ThumbnailsResponse,
            crate::services::upload::UploadImageRequest,
            crate::services::upload::UploadVideoRequest,
            crate::services::upload::UploadImageMetadataRequest,
            crate::services::upload::UploadImageMetadataResponse,
            crate::services::upload::UploadVideoMetadataRequest,
            crate::services::upload::UploadVideoMetadataResponse,
            crate::services::upload::CheckImagesExistRequest,
            crate::services::upload::CheckImagesExistResponse,
            crate::services::upload::CheckVideosExistRequest,
            crate::services::upload::CheckVideosExistResponse,
            crate::services::media::ImageMetadata,
            crate::services::media::StarResponse,
            crate::services::media::DeviceIdsResponse,
            crate::services::media::RandomImageResponse,
            crate::services::embedding::SearchResult,
            crate::services::auth::RegisterRequest,
            crate::services::auth::UserLoginRequest,
            crate::services::health::HealthCheckResponse,
            crate::services::system_stats::SystemStatsResponse,
            crate::services::system_stats::P2PDaemonStatus,
            crate::services::p2p_status::P2PBackupStatusResponse,
            crate::services::p2p_status::ConnectionInfoResponse,
            crate::services::p2p_status::BackupListResponse,
            crate::services::p2p_status::BackupEntry,
            crate::services::p2p_status::BackupTimestampsResponse,
            crate::services::p2p_status::DiscoveredPeersResponse,
            crate::services::p2p_status::DiscoveredPeer,
            crate::services::p2p_status::InviteStatusResponse,
            crate::services::p2p_status::MembershipInfo,
            crate::services::p2p_status::VerificationResult,
            crate::services::p2p_status::FileVerifyResult,
            crate::services::stats::StatsResponse,
            crate::services::pool_stats::PoolStatsResponse,
            crate::services::pool_stats::PoolMetrics,
            crate::services::geodb_stats::GeoDbStatsResponse,
            crate::services::geocoding::GeocodeResult,
            crate::services::geocoding::PlaceSearchQuery,
            crate::services::ai_settings::AiSettingsResponse,
            crate::services::ai_settings::UpdateAiSettingsRequest,
            crate::services::person::Person,
            crate::services::person::PersonsResponse,
            crate::services::person::PersonResponse,
            crate::services::person::PersonImage,
            crate::services::person::PersonImagesResponse,
            crate::services::person::UpdatePersonNameRequest,
            crate::services::person::MergePersonsRequest,
            crate::services::system_stats::SystemStatsResponse,
            crate::services::label::Label,
            crate::services::label::LabelsResponse,
            crate::services::label::CreateLabelRequest,
            crate::services::label::AddLabelToMediaRequest,
            crate::services::label::MediaLabelsResponse,
            crate::services::import_dir::ImportDirectoryRequest,
            crate::services::import_dir::ImportDirectoryResponse,
            Claims
        )
    ),
    tags((name = "reminisce", description = "Reminisce: Self-hosted photo and video memory vault."))
)]
struct ApiDoc;

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
        let target = request.uri().to_string();
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

    if config.database_url.is_none() {
        error!("❌ Headless P2P storage node mode has been removed!");
        return Err(std::io::Error::new(
            std::io::ErrorKind::Other,
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

    let pool = db::create_pool_with_options(&database_url, pool_options.clone())
        .expect("Failed to create database pool");
    let geotagging_pool = db::create_pool_with_options(&config.geotagging_database_url, pool_options)
        .expect("Failed to create geotagging database pool");

    let main_pool = db::MainDbPool(pool.clone());
    let geo_pool = db::GeotaggingDbPool(geotagging_pool.clone());

    let config_data = web::Data::new(config.clone());

    // --- P2P Identity & Service ---
    let p2p_data_path = std::path::Path::new(&config.p2p_data_dir);
    if !p2p_data_path.exists() {
        std::fs::create_dir_all(p2p_data_path).expect("Failed to create P2P data directory");
    }

    let identity_path = p2p_data_path.join("node.key");
    let identity = if identity_path.exists() {
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

        let p2p_service = Arc::new(np2p::network::P2PService::new(
            "0.0.0.0:0".parse().unwrap(),
            identity
        ).await.expect("Failed to initialize P2P service"));
    
        if let Err(e) = services::ai_settings::load_ai_settings_from_db(&pool, &config).await {
            error!("Failed to load AI settings from database: {}", e);
        }
            tokio::spawn(
            verification_worker::start_verification_worker(
                web::Data::new(main_pool.clone()),
                config_data.clone()
            )
        );
    
        tokio::spawn(
            crate::ai_worker::start_ai_worker(
                web::Data::new(main_pool.clone()),
                config_data.clone()
            )
        );
    
        tokio::spawn(
            metrics_collector::start_metrics_collector(
                web::Data::new(main_pool.clone()),
                web::Data::new(geo_pool.clone()),
                config_data.clone()
            )
        );
    
        if config.database_url.is_some() {
            let replication_pool = main_pool.0.clone();
            let replication_config = config.clone();
            let replication_service = p2p_service.clone();
            tokio::spawn(async move {
                media_replication_worker::media_replication_loop(
                    replication_pool,
                    replication_config,
                    replication_service
                ).await;
            });
        }
    
        // Auto-register configured P2P peers at startup
        if !config.p2p_peers.is_empty() {
            if let Err(e) = crate::shard_rebalance_worker::ensure_peers_registered(&main_pool.0, &config).await {
                error!("Failed to register P2P peers at startup: {}", e);
            }
        }

            tokio::spawn(
                crate::p2p_audit_worker::start_audit_worker(
                    main_pool.0.clone(),
                    config.clone(),
                    p2p_service.clone()
                )
            );

            tokio::spawn(
                crate::shard_rebalance_worker::start_rebalance_worker(
                    main_pool.0.clone(),
                    config.clone(),
                    p2p_service.clone()
                )
            );
        
        let p2p_service_data = web::Data::new(p2p_service.clone());

    // --- Start P2P Accept Loop ---
    let accept_pool = main_pool.0.clone();
    let accept_config = Arc::new(config.clone());
    let accept_service = p2p_service.clone();
    let ingest_handler = Arc::new(services::p2p_ingest::IngestHandler::new(accept_pool, accept_config));
    
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
                let handler_impl = ingest_handler.clone();
                
                tokio::spawn(async move {
                    match incoming.await {
                        Ok(conn) => {
                            let handler = np2p::network::ConnectionHandler::new(conn, storage, identity)
                                .with_custom_handler(handler_impl);
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

    HttpServer::new(move || {
        let cors = actix_cors::Cors::permissive();

        App::new()
            .wrap(TracingLogger::<CustomRootSpanBuilder>::new())
            .wrap(prom_metrics.clone())
            .wrap(cors)
            .app_data(web::Data::new(main_pool.clone()))
            .app_data(web::Data::new(geo_pool.clone()))
            .app_data(config_data.clone())
            .app_data(p2p_service_data.clone())
            .service(
                SwaggerUi::new("/swagger-ui/{_:.*}").url("/api-doc/openapi.json", ApiDoc::openapi())
            )
            .route("/metrics", web::get().to(metrics_handler))
            .service(ping)
            .service(health_check)
            .service(
                web::scope("/api")
                    .service(register_user)
                    .service(user_login)
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
                    .service(search_images)
                    .service(get_stats)
                    .service(get_pool_stats)
                    .service(get_geodb_stats)
                    .service(get_device_ids)
                    .service(search_places)
                    .service(get_ai_settings)
                    .service(update_ai_settings)
                    .service(services::person::get_persons)
                    .service(services::person::get_person)
                    .service(services::person::get_person_images)
                    .service(services::person::update_person_name)
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
                    .service(services::p2p_status::get_p2p_backup_status)
                    .service(services::p2p_status::verify_p2p_backup)
                    .service(services::p2p_status::list_p2p_backups)
                    .service(services::p2p_status::list_backup_timestamps)
                    .service(services::p2p_status::get_p2p_connection_info)
                    .service(services::p2p_status::get_discovered_peers)
                    .service(services::p2p_status::get_invite_status)
            )
    })
    .bind(format!("0.0.0.0:{}", config.port))?
    .run().await
}
