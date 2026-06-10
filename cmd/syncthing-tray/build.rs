//! 生成托盘图标 ICO 文件（32x32 硬盘图标）
//!
//! 在 OUT_DIR 输出 tray-icon.ico，嵌入到二进制中。

fn main() {
    let out_dir = std::env::var("OUT_DIR").unwrap();
    let icon_path = std::path::Path::new(&out_dir).join("tray-icon.ico");

    // 32x32 32bpp ARGB icon — simple hard drive shape
    let icon = generate_icon();
    std::fs::write(&icon_path, &icon).unwrap();

    println!("cargo:rerun-if-changed=build.rs");
}

fn generate_icon() -> Vec<u8> {
    let width = 32u32;
    let height = 32u32;

    // 32-bit BGRA pixel data (top-down)
    let mut pixels = vec![0u8; (width * height * 4) as usize];

    // Color palette
    let _bg = [0, 0, 0, 0]; // transparent
    let body = [74, 144, 226, 255]; // blue #4A90E2
    let _accent = [100, 170, 240, 255]; // lighter blue
    let dark = [40, 100, 180, 255]; // darker blue
    let silver = [180, 190, 200, 255]; // disk silver
    let led_green = [80, 220, 100, 255]; // LED green
    let led_red = [220, 80, 80, 255]; // LED red

    let set_pixel = |dst: &mut [u8], x: u32, y: u32, c: [u8; 4]| {
        if x < width && y < height {
            let idx = ((y * width + x) * 4) as usize;
            dst[idx] = c[0]; // B
            dst[idx + 1] = c[1]; // G
            dst[idx + 2] = c[2]; // R
            dst[idx + 3] = c[3]; // A
        }
    };

    // Draw rounded rectangle body
    for y in 0..height {
        for x in 0..width {
            let cx = x as i32 - 16;
            let cy = y as i32 - 16;

            // Main body: rounded rect
            let in_body = cx.abs() < 14 && (-12..10).contains(&cy);
            let in_top_round = cx.abs() < 14
                && (-14..=-12).contains(&cy)
                && (cx * cx + (cy + 12) * (cy + 12)) < 20;
            let in_bottom_round =
                cx.abs() < 14 && (10..=12).contains(&cy) && (cx * cx + (cy - 10) * (cy - 10)) < 20;

            if in_body || in_top_round || in_bottom_round {
                // Body gradient from top to bottom
                let t = (cy + 14) as f32 / 26.0;
                let r = (body[2] as f32 * (1.0 - t) + dark[2] as f32 * t) as u8;
                let g = (body[1] as f32 * (1.0 - t) + dark[1] as f32 * t) as u8;
                let b = (body[0] as f32 * (1.0 - t) + dark[0] as f32 * t) as u8;
                set_pixel(&mut pixels, x, y, [b, g, r, 255]);
            }

            // Silver disk plates (horizontal lines)
            if cx.abs() < 12 && (-8..=0).contains(&cy) && (cy + 8) % 3 == 0 {
                set_pixel(&mut pixels, x, y, silver);
            }
            if cx.abs() < 12 && (1..=6).contains(&cy) && (cy - 1) % 3 == 0 {
                set_pixel(&mut pixels, x, y, silver);
            }

            // LEDs on right side
            if cx > 10 && cx < 14 && cy == -4 {
                set_pixel(&mut pixels, x, y, led_green);
            }
            if cx > 10 && cx < 14 && cy == 2 {
                set_pixel(&mut pixels, x, y, led_red);
            }
        }
    }

    // Build ICO file
    let mut ico = Vec::new();

    // ICO header
    ico.extend_from_slice(&[0, 0]); // reserved
    ico.extend_from_slice(&[1, 0]); // type = ICO
    ico.extend_from_slice(&[1, 0]); // count = 1

    // AND mask size (1bpp, row-aligned to 4 bytes)
    let mask_row_size = (width.div_ceil(32) * 4) as usize;
    let mask_size = mask_row_size * height as usize;

    // BMP info header (40 bytes) + XOR pixel data + AND mask
    let image_size = 40 + pixels.len() + mask_size;
    let ico_entry_offset = 6 + 16; // ICONDIR + 1 ICONDIRENTRY

    // ICO entry
    ico.push(32); // width (0 = 256)
    ico.push(32); // height (0 = 256)
    ico.push(0); // colors
    ico.push(0); // reserved
    ico.extend_from_slice(&[1, 0]); // planes
    ico.extend_from_slice(&[32, 0]); // bpp
    ico.extend_from_slice(&(image_size as u32).to_le_bytes()); // size
    ico.extend_from_slice(&(ico_entry_offset as u32).to_le_bytes()); // offset

    // BMP data: BITMAPINFOHEADER + pixels (bottom-up for ICO)
    let header_size = 40u32;
    ico.extend_from_slice(&header_size.to_le_bytes());
    ico.extend_from_slice(&width.to_le_bytes());
    ico.extend_from_slice(&(height * 2).to_le_bytes()); // double height (XOR + AND mask)
    ico.extend_from_slice(&[1, 0]); // planes
    ico.extend_from_slice(&[32, 0]); // bpp
    ico.extend_from_slice(&[0; 4]); // compression = BI_RGB
    ico.extend_from_slice(&[0; 4]); // image size (can be 0 for BI_RGB)
    ico.extend_from_slice(&[0; 16]); // resolution + colors (unused)

    // Pixels: bottom-up
    for y in (0..height).rev() {
        let row_start = (y * width * 4) as usize;
        ico.extend_from_slice(&pixels[row_start..row_start + width as usize * 4]);
    }
    // AND mask (all transparent = all 0); 32bpp uses alpha, but ICO still requires the mask
    ico.extend(std::iter::repeat_n(0u8, mask_size));

    ico
}
