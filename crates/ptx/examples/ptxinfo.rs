fn main() {
    let Some(path) = std::env::args().nth(1) else { return eprintln!("usage: ptxinfo <session.ptx>") };
    let bytes = std::fs::read(&path).unwrap_or_default();
    match openpt_ptx::read(&bytes) {
        Ok(s) => {
            println!("{} · {:?} Hz · {:?} bit", s.app, s.sample_rate, s.bit_depth);
            for t in &s.tracks {
                println!("  {:?} {} {}", t.kind, if t.stereo { "stereo" } else { "mono" }, t.name);
            }
            println!("{} audio files", s.audio_files.len());
        }
        Err(e) => eprintln!("{path}: {e}"),
    }
}
