use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

mod config;

use config::AppConfig;
use tauri::{
    image::Image,
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            let config = AppConfig::load_or_create()?;
            let normal_title = Arc::new(config.normal_title());
            let stealth_title = Arc::new(config.stealth_title().to_string());
            let is_hidden = Arc::new(AtomicBool::new(false));
            let toggle_state = Arc::clone(&is_hidden);
            let click_normal_title = Arc::clone(&normal_title);
            let click_stealth_title = Arc::clone(&stealth_title);

            TrayIconBuilder::new()
                .icon(menu_bar_icon())
                .icon_as_template(true)
                .title(normal_title.as_str())
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
                            click_stealth_title.as_str()
                        } else {
                            click_normal_title.as_str()
                        };

                        if let Err(error) = tray.set_title(Some(title)) {
                            eprintln!("failed to update tray title: {error}");
                        }
                    }
                })
                .build(app)?;

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
