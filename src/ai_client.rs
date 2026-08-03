pub mod proto {
    tonic::include_proto!("ai_service");
}

use proto::ai_service_client::AiServiceClient;
use proto::*;
use tonic::transport::Channel;
use tonic::Request;

const MAX_MESSAGE_SIZE: usize = 64 * 1024 * 1024;

/// Result of an AI orientation detection call.
#[derive(Debug, Clone)]
pub struct DetectedOrientation {
    /// EXIF orientation value (1 = normal, 3 = 180°, 6 = 90° CW, 8 = 90° CCW).
    /// 0 means the service could not determine a rotation.
    pub orientation_value: i32,
    /// Raw classifier label, one of "0", "90", "180", "270".
    pub label: String,
    /// Softmax confidence of the predicted label, 0.0..1.0.
    pub confidence: f32,
}

/// True if a gRPC error string (from `tonic::Status`) represents a permanent, non-retryable
/// failure. The AI server returns `INVALID_ARGUMENT` for malformed/oversized inputs and
/// `UNAUTHENTICATED`/`PERMISSION_DENIED` for auth problems — these will never succeed on
/// retry. Transport-level codes (Unavailable, DeadlineExceeded, …) are transient and are
/// deliberately NOT treated as permanent so workers retry them.
pub fn is_permanent_failure(err: &str) -> bool {
    const PERMANENT_CODES: [&str; 6] = [
        "InvalidArgument",
        "PermissionDenied",
        "Unauthenticated",
        "Unimplemented",
        "FailedPrecondition",
        "OutOfRange",
    ];
    PERMANENT_CODES.iter().any(|code| err.contains(code))
}

static SHARED: std::sync::OnceLock<std::sync::Arc<AiClient>> = std::sync::OnceLock::new();

#[derive(Clone)]
pub struct AiClient {
    grpc_url: String,
    api_key: String,
}

impl AiClient {
    pub fn new(grpc_url: String, api_key: String) -> Self {
        if api_key.is_empty() {
            log::warn!("AiClient created with an empty API key — gRPC calls will be rejected if the AI service requires authentication");
        }
        Self {
            grpc_url,
            api_key,
        }
    }

    /// Process-wide shared `AiClient`, so the (state-less) client plus connection
    /// settings are reused across RPCs. Each RPC builds its own tonic `Channel` to the
    /// configured gRPC URL; the transport connect-or-fail behaviour is exercised by
    /// `channel`/`client` below and by the ai_speed integration test.
    pub fn shared(config: &crate::config::Config) -> std::sync::Arc<AiClient> {
        SHARED
            .get_or_init(|| {
                let api_key = config.get_api_key().unwrap_or("").to_string();
                std::sync::Arc::new(AiClient::new(config.ai_grpc_url.clone(), api_key))
            })
            .clone()
    }

    async fn channel(&self) -> Result<Channel, String> {
        let endpoint = Channel::from_shared(self.grpc_url.clone())
            .map_err(|e| format!("invalid gRPC URL '{}': {}", self.grpc_url, e))?;
        let channel = endpoint.connect().await.map_err(|e| format!("gRPC connect failed: {}", e))?;
        Ok(channel)
    }

    async fn client(&self) -> Result<AiServiceClient<Channel>, String> {
        let channel = self.channel().await?;
        Ok(AiServiceClient::new(channel)
            .max_decoding_message_size(MAX_MESSAGE_SIZE)
            .max_encoding_message_size(MAX_MESSAGE_SIZE))
    }

    fn make_request<T>(&self, message: T) -> Request<T> {
        let mut req = Request::new(message);
        if let Ok(key_val) = self.api_key.parse() {
            req.metadata_mut().insert("x-api-key", key_val);
        }
        req
    }

    pub async fn embed_image(&self, image_data: Vec<u8>) -> Result<Vec<f32>, String> {
        let mut client = self.client().await?;
        let request = self.make_request(EmbedImageRequest { image_data });

        let response = client.embed_image(request).await.map_err(|e| format!("EmbedImage gRPC call failed: {}", e))?;
        Ok(response.into_inner().embedding)
    }

    pub async fn embed_text(&self, text: String) -> Result<Vec<f32>, String> {
        let mut client = self.client().await?;
        let request = self.make_request(EmbedTextRequest { text });

        let response = client.embed_text(request).await.map_err(|e| format!("EmbedText gRPC call failed: {}", e))?;
        Ok(response.into_inner().embedding)
    }

    pub async fn describe_image(&self, image_data: Vec<u8>, use_qwen: bool) -> Result<(String, String), String> {
        let mut client = self.client().await?;
        let request = self.make_request(DescribeImageRequest { image_data, use_qwen });

        let response = client.describe_image(request).await.map_err(|e| format!("DescribeImage gRPC call failed: {}", e))?;
        let res = response.into_inner();
        Ok((res.description, res.model_used))
    }

    pub async fn detect_faces(&self, image_data: Vec<u8>) -> Result<Vec<FaceBoundingBox>, String> {
        let mut client = self.client().await?;
        let request = self.make_request(DetectFacesRequest { image_data });

        let response = client.detect_faces(request).await.map_err(|e| format!("DetectFaces gRPC call failed: {}", e))?;
        Ok(response.into_inner().faces)
    }

    /// Ask the AI service to detect a photo's rotation using its orientation
    /// classifier, returning the equivalent EXIF orientation value (1/3/6/8),
    /// the model's raw label, and the softmax confidence. The service sets
    /// `orientation_value` to 0 when it cannot determine a rotation.
    pub async fn detect_orientation(&self, image_data: Vec<u8>) -> Result<DetectedOrientation, String> {
        let mut client = self.client().await?;
        let request = self.make_request(DetectOrientationRequest { image_data });

        let response = client.detect_orientation(request).await.map_err(|e| format!("DetectOrientation gRPC call failed: {}", e))?;
        let res = response.into_inner();
        Ok(DetectedOrientation {
            orientation_value: res.orientation_value,
            label: res.label,
            confidence: res.confidence,
        })
    }

    pub async fn quality_score(&self, image_data: Vec<u8>) -> Result<QualityScoreResponse, String> {
        let mut client = self.client().await?;
        let request = self.make_request(QualityScoreRequest { image_data });

        let response = client.quality_score(request).await.map_err(|e| format!("QualityScore gRPC call failed: {}", e))?;
        Ok(response.into_inner())
    }

    pub async fn enhance_image(&self, image_data: Vec<u8>, mode: String) -> Result<EnhanceImageResponse, String> {
        let mut client = self.client().await?;
        let request = self.make_request(EnhanceImageRequest { image_data, mode });

        let response = client.enhance_image(request).await.map_err(|e| format!("EnhanceImage gRPC call failed: {}", e))?;
        Ok(response.into_inner())
    }

    pub async fn health_check(&self) -> Result<HealthCheckResponse, String> {
        let mut client = match self.client().await {
            Ok(c) => c,
            Err(_) => {
                tokio::time::sleep(std::time::Duration::from_millis(150)).await;
                self.client().await?
            }
        };
        let request = self.make_request(HealthCheckRequest {});

        match client.health_check(request).await {
            Ok(resp) => Ok(resp.into_inner()),
            Err(e) => {
                let err_str = e.to_string();
                if !is_permanent_failure(&err_str) {
                    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                    if let Ok(mut c) = self.client().await {
                        let request = self.make_request(HealthCheckRequest {});
                        if let Ok(resp) = c.health_check(request).await {
                            return Ok(resp.into_inner());
                        }
                    }
                }
                Err(format!("HealthCheck gRPC call failed: {}", err_str))
            }
        }
    }
}
