fn main() {
    let v = openpt_ui_egui::menus::parity();
    println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
}
