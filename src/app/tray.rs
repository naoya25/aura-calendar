use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex, RwLock,
};

use chrono::{Datelike, Duration, Local, Timelike, Utc};
use tauri::image::Image;
use tauri::menu::{IconMenuItem, IsMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{App, Manager};
use tauri_plugin_global_shortcut::GlobalShortcutExt;
use tokio::sync::broadcast;

use crate::config::AppConfig;
use crate::services::calendar::{fetch, render_title, CachedEvent};
use crate::ui::icon::menu_bar_icon;

use super::commands::{self, ConfigState, RefreshSignal};

const FALLBACK_NO_CALENDAR_TITLE: &str = "Aura: no calendar";
const FALLBACK_CALENDAR_ERROR_TITLE: &str = "Aura: calendar error";

pub struct ShutdownHandle(pub broadcast::Sender<()>);

pub struct AllowExit(pub Arc<AtomicBool>);

pub struct StealthState {
    pub is_hidden: Arc<AtomicBool>,
    pub normal_title: Arc<Mutex<String>>,
}

pub struct CachedEvents(pub Mutex<Option<Vec<CachedEvent>>>);

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
        let spaced = format!(" {title}");
        if let Err(e) = tray.set_title(Some(spaced.as_str())) {
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

/// n日分の予定 + Preferences / Quit を含むトレイメニューを再構築してトレイに適用する。
pub fn rebuild_tray_menu(app: &tauri::AppHandle, schedule: &[CachedEvent]) {
    let config = app.state::<ConfigState>();
    let days_to_show = config.0.read().map(|g| g.tray_days_to_show).unwrap_or(4);

    let now_local = Local::now();
    let today = now_local.date_naive();
    let weekdays = ["日", "月", "火", "水", "木", "金", "土"];

    let mut all_items: Vec<Box<dyn IsMenuItem<tauri::Wry>>> = Vec::new();
    for day_offset in 0..days_to_show {
        if day_offset > 0 {
            if let Ok(sep) = PredefinedMenuItem::separator(app) {
                all_items.push(Box::new(sep));
            }
        }

        let date = today + Duration::days(day_offset as i64);
        let label = if date == today {
            "Today".to_string()
        } else {
            let wd = date.weekday().num_days_from_sunday() as usize;
            format!("{}/{} ({})", date.month(), date.day(), weekdays[wd])
        };
        if let Ok(header) =
            MenuItem::with_id(app, format!("date_{date}"), label, true, None::<&str>)
        {
            all_items.push(Box::new(header));
        }

        let day_events: Vec<(usize, &CachedEvent, chrono::DateTime<Local>)> = schedule
            .iter()
            .enumerate()
            .filter_map(|(i, event)| {
                let local_start = event.start.with_timezone(&Local);
                (local_start.date_naive() == date).then_some((i, event, local_start))
            })
            .collect();

        if day_events.is_empty() {
            if let Ok(item) = MenuItem::with_id(
                app,
                format!("event_none_{date}"),
                "none",
                true,
                None::<&str>,
            ) {
                all_items.push(Box::new(item));
            }
            continue;
        }

        for (i, event, local_start) in day_events {
            let label = format_event_label(event, local_start);
            let icon = calendar_dot_icon(&event.calendar_color);
            if let Ok(item) = IconMenuItem::with_id(
                app,
                format!("event_{i}"),
                label.clone(),
                true,
                icon,
                None::<&str>,
            ) {
                all_items.push(Box::new(item));
            } else if let Ok(item) =
                MenuItem::with_id(app, format!("event_{i}"), label, true, None::<&str>)
            {
                all_items.push(Box::new(item));
            }
        }
    }

    if let Ok(sep) = PredefinedMenuItem::separator(app) {
        all_items.push(Box::new(sep));
    }
    if let Ok(pref) = MenuItem::with_id(app, "preferences", "Preferences...", true, None::<&str>) {
        all_items.push(Box::new(pref));
    }
    if let Ok(quit) = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>) {
        all_items.push(Box::new(quit));
    }

    let refs: Vec<&dyn IsMenuItem<tauri::Wry>> = all_items.iter().map(|b| b.as_ref()).collect();

    match Menu::with_items(app, &refs) {
        Ok(menu) => {
            if let Some(tray) = app.tray_by_id("main-tray") {
                if let Err(e) = tray.set_menu(Some(menu)) {
                    eprintln!("failed to set tray menu: {e}");
                }
            }
        }
        Err(e) => eprintln!("failed to build tray menu: {e}"),
    }
}

pub fn setup(app: &mut App) -> Result<(), Box<dyn std::error::Error>> {
    let config = AppConfig::load_or_create()?;
    let config_arc = Arc::new(RwLock::new(config.clone()));

    app.manage(ConfigState(Arc::clone(&config_arc)));
    let refresh_notify = Arc::new(tokio::sync::Notify::new());
    app.manage(RefreshSignal(Arc::clone(&refresh_notify)));

    let is_hidden = Arc::new(AtomicBool::new(false));
    let normal_title = Arc::new(Mutex::new(config.normal_title()));

    app.manage(StealthState {
        is_hidden: Arc::clone(&is_hidden),
        normal_title: Arc::clone(&normal_title),
    });
    app.manage(CachedEvents(Mutex::new(None)));

    let (shutdown_tx, _) = broadcast::channel::<()>(1);
    app.manage(ShutdownHandle(shutdown_tx.clone()));

    let allow_exit = Arc::new(AtomicBool::new(false));
    app.manage(AllowExit(Arc::clone(&allow_exit)));

    // 初期メニュー（予定取得前）
    let initial_menu = Menu::with_items(
        app,
        &[
            &MenuItem::with_id(app, "preferences", "Preferences...", true, None::<&str>)?,
            &MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?,
        ],
    )?;

    let initial_title = normal_title
        .lock()
        .map(|v| v.clone())
        .unwrap_or_else(|_| FALLBACK_NO_CALENDAR_TITLE.to_string());

    let app_handle_menu = app.handle().clone();
    app.on_menu_event(move |_, event| match event.id.as_ref() {
        "preferences" => commands::open_settings_window(&app_handle_menu),
        "quit" => {
            let allow_exit = app_handle_menu.state::<AllowExit>();
            allow_exit.0.store(true, Ordering::Relaxed);
            app_handle_menu.exit(0);
        }
        _ => {}
    });

    TrayIconBuilder::with_id("main-tray")
        .icon(menu_bar_icon())
        .icon_as_template(false)
        .title(format!(" {initial_title}").leak())
        .tooltip("AuraCalendar")
        .menu(&initial_menu)
        .show_menu_on_left_click(true)
        .build(app)?;

    register_stealth_shortcut(app.handle(), &config.stealth_shortcut)
        .unwrap_or_else(|e| eprintln!("failed to register stealth shortcut: {e}"));

    // ── 取得タスク（長周期）──────────────────────────────────────────
    {
        let app_handle = app.handle().clone();
        let fetch_config = Arc::clone(&config_arc);
        let fetch_refresh = Arc::clone(&refresh_notify);
        let mut shutdown_rx = shutdown_tx.subscribe();

        tauri::async_runtime::spawn(async move {
            loop {
                let config = fetch_config.read().map(|g| g.clone()).unwrap_or_default();
                let fetch_duration =
                    std::time::Duration::from_secs(config.refresh_interval_seconds);

                let (tray_events, schedule_events) = match fetch(&config).await {
                    Ok(result) => (result.tray_events, result.schedule_events),
                    Err(e) => {
                        eprintln!("failed to fetch calendar: {e}");
                        (None, Vec::new())
                    }
                };

                if let Ok(mut cached) = app_handle.state::<CachedEvents>().0.lock() {
                    *cached = tray_events;
                }

                rebuild_tray_menu(&app_handle, &schedule_events);

                tokio::select! {
                    _ = shutdown_rx.recv() => break,
                    _ = fetch_refresh.notified() => {}
                    _ = tokio::time::sleep(fetch_duration) => {}
                }
            }
        });
    }

    // ── 表示タスク（短周期）──────────────────────────────────────────
    {
        let app_handle = app.handle().clone();
        let disp_config = Arc::clone(&config_arc);
        let disp_hidden = Arc::clone(&is_hidden);
        let disp_title = Arc::clone(&normal_title);
        let mut shutdown_rx = shutdown_tx.subscribe();

        tauri::async_runtime::spawn(async move {
            loop {
                let config = disp_config.read().map(|g| g.clone()).unwrap_or_default();
                let disp_duration = std::time::Duration::from_secs(config.display_interval_seconds);
                let now = Utc::now();

                let next_title = {
                    let events = app_handle
                        .state::<CachedEvents>()
                        .0
                        .lock()
                        .ok()
                        .and_then(|guard| guard.clone());
                    match events {
                        Some(ref evts) => render_title(&config, evts, now),
                        None => config.normal_title(),
                    }
                };

                if let Ok(mut t) = disp_title.lock() {
                    *t = next_title.clone();
                }

                if !disp_hidden.load(Ordering::Relaxed) {
                    if let Some(tray) = app_handle.tray_by_id("main-tray") {
                        let spaced = format!(" {next_title}");
                        if let Err(e) = tray.set_title(Some(spaced.as_str())) {
                            eprintln!("failed to refresh tray title: {e}");
                        }
                    }
                }

                tokio::select! {
                    _ = shutdown_rx.recv() => break,
                    _ = tokio::time::sleep(disp_duration) => {}
                }
            }
        });
    }

    Ok(())
}

fn format_event_label(event: &CachedEvent, local_start: chrono::DateTime<Local>) -> String {
    let start_hm = format!("{:02}:{:02}", local_start.hour(), local_start.minute());
    let end_str = match event.end {
        None => String::new(),
        Some(end_utc) => {
            let local_end = end_utc.with_timezone(&Local);
            let day_diff = (local_end.date_naive() - local_start.date_naive()).num_days();
            match day_diff {
                0 => format!("{:02}:{:02}", local_end.hour(), local_end.minute()),
                1 => format!("{:02}:{:02}", local_end.hour() + 24, local_end.minute()),
                _ => String::new(),
            }
        }
    };
    let title = truncate_chars(&event.title, 20);
    if end_str.is_empty() {
        format!("{}~ {}", start_hm, title)
    } else {
        format!("{}~{} {}", start_hm, end_str, title)
    }
}

fn truncate_chars(s: &str, max_chars: usize) -> String {
    let mut chars = s.chars();
    let truncated: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{}...", truncated)
    } else {
        truncated
    }
}

fn calendar_dot_icon(color: &str) -> Option<Image<'static>> {
    let [red, green, blue, alpha] = parse_hex_color(color)?;
    let size = 12usize;
    let mut rgba = vec![0u8; size * size * 4];
    let center = (size as f32 - 1.0) / 2.0;
    let half_side = 3.2_f32;

    for y in 0..size {
        for x in 0..size {
            let dx = x as f32 - center;
            let dy = y as f32 - center;
            if dx.abs() <= half_side && dy.abs() <= half_side {
                let idx = (y * size + x) * 4;
                rgba[idx] = red;
                rgba[idx + 1] = green;
                rgba[idx + 2] = blue;
                rgba[idx + 3] = alpha;
            }
        }
    }

    Some(Image::new_owned(rgba, size as u32, size as u32))
}

fn parse_hex_color(value: &str) -> Option<[u8; 4]> {
    let hex = value.trim().trim_start_matches('#');
    if hex.len() != 6 && hex.len() != 8 {
        return None;
    }

    let rgb = if hex.len() == 6 { hex } else { &hex[..6] };
    let red = u8::from_str_radix(&rgb[0..2], 16).ok()?;
    let green = u8::from_str_radix(&rgb[2..4], 16).ok()?;
    let blue = u8::from_str_radix(&rgb[4..6], 16).ok()?;
    let alpha = if hex.len() == 8 {
        u8::from_str_radix(&hex[6..8], 16).ok()?
    } else {
        0xFF
    };

    Some([red, green, blue, alpha])
}
