use actix_web::web::Bytes;

/// Core multipart builder. All public functions delegate here.
fn build_multipart(
    hash: &str,
    name: &str,
    device_id: &str,
    file_field_name: &str,
    file_name: &str,
    file_content_type: &str,
    file_bytes: &[u8],
    thumbnail_bytes: Option<&[u8]>,
    created_at: Option<&str>,
) -> (Bytes, String) {
    let boundary = "----WebKitFormBoundary7MA4YWxkTrZu0gW";
    let mut body = Vec::new();

    // Text fields: hash, name, device_id
    for (field, value) in [("hash", hash), ("name", name), ("device_id", device_id)] {
        body.extend_from_slice(format!("--{}\r\n", boundary).as_bytes());
        body.extend_from_slice(format!("Content-Disposition: form-data; name=\"{}\"\r\n\r\n", field).as_bytes());
        body.extend_from_slice(value.as_bytes());
        body.extend_from_slice(b"\r\n");
    }

    // Optional created_at field
    if let Some(ts) = created_at {
        body.extend_from_slice(format!("--{}\r\n", boundary).as_bytes());
        body.extend_from_slice(b"Content-Disposition: form-data; name=\"created_at\"\r\n\r\n");
        body.extend_from_slice(ts.as_bytes());
        body.extend_from_slice(b"\r\n");
    }

    // Media file field
    body.extend_from_slice(format!("--{}\r\n", boundary).as_bytes());
    body.extend_from_slice(format!("Content-Disposition: form-data; name=\"{}\"; filename=\"{}\"\r\n", file_field_name, file_name).as_bytes());
    body.extend_from_slice(format!("Content-Type: {}\r\n\r\n", file_content_type).as_bytes());
    body.extend_from_slice(file_bytes);
    body.extend_from_slice(b"\r\n");

    // Optional thumbnail
    if let Some(thumb) = thumbnail_bytes {
        body.extend_from_slice(format!("--{}\r\n", boundary).as_bytes());
        body.extend_from_slice(b"Content-Disposition: form-data; name=\"thumbnail\"; filename=\"test_thumbnail.jpg\"\r\n");
        body.extend_from_slice(b"Content-Type: image/jpeg\r\n\r\n");
        body.extend_from_slice(thumb);
        body.extend_from_slice(b"\r\n");
    }

    body.extend_from_slice(format!("--{}--\r\n", boundary).as_bytes());
    (Bytes::from(body), format!("multipart/form-data; boundary={}", boundary))
}

#[allow(dead_code)]
pub fn create_multipart_payload(
    hash: &str, name: &str, image_bytes: &[u8], thumbnail_bytes: &[u8],
) -> (Bytes, String) {
    build_multipart(hash, name, "test_device_id", "image", "test_image.jpg", "image/jpeg", image_bytes, Some(thumbnail_bytes), None)
}

#[allow(dead_code)]
pub fn create_multipart_payload_with_device_id(
    hash: &str, name: &str, image_bytes: &[u8], thumbnail_bytes: &[u8], device_id: &str,
) -> (Bytes, String) {
    build_multipart(hash, name, device_id, "image", "test_image.jpg", "image/jpeg", image_bytes, Some(thumbnail_bytes), None)
}

#[allow(dead_code)]
pub fn create_multipart_payload_with_created_at(
    hash: &str, name: &str, image_bytes: &[u8], thumbnail_bytes: &[u8], created_at: &str,
) -> (Bytes, String) {
    build_multipart(hash, name, "test_device_id", "image", "test_image.jpg", "image/jpeg", image_bytes, Some(thumbnail_bytes), Some(created_at))
}

#[allow(dead_code)]
pub fn create_video_multipart_payload(
    hash: &str, name: &str, video_bytes: &[u8], thumbnail_bytes: &[u8],
) -> (Bytes, String) {
    build_multipart(hash, name, "test_device_id", "video", "test_video.mp4", "video/mp4", video_bytes, Some(thumbnail_bytes), None)
}

#[allow(dead_code)]
pub fn create_video_multipart_payload_with_device_id(
    hash: &str, name: &str, video_bytes: &[u8], thumbnail_bytes: &[u8], device_id: &str,
) -> (Bytes, String) {
    build_multipart(hash, name, device_id, "video", "test_video.mp4", "video/mp4", video_bytes, Some(thumbnail_bytes), None)
}

#[allow(dead_code)]
pub fn create_video_multipart_payload_with_created_at(
    hash: &str, name: &str, video_bytes: &[u8], thumbnail_bytes: &[u8], created_at: &str,
) -> (Bytes, String) {
    build_multipart(hash, name, "test_device_id", "video", "test_video.mp4", "video/mp4", video_bytes, Some(thumbnail_bytes), Some(created_at))
}

#[allow(dead_code)]
pub fn create_multipart_payload_without_thumbnail(
    hash: &str, name: &str, image_bytes: &[u8],
) -> (Bytes, String) {
    build_multipart(hash, name, "test_device_id", "image", "test_image.jpg", "image/jpeg", image_bytes, None, None)
}

#[allow(dead_code)]
pub fn create_video_multipart_payload_without_thumbnail(
    hash: &str, name: &str, video_bytes: &[u8],
) -> (Bytes, String) {
    build_multipart(hash, name, "test_device_id", "video", "test_video.mp4", "video/mp4", video_bytes, None, None)
}
