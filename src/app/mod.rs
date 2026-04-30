pub mod tray;

pub fn run() {
    tauri::Builder::default()
        .setup(tray::setup)
        .run(tauri::generate_context!())
        .expect("failed to run AuraCalendar");
}
