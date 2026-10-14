mod common;

fn main() {
    let dir = common::fixture_dir();
    let gain = openpt_vst3_host::scan_paths(&[dir.to_path_buf()]).into_iter().find(|d| d.name == "Fixture Gain").expect("fixture").id;
    let p = openpt_vst3_host::instantiate_plugin(&gain).expect("instantiate");
    drop(p);
    println!("module_exit: ok");
}
