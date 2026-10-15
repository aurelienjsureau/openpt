fn main() {
    let Some(path) = std::env::args().nth(1) else { return eprintln!("usage: ptxinfo <session.ptx>") };
    let bytes = std::fs::read(&path).unwrap_or_default();
    match openpt_ptx::read(&bytes) {
        Ok(s) => {
            println!("{} · {:?} Hz · {:?} bit", s.app, s.sample_rate, s.bit_depth);
            println!("tempos {:?}", s.tempos.iter().take(12).collect::<Vec<_>>());
            println!("meters {} {:?}", s.meters.len(), s.meters.iter().take(6).collect::<Vec<_>>());
            for m in &s.markers {
                println!("marker {} {} {:?} {:?}", m.number, m.name, m.start, m.end);
            }
            for t in &s.tracks {
                println!(
                    "  {:?} {} {}  in={:?} out={:?} vol={} pan={:?}",
                    t.kind,
                    if t.stereo { "stereo" } else { "mono" },
                    t.name,
                    t.input,
                    t.output,
                    t.volume_db,
                    t.pan
                );
                for x in &t.sends {
                    println!("      send {} -> {} {} dB pan {}", (b'A' + x.slot as u8) as char, x.target, x.level_db, x.pan);
                }
                for i in &t.inserts {
                    println!("      {}{} {:?} ({} bytes)", i.manufacturer, i.product, i.name, i.state.len());
                }
            }
            println!(
                "busses {:?}
hardware {:?}",
                s.busses, s.hardware
            );
            println!("{} audio files", s.audio_files.len());
            for (f, d) in s.audio_files.iter().zip(&s.audio_folders) {
                println!("  file {f} in {d:?}");
            }
            println!("{} regions, {} placements", s.regions.len(), s.placements.len());
            for p in s.placements.iter().take(40) {
                let r = s.regions.get(p.region);
                println!(
                    "  {} ch{} @{}: {:?}",
                    p.track,
                    p.channel,
                    p.start,
                    r.map(|r| (&r.name, r.file.and_then(|f| s.audio_files.get(f)), r.offset, r.length))
                );
            }
        }
        Err(e) => eprintln!("{path}: {e}"),
    }
}
