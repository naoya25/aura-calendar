use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex, RwLock,
};

use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{App, Manager};
use tauri_plugin_global_shortcut::GlobalShortcutExt;
use tokio::sync::oneshot;

use crate::config::AppConfig;
use crate::services::calendar::fetch_next_title;
use crate::ui::icon::menu_bar_icon;

use super::commands::{self, ConfigState, RefreshSignal};

const FALLBACK_NO_CALENDAR_TITLE: &str = "Aura: no calendar";
const FALLBACK_CALENDAR_ERROR_TITLE: &str = "Aura: calendar error";

pub struct ShutdownHandle(pub Mutex<Option<oneshot::Sender<()>>>);

pub struct StealthState {
    pub is_hidden: Arc<AtomicBool>,
    pub normal_title: Arc<Mutex<String>>,
}

pub fn toggle_stealth(app: &tauri::AppHandle) {
    let stealth = app.state::<StealthState>();
    let config = app.state::<ConfigState>();

    let next_hidden = !stealth.is_hidden.fetch_xor(true, Ordering::Relaxed);
    let title = if next_hidden {
        config
            .0
            .read()
            .map(|g| g.stealth_title().to_string())
            .unwrap_or_else(|_| "***".to_string())
    } else {
        stealth
            .normal_title
            .lock()
            .map(|v| v.clone())
            .unwrap_or_else(|_| FALLBACK_CALENDAR_ERROR_TITLE.to_string())
    };

    if let Some(tray) = app.tray_by_id("main-tray") {
        if let Err(e) = tray.set_title(Some(title.as_str())) {
            eprintln!("failed to update tray title: {e}");
        }
    }
}

pub fn register_stealth_shortcut(app: &tauri::AppHandle, shortcut: &str) -> Result<(), String> {
    app.global_shortcut()
        .register(shortcut)
        .map_err(|e| e.to_string())
}

pub fn unregister_all_shortcuts(app: &tauri::AppHandle) {
    if let Err(e) = app.global_shortcut().unregister_all() {
        eprintln!("failed to unregister shortcuts: {e}");
    }
}

pub fn setup(app: &mut App) -> Result<(), Box<dyn std::error::Error>> {
    let config = AppConfig::load_or_create()?;
    let config_arc = Arc::new(RwLock::new(config.clone()));

    // 設定画面コマンドと共有する状態を登録
    app.manage(ConfigState(Arc::clone(&config_arc)));
    let refresh_notify = Arc::new(tokio::sync::Notify::new());
    app.manage(RefreshSignal(Arc::clone(&refresh_notify)));

    let is_hidden = Arc::new(AtomicBool::new(false));
    let normal_title = Arc::new(Mutex::new(config.normal_title()));

    app.manage(StealthState {
        is_hidden: Arc::clone(&is_hidden),
        normal_title: Arc::clone(&normal_title),
    });

    let updater_hidden = Arc::clone(&is_hidden);
    let updater_title = Arc::clone(&normal_title);
    let updater_config = Arc::clone(&config_arc);
    let updater_refresh = Arc::clone(&refresh_notify);

    let (cancel_tx, cancel_rx) = oneshot::channel::<()>();
    app.manage(ShutdownHandle(Mutex::new(Some(cancel_tx))));

    let menu = Menu::with_items(
        app,
        &[
            &MenuItem::with_id(app, "preferences", "Preferences...", true, None::<&str>)?,
            &MenuItem::with_id(app, "quit", "Quit AuraCalendar", true, None::<&str>)?,
        ],
    )?;

    let initial_title = normal_title
        .lock()
        .map(|v| v.clone())
        .unwrap_or_else(|_| FALLBACK_NO_CALENDAR_TITLE.to_string());

    let app_handle_menu = app.handle().clone();

    app.on_menu_event(move |_, event| match event.id.as_ref() {
        "preferences" => commands::open_settings_window(&app_handle_menu),
        "quit" => app_handle_menu.exit(0),
        _ => {}
    });

    let app_handle_click = app.handle().clone();

    TrayIconBuilder::with_id("main-tray")
        .icon(menu_bar_icon())
        .icon_as_template(true)
        .title(initial_title.as_str())
        .tooltip("AuraCalendar")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_tray_icon_event(move |_tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                toggle_stealth(&app_handle_click);
            }
        })
        .build(app)?;

    register_stealth_shortcut(app.handle(), &config.stealth_shortcut)
        .unwrap_or_else(|e| eprintln!("failed to register stealth shortcut: {e}"));

    let app_handle_loop = app.handle().clone();

    tauri::async_runtime::spawn(async move {
        tokio::pin!(cancel_rx);

        loop {
            let (next_title, duration) = {
                let config = updater_config
                    .read()
                    .map(|g| g.clone())
                    .unwrap_or_default();
                let duration =
                    std::time::Duration::from_secs(config.refresh_interval_seconds);
                let title = match fetch_next_title(&config).await {
                    Ok(Some(t)) => t,
                    Ok(None) => config.normal_title(),
                    Err(e) => {
                        eprintln!("failed to fetch calendar: {e}");
                        config.normal_title()
                    }
                };
                (title, duration)
            };

            if let Ok(mut t) = updater_title.lock() {
                *t = next_title.clone();
            }

            if !updater_hidden.load(Ordering::Relaxed) {
                if let Some(tray) = app_handle_loop.tray_by_id("main-tray") {
                    if let Err(e) = tray.set_title(Some(next_title.as_str())) {
                        eprintln!("failed to refresh tray title: {e}");
                    }
                }
            }

            tokio::select! {
                _ = &mut cancel_rx => break,
                _ = updater_refresh.notified() => {}
                _ = tokio::time::sleep(duration) => {}
            }
        }
    });

    Ok(())
}
