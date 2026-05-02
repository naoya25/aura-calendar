pub mod commands;
pub mod tray;

use tauri::Manager;

pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            commands::get_config,
            commands::save_config,
            commands::preview_format,
        ])
        .setup(tray::setup)
        .build(tauri::generate_context!())
        .expect("failed to build AuraCalendar")
        .run(|app_handle, event| match event {
            // バックグラウンドループの停止信号を送る
            tauri::RunEvent::Exit => {
                let handle = app_handle.state::<tray::ShutdownHandle>();
                let tx = handle.0.lock().ok().and_then(|mut guard| guard.take());
                if let Some(tx) = tx {
                    let _ = tx.send(());
                }
            }
            // 全ウィンドウを閉じてもアプリを終了しない（トレイアプリとして常駐）
            tauri::RunEvent::ExitRequested { api, .. } => {
                api.prevent_exit();
            }
            // 設定ウィンドウの×ボタンは閉じるのではなく隠す
            tauri::RunEvent::WindowEvent {
                label,
                event: tauri::WindowEvent::CloseRequested { api, .. },
                ..
            } => {
                if label == "settings" {
                    api.prevent_close();
                    if let Some(win) = app_handle.get_webview_window("settings") {
                        let _ = win.hide();
                    }
                }
            }
            _ => {}
        });
}
