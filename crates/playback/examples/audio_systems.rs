fn main() {
    for s in openpt_playback::devices::audio_systems() {
        println!("{}{}", s.name, if s.is_default { " (default)" } else { "" });
        for d in &s.outputs {
            println!("  out {} : {} ch, {:?} Hz, buffer {:?}..{:?}", d.name, d.max_channels, d.sample_rates, d.buffer_min, d.buffer_max);
        }
        for d in &s.inputs {
            println!("  in  {} : {} ch, {:?} Hz", d.name, d.max_channels, d.sample_rates);
        }
    }
}
