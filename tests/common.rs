use env_logger;

#[path = "utils.rs"]
pub mod utils;
#[path = "multipart_builder.rs"]
pub mod multipart_builder;

// Initialize logging for tests once
#[allow(dead_code)]
static INIT: std::sync::Once = std::sync::Once::new();

#[allow(dead_code)]
pub fn init_log() {
    INIT.call_once(|| {
        env_logger
            ::builder()
            .filter(None, log::LevelFilter::Info)
            .try_init()
            .ok();
    });
}

#[allow(dead_code)]
pub const TEST_IMAGE_HASH: &str = "af29ca6fd22f34f3c51c3dc5326ff277b80ad6344a3a9af35bb5548ccf8cdb16"; // BLAKE3 of tests/test_image.jpg
#[allow(dead_code)]
pub const TEST_IMAGE_HASH2: &str = "6bed93e776c244d03857973e3b1c9cbdaa6cba2ed62c4e00a0fe1b984cb26a8d"; // BLAKE3 of tests/test_image2.jpg
#[allow(dead_code)]
pub const TEST_IMAGE_NAME: &str = "IMG-20231222-191241.jpg";
#[allow(dead_code)]
pub const TEST_IMAGE_NAME2: &str = "IMG-20231222-191241.jpg";
#[allow(dead_code)]
pub const TEST_VIDEO_HASH: &str = "359e03c57e2fbe3af680cb73cc7e553893548adf52b3a8e0a2b7708da0f56398"; // BLAKE3 of tests/test_video.mp4
#[allow(dead_code)]
pub const TEST_VIDEO_NAME: &str = "/storage/emulated/0/DCIM/Camera/VID_20250614_224725.mp4";
#[allow(dead_code)]
pub const TEST_VIDEO_HASH2: &str = "cc32a44125af17256768285da55c0c8ca5fae4f9425c4202be2a005fff60d9b7"; // BLAKE3 of tests/test_video2.mp4
#[allow(dead_code)]
pub const TEST_VIDEO_NAME2: &str = "/storage/emulated/0/DCIM/Camera/VID_20250614_224726.mp4";
#[allow(dead_code)]
pub const TEST_CHECK_HASH: &str = "test_check_hash";
#[allow(dead_code)]
pub const TEST_THUMBNAILS_HASH: &str = "test_thumbnails_hash";
#[allow(dead_code)]
pub const TEST_UPLOAD_DIR: &str = "uploaded_images_test";
#[allow(dead_code)]
pub const TEST_VIDEOS_DIR: &str = "uploaded_videos_test";

