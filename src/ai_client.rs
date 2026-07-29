pub mod proto {
    tonic::include_proto!("ai_service");
}

use proto::ai_service_client::AiServiceClient;
use proto::*;
use tonic::transport::Channel;
use tonic::Request;

#[derive(Clone)]
pub struct AiClient {
    grpc_url: String,
    api_key: String,
}

impl AiClient {
    pub fn new(grpc_url: String, api_key: String) -> Self {
        Self { grpc_url, api_key }
    }

    async fn connect(&self) -> Result<AiServiceClient<Channel>, tonic::transport::Error> {
        let channel = Channel::from_shared(self.grpc_url.clone())
            .expect("Invalid gRPC URL")
            .connect()
            .await?;
        Ok(AiServiceClient::new(channel))
    }

    fn make_request<T>(&self, message: T) -> Request<T> {
        let mut req = Request::new(message);
        if let Ok(key_val) = self.api_key.parse() {
            req.metadata_mut().insert("x-api-key", key_val);
        }
        req
    }

    pub async fn embed_image(&self, image_data: Vec<u8>) -> Result<Vec<f32>, String> {
        let mut client = self.connect().await.map_err(|e| format!("gRPC connection failed: {}", e))?;
        let request = self.make_request(EmbedImageRequest { image_data });

        let response = client.embed_image(request).await.map_err(|e| format!("EmbedImage gRPC call failed: {}", e))?;
        Ok(response.into_inner().embedding)
    }

    pub async fn embed_text(&self, text: String) -> Result<Vec<f32>, String> {
        let mut client = self.connect().await.map_err(|e| format!("gRPC connection failed: {}", e))?;
        let request = self.make_request(EmbedTextRequest { text });

        let response = client.embed_text(request).await.map_err(|e| format!("EmbedText gRPC call failed: {}", e))?;
        Ok(response.into_inner().embedding)
    }

    pub async fn describe_image(&self, image_data: Vec<u8>, use_qwen: bool) -> Result<(String, String), String> {
        let mut client = self.connect().await.map_err(|e| format!("gRPC connection failed: {}", e))?;
        let request = self.make_request(DescribeImageRequest { image_data, use_qwen });

        let response = client.describe_image(request).await.map_err(|e| format!("DescribeImage gRPC call failed: {}", e))?;
        let res = response.into_inner();
        Ok((res.description, res.model_used))
    }

    pub async fn detect_faces(&self, image_data: Vec<u8>) -> Result<Vec<FaceBoundingBox>, String> {
        let mut client = self.connect().await.map_err(|e| format!("gRPC connection failed: {}", e))?;
        let request = self.make_request(DetectFacesRequest { image_data });

        let response = client.detect_faces(request).await.map_err(|e| format!("DetectFaces gRPC call failed: {}", e))?;
        Ok(response.into_inner().faces)
    }

    pub async fn health_check(&self) -> Result<HealthCheckResponse, String> {
        let mut client = self.connect().await.map_err(|e| format!("gRPC connection failed: {}", e))?;
        let request = self.make_request(HealthCheckRequest {});

        let response = client.health_check(request).await.map_err(|e| format!("HealthCheck gRPC call failed: {}", e))?;
        Ok(response.into_inner())
    }
}
