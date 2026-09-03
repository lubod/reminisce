use actix_web::web;
use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use futures::TryStreamExt;
use log::{error, info, warn};
use std::path::PathBuf;
use std::time::Duration;
use tokio::fs;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::constants::media;
use crate::db::MainDbPool;
use crate::query_builder::MediaQueryBuilder;
use crate::services::thumbnail::ThumbnailItem;

// ---- EXIF Orientation Helpers ------------------------------------------------

/// Apply an EXIF orientation value to a `DynamicImage`, returning the correctly rotated image.
/// Orientation 1 (normal) and any unknown value are returned unchanged.
/// Rotate an EXIF orientation value 90 degrees clockwise.
pub fn rotate_orientation_cw(current: Option<i16>) -> i16 {
    match current.unwrap_or(1) {
        1 => 6,
        6 => 3,
        3 => 8,
        8 => 1,
        2 => 7,
        7 => 4,
        4 => 5,
        5 => 2,
        _ => 6,
    }
}

/// Rotate an EXIF orientation value 90 degrees counter-clockwise.
pub fn rotate_orientation_ccw(current: Option<i16>) -> i16 {
    match current.unwrap_or(1) {
        1 => 8,
        8 => 3,
        3 => 6,
        6 => 1,
        2 => 5,
        5 => 4,
        4 => 7,
        7 => 2,
        _ => 8,
    }
}

/// Rotate an EXIF orientation value 180 degrees.
pub fn rotate_orientation_180(current: Option<i16>) -> i16 {
    match current.unwrap_or(1) {
        1 => 3,
        3 => 1,
        6 => 8,
        8 => 6,
        2 => 4,
        4 => 2,
        5 => 7,
        7 => 5,
        _ => 3,
    }
}

pub fn apply_orientation_to_image(
    img: image::DynamicImage,
    orientation: u16,
) -> image::DynamicImage {
    match orientation {
        2 => img.fliph(),
        3 => img.rotate180(),
        4 => img.flipv(),
        5 => img.rotate90().fliph(),
        6 => img.rotate90(),
        7 => img.rotate270().fliph(),
        8 => img.rotate270(),
        _ => img, // 1 = normal; unknown values are left unchanged
    }
}

/// Read the EXIF orientation tag from in-memory image bytes.
/// Returns `None` if no orientation tag is present or the bytes aren't valid EXIF.
pub fn read_exif_orientation_from_bytes(data: &[u8]) -> Option<u16> {
    let cursor = std::io::Cursor::new(data);
    let mut bufreader = std::io::BufReader::new(cursor);
    kamadak_exif::Reader::new()
        .read_from_container(&mut bufreader)
        .ok()
        .and_then(|exif| {
            exif.get_field(kamadak_exif::Tag::Orientation, kamadak_exif::In::PRIMARY)
                .and_then(|f| {
                    if let kamadak_exif::Value::Short(ref v) = f.value {
                        v.first().copied()
                    } else {
                        None
                    }
                })
        })
}

/// Read the EXIF orientation tag by opening the file at `path`.
/// Returns `None` if the file can't be opened, has no EXIF, or has no orientation tag.
pub fn read_exif_orientation_from_path(path: &std::path::Path) -> Option<u16> {
    let file = std::fs::File::open(path).ok()?;
    let mut bufreader = std::io::BufReader::new(&file);
    kamadak_exif::Reader::new()
        .read_from_container(&mut bufreader)
        .ok()
        .and_then(|exif| {
            exif.get_field(kamadak_exif::Tag::Orientation, kamadak_exif::In::PRIMARY)
                .and_then(|f| {
                    if let kamadak_exif::Value::Short(ref v) = f.value {
                        v.first().copied()
                    } else {
                        None
                    }
                })
        })
}

// ---- JPEG EXIF Orientation Injection ----------------------------------------

/// Inject a minimal EXIF APP1 block carrying only the Orientation tag into a
/// JPEG byte stream.  Returns the bytes unchanged if:
///   - the input is not a JPEG (no SOI marker), or
///   - an APP1 EXIF block is already present (shouldn't happen under the
///     `exif IS NULL` DB guard, but safe to handle).
///
/// The injected block is 36 bytes inserted right after the 2-byte SOI marker.
/// No decode/re-encode — pixel data is never touched.
pub fn inject_exif_orientation(jpeg_bytes: &[u8], orientation: u16) -> Vec<u8> {
    // Must start with JPEG SOI (FF D8)
    if jpeg_bytes.len() < 4 || jpeg_bytes[0] != 0xFF || jpeg_bytes[1] != 0xD8 {
        return jpeg_bytes.to_vec();
    }

    // An existing Exif APP1 means we cannot prepend another one (undefined
    // behavior across parsers) — leave the file untouched. The richer splice
    // path in ensure_exif_orientation handles this case when needed.
    if find_exif_app1(jpeg_bytes).is_some() {
        return jpeg_bytes.to_vec();
    }

    // Build a 36-byte minimal EXIF APP1 block (little-endian TIFF, 1 IFD entry)
    let mut app1 = [0u8; 36];
    app1[0..2].copy_from_slice(&[0xFF, 0xE1]);         // APP1 marker
    app1[2..4].copy_from_slice(&[0x00, 0x22]);         // length = 34
    app1[4..10].copy_from_slice(b"Exif\0\0");          // EXIF header
    app1[10..12].copy_from_slice(&[0x49, 0x49]);       // "II" little-endian
    app1[12..14].copy_from_slice(&[0x2A, 0x00]);       // TIFF magic
    app1[14..18].copy_from_slice(&[0x08, 0x00, 0x00, 0x00]); // IFD0 at offset 8
    app1[18..20].copy_from_slice(&[0x01, 0x00]);       // 1 IFD entry
    app1[20..22].copy_from_slice(&[0x12, 0x01]);       // tag 0x0112 = Orientation
    app1[22..24].copy_from_slice(&[0x03, 0x00]);       // type SHORT
    app1[24..28].copy_from_slice(&[0x01, 0x00, 0x00, 0x00]); // count 1
    app1[28] = (orientation & 0xFF) as u8;             // value low byte
    app1[29] = ((orientation >> 8) & 0xFF) as u8;      // value high byte
    // bytes 30-35 stay zero (SHORT padding + next-IFD offset)

    let mut out = Vec::with_capacity(jpeg_bytes.len() + 36);
    out.extend_from_slice(&jpeg_bytes[..2]); // SOI
    out.extend_from_slice(&app1);
    out.extend_from_slice(&jpeg_bytes[2..]);
    out
}

/// Ensure a JPEG's EXIF carries `orientation`.
///
/// - JPEG without an Exif APP1 → minimal APP1 inserted (existing helper).
/// - Exif APP1 already HAS the Orientation tag → value rewritten in place.
/// - Exif APP1 WITHOUT an Orientation entry → new IFD0 entry spliced in:
///   entry count +1, data after the IFD0 entries shifts by 12 bytes, and every
///   out-of-line value offset / next-IFD pointer is fixed up.
/// - Anything malformed, oversized, or non-JPEG returns untouched — worst case
///   is today's behavior (served unpatched), never a corrupt image.
pub fn ensure_exif_orientation(jpeg_bytes: &[u8], orientation: u16) -> Vec<u8> {
    if !(1..=8).contains(&orientation) {
        return jpeg_bytes.to_vec();
    }
    if read_exif_orientation_from_bytes(jpeg_bytes) == Some(orientation) {
        return jpeg_bytes.to_vec();
    }
    // Only touch plausible complete JPEGs (must reach a Start-of-Scan).
    let has_sos = jpeg_bytes.windows(2).any(|w| w == b"\xFF\xDA");
    if !has_sos {
        return jpeg_bytes.to_vec();
    }
    match find_exif_app1(jpeg_bytes) {
        None => inject_exif_orientation(jpeg_bytes, orientation),
        Some(loc) => splice_orientation_into_app1(jpeg_bytes, loc, orientation)
            .unwrap_or_else(|| jpeg_bytes.to_vec()),
    }
}

/// Locate the first Exif APP1 segment. Returns `(marker_pos, total_len)`
/// where total_len includes the two marker bytes.
fn find_exif_app1(jpeg: &[u8]) -> Option<(usize, usize)> {
    if jpeg.len() < 4 || jpeg[0] != 0xFF || jpeg[1] != 0xD8 {
        return None;
    }
    let mut pos = 2usize;
    while pos + 4 <= jpeg.len() {
        if jpeg[pos] != 0xFF {
            break;
        }
        let marker = jpeg[pos + 1];
        if marker == 0xFF {
            pos += 1; // fill byte
            continue;
        }
        if marker == 0xDA {
            break; // start of scan — no EXIF beyond this point
        }
        let seg_len = u16::from_be_bytes([jpeg[pos + 2], jpeg[pos + 3]]) as usize;
        if seg_len < 2 {
            break;
        }
        if marker == 0xE1
            && pos + 10 <= jpeg.len()
            && &jpeg[pos + 4..pos + 10] == b"Exif\0\0"
        {
            return Some((pos, seg_len + 2));
        }
        pos += 2 + seg_len;
    }
    None
}

#[derive(Clone, Copy, PartialEq)]
enum Endian {
    Little,
    Big,
}

impl Endian {
    fn parse(b: &[u8]) -> Option<Endian> {
        match b {
            b"II" => Some(Endian::Little),
            b"MM" => Some(Endian::Big),
            _ => None,
        }
    }
    fn u16(self, b: &[u8]) -> Option<u16> {
        b.get(0..2).map(|s| match self {
            Endian::Little => u16::from_le_bytes([s[0], s[1]]),
            Endian::Big => u16::from_be_bytes([s[0], s[1]]),
        })
    }
    fn u32(self, b: &[u8]) -> Option<u32> {
        b.get(0..4).map(|s| match self {
            Endian::Little => u32::from_le_bytes([s[0], s[1], s[2], s[3]]),
            Endian::Big => u32::from_be_bytes([s[0], s[1], s[2], s[3]]),
        })
    }
    fn push_u16(self, out: &mut Vec<u8>, v: u16) {
        out.extend_from_slice(&match self {
            Endian::Little => v.to_le_bytes(),
            Endian::Big => v.to_be_bytes(),
        });
    }
    fn push_u32(self, out: &mut Vec<u8>, v: u32) {
        out.extend_from_slice(&match self {
            Endian::Little => v.to_le_bytes(),
            Endian::Big => v.to_be_bytes(),
        });
    }
}

/// Byte size of one IFD value of `typ` × `count` (None for unknown types).
fn tiff_value_size(typ: u16, count: u32) -> Option<u32> {
    let unit: u32 = match typ {
        1 | 2 | 6 | 7 => 1,
        3 | 8 => 2,
        4 | 9 | 11 => 4,
        5 | 10 | 12 => 8,
        _ => return None,
    };
    unit.checked_mul(count)
}

const ORIENTATION_TAG: u16 = 0x0112;
const EXIF_SUB_IFD_TAG: u16 = 0x8769;
const GPS_SUB_IFD_TAG: u16 = 0x8825;

fn splice_orientation_into_app1(
    jpeg: &[u8],
    (app1_pos, app1_total): (usize, usize),
    orientation: u16,
) -> Option<Vec<u8>> {
    // Segment layout: FF E1 | len(2) | "Exif\0\0"(6) | tiff...
    if app1_total < 14 || app1_pos + app1_total > jpeg.len() {
        return None;
    }
    let seg = &jpeg[app1_pos..app1_pos + app1_total];
    let tiff = &seg[10..];
    if tiff.len() < 8 {
        return None;
    }
    #[cfg(test)]
    eprintln!("DBG enter: pos={} total={} jlen={}", app1_pos, app1_total, jpeg.len());
    let endian = Endian::parse(&tiff[0..2])?;
    if endian.u16(&tiff[2..4])? != 42 {
        return None;
    }
    let ifd0_off = endian.u32(&tiff[4..8])? as usize;
    if ifd0_off < 8 || ifd0_off + 2 > tiff.len() {
        return None;
    }
    let count = endian.u16(&tiff[ifd0_off..ifd0_off + 2])? as usize;
    let entries_end = ifd0_off + 2 + count * 12;
    if count == 0 || count >= 0xFFFF || entries_end + 4 > tiff.len() {
        return None;
    }

    // Walk IFD0 entries once.
    let mut orient_val_tiff_off: Option<usize> = None;
    for i in 0..count {
        let e = ifd0_off + 2 + i * 12;
        let tag = endian.u16(tiff.get(e..e + 12)?.get(0..2)?)?;
        if tag == ORIENTATION_TAG {
            orient_val_tiff_off = Some(e + 8);
        }
    }

    if let Some(voff) = orient_val_tiff_off {
        // ── Case A: tag exists → rewrite its 2-byte SHORT value in place.
        let abs = app1_pos + 10 + voff;
        if abs + 2 > jpeg.len() {
            return None;
        }
        let mut out = jpeg.to_vec();
        out[abs..abs + 2].copy_from_slice(&match endian {
            Endian::Little => orientation.to_le_bytes(),
            Endian::Big => orientation.to_be_bytes(),
        });
        return Some(out);
    }

    // ── Case B: insert a new entry. Segment grows by 12 bytes.
    if app1_total + 12 - 2 > 0xFFFF {
        return None; // would overflow the APP1 length field
    }

    let e4: usize = entries_end + 4; // old start of the out-of-line value area

    // ── Discover every pointer field that must shift +12.
    //
    // Inserting the Orientation entry moves everything from the original
    // next-IFD field onward by 12 bytes — including Exif sub-IFD tables,
    // GPS tables and the IFD1 thumbnail chain. Every out-of-line value
    // offset and every chained-IFD root pointer pointing into that region
    // has to be bumped, not just IFD0's own pointers.
    let mut marks: std::collections::HashSet<usize> = std::collections::HashSet::new();
    fn walk_ifd(
        tiff: &[u8],
        endian: Endian,
        off: usize,
        depth: u8,
        e4: usize,
        marks: &mut std::collections::HashSet<usize>,
    ) -> Option<()> {
        if off + 2 > tiff.len() {
            return None;
        }
        let n = endian.u16(tiff.get(off..off + 2)?)? as usize;
        let ifd_entries_end = off.checked_add(2)?.checked_add(n.checked_mul(12)?)?;
        if n == 0 || n > 0x2000 || ifd_entries_end + 4 > tiff.len() {
            return None;
        }
        for i in 0..n {
            let e = off + 2 + i * 12;
            let entry = tiff.get(e..e + 12)?;
            let tag = endian.u16(entry.get(0..2)?)?;
            let typ = endian.u16(entry.get(2..4)?)?;
            let cnt = endian.u32(entry.get(4..8)?)?;
            let size = tiff_value_size(typ, cnt)?;
            let old = endian.u32(entry.get(8..12)?)? as usize;
            if size > 4 {
                // Out-of-line value: field holds a TIFF-space offset.
                if old >= e4 && old <= tiff.len() {
                    marks.insert(e + 8);
                    if depth < 2 && matches!(tag, EXIF_SUB_IFD_TAG | GPS_SUB_IFD_TAG) {
                        walk_ifd(tiff, endian, old, depth + 1, e4, marks)?;
                    }
                }
            } else if cnt == 1 && typ == 4 {
                // Inline LONG: only sub-IFD roots store table offsets here.
                if matches!(tag, EXIF_SUB_IFD_TAG | GPS_SUB_IFD_TAG)
                    && old >= e4
                    && old <= tiff.len()
                {
                    marks.insert(e + 8);
                    walk_ifd(tiff, endian, old, depth + 1, e4, marks)?;
                }
            }
        }
        // Chain to IFD1 (thumbnail IFD): same shifting rules apply.
        let next = endian.u32(tiff.get(ifd_entries_end..ifd_entries_end + 4)?)? as usize;
        if next != 0 && next >= e4 && next <= tiff.len() {
            marks.insert(ifd_entries_end);
            if depth == 0 {
                walk_ifd(tiff, endian, next, depth + 1, e4, marks)?;
            }
        }
        Some(())
    }
    walk_ifd(tiff, endian, ifd0_off, 0, e4, &mut marks)?;

    let mut out: Vec<u8> = Vec::with_capacity(jpeg.len() + 12);
    out.extend_from_slice(&jpeg[..app1_pos]); // everything before the segment
    out.extend_from_slice(&[0xFF, 0xE1]);
    out.extend_from_slice(&((app1_total + 12 - 2) as u16).to_be_bytes()); // new len
    out.extend_from_slice(&seg[4..10]); // "Exif\0\0"

    // TIFF header (endian, magic, IFD0 offset) — unchanged.
    out.extend_from_slice(&tiff[..ifd0_off]);

    // Entry count, +1.
    endian.push_u16(&mut out, (count as u16) + 1);

    // Original entries; any marked pointer field shifts +12.
    for i in 0..count {
        let e = ifd0_off + 2 + i * 12;
        let entry = &tiff[e..e + 12];
        out.extend_from_slice(entry);
        if marks.contains(&(e + 8)) {
            let old = endian.u32(entry.get(8..12)?)? as usize;
            let idx = out.len() - 4;
            out[idx..idx + 4].copy_from_slice(&match endian {
                Endian::Little => ((old + 12) as u32).to_le_bytes(),
                Endian::Big => ((old + 12) as u32).to_be_bytes(),
            });
        }
    }

    // The inserted Orientation entry.
    endian.push_u16(&mut out, ORIENTATION_TAG);
    endian.push_u16(&mut out, 3); // SHORT
    endian.push_u32(&mut out, 1); // count
    endian.push_u16(&mut out, orientation); // value
    endian.push_u16(&mut out, 0); // padding

    // Next-IFD pointer (usually 0 = none); shifts if marked.
    let next_field = entries_end;
    let next_ifd = endian.u32(&tiff[next_field..next_field + 4])? as usize;
    if marks.contains(&next_field) {
        endian.push_u32(&mut out, (next_ifd + 12) as u32);
    } else {
        endian.push_u32(&mut out, next_ifd as u32);
    }

    // Everything after the original IFD0 block moves verbatim, except marked
    // pointer fields. Old TIFF offset p maps to new TIFF offset p + 12.
    #[cfg(test)]
    eprintln!("DBG parts: marker2+len2={} exif6={} hdr={} cnt2={} entries={} orient12={} nxt4={} rest={}",
        4, 6, ifd0_off, 2, count*12, 12, 4, tiff.len()-e4);
    let rest_start_out = out.len();
    out.extend_from_slice(&tiff[e4..]);
    #[cfg(test)]
    eprintln!("DBG mid_out={} expected_final={} jlen={}", out.len(), jpeg.len()+12, jpeg.len());
    for &m in &marks {
        if m >= e4 && m + 4 <= tiff.len() {
            let old = endian.u32(&tiff[m..m + 4])? as usize;
            if old >= e4 && (old as u64) + 12 <= tiff.len() as u64 {
                let idx = rest_start_out + (m - e4);
                out[idx..idx + 4].copy_from_slice(&match endian {
                    Endian::Little => ((old + 12) as u32).to_le_bytes(),
                    Endian::Big => ((old + 12) as u32).to_be_bytes(),
                });
            }
        }
    }

    out.extend_from_slice(&jpeg[app1_pos + app1_total..]);
    debug_assert_eq!(out.len(), jpeg.len() + 12);
    Some(out)
}

/// Rotate a PNG's pixel data according to an EXIF orientation value and
/// re-encode as PNG (lossless).  Returns `None` if the bytes cannot be decoded.
pub fn rotate_png_bytes(png_bytes: &[u8], orientation: u16) -> Option<Vec<u8>> {
    let img = image::load_from_memory(png_bytes).ok()?;
    let rotated = apply_orientation_to_image(img, orientation);
    let mut buf = std::io::Cursor::new(Vec::new());
    rotated.write_to(&mut buf, image::ImageOutputFormat::Png).ok()?;
    Some(buf.into_inner())
}

// ---- Path / Type Helpers ----------------------------------------------------

/// Compute the BLAKE3 hash of a file, returning the hex string.
pub async fn hash_file_blake3(path: &std::path::Path) -> Result<String, std::io::Error> {
    let mut file = fs::File::open(path).await?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = [0u8; 8192];
    loop {
        match file.read(&mut buffer).await {
            Ok(0) => break,
            Ok(n) => { hasher.update(&buffer[..n]); }
            Err(e) => return Err(e),
        }
    }
    Ok(hasher.finalize().to_hex().to_string())
}

/// Stream a multipart field to a temp file while computing its BLAKE3 hash.
/// Aborts with 413 once more than `max_bytes` have been written (the temp file
/// is removed), so an authenticated user cannot fill the disk with one request.
/// Returns `(temp_path, blake3_hex_hash)`.
pub async fn streaming_hash_to_temp(
    field: &mut actix_multipart::Field,
    temp_dir: &std::path::Path,
    max_bytes: u64,
) -> Result<(PathBuf, String), actix_web::Error> {
    let effective_temp_dir = if tokio::fs::create_dir_all(temp_dir).await.is_ok() {
        temp_dir.to_path_buf()
    } else {
        let fallback = std::env::temp_dir().join("reminisce_uploads");
        let _ = tokio::fs::create_dir_all(&fallback).await;
        fallback
    };

    let temp_filename = format!("{}.tmp", uuid::Uuid::new_v4());
    let mut temp_path = effective_temp_dir.join(&temp_filename);

    let mut f = match tokio::fs::File::create(&temp_path).await {
        Ok(file) => file,
        Err(e) => {
            log::warn!("Failed to create temp file at {:?}: {}. Falling back to system temp dir.", temp_path, e);
            let fallback_dir = std::env::temp_dir().join("reminisce_uploads");
            let _ = tokio::fs::create_dir_all(&fallback_dir).await;
            temp_path = fallback_dir.join(&temp_filename);
            tokio::fs::File::create(&temp_path).await.map_err(|err| {
                log::error!("Fallback temp file creation failed at {:?}: {}", temp_path, err);
                actix_web::error::ErrorInternalServerError("Failed to create temp file")
            })?
        }
    };

    let mut hasher = blake3::Hasher::new();
    let mut written: u64 = 0;
    while let Ok(Some(chunk)) = field.try_next().await {
        written += chunk.len() as u64;
        if written > max_bytes {
            drop(f);
            let _ = tokio::fs::remove_file(&temp_path).await;
            log::warn!("Upload aborted: exceeded {} byte limit after {} bytes", max_bytes, written);
            return Err(actix_web::error::ErrorPayloadTooLarge(format!(
                "File exceeds the {} MB upload limit",
                max_bytes / (1024 * 1024)
            )));
        }
        hasher.update(&chunk);
        f.write_all(&chunk).await
            .map_err(|e| {
                log::error!("Failed to write temp file {:?}: {}", temp_path, e);
                actix_web::error::ErrorInternalServerError("Failed to write temp file")
            })?;
    }
    // Ensure every byte is actually in the file before any reader (magic-byte
    // validation, move/rename) touches the path — and make it crash-durable
    // while we're at it.
    f.flush().await.map_err(|e| {
        log::error!("Failed to flush temp file {:?}: {}", temp_path, e);
        actix_web::error::ErrorInternalServerError("Failed to flush temp file")
    })?;
    f.sync_all().await.map_err(|e| {
        log::error!("Failed to sync temp file {:?}: {}", temp_path, e);
        actix_web::error::ErrorInternalServerError("Failed to sync temp file")
    })?;
    Ok((temp_path, hasher.finalize().to_hex().to_string()))
}

/// Upper bound for a single multipart *metadata* field (name, device_id, dates…).
/// These are small strings; anything larger is client abuse or a bug.
const MAX_METADATA_FIELD_BYTES: usize = 64 * 1024;

/// Drain a multipart text field into a `String`, capped at
/// `MAX_METADATA_FIELD_BYTES`. The stream is still drained to completion so the
/// multipart framing stays intact, but overflowed bytes are discarded instead of
/// being buffered in RAM (memory-exhaustion protection).
pub async fn read_field_string(field: &mut actix_multipart::Field) -> String {
    let mut bytes = Vec::new();
    let mut overflowed = false;
    while let Ok(Some(chunk)) = field.try_next().await {
        if bytes.len() < MAX_METADATA_FIELD_BYTES {
            let remaining = MAX_METADATA_FIELD_BYTES - bytes.len();
            bytes.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
        } else {
            overflowed = true;
        }
    }
    if overflowed {
        log::warn!("Multipart metadata field exceeded {} bytes — truncated", MAX_METADATA_FIELD_BYTES);
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Decode `image_data`, apply the given EXIF `orientation`, and re-encode as JPEG (quality 90).
pub fn orient_image_to_jpeg(image_data: &[u8], orientation: u16) -> Result<Vec<u8>, String> {
    let img = image::load_from_memory(image_data)
        .map_err(|e| format!("Failed to decode image: {}", e))?;
    let oriented = apply_orientation_to_image(img, orientation);
    let mut output = std::io::Cursor::new(Vec::new());
    oriented
        .write_to(&mut output, image::ImageOutputFormat::Jpeg(90))
        .map_err(|e| format!("Failed to encode oriented image: {}", e))?;
    Ok(output.into_inner())
}

/// Generates a two-character subdirectory path from the first two characters of a hash.
pub fn get_subdirectory_path(base_dir: &str, hash: &str) -> PathBuf {
    if hash.len() < 2 {
        return PathBuf::from(base_dir);
    }
    PathBuf::from(base_dir).join(&hash[..2])
}

/// True if `s` is a 64-character lowercase hex string (the BLAKE3 content-hash format).
/// Content hashes are interpolated into filesystem paths, so anything else is rejected
/// to prevent path traversal.
pub fn is_valid_content_hash(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// True if `s` is a safe media-file extension: 1–10 alphanumeric characters.
/// `ext` is interpolated into the served filename, so any path separator, dot, or
/// control character is rejected to prevent path traversal.
pub fn is_valid_media_ext(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 10
        && s.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// Resolve the on-disk path for a content-addressed media file, canonicalizing the
/// base directory and verifying the resolved file stays inside it. Prevents LFI /
/// path traversal via user-supplied hash or ext.
pub fn safe_resolve_content_path(
    base_dir: &str,
    hash: &str,
    ext: &str,
) -> Result<PathBuf, String> {
    if !is_valid_content_hash(hash) {
        return Err(format!("Invalid content hash: {}", hash));
    }
    if !is_valid_media_ext(ext) {
        return Err(format!("Invalid media extension: {}", ext));
    }

    let base = std::fs::canonicalize(base_dir)
        .map_err(|e| format!("Cannot canonicalize base dir {}: {}", base_dir, e))?;
    let resolved = base.join(&hash[..2]).join(format!("{}.{}", hash, ext));
    let canonical = std::fs::canonicalize(&resolved)
        .map_err(|e| format!("Cannot canonicalize {}: {}", resolved.display(), e))?;
    if !canonical.starts_with(&base) {
        return Err(format!("Resolved path escapes media directory: {}", canonical.display()));
    }
    Ok(canonical)
}

pub fn determine_image_type(image_name: &str) -> String {
    let lower_name = image_name.to_lowercase();
    if lower_name.contains("dcim/camera") {
        media::TYPE_CAMERA.to_string()
    } else if lower_name.contains("whatsapp") {
        media::TYPE_WHATSAPP.to_string()
    } else if lower_name.contains("screenshot") {
        media::TYPE_SCREENSHOT.to_string()
    } else {
        media::TYPE_OTHER.to_string()
    }
}

pub fn determine_video_type(video_name: &str) -> String {
    let lower_name = video_name.to_lowercase();
    if lower_name.contains("dcim/camera") || lower_name.contains("dji") {
        media::TYPE_CAMERA.to_string()
    } else if lower_name.contains("whatsapp") {
        media::TYPE_WHATSAPP.to_string()
    } else if lower_name.contains("screen") {
        media::TYPE_SCREEN_RECORDING.to_string()
    } else {
        media::TYPE_OTHER.to_string()
    }
}

// ---- Existence Check --------------------------------------------------------

#[derive(serde::Serialize)]
pub struct ExistenceCheckResult {
    pub exists_for_user: bool,
    pub exists_verified: bool,
}

pub async fn check_if_exists(
    hash: &str,
    user_id: &uuid::Uuid,
    table: &str,
    pool: web::Data<MainDbPool>,
) -> Result<ExistenceCheckResult, String> {
    // Pool exhaustion under load used to panic here; degrade to an error
    // response instead of tearing down the request task.
    let client = pool.0.get().await.map_err(|e| {
        error!("Failed to get database client in check_if_exists: {}", e);
        format!("database pool unavailable: {}", e)
    })?;

    let query_string = match table {
        "images" => "
            SELECT
                EXISTS(SELECT 1 FROM images WHERE user_id = $1 AND hash = $2 AND deleted_at IS NULL) as exists_for_user,
                EXISTS(SELECT 1 FROM images WHERE user_id = $1 AND hash = $2 AND verification_status = 1 AND deleted_at IS NULL) as exists_verified
        ",
        "videos" => "
            SELECT
                EXISTS(SELECT 1 FROM videos WHERE user_id = $1 AND hash = $2 AND deleted_at IS NULL) as exists_for_user,
                EXISTS(SELECT 1 FROM videos WHERE user_id = $1 AND hash = $2 AND verification_status = 1 AND deleted_at IS NULL) as exists_verified
        ",
        _ => {
            warn!("Invalid table name provided to check_if_exists: {}", table);
            return Err(format!("Invalid table name provided to check_if_exists: {}", table));
        }
    };

    let row = client.query_one(query_string, &[user_id, &hash])
        .await
        .map_err(|e| e.to_string())?;
    Ok(ExistenceCheckResult {
        exists_for_user: row.get(0),
        exists_verified: row.get(1),
    })
}

// ---- Thumbnail Listing ------------------------------------------------------

#[allow(clippy::too_many_arguments)]
pub async fn list_thumbnails(
    user_id: &str,
    device_id: Option<&str>,
    table: &str,
    media_type: &str,
    offset: usize,
    limit: usize,
    starred_only: bool,
    start_date: Option<&str>,
    end_date: Option<&str>,
    location_lat: Option<f64>,
    location_lon: Option<f64>,
    location_radius_km: Option<f64>,
    label_id: Option<i32>,
    apply_user_id_filter: bool,
    no_exif: bool,
    sort_by: Option<&str>,
    sort_order: Option<&str>,
    pool: &web::Data<MainDbPool>,
) -> Result<Vec<ThumbnailItem>, Box<dyn std::error::Error>> {
    let client = pool.0.get().await.map_err(|e| {
        error!("Failed to get database client for list_thumbnails: {}", e);
        Box::new(e) as Box<dyn std::error::Error>
    })?;

    let user_uuid = uuid::Uuid::parse_str(user_id).map_err(|e| {
        error!("Failed to parse user_id as UUID: {}", e);
        Box::new(e) as Box<dyn std::error::Error>
    })?;

    let apply_filters = |builder: &mut MediaQueryBuilder| {
        builder.with_user_id();
        if apply_user_id_filter {
            builder.with_user_id_filter();
        }
        if device_id.is_some() {
            builder.with_device_id();
        }
        // The `type` column is unpopulated, so media type must be scoped by table:
        // `all` keeps both tables, `image`/`video` keep only their own and zero out
        // the other (postgres `type` column cannot be trusted for this filter).
        let wrong_table = (media_type == media::TYPE_IMAGE && !builder.is_images_table())
            || (media_type == media::TYPE_VIDEO && !builder.is_videos_table());
        if wrong_table {
            builder.add_custom_condition("1 = 0".to_string());
        }
        if starred_only {
            builder.with_starred_only();
        }
        if label_id.is_some() {
            builder.with_label_id();
        }
        if start_date.is_some() {
            builder.with_start_date();
        }
        if end_date.is_some() {
            builder.with_end_date();
        }
        if no_exif && builder.is_images_table() {
            builder.with_no_exif();
        }
    };

    let query_string;
    let has_location_filter = location_lat.is_some() && location_lon.is_some();
    let limit_param;
    let offset_param;
    let mut lon_param_idx = None;
    let mut lat_param_idx = None;

    if table == "all" {
        let mut img_builder = MediaQueryBuilder::new("images");
        apply_filters(&mut img_builder);

        if has_location_filter {
            let radius_km = location_radius_km.unwrap_or(10.0);
            let radius_meters = radius_km * 1000.0;
            let lon_param = img_builder.param_count() + 1;
            let lat_param = img_builder.param_count() + 2;
            img_builder.add_custom_condition("t.location IS NOT NULL".to_string());
            img_builder.add_custom_condition(format!(
                "ST_DWithin(t.location, ST_MakePoint(${}, ${})::geography, {})",
                lon_param, lat_param, radius_meters
            ));
            lon_param_idx = Some(lon_param);
            lat_param_idx = Some(lat_param);
        }

        let mut vid_builder = MediaQueryBuilder::new("videos");
        apply_filters(&mut vid_builder);
        if has_location_filter {
            vid_builder.add_custom_condition("1 = 0".to_string());
        }

        let max_param =
            img_builder.param_count() + (if has_location_filter { 2 } else { 0 });
        limit_param = max_param + 1;
        offset_param = max_param + 2;

        let img_body = img_builder.build_select_body(lon_param_idx, lat_param_idx);
        let vid_body = vid_builder.build_select_body(None, None);

        let dir = if sort_order == Some("asc") { "ASC" } else { "DESC" };
        let order_clause = if sort_by == Some("size") {
            format!("ORDER BY file_size_bytes {} NULLS LAST, hash {}", dir, dir)
        } else if sort_by == Some("quality") {
            format!("ORDER BY aesthetic_score {} NULLS LAST, hash {}", dir, dir)
        } else {
            format!("ORDER BY created_at {}, hash {}", dir, dir)
        };

        query_string = format!(
            "SELECT * FROM (\
                SELECT DISTINCT ON (hash) hash, name, created_at, place, deviceid, starred, \
                    distance_km, media_type, file_size_bytes, aesthetic_score \
                FROM ({} UNION ALL {}) combined \
                ORDER BY hash, aesthetic_score DESC NULLS LAST\
            ) deduped {} LIMIT ${} OFFSET ${}",
            img_body, vid_body, order_clause, limit_param, offset_param
        );
    } else {
        let mut builder = MediaQueryBuilder::new(table);
        apply_filters(&mut builder);

        if has_location_filter && table == "images" {
            let radius_km = location_radius_km.unwrap_or(10.0);
            let radius_meters = radius_km * 1000.0;
            let lon_param = builder.param_count() + 1;
            let lat_param = builder.param_count() + 2;
            builder.add_custom_condition("t.location IS NOT NULL".to_string());
            builder.add_custom_condition(format!(
                "ST_DWithin(t.location, ST_MakePoint(${}, ${})::geography, {})",
                lon_param, lat_param, radius_meters
            ));
            lon_param_idx = Some(lon_param);
            lat_param_idx = Some(lat_param);
        }

        limit_param = builder.param_count()
            + 1
            + (if has_location_filter && table == "images" { 2 } else { 0 });
        offset_param = builder.param_count()
            + 2
            + (if has_location_filter && table == "images" { 2 } else { 0 });

        query_string = builder.build_select_query(
            limit_param,
            offset_param,
            lon_param_idx,
            lat_param_idx,
            sort_by,
            sort_order,
        );
    }

    let limit_i64 = limit as i64;
    let offset_i64 = offset as i64;

    use chrono::TimeZone;
    let start_datetime: Option<DateTime<Utc>> = start_date.and_then(|d| {
        NaiveDate::parse_from_str(d, "%Y-%m-%d")
            .ok()
            .and_then(|nd| nd.and_hms_opt(0, 0, 0))
            .map(|ndt| Utc.from_utc_datetime(&ndt))
    });
    let end_datetime: Option<DateTime<Utc>> = end_date.and_then(|d| {
        NaiveDate::parse_from_str(d, "%Y-%m-%d")
            .ok()
            .and_then(|nd| nd.and_hms_opt(23, 59, 59))
            .and_then(|ndt| ndt.checked_add_signed(chrono::Duration::seconds(1)))
            .map(|ndt| Utc.from_utc_datetime(&ndt))
    });

    let device_id_value;
    let label_id_value;
    let mut params: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = vec![&user_uuid];

    if let Some(dev_id) = device_id {
        device_id_value = dev_id;
        params.push(&device_id_value as &(dyn tokio_postgres::types::ToSql + Sync));
    }

    if let Some(lbl_id) = label_id {
        label_id_value = lbl_id;
        params.push(&label_id_value as &(dyn tokio_postgres::types::ToSql + Sync));
    }
    if let Some(ref sd) = start_datetime {
        params.push(sd as &(dyn tokio_postgres::types::ToSql + Sync));
    }
    if let Some(ref ed) = end_datetime {
        params.push(ed as &(dyn tokio_postgres::types::ToSql + Sync));
    }

    let lat_value;
    let lon_value;
    if lon_param_idx.is_some() {
        lat_value = location_lat.expect("lon_param_idx is only set when location_lat is Some");
        lon_value = location_lon.expect("lon_param_idx is only set when location_lon is Some");
        params.push(&lon_value as &(dyn tokio_postgres::types::ToSql + Sync));
        params.push(&lat_value as &(dyn tokio_postgres::types::ToSql + Sync));
    }

    params.push(&limit_i64 as &(dyn tokio_postgres::types::ToSql + Sync));
    params.push(&offset_i64 as &(dyn tokio_postgres::types::ToSql + Sync));

    let rows = client.query(&query_string, &params).await?;

    let thumbnails = rows
        .into_iter()
        .map(|row| {
            let distance_km: Option<f64> = row.get("distance_km");
            let media_type_val: Option<String> = row.try_get("media_type").ok();
            let final_media_type = media_type_val.or_else(|| {
                if table == "images" {
                    Some("image".to_string())
                } else if table == "videos" {
                    Some("video".to_string())
                } else {
                    None
                }
            });
            let hash: String = row.get("hash");
            ThumbnailItem {
                hash: hash.clone(),
                name: row.get("name"),
                created_at: row.get("created_at"),
                place: row.get("place"),
                device_id: row.get("deviceid"),
                starred: row.get("starred"),
                distance_km: distance_km.map(|d| d as f32),
                media_type: final_media_type,
                thumbnail_url: format!("/api/thumbnail/{}", hash),
                file_size_bytes: row.try_get("file_size_bytes").unwrap_or(None),
                aesthetic_score: row.try_get("aesthetic_score").unwrap_or(None),
            }
        })
        .collect();

    Ok(thumbnails)
}

#[allow(clippy::too_many_arguments)]
pub async fn total_thumbnails(
    user_id: &str,
    device_id: Option<&str>,
    table: &str,
    media_type: &str,
    starred_only: bool,
    start_date: Option<&str>,
    end_date: Option<&str>,
    location_lat: Option<f64>,
    location_lon: Option<f64>,
    location_radius_km: Option<f64>,
    label_id: Option<i32>,
    apply_user_id_filter: bool,
    no_exif: bool,
    pool: &web::Data<MainDbPool>,
) -> i64 {
    let client = match pool.0.get().await {
        Ok(c) => c,
        Err(e) => {
            error!("Failed to get database client for total_thumbnails: {}", e);
            return 0;
        }
    };

    let user_uuid = match uuid::Uuid::parse_str(user_id) {
        Ok(u) => u,
        Err(e) => {
            error!("Failed to parse user_id as UUID in total_thumbnails: {}", e);
            return 0;
        }
    };

    let apply_filters = |builder: &mut MediaQueryBuilder| {
        builder.with_has_thumbnail();
        builder.with_user_id();
        if apply_user_id_filter {
            builder.with_user_id_filter();
        }
        if device_id.is_some() {
            builder.with_device_id();
        }
        // The `type` column is unpopulated, so media type must be scoped by table:
        // `all` keeps both tables, `image`/`video` keep only their own and zero out
        // the other (postgres `type` column cannot be trusted for this filter).
        let wrong_table = (media_type == media::TYPE_IMAGE && !builder.is_images_table())
            || (media_type == media::TYPE_VIDEO && !builder.is_videos_table());
        if wrong_table {
            builder.add_custom_condition("1 = 0".to_string());
        }
        if starred_only {
            builder.with_starred_only();
        }
        if label_id.is_some() {
            builder.with_label_id();
        }
        if start_date.is_some() {
            builder.with_start_date();
        }
        if end_date.is_some() {
            builder.with_end_date();
        }
        if no_exif && builder.is_images_table() {
            builder.with_no_exif();
        }
    };

    let query_string;
    let has_location_filter = location_lat.is_some() && location_lon.is_some();
    let mut lon_param_idx = None;

    if table == "all" {
        let mut img_builder = MediaQueryBuilder::new("images");
        apply_filters(&mut img_builder);

        if has_location_filter {
            let radius_km = location_radius_km.unwrap_or(10.0);
            let radius_meters = radius_km * 1000.0;
            let lon_param = img_builder.param_count() + 1;
            let lat_param = img_builder.param_count() + 2;
            img_builder.add_custom_condition("t.location IS NOT NULL".to_string());
            img_builder.add_custom_condition(format!(
                "ST_DWithin(t.location, ST_MakePoint(${}, ${})::geography, {})",
                lon_param, lat_param, radius_meters
            ));
            lon_param_idx = Some(lon_param);
        }

        let mut vid_builder = MediaQueryBuilder::new("videos");
        apply_filters(&mut vid_builder);
        if has_location_filter {
            vid_builder.add_custom_condition("1 = 0".to_string());
        }

        let img_count_query = img_builder.build_count_query(starred_only);
        let vid_count_query = vid_builder.build_count_query(starred_only);
        query_string = format!("SELECT ({}) + ({})", img_count_query, vid_count_query);
    } else {
        let mut builder = MediaQueryBuilder::new(table);
        apply_filters(&mut builder);

        if has_location_filter && table == "images" {
            let radius_km = location_radius_km.unwrap_or(10.0);
            let radius_meters = radius_km * 1000.0;
            let lon_param = builder.param_count() + 1;
            let lat_param = builder.param_count() + 2;
            builder.add_custom_condition("t.location IS NOT NULL".to_string());
            builder.add_custom_condition(format!(
                "ST_DWithin(t.location, ST_MakePoint(${}, ${})::geography, {})",
                lon_param, lat_param, radius_meters
            ));
            lon_param_idx = Some(lon_param);
        }

        query_string = builder.build_count_query(starred_only);
    }

    use chrono::TimeZone;
    let start_datetime: Option<DateTime<Utc>> = start_date.and_then(|d| {
        NaiveDate::parse_from_str(d, "%Y-%m-%d")
            .ok()
            .and_then(|nd| nd.and_hms_opt(0, 0, 0))
            .map(|ndt| Utc.from_utc_datetime(&ndt))
    });
    let end_datetime: Option<DateTime<Utc>> = end_date.and_then(|d| {
        NaiveDate::parse_from_str(d, "%Y-%m-%d")
            .ok()
            .and_then(|nd| nd.and_hms_opt(23, 59, 59))
            .and_then(|ndt| ndt.checked_add_signed(chrono::Duration::seconds(1)))
            .map(|ndt| Utc.from_utc_datetime(&ndt))
    });

    let device_id_value;
    let label_id_value;
    let mut params: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = vec![&user_uuid];

    if let Some(dev_id) = device_id {
        device_id_value = dev_id;
        params.push(&device_id_value as &(dyn tokio_postgres::types::ToSql + Sync));
    }

    if let Some(lbl_id) = label_id {
        label_id_value = lbl_id;
        params.push(&label_id_value as &(dyn tokio_postgres::types::ToSql + Sync));
    }
    if let Some(ref sd) = start_datetime {
        params.push(sd as &(dyn tokio_postgres::types::ToSql + Sync));
    }
    if let Some(ref ed) = end_datetime {
        params.push(ed as &(dyn tokio_postgres::types::ToSql + Sync));
    }

    let lat_value;
    let lon_value;
    if lon_param_idx.is_some() {
        lat_value = location_lat.expect("lon_param_idx is only set when location_lat is Some");
        lon_value = location_lon.expect("lon_param_idx is only set when location_lon is Some");
        params.push(&lon_value as &(dyn tokio_postgres::types::ToSql + Sync));
        params.push(&lat_value as &(dyn tokio_postgres::types::ToSql + Sync));
    }

    let row = match client.query_one(&query_string, &params).await {
        Ok(r) => r,
        Err(e) => {
            error!("Failed to get total count: {}", e);
            return 0;
        }
    };

    let total: i64 = row.get(0);
    if let Some(dev_id) = device_id {
        info!("Total thumbnails for device {}: {}", dev_id, total);
    } else {
        info!("Total thumbnails (all devices): {}", total);
    }
    total
}

// ---- Date Parsing -----------------------------------------------------------

fn try_parse_datetime_underscore(name: &str, start_pos: usize) -> Option<DateTime<Utc>> {
    if name.len() < start_pos + 15 {
        return None;
    }
    if name.chars().nth(start_pos + 8) != Some('_') {
        return None;
    }
    let date_part = name.get(start_pos..start_pos + 8)?;
    let time_part = name.get(start_pos + 9..start_pos + 15)?;
    if !date_part.chars().all(|c| c.is_ascii_digit())
        || !time_part.chars().all(|c| c.is_ascii_digit())
    {
        return None;
    }
    let datetime_str = format!("{} {}", date_part, time_part);
    NaiveDateTime::parse_from_str(&datetime_str, "%Y%m%d %H%M%S")
        .ok()
        .map(|dt| DateTime::<Utc>::from_naive_utc_and_offset(dt, Utc))
}

fn try_parse_whatsapp_format(name: &str, start_pos: usize) -> Option<DateTime<Utc>> {
    let date_end = start_pos + 8;
    if name.len() < date_end + 7 || name.get(date_end..date_end + 3) != Some("-wa") {
        return None;
    }
    let date_part = name.get(start_pos..date_end)?;
    let millis_part = name.get(date_end + 3..date_end + 7)?;
    if !date_part.chars().all(|c| c.is_ascii_digit())
        || !millis_part.chars().all(|c| c.is_ascii_digit())
    {
        return None;
    }
    let naive_date = NaiveDate::parse_from_str(date_part, "%Y%m%d").ok()?;
    let millis = millis_part.parse::<u32>().ok()?;
    let actual_millis = millis % 1000;
    Some(DateTime::<Utc>::from_naive_utc_and_offset(
        naive_date.and_hms_milli_opt(0, 0, 0, actual_millis)?,
        Utc,
    ))
}

fn try_parse_date_only(name: &str, start_pos: usize) -> Option<DateTime<Utc>> {
    let date_part = name.get(start_pos..start_pos + 8)?;
    if !date_part.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    NaiveDate::parse_from_str(date_part, "%Y%m%d")
        .ok()
        .map(|date| {
            DateTime::<Utc>::from_naive_utc_and_offset(
                date.and_hms_opt(0, 0, 0).unwrap(),
                Utc,
            )
        })
}

pub fn parse_date_from_image_name(image_name: &str) -> Option<DateTime<Utc>> {
    let lower_name = image_name.to_lowercase();

    if let Some(pos) = lower_name.find("img_") {
        if let Some(dt) = try_parse_datetime_underscore(&lower_name, pos + 4) {
            return Some(dt);
        }
    }
    if let Some(pos) = lower_name.find("img-") {
        if let Some(dt) = try_parse_whatsapp_format(&lower_name, pos + 4) {
            return Some(dt);
        }
        if let Some(dt) = try_parse_date_only(&lower_name, pos + 4) {
            return Some(dt);
        }
    }

    None
}

pub fn parse_date_from_video_name(video_name: &str) -> Option<DateTime<Utc>> {
    let lower_name = video_name.to_lowercase();

    if let Some(pos) = lower_name.find("dji_") {
        if let Some(dt) = try_parse_datetime_underscore(&lower_name, pos + 4) {
            return Some(dt);
        }
    }
    if let Some(pos) = lower_name.find("sl_mo_vid_") {
        if let Some(dt) = try_parse_datetime_underscore(&lower_name, pos + 10) {
            return Some(dt);
        }
    }
    if let Some(pos) = lower_name.find("vid_") {
        if let Some(dt) = try_parse_datetime_underscore(&lower_name, pos + 4) {
            return Some(dt);
        }
    }
    if let Some(pos) = lower_name.find("vid-") {
        if let Some(dt) = try_parse_whatsapp_format(&lower_name, pos + 4) {
            return Some(dt);
        }
        if let Some(dt) = try_parse_date_only(&lower_name, pos + 4) {
            return Some(dt);
        }
    }

    None
}

// ---- Temp File Cleanup ------------------------------------------------------

pub async fn cleanup_temp_files(
    image_temp_path: Option<PathBuf>,
    thumbnail_temp_path: Option<PathBuf>,
) {
    if let Some(path) = image_temp_path {
        if path.exists() {
            if let Err(e) = fs::remove_file(&path).await {
                warn!("Failed to remove temporary image file {:?}: {}", path, e);
            }
        }
    }
    if let Some(path) = thumbnail_temp_path {
        if path.exists() {
            if let Err(e) = fs::remove_file(&path).await {
                warn!("Failed to remove temporary thumbnail file {:?}: {}", path, e);
            }
        }
    }
}

/// Fire-and-forget version of `cleanup_temp_files` for use in error paths.
pub fn cleanup_temp_files_spawn(
    image_temp_path: Option<PathBuf>,
    thumbnail_temp_path: Option<PathBuf>,
) {
    tokio::spawn(async move {
        cleanup_temp_files(image_temp_path, thumbnail_temp_path).await;
    });
}

/// Extract video keyframe JPEG images at regular intervals using ffmpeg.
/// Returns a list of (timestamp_secs, jpeg_bytes) tuples.
pub async fn extract_video_keyframes(
    video_path: &std::path::Path,
    interval_secs: f32,
    max_keyframes: usize,
) -> Result<Vec<(f32, Vec<u8>)>, String> {
    let video_str = video_path.to_str().ok_or("Invalid video path")?;
    let temp_dir = std::env::temp_dir().join(format!("keyframes_{}", uuid::Uuid::new_v4()));
    tokio::fs::create_dir_all(&temp_dir)
        .await
        .map_err(|e| format!("Failed to create temp keyframes dir: {}", e))?;

    let output_pattern = temp_dir.join("frame_%03d.jpg");
    let output_pattern_str = output_pattern.to_str().ok_or("Invalid output pattern path")?;

    let fps_filter = format!("fps=1/{}", interval_secs);
    // A hung/corrupt video must not pin an AI task (and its DB client) forever:
    // 5-minute ceiling, child killed on expiry.
    const FFMPEG_TIMEOUT: Duration = Duration::from_secs(300);
    let result = tokio::time::timeout(
        FFMPEG_TIMEOUT,
        tokio::process::Command::new("ffmpeg")
            .args([
                "-i", video_str,
                "-vf", &fps_filter,
                "-vframes", &max_keyframes.to_string(),
                "-q:v", "3",
                "-y",
                output_pattern_str,
            ])
            .kill_on_drop(true)
            .output(),
    )
    .await;

    let result = match result {
        Ok(r) => r,
        Err(_) => {
            let _ = tokio::fs::remove_dir_all(&temp_dir).await;
            return Err(format!("ffmpeg keyframe extraction timed out after {}s", FFMPEG_TIMEOUT.as_secs()));
        }
    };

    match result {
        Ok(output) if output.status.success() => {},
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let _ = tokio::fs::remove_dir_all(&temp_dir).await;
            return Err(format!("ffmpeg keyframe extraction failed: {}", stderr));
        }
        Err(e) => {
            let _ = tokio::fs::remove_dir_all(&temp_dir).await;
            return Err(format!("Failed to run ffmpeg: {}", e));
        }
    }

    let mut keyframes = Vec::new();
    let mut entries = tokio::fs::read_dir(&temp_dir)
        .await
        .map_err(|e| format!("Failed to read temp keyframes dir: {}", e))?;

    let mut frame_files = Vec::new();
    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) == Some("jpg") {
            frame_files.push(path);
        }
    }
    frame_files.sort();

    for (idx, path) in frame_files.into_iter().enumerate() {
        if idx >= max_keyframes {
            break;
        }
        let timestamp = (idx as f32) * interval_secs;
        if let Ok(bytes) = tokio::fs::read(&path).await {
            keyframes.push((timestamp, bytes));
        }
    }

    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
    info!("Extracted {} keyframes from video {:?}", keyframes.len(), video_path);
    Ok(keyframes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_hash_validation() {
        let ok = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        assert!(is_valid_content_hash(ok));
        assert!(!is_valid_content_hash(""));
        assert!(!is_valid_content_hash(&ok[..63]), "too short rejected");
        assert!(!is_valid_content_hash(&ok.to_uppercase()), "uppercase rejected");
        assert!(
            !is_valid_content_hash("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz"),
            "non-hex chars rejected"
        );
    }

    #[test]
    fn media_ext_validation() {
        assert!(is_valid_media_ext("jpg"));
        assert!(is_valid_media_ext("MP4"));
        assert!(!is_valid_media_ext(""));
        assert!(!is_valid_media_ext("a/b"), "path separator rejected");
        assert!(!is_valid_media_ext(".."), "dotdot rejected");
        assert!(!is_valid_media_ext(".hidden"), "leading dot rejected");
        assert!(!is_valid_media_ext("superlongext"), "too long rejected");
        assert!(!is_valid_media_ext("a b"), "whitespace rejected");
    }

    #[test]
    fn safe_resolve_rejects_traversal_and_missing_files() {
        assert!(safe_resolve_content_path("/tmp", "..", "jpg").is_err(), "traversal hash rejected");

        let valid_hash = "a".repeat(64);
        assert!(
            safe_resolve_content_path("/tmp", &valid_hash, "../x").is_err(),
            "traversal ext rejected"
        );
        assert!(
            safe_resolve_content_path("/tmp", &valid_hash, "jpg").is_err(),
            "nonexistent file must not silently succeed"
        );
    }

    #[test]
    fn subdirectory_path_uses_first_two_chars() {
        assert!(get_subdirectory_path("/base", "ab1234").ends_with("ab"));
        assert_eq!(get_subdirectory_path("/base", "a"), std::path::PathBuf::from("/base"));
    }

    #[test]
    fn parse_image_name_dates() {
        let dt = parse_date_from_image_name("IMG_20231222_191241.jpg").expect("camera-style name");
        assert_eq!(
            dt.to_rfc3339(),
            "2023-12-22T19:12:41+00:00",
            "IMG_YYYYMMDD_HHMMSS parses to a UTC datetime"
        );
        assert!(parse_date_from_image_name("IMG-20240115-WA0000.jpg").is_some(), "WhatsApp format");
        assert!(parse_date_from_image_name("IMG-20240115.jpg").is_some(), "date-only format");
        assert!(parse_date_from_image_name("photo.png").is_none());
        assert!(parse_date_from_image_name("VID_20240101_000000.mp4").is_none(), "video names excluded");
    }

    #[test]
    fn determines_media_type_from_name() {
        assert_eq!(determine_image_type("/storage/emulated/0/DCIM/Camera/IMG_1.jpg"), media::TYPE_CAMERA);
        assert_eq!(determine_image_type("WhatsApp Image x.jpg"), media::TYPE_WHATSAPP);
        assert_eq!(determine_image_type("Screenshot_1.png"), media::TYPE_SCREENSHOT);
        assert_eq!(determine_image_type("photo.png"), media::TYPE_OTHER);

        assert_eq!(determine_video_type("/DCIM/Camera/VID_1.mp4"), media::TYPE_CAMERA);
        assert_eq!(determine_video_type("DJI_0001.mp4"), media::TYPE_CAMERA);
        assert_eq!(determine_video_type("WhatsApp Video.mp4"), media::TYPE_WHATSAPP);
        assert_eq!(determine_video_type("screen recording.mp4"), media::TYPE_SCREEN_RECORDING);
        assert_eq!(determine_video_type("clip.mp4"), media::TYPE_OTHER);
    }

    #[test]
    fn parses_video_name_dates() {
        let dt = parse_date_from_video_name("VID_20240220_101530.mp4").expect("camera video");
        assert_eq!(dt.to_rfc3339(), "2024-02-20T10:15:30+00:00");
        assert!(parse_date_from_video_name("IMG_20240101_000000.jpg").is_none(), "image names excluded");
        assert!(parse_date_from_video_name("clip.mp4").is_none());
    }

    #[test]
    fn orient_image_to_jpeg_rejects_garbage() {
        assert!(orient_image_to_jpeg(b"not an image", 6).is_err());
    }

    #[test]
    fn rotate_png_rejects_garbage() {
        assert!(rotate_png_bytes(b"not a png", 6).is_none());
    }

    #[test]
    fn exif_orientation_round_trip() {
        // Not a JPEG -> no orientation, inject returns input unchanged.
        assert_eq!(read_exif_orientation_from_bytes(b"garbage"), None);
        let unchanged = inject_exif_orientation(b"garbage", 6);
        assert_eq!(unchanged, b"garbage".to_vec());

        // Minimal JPEG: SOI + a JFIF APP0 segment + EOI.
        let jpeg: Vec<u8> = vec![
            0xFF, 0xD8,
            0xFF, 0xE0, 0x00, 0x10, b'J', b'F', b'I', b'F', 0x00, 0x01, 0x01, 0x00, 0x00, 0x01, 0x00, 0x01,
            0xFF, 0xD9,
        ];

        // Inject orientation 6, then read it back.
        let out = inject_exif_orientation(&jpeg, 6);
        assert!(out.len() > jpeg.len(), "APP1 block appended");
        assert_eq!(&out[6..12], b"Exif\0\0");
        assert_eq!(read_exif_orientation_from_bytes(&out), Some(6), "round-trip orientation");

        // Already contains EXIF APP1 -> returned byte-for-byte unchanged.
        let again = inject_exif_orientation(&out, 8);
        assert_eq!(again, out);
    }

    #[actix_web::test]
    async fn hash_file_is_deterministic_64_hex() {
        let dir = std::env::temp_dir().join(format!("reminisce_hash_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("f.bin");
        std::fs::write(&p, b"hello coverage bytes").unwrap();

        let h1 = hash_file_blake3(&p).await.expect("hash ok");
        let h2 = hash_file_blake3(&p).await.expect("hash ok");
        assert_eq!(h1.len(), 64);
        assert!(h1.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_eq!(h1, h2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[actix_web::test]
    async fn extract_video_keyframes_reports_missing_input() {
        let missing = std::env::temp_dir().join(format!("no_such_video_{}.mp4", std::process::id()));
        let res = extract_video_keyframes(&missing, 1.0, 3).await;
        assert!(res.is_err(), "missing input must be an error, got {:?}", res);
    }

}


#[cfg(test)]
mod exif_orientation_tests {
    use super::*;

    fn jpeg_with_exif(entries: &[u8], value_area: &[u8]) -> Vec<u8> {
        // Minimal II-endian TIFF IFD0 with the given raw entry bytes.
        let mut tiff: Vec<u8> = vec![];
        tiff.extend_from_slice(b"II");
        tiff.extend_from_slice(&42u16.to_le_bytes());
        tiff.extend_from_slice(&8u32.to_le_bytes()); // IFD0 at 8
        tiff.extend_from_slice(&((entries.len() / 12) as u16).to_le_bytes());
        tiff.extend_from_slice(entries);
        tiff.extend_from_slice(&0u32.to_le_bytes()); // next IFD
        tiff.extend_from_slice(value_area);

        let mut seg: Vec<u8> = Vec::new();
        seg.extend_from_slice(b"Exif\x00\x00");
        seg.extend_from_slice(&tiff);
        let mut out: Vec<u8> = vec![0xFF, 0xD8];
        out.extend_from_slice(&[0xFF, 0xE1]);
        // Segment length counts itself plus the payload.
        out.extend_from_slice(&((seg.len() + 2) as u16).to_be_bytes());
        out.extend_from_slice(&seg);
        out.extend_from_slice(&[0xFF, 0xDA, 0x00, 0x02]); // SOS (plausibility)
        out.extend_from_slice(&[0xFF, 0xD9]); // EOI
        out
    }

    fn minimal_jpeg() -> Vec<u8> {
        // SOI + DQT + SOS + EOI: structurally plausible enough for the
        // plausibility guard (reaches Start-of-Scan).
        vec![0xFF, 0xD8, 0xFF, 0xDB, 0x00, 0x02, 0x00, 0xFF, 0xDA, 0x00, 0x02, 0xFF, 0xD9]
    }

    #[test]
    fn jpeg_without_app1_gets_minimal_tag() {
        let jpg = minimal_jpeg();
        let out = ensure_exif_orientation(&jpg, 6);
        assert_eq!(read_exif_orientation_from_bytes(&out), Some(6));
    }

    #[test]
    fn app1_without_orientation_entry_splices_one_in() {
        // One Make (ASCII, 5B, out-of-line) entry.
        let mut entries: Vec<u8> = Vec::new();
        entries.extend_from_slice(&0x010Fu16.to_le_bytes()); // Make
        entries.extend_from_slice(&2u16.to_le_bytes());      // ASCII
        entries.extend_from_slice(&5u32.to_le_bytes());      // count
        entries.extend_from_slice(&26u32.to_le_bytes());     // offset -> just past IFD block
        let jpg = jpeg_with_exif(&entries, b"Test\x00");
        assert_eq!(read_exif_orientation_from_bytes(&jpg), None);

        let out = ensure_exif_orientation(&jpg, 6);
        assert_eq!(out.len(), jpg.len() + 12);
        assert_eq!(read_exif_orientation_from_bytes(&out), Some(6));
        // The pre-existing Make field must survive the splice.
        let cursor = std::io::Cursor::new(&out);
        let exif = kamadak_exif::Reader::new()
            .read_from_container(&mut std::io::BufReader::new(cursor))
            .expect("spliced file must still parse as EXIF");
        let make = exif.get_field(kamadak_exif::Tag::Make, kamadak_exif::In::PRIMARY)
            .and_then(|f| match &f.value {
                kamadak_exif::Value::Ascii(v) => Some(String::from_utf8_lossy(&v.iter().flatten().cloned().collect::<Vec<u8>>()).into_owned()),
                _ => None,
            })
            .expect("Make must survive");
        assert!(make.starts_with("Test"));
    }

    #[test]
    fn app1_with_matching_tag_is_byte_identical() {
        let jpg = minimal_jpeg();
        let tagged = inject_exif_orientation(&jpg, 6);
        let out = ensure_exif_orientation(&tagged, 6);
        assert_eq!(out, tagged);
    }

    #[test]
    fn app1_with_different_tag_is_rewritten() {
        let jpg = minimal_jpeg();
        let tagged = inject_exif_orientation(&jpg, 1);
        assert_eq!(tagged.len(), jpg.len() + 36);
        assert!(tagged.windows(2).any(|w| w == [0xFF, 0xDA]), "fixture must keep SOS");
        let out = ensure_exif_orientation(&tagged, 8);
        assert_eq!(out.len(), tagged.len());
        assert_eq!(read_exif_orientation_from_bytes(&out), Some(8));
    }

    #[test]
    fn real_camera_jpg_case_a_rewrites_without_corruption() {
        // Real camera fixture (has IFD0 tags incl. an Exif sub-IFD pointer).
        // The serve path hits Case A here (tag exists): rewrite must be
        // in-place and must not disturb the sub-IFD chain.
        let data = std::fs::read("tests/test_image.jpg").expect("fixture");
        let loc = find_exif_app1(&data).expect("fixture has EXIF");
        let current = read_exif_orientation_from_bytes(&data);
        let target = if current == Some(6) { 8u16 } else { 6u16 };

        let out = super::splice_orientation_into_app1(&data, loc, target)
            .expect("case-A rewrite must succeed");
        assert_eq!(out.len(), data.len(), "case-A must be in-place");
        assert_eq!(read_exif_orientation_from_bytes(&out), Some(target));

        let cursor = std::io::Cursor::new(&out);
        let exif = kamadak_exif::Reader::new()
            .read_from_container(&mut std::io::BufReader::new(cursor))
            .expect("rewritten camera file must still parse");
        assert!(exif.get_field(kamadak_exif::Tag::Make, kamadak_exif::In::PRIMARY).is_some());
        assert!(exif.get_field(kamadak_exif::Tag::DateTimeOriginal, kamadak_exif::In::PRIMARY).is_some(),
            "sub-IFD corrupted during case-A rewrite");
    }

    #[test]
    fn synthetic_sub_ifd_case_b_splice_keeps_chained_offsets() {
        // II-endian layout exercising the dangerous path: IFD0 holds
        //  - Make          (ASCII, out-of-line value AFTER the IFD block)
        //  - Exif sub-IFD  (inline LONG pointer to a table further down)
        // and the sub-IFD holds DateTimeOriginal (LONG, out-of-line).
        // Inserting Orientation shifts everything past IFD0 by 12 — every
        // chained offset above must move with it.
        // Layout: hdr(8) | 2 entries(24) | nextIFD(4) => values start @38.
        // Make value @38..43, sub-IFD table @43..61, its value @61..65.
        const MAKE_OFF: u32 = 38;
        const SUB_OFF: u32 = 43;
        const DTO_VAL_OFF: u32 = 61;

        let mut tiff: Vec<u8> = Vec::new();
        tiff.extend_from_slice(b"II");
        tiff.extend_from_slice(&42u16.to_le_bytes());
        tiff.extend_from_slice(&8u32.to_le_bytes());
        tiff.extend_from_slice(&2u16.to_le_bytes()); // IFD0 entries
        // Make: ASCII(2), count 5, out-of-line @26
        tiff.extend_from_slice(&0x010Fu16.to_le_bytes());
        tiff.extend_from_slice(&2u16.to_le_bytes());
        tiff.extend_from_slice(&5u32.to_le_bytes());
        tiff.extend_from_slice(&MAKE_OFF.to_le_bytes());
        // Exif sub-IFD root: LONG(4), count 1, inline offset
        tiff.extend_from_slice(&0x8769u16.to_le_bytes());
        tiff.extend_from_slice(&4u16.to_le_bytes());
        tiff.extend_from_slice(&1u32.to_le_bytes());
        tiff.extend_from_slice(&SUB_OFF.to_le_bytes());
        tiff.extend_from_slice(&0u32.to_be_bytes()); // next IFD (BE marker junk on purpose? no—zero)
        tiff.extend_from_slice(b"Test\x00");          // Make value
        while (tiff.len() as u32) < SUB_OFF {
            tiff.push(0);
        }

        // Sub-IFD table @SUB_OFF
        tiff.extend_from_slice(&1u16.to_le_bytes()); // 1 entry
        tiff.extend_from_slice(&0x9003u16.to_le_bytes()); // DateTimeOriginal
        tiff.extend_from_slice(&4u16.to_le_bytes()); // LONG
        tiff.extend_from_slice(&1u32.to_le_bytes()); // count
        tiff.extend_from_slice(&DTO_VAL_OFF.to_le_bytes()); // out-of-line
        tiff.extend_from_slice(&0u32.to_le_bytes()); // next IFD
        while (tiff.len() as u32) < DTO_VAL_OFF {
            tiff.push(0);
        }
        tiff.extend_from_slice(&1717171717u32.to_be_bytes()); // arbitrary datetime-ish value

        let mut jpg: Vec<u8> = vec![0xFF, 0xD8];
        jpg.extend_from_slice(&[0xFF, 0xE1]);
        jpg.extend_from_slice(&((6 + tiff.len() + 2) as u16).to_be_bytes());
        jpg.extend_from_slice(b"Exif\x00\x00");
        jpg.extend_from_slice(&tiff);
        jpg.extend_from_slice(&[0xFF, 0xDA, 0x00, 0x02, 0xFF, 0xD9]);

        // Sanity: parseable pre-splice, no Orientation yet.
        {
            let c = std::io::Cursor::new(&jpg);
            let exif = kamadak_exif::Reader::new()
                .read_from_container(&mut std::io::BufReader::new(c))
                .expect("fixture parses");
            assert!(read_exif_orientation_from_bytes(&jpg).is_none());
            assert!(exif.get_field(kamadak_exif::Tag::DateTimeOriginal, kamadak_exif::In::PRIMARY).is_some(),
                "fixture must carry a resolvable sub-IFD DateTimeOriginal");
        }

        let loc = find_exif_app1(&jpg).unwrap();
        let out = super::splice_orientation_into_app1(&jpg, loc, 6)
            .expect("sub-IFD splice must succeed");
        assert_eq!(out.len(), jpg.len() + 12);
        assert_eq!(read_exif_orientation_from_bytes(&out), Some(6));

        let c = std::io::Cursor::new(&out);
        let exif = kamadak_exif::Reader::new()
            .read_from_container(&mut std::io::BufReader::new(c))
            .expect("post-splice parse");
        let make = exif.get_field(kamadak_exif::Tag::Make, kamadak_exif::In::PRIMARY)
            .and_then(|f| match &f.value {
                kamadak_exif::Value::Ascii(v) => Some(String::from_utf8_lossy(&v.iter().flatten().cloned().collect::<Vec<u8>>()).into_owned()),
                _ => None,
            });
        assert_eq!(make.as_deref(), Some("Test"), "IFD0 out-of-line value corrupted");
        assert!(exif.get_field(kamadak_exif::Tag::DateTimeOriginal, kamadak_exif::In::PRIMARY).is_some(),
            "chained sub-IFD corrupted: DateTimeOriginal lost");
    }

    #[test]
    fn big_endian_mm_fixture_splices_and_reads_back() {
        // Minimal MM-endian TIFF: IFD0 with one out-of-line ASCII entry.
        let mut tiff: Vec<u8> = Vec::new();
        tiff.extend_from_slice(b"MM");
        tiff.extend_from_slice(&42u16.to_be_bytes());
        tiff.extend_from_slice(&8u32.to_be_bytes());
        tiff.extend_from_slice(&1u16.to_be_bytes()); // 1 entry
        tiff.extend_from_slice(&0x010Fu16.to_be_bytes()); // Make
        tiff.extend_from_slice(&2u16.to_be_bytes()); // ASCII
        tiff.extend_from_slice(&5u32.to_be_bytes());
        tiff.extend_from_slice(&26u32.to_be_bytes()); // value offset
        tiff.extend_from_slice(&0u32.to_be_bytes()); // next IFD
        tiff.extend_from_slice(b"Test\x00");

        let mut jpg: Vec<u8> = vec![0xFF, 0xD8];
        jpg.extend_from_slice(&[0xFF, 0xE1]);
        jpg.extend_from_slice(&((6 + tiff.len() + 2) as u16).to_be_bytes());
        jpg.extend_from_slice(b"Exif\x00\x00");
        jpg.extend_from_slice(&tiff);
        jpg.extend_from_slice(&[0xFF, 0xDA, 0x00, 0x02, 0xFF, 0xD9]);

        let out = ensure_exif_orientation(&jpg, 3);
        assert_eq!(out.len(), jpg.len() + 12);
        assert_eq!(read_exif_orientation_from_bytes(&out), Some(3));
        let cursor = std::io::Cursor::new(&out);
        let exif = kamadak_exif::Reader::new()
            .read_from_container(&mut std::io::BufReader::new(cursor))
            .expect("MM splice must still parse");
        let make = exif.get_field(kamadak_exif::Tag::Make, kamadak_exif::In::PRIMARY)
            .and_then(|f| match &f.value {
                kamadak_exif::Value::Ascii(v) => Some(String::from_utf8_lossy(&v.iter().flatten().cloned().collect::<Vec<u8>>()).into_owned()),
                _ => None,
            })
            .expect("Make must survive MM splice");
        assert!(make.starts_with("Test"));
    }
    #[test]
    fn malformed_input_returned_unchanged() {
        let bad = vec![0xFF, 0xD8, 0xFF, 0xE1, 0x00]; // truncated segment
        assert_eq!(ensure_exif_orientation(&bad, 6), bad);
        assert_eq!(ensure_exif_orientation(&[], 6), Vec::<u8>::new());
    }

    #[test]
    fn test_rotate_orientation_cycles() {
        use super::{rotate_orientation_cw, rotate_orientation_ccw, rotate_orientation_180};

        // None defaults to 1
        assert_eq!(rotate_orientation_cw(None), 6);
        assert_eq!(rotate_orientation_ccw(None), 8);
        assert_eq!(rotate_orientation_180(None), 3);

        // CW full cycle: 1 -> 6 -> 3 -> 8 -> 1
        assert_eq!(rotate_orientation_cw(Some(1)), 6);
        assert_eq!(rotate_orientation_cw(Some(6)), 3);
        assert_eq!(rotate_orientation_cw(Some(3)), 8);
        assert_eq!(rotate_orientation_cw(Some(8)), 1);

        // CCW full cycle: 1 -> 8 -> 3 -> 6 -> 1
        assert_eq!(rotate_orientation_ccw(Some(1)), 8);
        assert_eq!(rotate_orientation_ccw(Some(8)), 3);
        assert_eq!(rotate_orientation_ccw(Some(3)), 6);
        assert_eq!(rotate_orientation_ccw(Some(6)), 1);

        // 180 degrees
        assert_eq!(rotate_orientation_180(Some(1)), 3);
        assert_eq!(rotate_orientation_180(Some(3)), 1);
        assert_eq!(rotate_orientation_180(Some(6)), 8);
        assert_eq!(rotate_orientation_180(Some(8)), 6);
    }
}
