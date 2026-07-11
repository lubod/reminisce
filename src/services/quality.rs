use base64::Engine as _;
use serde::Deserialize;

use crate::config::Config;

pub struct QualityScore {
    pub aesthetic_score: f32,
    pub sharpness_score: f32,
    pub width: i32,
    pub height: i32,
}

#[derive(Deserialize)]
struct QualityResponse {
    aesthetic_score: f32,
    sharpness_score: f32,
    width: i32,
    height: i32,
}

/// Quality scoring client. This is a fully functional component that queries
/// the AI service's `/quality` endpoint to compute aesthetic and sharpness scores.

pub async fn get_quality_score(image_data: &[u8], config: &Config) -> Result<QualityScore, String> {
    let client = crate::utils::get_http_client();

    let base64_image = base64::engine::general_purpose::STANDARD.encode(image_data);
    let url = format!("{}/quality", config.embedding_service_url);

    let response = client
        .post(&url)
        .bearer_auth(config.get_api_key().unwrap())
        .json(&serde_json::json!({"image": base64_image}))
        .send()
        .await
        .map_err(|e| format!("Failed to send request to AI service: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("AI service returned {} - {}", status, body));
    }

    let resp: QualityResponse = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse quality response: {}", e))?;

    Ok(QualityScore {
        aesthetic_score: resp.aesthetic_score,
        sharpness_score: resp.sharpness_score,
        width: resp.width,
        height: resp.height,
    })
}
