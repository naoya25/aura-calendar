pub mod commands;
pub mod tray;

use tauri::Manager;
use tauri_plugin_global_shortcut::ShortcutState;

pub fn run() {
    tauri::Builder::default()
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    if event.state() == ShortcutState::Pressed {
                        tray::toggle_stealth(app);
                    }
                })
                .build(),
        )
        .invoke_handler(tauri::generate_handler![
            commands::get_config,
            commands::get_default_config,
            commands::save_config,
            commands::preview_format,
            commands::close_settings_window,
        ])
        .setup(tray::setup)
        .build(tauri::generate_context!())
        .expect("failed to build AuraCalendar")
        .run(|app_handle, event| match event {
            // バックグラウンドループの停止信号を送る
            tauri::RunEvent::Exit => {
                let handle = app_handle.state::<tray::ShutdownHandle>();
                let _ = handle.0.send(());
            }
            // 全ウィンドウを閉じてもアプリを終了しない（トレイアプリとして常駐）
            tauri::RunEvent::ExitRequested { api, .. } => {
                let allow_exit = app_handle.state::<tray::AllowExit>();
                if allow_exit.0.load(std::sync::atomic::Ordering::Relaxed) {
                    allow_exit
                        .0
                        .store(false, std::sync::atomic::Ordering::Relaxed);
                } else {
                    api.prevent_exit();
                }
            }
            // 設定ウィンドウの×ボタンは閉じるのではなく隠す
            tauri::RunEvent::WindowEvent {
                label: ref l,
                event: tauri::WindowEvent::CloseRequested { api, .. },
                ..
            } if l == "settings" => {
                api.prevent_close();
                if let Some(win) = app_handle.get_webview_window("settings") {
                    let _ = win.hide();
                }
            }
            _ => {}
        });
}
