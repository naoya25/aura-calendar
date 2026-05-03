use tauri::image::Image;

/// Euclidean distance from point to line segment.
fn seg_dist(px: f32, py: f32, ax: f32, ay: f32, bx: f32, by: f32) -> f32 {
    let dx = bx - ax;
    let dy = by - ay;
    let len2 = dx * dx + dy * dy;
    if len2 < 1e-6 {
        return ((px - ax).powi(2) + (py - ay).powi(2)).sqrt();
    }
    let t = (((px - ax) * dx + (py - ay) * dy) / len2).clamp(0.0, 1.0);
    ((px - ax - t * dx).powi(2) + (py - ay - t * dy).powi(2)).sqrt()
}

/// Signed distance from a sharp rectangle (corner radius = 0) centered at origin.
fn sdf_rect(px: f32, py: f32, hx: f32, hy: f32) -> f32 {
    let qx = px.abs() - hx;
    let qy = py.abs() - hy;
    qx.max(qy).min(0.0) + (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt()
}


pub fn menu_bar_icon() -> Image<'static> {
    let width: usize = 18;
    let height: usize = 18;
    let mut rgba = vec![0u8; width * height * 4];

    let cx = 9.0_f32;
    let cy = 9.0_f32;
    let hx = 7.0_f32; // square half-width  → x ∈ [2, 16]
    let hy = 7.0_f32; // square half-height → y ∈ [2, 16]

    // Neon line: top-right corner → bottom edge 1/3 from left
    // Pull both ends inward by `gap` px so the line doesn't touch the square.
    let ax_full = cx + hx;                              // 16.0 (top-right)
    let ay_full = cy - hy;                              //  2.0
    let bx_full = cx - hx + (2.0 * hx) * (1.0 / 3.0); //  6.7 (bottom 1/3)
    let by_full = cy + hy;                              // 16.0

    let gap = 2.5_f32;
    let full_len = ((bx_full - ax_full).powi(2) + (by_full - ay_full).powi(2)).sqrt();
    let ndx = (bx_full - ax_full) / full_len;
    let ndy = (by_full - ay_full) / full_len;
    let lax = ax_full + gap * ndx;
    let lay = ay_full + gap * ndy;
    let lbx = bx_full - gap * ndx;
    let lby = by_full - gap * ndy;

    for row in 0..height {
        for col in 0..width {
            let px = col as f32 + 0.5;
            let py = row as f32 + 0.5;
            let i  = (row * width + col) * 4;

            let d_sq   = sdf_rect(px - cx, py - cy, hx, hy).abs();
            let d_line = seg_dist(px, py, lax, lay, lbx, lby);

            let mut acc_r = 0.0_f32;
            let mut acc_g = 0.0_f32;
            let mut acc_b = 0.0_f32;
            let mut acc_a = 0.0_f32;

            // ── Square outline: crisp white ────────────────────────────────
            if d_sq < 1.0 {
                acc_r = 255.0;
                acc_g = 255.0;
                acc_b = 255.0;
                acc_a = 255.0;
            } else if d_sq < 2.2 {
                let t = (d_sq - 1.0) / 1.2;
                let a = (1.0 - t) * 200.0;
                acc_r = 255.0;
                acc_g = 255.0;
                acc_b = 255.0;
                acc_a = a;
            }

            // ── Neon line: electric blue glow ─────────────────────────────
            let line_glow = 3.5_f32;
            if d_line < line_glow {
                let (cr, cg, cb, ca): (f32, f32, f32, f32) = if d_line < 0.4 {
                    // White-hot core
                    (220.0, 240.0, 255.0, 255.0)
                } else if d_line < 1.2 {
                    // Inner bright blue
                    let t = (d_line - 0.4) / 0.8;
                    let a = 255.0 - t * 50.0;
                    (0.0, (120.0 + 80.0 * (1.0 - t)).min(255.0), 255.0, a)
                } else {
                    // Outer soft halo
                    let t = (d_line - 1.2) / (line_glow - 1.2);
                    let a = (1.0 - t).powi(2) * 180.0;
                    (0.0, 80.0, 255.0, a)
                };
                acc_r = (acc_r + cr * ca / 255.0).min(255.0);
                acc_g = (acc_g + cg * ca / 255.0).min(255.0);
                acc_b = (acc_b + cb * ca / 255.0).min(255.0);
                acc_a = (acc_a + ca).min(255.0);
            }

            if acc_a > 0.5 {
                rgba[i]     = acc_r as u8;
                rgba[i + 1] = acc_g as u8;
                rgba[i + 2] = acc_b as u8;
                rgba[i + 3] = acc_a.min(255.0) as u8;
            }
        }
    }

    Image::new_owned(rgba, width as u32, height as u32)
}
