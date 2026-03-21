/// AI Service Speed Tests
///
/// Tests all AI service endpoints against the running ai-server container
/// and reports latency for each operation. Requires the AI server to be
/// running at http://localhost:8081 (e.g. via docker-compose-dev.yml).
///
/// Run with: cargo test --test ai_speed_test -- --nocapture
use std::time::Instant;
use serial_test::serial;

const AI_SERVICE_URL: &str = "http://localhost:8081";

fn load_test_image() -> Vec<u8> {
    std::fs::read("tests/test_image.jpg").expect("Failed to read tests/test_image.jpg")
}

fn to_base64(data: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(data)
}

#[tokio::test]
#[serial]
async fn test_health_check() {
    let client = reqwest::Client::new();
    let start = Instant::now();
    let resp = client
        .get(format!("{}/health", AI_SERVICE_URL))
        .send()
        .await
        .expect("AI service not reachable — is the container running?");
    let elapsed = start.elapsed();

    assert!(resp.status().is_success(), "Health check failed: {}", resp.status());
    let body: serde_json::Value = resp.json().await.unwrap();

    let models = &body["models_loaded"];
    assert_eq!(models["siglip2"], true, "SigLIP2 not loaded");
    assert_eq!(models["smolvlm"], true, "SmolVLM not loaded");
    assert_eq!(models["qwen25_vl"], true, "Qwen2.5-VL not loaded");
    assert_eq!(models["insightface"], true, "InsightFace not loaded");

    println!("\n=== Health Check ===");
    println!("  Device:      {}", body["device"]);
    println!("  SigLIP2:     {}", models["siglip2"]);
    println!("  SmolVLM:     {}", models["smolvlm"]);
    println!("  Qwen2.5-VL:  {}", models["qwen25_vl"]);
    println!("  InsightFace: {}", models["insightface"]);
    println!("  Latency:    {:.1}ms", elapsed.as_secs_f64() * 1000.0);
}

#[tokio::test]
#[serial]
async fn test_image_embedding_speed() {
    let image_data = load_test_image();
    let base64_image = to_base64(&image_data);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .unwrap();

    // Warmup
    let _ = client
        .post(format!("{}/embed/image", AI_SERVICE_URL))
        .json(&serde_json::json!({"image": &base64_image}))
        .send()
        .await;

    // Timed runs
    let mut times = Vec::new();
    for _ in 0..5 {
        let start = Instant::now();
        let resp = client
            .post(format!("{}/embed/image", AI_SERVICE_URL))
            .json(&serde_json::json!({"image": &base64_image}))
            .send()
            .await
            .expect("Image embedding request failed");
        let elapsed = start.elapsed();

        assert!(resp.status().is_success(), "Image embedding failed: {}", resp.status());
        let body: serde_json::Value = resp.json().await.unwrap();
        assert_eq!(body["dimension"], 1152, "Wrong embedding dimension");

        times.push(elapsed.as_secs_f64() * 1000.0);
    }

    let avg = times.iter().sum::<f64>() / times.len() as f64;
    let min = times.iter().cloned().fold(f64::INFINITY, f64::min);
    let max = times.iter().cloned().fold(f64::NEG_INFINITY, f64::max);

    println!("\n=== Image Embedding (SigLIP2, {} KB image, 5 runs) ===", image_data.len() / 1024);
    println!("  Dimension: 1152");
    println!("  Min:       {:.1}ms", min);
    println!("  Avg:       {:.1}ms", avg);
    println!("  Max:       {:.1}ms", max);
    println!("  All runs:  {:?}", times.iter().map(|t| format!("{:.1}ms", t)).collect::<Vec<_>>());
}

#[tokio::test]
#[serial]
async fn test_text_embedding_speed() {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .unwrap();

    let queries = [
        "a dog playing in the snow",
        "sunset over the ocean with orange sky",
        "family photo at a birthday party",
        "mountain landscape with a lake",
        "close up portrait of a smiling woman",
    ];

    // Warmup
    let _ = client
        .post(format!("{}/embed/text", AI_SERVICE_URL))
        .json(&serde_json::json!({"text": "warmup"}))
        .send()
        .await;

    let mut times = Vec::new();
    for query in &queries {
        let start = Instant::now();
        let resp = client
            .post(format!("{}/embed/text", AI_SERVICE_URL))
            .json(&serde_json::json!({"text": query}))
            .send()
            .await
            .expect("Text embedding request failed");
        let elapsed = start.elapsed();

        assert!(resp.status().is_success(), "Text embedding failed: {}", resp.status());
        let body: serde_json::Value = resp.json().await.unwrap();
        assert_eq!(body["dimension"], 1152, "Wrong embedding dimension");

        times.push(elapsed.as_secs_f64() * 1000.0);
    }

    let avg = times.iter().sum::<f64>() / times.len() as f64;
    let min = times.iter().cloned().fold(f64::INFINITY, f64::min);
    let max = times.iter().cloned().fold(f64::NEG_INFINITY, f64::max);

    println!("\n=== Text Embedding (SigLIP2, {} queries) ===", queries.len());
    println!("  Dimension: 1152");
    println!("  Min:       {:.1}ms", min);
    println!("  Avg:       {:.1}ms", avg);
    println!("  Max:       {:.1}ms", max);
    for (i, query) in queries.iter().enumerate() {
        println!("  [{}] {:.1}ms  \"{}\"", i + 1, times[i], query);
    }
}

#[tokio::test]
#[serial]
async fn test_image_description_speed() {
    let image_data = load_test_image();
    let base64_image = to_base64(&image_data);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .unwrap();

    // Warmup
    let _ = client
        .post(format!("{}/describe/qwen", AI_SERVICE_URL))
        .json(&serde_json::json!({"image": &base64_image}))
        .send()
        .await;

    // Timed runs
    let mut times = Vec::new();
    let mut descriptions = Vec::new();
    for _ in 0..3 {
        let start = Instant::now();
        let resp = client
            .post(format!("{}/describe/qwen", AI_SERVICE_URL))
            .json(&serde_json::json!({"image": &base64_image}))
            .send()
            .await
            .expect("Description request failed");
        let elapsed = start.elapsed();

        assert!(resp.status().is_success(), "Description failed: {}", resp.status());
        let body: serde_json::Value = resp.json().await.unwrap();
        let desc = body["description"].as_str().unwrap_or("").to_string();
        assert!(!desc.is_empty(), "Empty description returned");

        descriptions.push(desc);
        times.push(elapsed.as_secs_f64() * 1000.0);
    }

    let avg = times.iter().sum::<f64>() / times.len() as f64;
    let min = times.iter().cloned().fold(f64::INFINITY, f64::min);
    let max = times.iter().cloned().fold(f64::NEG_INFINITY, f64::max);

    println!("\n=== Image Description (Qwen2.5-VL-3B /describe/qwen, {} KB image, 3 runs) ===", image_data.len() / 1024);
    println!("  Min:         {:.1}ms", min);
    println!("  Avg:         {:.1}ms", avg);
    println!("  Max:         {:.1}ms", max);
    println!("  Description: \"{}\"", descriptions[0]);
    println!("  All runs:    {:?}", times.iter().map(|t| format!("{:.1}ms", t)).collect::<Vec<_>>());
}

#[tokio::test]
#[serial]
async fn test_face_detection_speed() {
    let image_data = load_test_image();
    let base64_image = to_base64(&image_data);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .unwrap();

    // Warmup
    let _ = client
        .post(format!("{}/detect", AI_SERVICE_URL))
        .json(&serde_json::json!({"image": &base64_image}))
        .send()
        .await;

    // Timed runs
    let mut times = Vec::new();
    let mut face_counts = Vec::new();
    for _ in 0..5 {
        let start = Instant::now();
        let resp = client
            .post(format!("{}/detect", AI_SERVICE_URL))
            .json(&serde_json::json!({"image": &base64_image}))
            .send()
            .await
            .expect("Face detection request failed");
        let elapsed = start.elapsed();

        assert!(resp.status().is_success(), "Face detection failed: {}", resp.status());
        let body: serde_json::Value = resp.json().await.unwrap();
        assert_eq!(body["status"], "success");

        let count = body["count"].as_i64().unwrap_or(0);
        face_counts.push(count);

        if let Some(faces) = body["faces"].as_array() {
            for face in faces {
                assert!(face["embedding"].as_array().map_or(false, |e| e.len() == 512),
                    "Face embedding should be 512-dim");
                assert!(face["confidence"].as_f64().unwrap_or(0.0) > 0.0,
                    "Face confidence should be > 0");
            }
        }

        times.push(elapsed.as_secs_f64() * 1000.0);
    }

    let avg = times.iter().sum::<f64>() / times.len() as f64;
    let min = times.iter().cloned().fold(f64::INFINITY, f64::min);
    let max = times.iter().cloned().fold(f64::NEG_INFINITY, f64::max);

    println!("\n=== Face Detection (InsightFace buffalo_l, {} KB image, 5 runs) ===", image_data.len() / 1024);
    println!("  Faces found: {}", face_counts[0]);
    println!("  Min:         {:.1}ms", min);
    println!("  Avg:         {:.1}ms", avg);
    println!("  Max:         {:.1}ms", max);
    println!("  All runs:    {:?}", times.iter().map(|t| format!("{:.1}ms", t)).collect::<Vec<_>>());
}

#[tokio::test]
#[serial]
async fn test_image_description_fast_speed() {
    let image_data = load_test_image();
    let base64_image = to_base64(&image_data);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .unwrap();

    // Warmup
    let _ = client
        .post(format!("{}/describe", AI_SERVICE_URL))
        .json(&serde_json::json!({"image": &base64_image}))
        .send()
        .await;

    // Timed runs
    let mut times = Vec::new();
    let mut descriptions = Vec::new();
    for _ in 0..3 {
        let start = Instant::now();
        let resp = client
            .post(format!("{}/describe", AI_SERVICE_URL))
            .json(&serde_json::json!({"image": &base64_image}))
            .send()
            .await
            .expect("Fast description request failed");
        let elapsed = start.elapsed();

        assert!(resp.status().is_success(), "Fast description failed: {}", resp.status());
        let body: serde_json::Value = resp.json().await.unwrap();
        let desc = body["description"].as_str().unwrap_or("").to_string();
        assert!(!desc.is_empty(), "Empty description returned");

        descriptions.push(desc);
        times.push(elapsed.as_secs_f64() * 1000.0);
    }

    let avg = times.iter().sum::<f64>() / times.len() as f64;
    let min = times.iter().cloned().fold(f64::INFINITY, f64::min);
    let max = times.iter().cloned().fold(f64::NEG_INFINITY, f64::max);

    println!("\n=== Image Description (SmolVLM-500M /describe, {} KB image, 3 runs) ===", image_data.len() / 1024);
    println!("  Min:         {:.1}ms", min);
    println!("  Avg:         {:.1}ms", avg);
    println!("  Max:         {:.1}ms", max);
    println!("  Description: \"{}\"", descriptions[0]);
    println!("  All runs:    {:?}", times.iter().map(|t| format!("{:.1}ms", t)).collect::<Vec<_>>());
}

#[tokio::test]
#[serial]
#[ignore] // run explicitly with: cargo test --test ai_speed_test -- --ignored test_all_services_summary
async fn test_all_services_summary() {
    let image_data = load_test_image();
    let base64_image = to_base64(&image_data);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .unwrap();

    // Health check first
    let resp = client.get(format!("{}/health", AI_SERVICE_URL)).send().await;
    if resp.is_err() || !resp.unwrap().status().is_success() {
        panic!("AI service not available at {}. Start it with: docker compose -p reminisce-dev -f docker-compose-dev.yml up -d ai-server", AI_SERVICE_URL);
    }

    println!("\n{}", "=".repeat(60));
    println!("  AI SERVICE SPEED BENCHMARK");
    println!("  Image: tests/test_image.jpg ({} KB)", image_data.len() / 1024);
    println!("{}", "=".repeat(60));

    // --- Warmup all endpoints ---
    let _ = client.post(format!("{}/embed/image", AI_SERVICE_URL))
        .json(&serde_json::json!({"image": &base64_image})).send().await;
    let _ = client.post(format!("{}/embed/text", AI_SERVICE_URL))
        .json(&serde_json::json!({"text": "warmup"})).send().await;
    let _ = client.post(format!("{}/describe", AI_SERVICE_URL))
        .json(&serde_json::json!({"image": &base64_image})).send().await;
    let _ = client.post(format!("{}/describe/qwen", AI_SERVICE_URL))
        .json(&serde_json::json!({"image": &base64_image})).send().await;
    let _ = client.post(format!("{}/detect", AI_SERVICE_URL))
        .json(&serde_json::json!({"image": &base64_image})).send().await;

    struct BenchResult {
        name: &'static str,
        times_ms: Vec<f64>,
        detail: String,
    }

    let mut results: Vec<BenchResult> = Vec::new();

    // 1. Image Embedding
    {
        let mut times = Vec::new();
        let mut dim = 0;
        for _ in 0..5 {
            let start = Instant::now();
            let resp = client.post(format!("{}/embed/image", AI_SERVICE_URL))
                .json(&serde_json::json!({"image": &base64_image}))
                .send().await.unwrap();
            times.push(start.elapsed().as_secs_f64() * 1000.0);
            let body: serde_json::Value = resp.json().await.unwrap();
            dim = body["dimension"].as_i64().unwrap_or(0);
        }
        results.push(BenchResult {
            name: "Image Embedding (SigLIP2)",
            times_ms: times,
            detail: format!("{}d vector", dim),
        });
    }

    // 2. Text Embedding
    {
        let mut times = Vec::new();
        let queries = ["sunset beach", "birthday party", "mountain lake", "portrait photo", "city skyline"];
        for q in &queries {
            let start = Instant::now();
            let resp = client.post(format!("{}/embed/text", AI_SERVICE_URL))
                .json(&serde_json::json!({"text": q}))
                .send().await.unwrap();
            times.push(start.elapsed().as_secs_f64() * 1000.0);
            let body: serde_json::Value = resp.json().await.unwrap();
            assert_eq!(body["dimension"], 1152);
        }
        results.push(BenchResult {
            name: "Text Embedding (SigLIP2)",
            times_ms: times,
            detail: "1152d vector".to_string(),
        });
    }

    // 3. Default Description (SmolVLM-500M)
    {
        let mut times = Vec::new();
        let mut desc = String::new();
        for _ in 0..3 {
            let start = Instant::now();
            let resp = client.post(format!("{}/describe", AI_SERVICE_URL))
                .json(&serde_json::json!({"image": &base64_image}))
                .send().await.unwrap();
            times.push(start.elapsed().as_secs_f64() * 1000.0);
            let body: serde_json::Value = resp.json().await.unwrap();
            if desc.is_empty() {
                desc = body["description"].as_str().unwrap_or("").to_string();
            }
        }
        results.push(BenchResult {
            name: "Description (SmolVLM-500M)",
            times_ms: times,
            detail: format!("\"{}\"", if desc.len() > 80 { &desc[..80] } else { &desc }),
        });
    }

    // 4. Quality Description (Qwen2.5-VL-3B)
    {
        let mut times = Vec::new();
        let mut desc = String::new();
        for _ in 0..3 {
            let start = Instant::now();
            let resp = client.post(format!("{}/describe/qwen", AI_SERVICE_URL))
                .json(&serde_json::json!({"image": &base64_image}))
                .send().await.unwrap();
            times.push(start.elapsed().as_secs_f64() * 1000.0);
            let body: serde_json::Value = resp.json().await.unwrap();
            if desc.is_empty() {
                desc = body["description"].as_str().unwrap_or("").to_string();
            }
        }
        results.push(BenchResult {
            name: "Description/Qwen (Qwen2.5-VL-3B)",
            times_ms: times,
            detail: format!("\"{}\"", if desc.len() > 80 { &desc[..80] } else { &desc }),
        });
    }

    // 6. Face Detection
    {
        let mut times = Vec::new();
        let mut face_count = 0i64;
        for _ in 0..5 {
            let start = Instant::now();
            let resp = client.post(format!("{}/detect", AI_SERVICE_URL))
                .json(&serde_json::json!({"image": &base64_image}))
                .send().await.unwrap();
            times.push(start.elapsed().as_secs_f64() * 1000.0);
            let body: serde_json::Value = resp.json().await.unwrap();
            face_count = body["count"].as_i64().unwrap_or(0);
        }
        results.push(BenchResult {
            name: "Face Detection (InsightFace)",
            times_ms: times,
            detail: format!("{} faces, 512d embeddings", face_count),
        });
    }

    // Print summary table
    println!("\n{}", "=".repeat(75));
    println!("  AI SERVICE SPEED BENCHMARK");
    println!("  Image: tests/test_image.jpg ({} KB)", image_data.len() / 1024);
    println!("{}", "=".repeat(75));
    println!("{:<32} {:>8} {:>8} {:>8} {:>5}", "Endpoint", "Min", "Avg", "Max", "Runs");
    println!("{}", "-".repeat(75));

    for r in &results {
        let avg = r.times_ms.iter().sum::<f64>() / r.times_ms.len() as f64;
        let min = r.times_ms.iter().cloned().fold(f64::INFINITY, f64::min);
        let max = r.times_ms.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        println!("{:<32} {:>7.1}ms {:>7.1}ms {:>7.1}ms {:>5}",
            r.name, min, avg, max, r.times_ms.len());
    }
    println!("{}", "-".repeat(75));
    for r in &results {
        println!("  {}: {}", r.name, r.detail);
    }
    println!("{}", "=".repeat(75));
}
