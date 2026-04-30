use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

mod calendar;
mod config;

use calendar::fetch_next_title;
use config::AppConfig;
use tauri::{
    image::Image,
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};

fn main() {
    tauri::Builder::default()
        .setup(|app| {
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
                .unwrap_or_else(|_| "Aura: no calendar".to_string());

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
                                .unwrap_or_else(|_| "Aura: calendar error".to_string())
                        };

                        if let Err(error) = tray.set_title(Some(title.as_str())) {
                            eprintln!("failed to update tray title: {error}");
                        }
                    }
                })
                .build(app)?;

            tauri::async_runtime::spawn(async move {
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

                    tokio::time::sleep(std::time::Duration::from_secs(
                        updater_config.refresh_interval_seconds,
                    ))
                    .await;
                }
            });

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("failed to run AuraCalendar");
}

fn menu_bar_icon() -> Image<'static> {
    let width = 18;
    let height = 18;
    let mut rgba = vec![0; width * height * 4];
    let center = 8.5_f32;

    for y in 0..height {
        for x in 0..width {
            let dx = x as f32 - center;
            let dy = y as f32 - center;
            let distance = (dx * dx + dy * dy).sqrt();
            let index = (y * width + x) * 4;

            if (4.5..=7.5).contains(&distance) || distance <= 2.0 {
                rgba[index] = 0;
                rgba[index + 1] = 0;
                rgba[index + 2] = 0;
                rgba[index + 3] = 255;
            }
        }
    }

    Image::new_owned(rgba, width as u32, height as u32)
}
