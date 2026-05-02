use tauri::image::Image;

pub fn menu_bar_icon() -> Image<'static> {
    let width: usize = 18;
    let height: usize = 18;
    let mut rgba = vec![0u8; width * height * 4];

    // 塗りつぶし角丸正方形: 余白 3px、角丸半径 2px
    let pad = 3.0_f32;
    let r = 2.0_f32;
    let x0 = pad;
    let y0 = pad;
    let x1 = (width as f32) - 1.0 - pad;
    let y1 = (height as f32) - 1.0 - pad;

    for row in 0..height {
        for col in 0..width {
            let px = col as f32 + 0.5;
            let py = row as f32 + 0.5;

            if px < x0 || px > x1 || py < y0 || py > y1 {
                continue;
            }

            // 角の領域だけ円弧でクリップ
            let in_corner_x = px < x0 + r || px > x1 - r;
            let in_corner_y = py < y0 + r || py > y1 - r;
            if in_corner_x && in_corner_y {
                let cx = if px < x0 + r { x0 + r } else { x1 - r };
                let cy = if py < y0 + r { y0 + r } else { y1 - r };
                if (px - cx) * (px - cx) + (py - cy) * (py - cy) > r * r {
                    continue;
                }
            }

            let i = (row * width + col) * 4;
            rgba[i + 3] = 255; // 黒の不透明ピクセル（R/G/B は 0 のまま）
        }
    }

    Image::new_owned(rgba, width as u32, height as u32)
}
