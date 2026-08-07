use actix_web::web;
use image::ImageEncoder;
use reminisce::verification_worker::verify_files;
use reminisce::test_utils::setup_test_database_with_instance;
use serial_test::serial;
use std::io::Write;

mod common;

const TEST_USER: &str = "550e8400-e29b-41d4-a716-446655440000";

fn user_uuid() -> uuid::Uuid {
    uuid::Uuid::parse_str(TEST_USER).unwrap()
}

/// Build a small, real, decodable JPEG in memory.
fn tiny_jpeg() -> Vec<u8> {
    let img = image::RgbImage::from_pixel(4, 4, image::Rgb([190, 90, 40]));
    let mut buf = std::io::Cursor::new(Vec::new());
    let enc = image::codecs::jpeg::JpegEncoder::new(&mut buf);
    enc.write_image(img.as_raw(), img.width(), img.height(), image::ColorType::Rgb8)
        .expect("encode jpeg");
    buf.into_inner()
}

fn hash_of(data: &[u8]) -> String {
    blake3::hash(data).to_hex().to_string()
}

/// Write `data` to the content-addressed media path used by the worker.
fn write_media(dir: &str, hash: &str, ext: &str, data: &[u8]) {
    let sub = std::path::Path::new(dir).join(&hash[..2]);
    std::fs::create_dir_all(&sub).unwrap();
    let path = sub.join(format!("{}.{}", hash, ext));
    let mut f = std::fs::File::create(&path).expect("create media file");
    f.write_all(data).unwrap();
}

fn thumb_path(dir: &str, hash: &str) -> std::path::PathBuf {
    std::path::Path::new(dir).join(&hash[..2]).join(format!("{}.thumb.jpg", hash))
}

async fn call_verify(pool: &deadpool_postgres::Pool, config: &reminisce::config::Config) -> bool {
    let main_pool = common::utils::wrap_main_pool(pool.clone());
    let data_pool = web::Data::new(main_pool);
    let data_cfg = web::Data::new(config.clone());
    verify_files(data_pool, data_cfg)
        .await
        .expect("verify_files must not error")
}

#[actix_web::test]
#[serial]
async fn test_verify_success_and_generate_missing_thumbnail() {
    let (pool, _db) = setup_test_database_with_instance().await;
    let config = common::utils::create_test_config();

    let jpeg = tiny_jpeg();
    let hash = hash_of(&jpeg);
    write_media(config.get_images_dir(), &hash, "jpg", &jpeg);
    // Clean any stale thumb.
    let _ = std::fs::remove_file(thumb_path(config.get_images_dir(), &hash));

    let client = pool.get().await.unwrap();
    client
        .execute(
            "INSERT INTO images (user_id, deviceid, hash, name, ext, type, has_thumbnail, verification_status) \
             VALUES ($1, 'test', $2, 'photo.jpg', 'jpg', 'camera', false, 0)",
            &[&user_uuid(), &hash],
        )
        .await
        .unwrap();

    let ran = call_verify(&pool, &config).await;
    assert!(ran, "expected work to be done");

    let row = client
        .query_one("SELECT verification_status, has_thumbnail FROM images WHERE hash = $1", &[&hash])
        .await
        .unwrap();
    let status: i32 = row.get(0);
    let has_thumb: bool = row.get(1);
    assert_eq!(status, 1, "file bytes matched the content hash");
    assert!(has_thumb, "missing thumbnail should have been generated");
    assert!(thumb_path(config.get_images_dir(), &hash).exists(), "thumb file on disk");
}

#[actix_web::test]
#[serial]
async fn test_verify_marks_mismatch_as_failed() {
    let (pool, _db) = setup_test_database_with_instance().await;
    let config = common::utils::create_test_config();

    // File on disk whose BLAKE3 does NOT match the hash column.
    let jpeg = tiny_jpeg();
    let hash = hash_of(b"some other bytes entirely");
    write_media(config.get_images_dir(), &hash, "jpg", &jpeg);

    let client = pool.get().await.unwrap();
    client
        .execute(
            "INSERT INTO images (user_id, deviceid, hash, name, ext, type, has_thumbnail, verification_status) \
             VALUES ($1, 'test', $2, 'photo.jpg', 'jpg', 'camera', false, 0)",
            &[&user_uuid(), &hash],
        )
        .await
        .unwrap();

    let ran = call_verify(&pool, &config).await;
    assert!(ran);

    let status: i32 = client
        .query_one("SELECT verification_status FROM images WHERE hash = $1", &[&hash])
        .await
        .unwrap()
        .get(0);
    assert_eq!(status, -1, "mismatched file marked as failed");
}

#[actix_web::test]
#[serial]
async fn test_verify_marks_missing_file_as_failed() {
    let (pool, _db) = setup_test_database_with_instance().await;
    let config = common::utils::create_test_config();

    // No file written at all.
    let hash = hash_of(b"never written");
    let client = pool.get().await.unwrap();
    client
        .execute(
            "INSERT INTO images (user_id, deviceid, hash, name, ext, type, has_thumbnail, verification_status) \
             VALUES ($1, 'test', $2, 'photo.jpg', 'jpg', 'camera', false, 0)",
            &[&user_uuid(), &hash],
        )
        .await
        .unwrap();

    let ran = call_verify(&pool, &config).await;
    assert!(ran);

    let status: i32 = client
        .query_one("SELECT verification_status FROM images WHERE hash = $1", &[&hash])
        .await
        .unwrap()
        .get(0);
    assert_eq!(status, -1);
}

#[actix_web::test]
#[serial]
async fn test_verify_video_success() {
    let (pool, _db) = setup_test_database_with_instance().await;
    let config = common::utils::create_test_config();

    // Match the video's content hash; a real decodable mp4 is not needed for the
    // verification hash check (thumb generation may fail, which is handled).
    let data = b"fake mp4 bytes for hashing only";
    let hash = hash_of(data);
    write_media(config.get_videos_dir(), &hash, "mp4", data);

    let client = pool.get().await.unwrap();
    client
        .execute(
            "INSERT INTO videos (user_id, deviceid, hash, name, ext, type, has_thumbnail, verification_status) \
             VALUES ($1, 'test', $2, 'clip.mp4', 'mp4', 'camera', false, 0)",
            &[&user_uuid(), &hash],
        )
        .await
        .unwrap();

    let ran = call_verify(&pool, &config).await;
    assert!(ran);

    let row = client
        .query_one("SELECT verification_status, last_verified_at FROM videos WHERE hash = $1", &[&hash])
        .await
        .unwrap();
    let status: i32 = row.get(0);
    let verified_at: Option<chrono::DateTime<chrono::Utc>> = row.get(1);
    assert_eq!(status, 1, "video hash matched");
    assert!(verified_at.is_some(), "last_verified_at recorded");
}

#[actix_web::test]
#[serial]
async fn test_verify_no_work_returns_false() {
    let (pool, _db) = setup_test_database_with_instance().await;
    let config = common::utils::create_test_config();

    let client = pool.get().await.unwrap();
    client
        .execute(
            "INSERT INTO images (user_id, deviceid, hash, name, ext, type, has_thumbnail, verification_status, last_verified_at) \
             VALUES ($1, 'test', 'fully_verified', 'pic.jpg', 'jpg', 'camera', true, 1, NOW())",
            &[&user_uuid()],
        )
        .await
        .unwrap();

    let ran = call_verify(&pool, &config).await;
    assert!(!ran, "nothing pending -> no work");
}
