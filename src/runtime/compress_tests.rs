use super::*;

const SAMPLE: &[u8] = b"hello content-encoding, hello content-encoding, hello content-encoding";

#[test]
fn gzip_round_trips() {
    let encoded = gzip_encode(SAMPLE).unwrap();
    assert!(encoded.starts_with(&[0x1f, 0x8b]), "gzip magic"); // gzip header
    assert_ne!(encoded, SAMPLE);
    assert_eq!(gzip_decode(&encoded).unwrap(), SAMPLE);
}

#[test]
fn deflate_round_trips_and_accepts_raw() {
    let encoded = deflate_encode(SAMPLE).unwrap();
    assert_eq!(deflate_decode(&encoded).unwrap(), SAMPLE);
}

#[test]
fn zstd_decodes_a_known_frame() {
    // A zstd frame of "hello zstd content-encoding" (ruzstd is decode-only, so the frame
    // is precomputed rather than produced here).
    let frame: &[u8] = &[
        0x28, 0xb5, 0x2f, 0xfd, 0x04, 0x58, 0xd9, 0x00, 0x00, 0x68, 0x65, 0x6c, 0x6c, 0x6f, 0x20,
        0x7a, 0x73, 0x74, 0x64, 0x20, 0x63, 0x6f, 0x6e, 0x74, 0x65, 0x6e, 0x74, 0x2d, 0x65, 0x6e,
        0x63, 0x6f, 0x64, 0x69, 0x6e, 0x67, 0x8e, 0x4b, 0xb5, 0x22,
    ];
    assert_eq!(zstd_decode(frame).unwrap(), b"hello zstd content-encoding");
}

#[test]
fn malformed_input_errors() {
    assert!(gzip_decode(b"not a gzip stream").is_err());
    assert!(zstd_decode(b"not a zstd frame at all").is_err());
}
