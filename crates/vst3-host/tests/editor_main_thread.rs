mod common;

fn main() {
    let dir = common::fixture_dir();
    let gain = openpt_vst3_host::scan_paths(&[dir.to_path_buf()]).into_iter().find(|d| d.name == "Fixture Gain").expect("fixture").id;
    let mut p = openpt_vst3_host::instantiate_plugin(&gain).expect("instantiate");
    if cfg!(any(target_os = "macos", windows)) {
        assert_eq!(p.editor_size(), Ok((320, 240)));
    } else {
        assert!(p.editor_size().is_err());
    }
    let mut ed = openpt_dsp::Plugin::editor(&mut p).expect("editor handle");
    assert!(!ed.is_open());
    assert_eq!(ed.idle().len(), 1, "the fixture's setup edit");
    ed.set_param("7", 1.5);
    ed.set_param("nope", 1.0);
    assert!(ed.idle().is_empty(), "set_param is not reported back as an edit");
    drop(p);
    assert!(!ed.is_open());
    assert!(ed.open().is_err(), "the plugin is gone");
    println!("editor_main_thread: ok");
}
