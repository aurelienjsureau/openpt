fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../../assets/app-icon/openpt.ico");
    println!("cargo:rerun-if-env-changed=OPENPT_REQUIRE_WINRES");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set_icon("../../assets/app-icon/openpt.ico")
        .set("ProductName", "OpenPT")
        .set("FileDescription", "OpenPT digital audio workstation")
        .set("CompanyName", "Learning Machines LLC")
        .set("LegalCopyright", "Copyright (c) the OpenPT authors. MIT OR Apache-2.0.")
        .set("OriginalFilename", "openpt.exe")
        .set("InternalName", "openpt");
    if let Err(e) = res.compile() {
        if std::env::var_os("OPENPT_REQUIRE_WINRES").is_some() {
            eprintln!("embedding Windows resources failed: {e}");
            std::process::exit(1);
        }
        println!("cargo:warning=openpt.exe built without icon/version resources: {e}");
    }
}
