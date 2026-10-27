//! The cICP colorspace tag on written snapshot PNGs: every file the snapshot
//! machinery writes is tagged with the colorspace its bytes are referred to
//! (`SnapshotOptions::color_space`, default sRGB), and tagging never changes
//! the pixels.

#![cfg(feature = "snapshot")]

use std::fs;
use std::io::BufReader;
use std::path::Path;

use egui_kittest::{SnapshotColorSpace, SnapshotOptions, try_image_snapshot_options};
use image::RgbaImage;

/// A pattern with a saturated red/orange so an sRGB-vs-P3 misreading of the
/// dump would be visible to a human inspecting it.
fn test_image() -> RgbaImage {
    RgbaImage::from_fn(4, 2, |x, y| {
        image::Rgba([(x * 64) as u8, (y * 128) as u8, 255 - (x * 32) as u8, 255])
    })
}

/// The decoded contents of a snapshot PNG, reduced to what these tests assert.
#[derive(Debug, PartialEq, Eq)]
struct DecodedPng {
    /// The cICP code points (primaries, transfer, matrix, full-range), if tagged.
    cicp: Option<[u8; 4]>,
    pixels: Vec<u8>,
}

/// Run one snapshot attempt against a fresh directory and return the single
/// PNG it wrote (the `.new.png` actual in test mode, or the snapshot itself
/// when `UPDATE_SNAPSHOTS` is set in the environment).
fn write_snapshot_png(name: &str, options: &SnapshotOptions) -> DecodedPng {
    let dir = tempfile::tempdir().expect("failed to create a tempdir");
    let options = options.clone().output_path(dir.path());
    let _ = try_image_snapshot_options(&test_image(), name, &options);

    let mut written: Vec<_> = fs::read_dir(dir.path())
        .expect("failed to list the snapshot output dir")
        .map(|entry| entry.expect("readable directory entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "png"))
        .collect();
    assert_eq!(
        written.len(),
        1,
        "expected exactly one written PNG, got {written:?}"
    );

    decode_png(&written.remove(0))
}

/// Scan the raw PNG chunk stream for the cICP chunk and return its four
/// code-point bytes.
///
/// Deliberately does not trust the `png` decoder's parse: this asserts on the
/// exact bytes the writer emitted.
fn raw_cicp_chunk(bytes: &[u8]) -> Option<[u8; 4]> {
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n", "not a PNG file");
    let mut pos = 8; // past the signature: [len][type][data][crc] chunks
    loop {
        let header = bytes.get(pos..pos + 8).expect("truncated PNG chunk stream");
        let len = u32::from_be_bytes(header[..4].try_into().expect("4 length bytes")) as usize;
        let kind = &header[4..8];
        let data = bytes
            .get(pos + 8..pos + 8 + len)
            .expect("truncated PNG chunk data");
        if kind == b"cICP" {
            return Some(data.try_into().expect("cICP data must be 4 bytes"));
        }
        if kind == b"IEND" {
            return None;
        }
        pos += 8 + len + 4; // header + data + CRC
    }
}

fn decode_png(path: &Path) -> DecodedPng {
    let bytes = fs::read(path).expect("failed to read the written PNG");
    let cicp = raw_cicp_chunk(&bytes);
    let decoder = png::Decoder::new(BufReader::new(std::io::Cursor::new(&bytes)));
    let mut reader = decoder
        .read_info()
        .expect("failed to read the written PNG header");
    let mut buffer = vec![
        0;
        reader
            .output_buffer_size()
            .expect("png reports buffer size")
    ];
    let output = reader
        .next_frame(&mut buffer)
        .expect("failed to decode the written PNG");
    DecodedPng {
        cicp,
        pixels: buffer[..output.buffer_size()].to_vec(),
    }
}

#[track_caller]
fn assert_cicp(decoded: &DecodedPng, expected: [u8; 4]) {
    assert_eq!(decoded.cicp, Some(expected), "unexpected cICP code points");
}

#[test]
fn default_snapshot_is_tagged_srgb() {
    let decoded = write_snapshot_png("colorspace_default", &SnapshotOptions::default());
    // Hardcoded so a change to `SnapshotColorSpace::Srgb`'s constants can't
    // ship a lying tag with green tests.
    assert_cicp(&decoded, [1, 13, 0, 1]);
    assert_eq!(decoded.pixels, test_image().into_raw());
}

#[test]
fn display_p3_knob_tags_display_p3() {
    let options = SnapshotOptions::default().color_space(SnapshotColorSpace::DisplayP3);
    let decoded = write_snapshot_png("colorspace_p3", &options);
    assert_cicp(&decoded, [12, 13, 0, 1]);
    assert_eq!(decoded.pixels, test_image().into_raw());
}

#[test]
fn linear_srgb_knob_tags_linear() {
    let options = SnapshotOptions::default().color_space(SnapshotColorSpace::LinearSrgb);
    let decoded = write_snapshot_png("colorspace_linear", &options);
    assert_cicp(&decoded, [1, 8, 0, 1]);
    assert_eq!(decoded.pixels, test_image().into_raw());
}

/// The tag is metadata only: a tagged write decodes to exactly the pixels of
/// an untagged write of the same image.
#[test]
fn tagged_pixels_match_untagged_write() {
    let image = test_image();

    let tagged = write_snapshot_png("tagged", &SnapshotOptions::default());
    assert!(tagged.cicp.is_some());

    // Untagged write: same encoder, no cICP chunk.
    let dir = tempfile::tempdir().expect("failed to create a tempdir");
    let untagged_path = dir.path().join("untagged.png");
    let file = fs::File::create(&untagged_path).expect("failed to create the untagged PNG");
    let mut encoder = png::Encoder::new(file, image.width(), image.height());
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder
        .write_header()
        .expect("failed to write the untagged PNG header");
    writer
        .write_image_data(image.as_raw())
        .expect("failed to write the untagged PNG pixels");
    writer.finish().expect("failed to finish the untagged PNG");
    let untagged = decode_png(&untagged_path);

    assert_eq!(untagged.cicp, None);
    assert_eq!(tagged.pixels, untagged.pixels);
}
