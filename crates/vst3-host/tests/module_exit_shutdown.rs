mod common;

use openpt_dsp::Plugin;
use std::process::{Command, Output};

fn child(exit_handler_only: bool) {
    let dir = common::fixture_dir();
    let gain = openpt_vst3_host::scan_paths(&[dir.to_path_buf()]).into_iter().find(|d| d.name == "Fixture Gain").expect("fixture").id;
    let mut p = openpt_vst3_host::instantiate_plugin(&gain).expect("instantiate");
    p.prepare(48_000.0, 64, 2);
    let mut io = vec![vec![0.5f32; 64]; 2];
    p.process(&mut io, 64);
    drop(p);
    if !exit_handler_only {
        openpt_vst3_host::shutdown();
        assert!(openpt_vst3_host::instantiate_plugin(&gain).is_err(), "loads fail after shutdown");
    }
}

fn run(case: &str) -> Output {
    Command::new(std::env::current_exe().expect("test binary")).arg(case).env("VST3_FIXTURE_LATE_STATE", "1").output().expect("run the child")
}

fn main() {
    match std::env::args().nth(1).as_deref() {
        Some("shutdown") => return child(false),
        Some("exit-handler") => return child(true),
        _ => {}
    }
    let up_front = run("shutdown");
    assert!(up_front.status.success(), "shutdown before exit: {:?}\n{}", up_front.status, String::from_utf8_lossy(&up_front.stderr));
    let too_late = run("exit-handler");
    let err = String::from_utf8_lossy(&too_late.stderr);
    if cfg!(windows) {
        assert!(too_late.status.success(), "the exit handler alone: {:?}\n{err}", too_late.status);
    } else {
        assert_eq!(too_late.status.code(), Some(3), "the exit handler alone comes too late: {err}");
        assert!(err.contains("module exit after the process exit tore down state"), "{err}");
    }
    println!("module_exit_shutdown: ok");
}
