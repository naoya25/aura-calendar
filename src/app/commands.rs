use std::sync::{Arc, RwLock};

use tauri::Manager;
use tokio::sync::Notify;

use crate::config::AppConfig;

pub struct ConfigState(pub Arc<RwLock<AppConfig>>);

pub struct RefreshSignal(pub Arc<Notify>);

#[tauri::command]
pub fn get_config(state: tauri::State<ConfigState>) -> Result<AppConfig, String> {
    state.0.read().map_err(|e| e.to_string()).map(|g| g.clone())
}

/// 2つのショートカット文字列が同じキー組み合わせを指すか（大小文字・前後空白を無視）
fn shortcuts_conflict(a: &str, b: &str) -> bool {
    a.trim().to_lowercase() == b.trim().to_lowercase()
}

#[tauri::command]
pub fn save_config(
    app: tauri::AppHandle,
    mut config: AppConfig,
    config_state: tauri::State<ConfigState>,
    refresh: tauri::State<RefreshSignal>,
) -> Result<(), String> {
    if shortcuts_conflict(&config.stealth_shortcut, &config.open_menu_shortcut) {
        return Err("ステルス切替とトレイ表示のショートカットが重複しています".to_string());
    }

    let old_shortcuts = config_state
        .0
        .read()
        .ok()
        .map(|g| (g.stealth_shortcut.clone(), g.open_menu_shortcut.clone()));

    super::tray::unregister_all_shortcuts(&app);
    let register_result = super::tray::register_stealth_shortcut(&app, &config.stealth_shortcut)
        .and_then(|_| super::tray::register_open_menu_shortcut(&app, &config.open_menu_shortcut));

    if let Err(e) = register_result {
        super::tray::unregister_all_shortcuts(&app);
        if let Some((old_stealth, old_open_menu)) = old_shortcuts {
            let _ = super::tray::register_stealth_shortcut(&app, &old_stealth);
            let _ = super::tray::register_open_menu_shortcut(&app, &old_open_menu);
        }
        return Err(format!("無効なショートカットキーです: {e}"));
    }

    config.normalize_calendar_emojis();
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

#[tauri::command]
pub fn get_default_config() -> crate::config::AppConfig {
    crate::config::AppConfig::default()
}

#[tauri::command]
pub fn preview_format(template: String) -> Result<String, String> {
    crate::format::preview(&template)
}

#[tauri::command]
pub fn close_settings_window(app: tauri::AppHandle) {
    if let Some(win) = app.get_webview_window("settings") {
        let _ = win.hide();
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcuts_conflict_should_detect_identical_shortcuts() {
        assert!(shortcuts_conflict("ctrl+a", "ctrl+a"));
    }

    #[test]
    fn shortcuts_conflict_should_ignore_case_and_whitespace() {
        assert!(shortcuts_conflict(" Ctrl+A ", "ctrl+a"));
    }

    #[test]
    fn shortcuts_conflict_should_allow_distinct_shortcuts() {
        assert!(!shortcuts_conflict("ctrl+a", "ctrl+s"));
    }
}
