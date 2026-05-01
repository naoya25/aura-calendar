use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{App, Manager};
use tokio::sync::oneshot;

use crate::config::AppConfig;
use crate::services::calendar::fetch_next_title;
use crate::ui::icon::menu_bar_icon;

const FALLBACK_NO_CALENDAR_TITLE: &str = "Aura: no calendar";
const FALLBACK_CALENDAR_ERROR_TITLE: &str = "Aura: calendar error";

/// アプリ終了時にバックグラウンドループへ停止信号を送るためのハンドル
pub struct ShutdownHandle(pub Mutex<Option<oneshot::Sender<()>>>);

pub fn setup(app: &mut App) -> Result<(), Box<dyn std::error::Error>> {
    let config = AppConfig::load_or_create()?;
    let normal_title = Arc::new(Mutex::new(config.normal_title()));
    let stealth_title = Arc::new(config.stealth_title().to_string());
    let is_hidden = Arc::new(AtomicBool::new(false));
    let toggle_state = Arc::clone(&is_hidden);
    let click_normal_title = Arc::clone(&normal_title);
    let click_stealth_title = Arc::clone(&stealth_title);
    let updater_hidden_state = Arc::clone(&is_hidden);
    let updater_title_state = Arc::clone(&normal_title);
    let updater_config = config.clone();
    let app_handle = app.handle().clone();
    let initial_title = normal_title
        .lock()
        .map(|value| value.clone())
        .unwrap_or_else(|_| FALLBACK_NO_CALENDAR_TITLE.to_string());

    // バックグラウンドループのキャンセルチャネルを用意し、アプリの状態として管理する
    let (cancel_tx, cancel_rx) = oneshot::channel::<()>();
    app.manage(ShutdownHandle(Mutex::new(Some(cancel_tx))));

    TrayIconBuilder::with_id("main-tray")
        .icon(menu_bar_icon())
        .icon_as_template(true)
        .title(initial_title.as_str())
        .tooltip("AuraCalendar")
        .on_tray_icon_event(move |tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let next_hidden = !toggle_state.fetch_xor(true, Ordering::Relaxed);
                let title = if next_hidden {
                    click_stealth_title.as_str().to_string()
                } else {
                    click_normal_title
                        .lock()
                        .map(|value| value.clone())
                        .unwrap_or_else(|_| FALLBACK_CALENDAR_ERROR_TITLE.to_string())
                };

                if let Err(error) = tray.set_title(Some(title.as_str())) {
                    eprintln!("failed to update tray title: {error}");
                }
            }
        })
        .build(app)?;

    tauri::async_runtime::spawn(async move {
        let duration =
            std::time::Duration::from_secs(updater_config.refresh_interval_seconds);

        // cancel_rx を pin して、ループをまたいで再利用できるようにする
        tokio::pin!(cancel_rx);

        loop {
            let next_title = match fetch_next_title(&updater_config).await {
                Ok(Some(title)) => title,
                Ok(None) => updater_config.normal_title(),
                Err(error) => {
                    eprintln!("failed to fetch calendar: {error}");
                    updater_config.normal_title()
                }
            };

            if let Ok(mut state_title) = updater_title_state.lock() {
                *state_title = next_title.clone();
            }

            if !updater_hidden_state.load(Ordering::Relaxed) {
                if let Some(tray) = app_handle.tray_by_id("main-tray") {
                    if let Err(error) = tray.set_title(Some(next_title.as_str())) {
                        eprintln!("failed to refresh tray title: {error}");
                    }
                }
            }

            // スリープ中に終了信号が来たらすぐにループを抜ける
            tokio::select! {
                _ = &mut cancel_rx => break,
                _ = tokio::time::sleep(duration) => {}
            }
        }
    });

    Ok(())
}
