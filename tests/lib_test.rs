use reminisce::utils::{parse_date_from_image_name, parse_date_from_video_name};
use chrono::{DateTime, NaiveDate, Utc};

mod common;

#[test]
fn test_parse_date_from_image_name_valid() {
    common::init_log();
    let image_name = "/storage/emulated/0/Android/media/com.whatsapp/WhatsApp/Media/WhatsApp Images/IMG-20250811-WA0006.jpg";
    let expected_date = Some(DateTime::<Utc>::from_naive_utc_and_offset(
        NaiveDate::from_ymd_opt(2025, 8, 11)
            .unwrap()
            .and_hms_milli_opt(0, 0, 0, 6)
            .unwrap(),
        Utc,
    ));
    assert_eq!(parse_date_from_image_name(image_name), expected_date);
}

#[test]
fn test_parse_date_from_image_name_valid_camera() {
    let image_name = "/storage/emulated/0/DCIM/Camera/IMG_20250108_140818.jpg";
    let expected_date = Some(DateTime::<Utc>::from_naive_utc_and_offset(
        NaiveDate::from_ymd_opt(2025, 1, 8)
            .unwrap()
            .and_hms_opt(14, 8, 18)
            .unwrap(),
        Utc,
    ));
    assert_eq!(parse_date_from_image_name(image_name), expected_date);
}

#[test]
fn test_parse_date_from_image_name_valid_camera_re() {
    let image_name = "/storage/emulated/0/DCIM/Camera/RECTIFY_IMG_20250113_114126.jpg";
    let expected_date = Some(DateTime::<Utc>::from_naive_utc_and_offset(
        NaiveDate::from_ymd_opt(2025, 1, 13)
            .unwrap()
            .and_hms_opt(11, 41, 26)
            .unwrap(),
        Utc,
    ));
    assert_eq!(parse_date_from_image_name(image_name), expected_date);
}

#[test]
fn test_parse_date_from_image_name_invalid_prefix() {
    let image_name = "IMAGE-20250811-WA0006.jpg";
    assert_eq!(parse_date_from_image_name(image_name), None);
}

#[test]
fn test_parse_date_from_image_name_invalid_date() {
    let image_name = "IMG-20251342-WA0006.jpg";
    assert_eq!(parse_date_from_image_name(image_name), None);
}

#[test]
fn test_parse_date_from_image_name_no_date() {
    let image_name = "my_image.jpg";
    assert_eq!(parse_date_from_image_name(image_name), None);
}

#[test]
fn test_parse_date_from_image_name_empty_string() {
    let image_name = "";
    assert_eq!(parse_date_from_image_name(image_name), None);
}

#[test]
fn test_parse_date_from_image_name_case_insensitive() {
    let image_name = "/storage/emulated/0/Android/media/com.whatsapp/WhatsApp/Media/WhatsApp Images/img-20250811-WA0006.jpg";
    let expected_date = Some(DateTime::<Utc>::from_naive_utc_and_offset(
        NaiveDate::from_ymd_opt(2025, 8, 11)
            .unwrap()
            .and_hms_milli_opt(0, 0, 0, 6)
            .unwrap(),
        Utc,
    ));
    assert_eq!(parse_date_from_image_name(image_name), expected_date);
}

#[test]
fn test_parse_date_from_video_name_vid_with_timestamp() {
    let video_name = "VID_20250614_232246.mp4";
    let expected_date = Some(DateTime::<Utc>::from_naive_utc_and_offset(
        NaiveDate::from_ymd_opt(2025, 6, 14)
            .unwrap()
            .and_hms_opt(23, 22, 46)
            .unwrap(),
        Utc,
    ));
    assert_eq!(parse_date_from_video_name(video_name), expected_date);
}

#[test]
fn test_parse_date_from_video_name_vid_with_date_only() {
    let video_name = "VID-20250707.mp4";
    let expected_date = Some(DateTime::<Utc>::from_naive_utc_and_offset(
        NaiveDate::from_ymd_opt(2025, 7, 7)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap(),
        Utc,
    ));
    assert_eq!(parse_date_from_video_name(video_name), expected_date);
}

#[test]
fn test_parse_date_from_video_name_case_insensitive() {
    let video_name = "vid_20250614_232246.mp4";
    let expected_date = Some(DateTime::<Utc>::from_naive_utc_and_offset(
        NaiveDate::from_ymd_opt(2025, 6, 14)
            .unwrap()
            .and_hms_opt(23, 22, 46)
            .unwrap(),
        Utc,
    ));
    assert_eq!(parse_date_from_video_name(video_name), expected_date);
}

#[test]
fn test_parse_date_from_video_name_with_path() {
    let video_name = "/storage/emulated/0/DCIM/Camera/VID_20250614_232246.mp4";
    let expected_date = Some(DateTime::<Utc>::from_naive_utc_and_offset(
        NaiveDate::from_ymd_opt(2025, 6, 14)
            .unwrap()
            .and_hms_opt(23, 22, 46)
            .unwrap(),
        Utc,
    ));
    assert_eq!(parse_date_from_video_name(video_name), expected_date);
}

#[test]
fn test_parse_date_from_video_name_with_your_example() {
    let video_name = "/storage/emulated/0/DCIM/Camera/VID_20250614_224725.mp4";
    let expected_date = Some(DateTime::<Utc>::from_naive_utc_and_offset(
        NaiveDate::from_ymd_opt(2025, 6, 14)
            .unwrap()
            .and_hms_opt(22, 47, 25)
            .unwrap(),
        Utc,
    ));
    assert_eq!(parse_date_from_video_name(video_name), expected_date);
}

#[test]
fn test_parse_date_from_video_name_samsung_slow_motion() {
    let video_name = "/storage/emulated/0/DCIM/Camera/SL_MO_VID_20250615_114841.mp4";
    let expected_date = Some(DateTime::<Utc>::from_naive_utc_and_offset(
        NaiveDate::from_ymd_opt(2025, 6, 15)
            .unwrap()
            .and_hms_opt(11, 48, 41)
            .unwrap(),
        Utc,
    ));
    assert_eq!(parse_date_from_video_name(video_name), expected_date);
}

#[test]
fn test_parse_date_from_video_name_samsung_slow_motion_short() {
    let video_name = "SL_MO_VID_20250615_114841.mp4";
    let expected_date = Some(DateTime::<Utc>::from_naive_utc_and_offset(
        NaiveDate::from_ymd_opt(2025, 6, 15)
            .unwrap()
            .and_hms_opt(11, 48, 41)
            .unwrap(),
        Utc,
    ));
    assert_eq!(parse_date_from_video_name(video_name), expected_date);
}

#[test]
fn test_parse_date_from_video_name_whatsapp() {
    let video_name = "/storage/emulated/0/Android/media/com.whatsapp/WhatsApp/Media/WhatsApp Video/VID-20250707-WA0011.mp4";
    let expected_date = Some(DateTime::<Utc>::from_naive_utc_and_offset(
        NaiveDate::from_ymd_opt(2025, 7, 7)
            .unwrap()
            .and_hms_milli_opt(0, 0, 0, 11)
            .unwrap(),
        Utc,
    ));
    assert_eq!(parse_date_from_video_name(video_name), expected_date);
}

#[test]
fn test_parse_date_from_video_name_whatsapp_short() {
    let video_name = "VID-20250707-WA0999.mp4";
    let expected_date = Some(DateTime::<Utc>::from_naive_utc_and_offset(
        NaiveDate::from_ymd_opt(2025, 7, 7)
            .unwrap()
            .and_hms_milli_opt(0, 0, 0, 999)
            .unwrap(),
        Utc,
    ));
    assert_eq!(parse_date_from_video_name(video_name), expected_date);
}

#[test]
fn test_parse_date_from_video_name_dji_drone() {
    let video_name = "/storage/emulated/0/DCIM/DJI Album/DJI_20250409_094146_27_null_video.mp4";
    let expected_date = Some(DateTime::<Utc>::from_naive_utc_and_offset(
        NaiveDate::from_ymd_opt(2025, 4, 9)
            .unwrap()
            .and_hms_opt(9, 41, 46)
            .unwrap(),
        Utc,
    ));
    assert_eq!(parse_date_from_video_name(video_name), expected_date);
}

#[test]
fn test_parse_date_from_video_name_dji_drone_short() {
    let video_name = "DJI_20250409_094146_27_null_video.mp4";
    let expected_date = Some(DateTime::<Utc>::from_naive_utc_and_offset(
        NaiveDate::from_ymd_opt(2025, 4, 9)
            .unwrap()
            .and_hms_opt(9, 41, 46)
            .unwrap(),
        Utc,
    ));
    assert_eq!(parse_date_from_video_name(video_name), expected_date);
}

#[test]
fn test_parse_date_from_video_name_dji_case_insensitive() {
    let video_name = "dji_20250409_094146_27_null_video.mp4";
    let expected_date = Some(DateTime::<Utc>::from_naive_utc_and_offset(
        NaiveDate::from_ymd_opt(2025, 4, 9)
            .unwrap()
            .and_hms_opt(9, 41, 46)
            .unwrap(),
        Utc,
    ));
    assert_eq!(parse_date_from_video_name(video_name), expected_date);
}

#[test]
fn test_parse_date_from_video_name_invalid_date() {
    let video_name = "VID_20251342_232246.mp4";
    assert_eq!(parse_date_from_video_name(video_name), None);
}

#[test]
fn test_parse_date_from_video_name_invalid_time() {
    let video_name = "VID_20250614_256789.mp4";
    assert_eq!(parse_date_from_video_name(video_name), None);
}

#[test]
fn test_parse_date_from_video_name_no_video_pattern() {
    let video_name = "my_video.mp4";
    assert_eq!(parse_date_from_video_name(video_name), None);
}

#[test]
fn test_parse_date_from_video_name_empty_string() {
    let video_name = "";
    assert_eq!(parse_date_from_video_name(video_name), None);
}

#[test]
fn test_parse_date_from_video_name_invalid_prefix() {
    let video_name = "VIDEO_20250614_232246.mp4";
    assert_eq!(parse_date_from_video_name(video_name), None);
}