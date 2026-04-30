use tauri::image::Image;

pub fn menu_bar_icon() -> Image<'static> {
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
