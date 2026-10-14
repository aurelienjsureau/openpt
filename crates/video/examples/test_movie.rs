fn main() {
    let mut args = std::env::args().skip(1);
    let out = args.next().unwrap_or_else(|| "test_pattern.mp4".into());
    let secs = args.next().and_then(|s| s.parse().ok()).unwrap_or(4);
    match openpt_video::synth::test_pattern_mp4(secs).map(|b| std::fs::write(&out, b)) {
        Ok(Ok(())) => println!("wrote {out}"),
        Ok(Err(e)) => eprintln!("{out}: {e}"),
        Err(e) => eprintln!("{e}"),
    }
}
