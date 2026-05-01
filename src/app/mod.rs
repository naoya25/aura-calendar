pub mod tray;

use tauri::Manager;

pub fn run() {
    tauri::Builder::default()
        .setup(tray::setup)
        .build(tauri::generate_context!())
        .expect("failed to build AuraCalendar")
        .run(|app_handle, event| {
            if let tauri::RunEvent::Exit = event {
                // アプリ終了時にバックグラウンドループへ停止信号を送る
                // guard をクロージャ内で drop させて、handle より先に解放する
                let handle = app_handle.state::<tray::ShutdownHandle>();
                let tx = handle.0.lock().ok().and_then(|mut guard| guard.take());
                if let Some(tx) = tx {
                    let _ = tx.send(());
                }
            }
        });
}
