//! 生成托盘图标 ICO 文件（多尺寸、多状态）
//!
//! 为获得像素级清晰的小尺寸渲染，每个状态提供三套 SVG 源：
//! - `{name}_16.svg` -> 16×16
//! - `{name}_24.svg` -> 24×24 / 32×32
//! - `{name}.svg`    -> 48×48 / 256×256
//!
//! 输出到 OUT_DIR，供 tray.rs 通过 include_bytes! 嵌入二进制。

use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    let out_dir = std::env::var("OUT_DIR").unwrap();

    // 托盘状态图标：按目标尺寸选择最优 SVG 源，避免非整数像素缩放导致的发虚。
    let tray_configs: [(&str, &[(u32, &str)]); 4] = [
        (
            "default",
            &[
                (16, "default_16"),
                (24, "default_24"),
                (32, "default_24"),
                (48, "default"),
                (256, "default"),
            ],
        ),
        (
            "idle",
            &[
                (16, "idle_16"),
                (24, "idle_24"),
                (32, "idle_24"),
                (48, "idle"),
                (256, "idle"),
            ],
        ),
        (
            "syncing",
            &[
                (16, "syncing_16"),
                (24, "syncing_24"),
                (32, "syncing_24"),
                (48, "syncing"),
                (256, "syncing"),
            ],
        ),
        (
            "error",
            &[
                (16, "error_16"),
                (24, "error_24"),
                (32, "error_24"),
                (48, "error"),
                (256, "error"),
            ],
        ),
    ];

    for (name, size_sources) in tray_configs {
        for (_, source) in size_sources {
            println!("cargo:rerun-if-changed=assets/icons/{source}.svg");
        }

        let icon_bytes = render_sized_svg_to_ico(size_sources);
        let out_path = Path::new(&out_dir).join(format!("tray-icon-{name}.ico"));
        std::fs::write(&out_path, &icon_bytes)
            .unwrap_or_else(|e| panic!("failed to write {}: {}", out_path.display(), e));
    }
}

/// 按尺寸读取对应的 SVG 源并渲染为包含多个尺寸的 ICO 文件。
fn render_sized_svg_to_ico(size_sources: &[(u32, &str)]) -> Vec<u8> {
    let mut images = Vec::with_capacity(size_sources.len());
    let mut sizes = Vec::with_capacity(size_sources.len());

    for (size, source) in size_sources {
        let svg_path = Path::new("assets/icons").join(format!("{source}.svg"));
        let svg = std::fs::read_to_string(&svg_path)
            .unwrap_or_else(|e| panic!("failed to read {}: {}", svg_path.display(), e));
        let tree = resvg::usvg::Tree::from_str(&svg, &resvg::usvg::Options::default())
            .unwrap_or_else(|e| panic!("failed to parse {}: {}", svg_path.display(), e));

        images.push(render_svg_to_bgra(&tree, *size));
        sizes.push(*size);
    }

    build_ico(&images, &sizes)
}

/// 将 SVG 渲染为指定尺寸的 bottom-up BGRA 像素数据。
fn render_svg_to_bgra(tree: &resvg::usvg::Tree, size: u32) -> Vec<u8> {
    let mut pixmap = tiny_skia::Pixmap::new(size, size).expect("failed to allocate pixmap");
    pixmap.fill(tiny_skia::Color::TRANSPARENT);

    let svg_width = tree.size().width();
    let svg_height = tree.size().height();
    let scale = (size as f32 / svg_width).min(size as f32 / svg_height);
    let tx = (size as f32 - svg_width * scale) / 2.0;
    let ty = (size as f32 - svg_height * scale) / 2.0;
    let transform = tiny_skia::Transform::from_translate(tx, ty).post_scale(scale, scale);

    resvg::render(tree, transform, &mut pixmap.as_mut());

    // tiny-skia 输出为 RGBA top-down；ICO 需要 BGRA bottom-up。
    let rgba = pixmap.data();
    let mut bgra_bottom_up = Vec::with_capacity((size * size * 4) as usize);
    for y in (0..size).rev() {
        let row_start = (y * size * 4) as usize;
        for x in 0..size {
            let idx = row_start + (x * 4) as usize;
            let r = rgba[idx];
            let g = rgba[idx + 1];
            let b = rgba[idx + 2];
            let a = rgba[idx + 3];
            bgra_bottom_up.extend_from_slice(&[b, g, r, a]);
        }
    }
    bgra_bottom_up
}

/// 组装 ICO 文件。
///
/// 输入 `images` 为 bottom-up BGRA 像素数据，`sizes` 为对应尺寸。
fn build_ico(images: &[Vec<u8>], sizes: &[u32]) -> Vec<u8> {
    let count = images.len();
    let entry_size = 16usize;
    let header_size = 6usize;
    let mut entries = Vec::with_capacity(count);
    let mut image_data = Vec::new();

    let mut offset = header_size + entry_size * count;

    for (idx, &size) in sizes.iter().enumerate() {
        let pixels = &images[idx];
        let mask_row_size = (size.div_ceil(32) * 4) as usize;
        let mask_size = mask_row_size * size as usize;
        let data_size = 40 + pixels.len() + mask_size;

        entries.push(IconEntry {
            width: if size == 256 { 0 } else { size as u8 },
            height: if size == 256 { 0 } else { size as u8 },
            size: data_size as u32,
            offset: offset as u32,
        });

        // BITMAPINFOHEADER
        image_data.extend_from_slice(&40u32.to_le_bytes()); // header size
        image_data.extend_from_slice(&size.to_le_bytes()); // width
        image_data.extend_from_slice(&(size * 2).to_le_bytes()); // height (XOR + AND)
        image_data.extend_from_slice(&1u16.to_le_bytes()); // planes
        image_data.extend_from_slice(&32u16.to_le_bytes()); // bpp
        image_data.extend_from_slice(&0u32.to_le_bytes()); // compression BI_RGB
        image_data.extend_from_slice(&0u32.to_le_bytes()); // image size
        image_data.extend_from_slice(&0u32.to_le_bytes()); // x ppm
        image_data.extend_from_slice(&0u32.to_le_bytes()); // y ppm
        image_data.extend_from_slice(&0u32.to_le_bytes()); // colors used
        image_data.extend_from_slice(&0u32.to_le_bytes()); // important colors

        // XOR pixel data (already bottom-up BGRA)
        image_data.extend_from_slice(pixels);

        // AND mask (all transparent — 32bpp uses alpha, but ICO still requires the mask)
        image_data.extend(std::iter::repeat_n(0u8, mask_size));

        offset += data_size;
    }

    let mut ico = Vec::new();
    // ICONDIR
    ico.extend_from_slice(&0u16.to_le_bytes()); // reserved
    ico.extend_from_slice(&1u16.to_le_bytes()); // type = ICO
    ico.extend_from_slice(&(count as u16).to_le_bytes()); // count

    // ICONDIRENTRYs
    for e in entries {
        ico.push(e.width);
        ico.push(e.height);
        ico.push(0); // colors
        ico.push(0); // reserved
        ico.extend_from_slice(&1u16.to_le_bytes()); // planes
        ico.extend_from_slice(&32u16.to_le_bytes()); // bpp
        ico.extend_from_slice(&e.size.to_le_bytes());
        ico.extend_from_slice(&e.offset.to_le_bytes());
    }

    ico.extend_from_slice(&image_data);
    ico
}

struct IconEntry {
    width: u8,
    height: u8,
    size: u32,
    offset: u32,
}
