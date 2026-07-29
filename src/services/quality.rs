use crate::config::Config;

pub struct QualityScore {
    pub aesthetic_score: f32,
    pub sharpness_score: f32,
    pub width: i32,
    pub height: i32,
}

/// Quality scoring client. This is a fully functional component that queries
/// the AI service's `QualityScore` gRPC method to compute aesthetic and sharpness scores.
pub async fn get_quality_score(image_data: &[u8], config: &Config) -> Result<QualityScore, String> {
    let api_key = config.get_api_key().unwrap_or("").to_string();
    let client = crate::ai_client::AiClient::new(config.ai_grpc_url.clone(), api_key);
    let resp = client.quality_score(image_data.to_vec()).await?;

    Ok(QualityScore {
        aesthetic_score: resp.aesthetic_score,
        sharpness_score: resp.sharpness_score,
        width: resp.width,
        height: resp.height,
    })
}
