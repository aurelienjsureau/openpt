mod common;

fn main() {
    std::thread::spawn(|| {
        let dir = common::fixture_dir();
        let gain = openpt_vst3_host::scan_paths(&[dir.to_path_buf()]).into_iter().find(|d| d.name == "Fixture Gain").expect("fixture").id;
        drop(openpt_vst3_host::instantiate_plugin(&gain).expect("instantiate"));
    })
    .join()
    .expect("worker thread");
    println!("module_exit_worker_thread: ok");
}
