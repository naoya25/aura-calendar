use tauri::image::Image;

pub fn menu_bar_icon() -> Image<'static> {
    let width = 18;
    let height = 18;
    let mut rgba = vec![0; width * height * 4];
    let center = 8.5_f32;

    // sqrt を避けて距離の二乗で比較する（4.5^2=20.25, 7.5^2=56.25, 2.0^2=4.0）
    for y in 0..height {
        for x in 0..width {
            let dx = x as f32 - center;
            let dy = y as f32 - center;
            let dist_sq = dx * dx + dy * dy;
            let index = (y * width + x) * 4;

            if (20.25..=56.25).contains(&dist_sq) || dist_sq <= 4.0 {
                rgba[index] = 0;
                rgba[index + 1] = 0;
                rgba[index + 2] = 0;
                rgba[index + 3] = 255;
            }
        }
    }

    Image::new_owned(rgba, width as u32, height as u32)
}
