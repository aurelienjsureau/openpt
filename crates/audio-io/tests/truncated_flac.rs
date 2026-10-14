use openpt_audio_io::{AudioBuffer, BitDepth, EncodeOptions, FileFormat, decode, encode};

#[test]
fn a_truncated_flac_keeps_its_declared_length() {
    let n = 48_000;
    let tone: Vec<f32> = (0..n).map(|i| 0.25 * (i as f32 * 0.05).sin()).collect();
    let buf = AudioBuffer { sample_rate: 48_000, channels: vec![tone] };
    let bytes = encode(&buf, &EncodeOptions { format: FileFormat::Flac, bit_depth: BitDepth::Int16, dither: false, bwf: None }).unwrap();
    let cut = &bytes[..bytes.len() / 2];
    let (_, back) = decode(cut, Some("flac")).unwrap();
    assert_eq!(back.frames(), n, "padded to the declared length");
}
