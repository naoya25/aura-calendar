use std::sync::{Arc, RwLock};

use tauri::Manager;
use tokio::sync::Notify;

use crate::config::AppConfig;

pub struct ConfigState(pub Arc<RwLock<AppConfig>>);

pub struct RefreshSignal(pub Arc<Notify>);

#[tauri::command]
pub fn get_config(state: tauri::State<ConfigState>) -> Result<AppConfig, String> {
    state
        .0
        .read()
        .map_err(|e| e.to_string())
        .map(|g| g.clone())
}

#[tauri::command]
pub fn save_config(
    config: AppConfig,
    config_state: tauri::State<ConfigState>,
    refresh: tauri::State<RefreshSignal>,
) -> Result<(), String> {
    config.save().map_err(|e| e.to_string())?;
    match config_state.0.write() {
        Ok(mut guard) => {
            *guard = config;
        }
        Err(e) => return Err(e.to_string()),
    }
    refresh.0.notify_one();
    Ok(())
}

pub fn open_settings_window(app: &tauri::AppHandle) {
    if let Some(win) = app.get_webview_window("settings") {
        let _ = win.show();
        let _ = win.set_focus();
        return;
    }
    if let Err(e) = tauri::WebviewWindowBuilder::new(
        app,
        "settings",
        tauri::WebviewUrl::App("settings.html".into()),
    )
    .title("AuraCalendar 設定")
    .inner_size(660.0, 580.0)
    .resizable(false)
    .build()
    {
        eprintln!("failed to open settings window: {e}");
    }
}
