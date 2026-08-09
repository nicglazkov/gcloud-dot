//! Draws the background for the installer window.
//!
//! The disk image opens to a Finder window showing two icons. Left to itself
//! that window is grey and says nothing, so this paints the wash the rest of
//! the product uses, an arrow pointing the way, and one line of instruction.
//!
//! Rendered at 1x and 2x. `tiffutil` combines them into the multi-resolution
//! file Finder needs to stay sharp on a Retina display.
//!
//! ```sh
//! cargo run -p gcloud-dot-app --example render_dmg_background -- build/dmg-bg
//! ```

use ab_glyph::{Font, FontRef, Glyph, PxScale, ScaleFont};

const FONT_DATA: &[u8] = include_bytes!("../../../assets/Inconsolata-Bold.ttf");

/// Window content size in points. The icon positions in the dmgbuild settings
/// are in this same space, so the arrow lands between them.
const W: u32 = 660;
const H: u32 = 420;

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "build/dmg-bg".to_string());
    if let Some(dir) = std::path::Path::new(&out).parent() {
        std::fs::create_dir_all(dir).expect("could not create the output directory");
    }

    for scale in [1u32, 2u32] {
        let rgba = render(scale);
        let path = if scale == 1 {
            format!("{out}.png")
        } else {
            format!("{out}@2x.png")
        };
        write_png(&path, &rgba, W * scale, H * scale);
        println!("wrote {path}");
    }
}

fn render(s: u32) -> Vec<u8> {
    let (w, h) = (W * s, H * s);
    let mut rgba = vec![0u8; (w * h * 4) as usize];
    let sf = s as f32;

    // A light wash. Finder does not repaint this for dark mode, so it has to
    // be legible on its own terms rather than adapting to anything.
    for y in 0..h {
        for x in 0..w {
            let fx = x as f32 / w as f32;
            let fy = y as f32 / h as f32;
            // Two soft radial pools, green at the left, amber at the right,
            // over near white. The same pair the website uses.
            let green = pool(fx, fy, 0.16, 0.0, 0.62);
            let amber = pool(fx, fy, 0.93, 0.06, 0.55);
            let base = (248.0, 250.0, 248.0);
            let r = base.0 + green * (-36.0) + amber * 6.0;
            let g = base.1 + green * 4.0 + amber * (-6.0);
            let b = base.2 + green * (-18.0) + amber * (-42.0);
            let i = ((y * w + x) * 4) as usize;
            rgba[i] = clamp(r);
            rgba[i + 1] = clamp(g);
            rgba[i + 2] = clamp(b);
            rgba[i + 3] = 255;
        }
    }

    // The arrow, between the two icons at y = 200.
    let ink = (110, 122, 112);
    arrow(
        &mut rgba,
        w,
        268.0 * sf,
        392.0 * sf,
        200.0 * sf,
        3.0 * sf,
        ink,
    );

    // One instruction, and one line saying what the thing is.
    text(
        &mut rgba,
        w,
        h,
        "Drag GCloud Dot to Applications",
        20.0 * sf,
        318.0 * sf,
        (44, 52, 46),
    );
    text(
        &mut rgba,
        w,
        h,
        "your gcloud session, in the menu bar",
        13.0 * sf,
        348.0 * sf,
        (122, 134, 124),
    );

    rgba
}

/// Falloff of a soft radial pool centred at (cx, cy) in unit coordinates.
fn pool(x: f32, y: f32, cx: f32, cy: f32, radius: f32) -> f32 {
    let dx = (x - cx) * 1.15;
    let dy = (y - cy) * 1.6;
    let d = (dx * dx + dy * dy).sqrt() / radius;
    (1.0 - d).clamp(0.0, 1.0).powf(1.7)
}

fn clamp(v: f32) -> u8 {
    v.round().clamp(0.0, 255.0) as u8
}

fn blend(dst: &mut [u8], i: usize, c: (u8, u8, u8), a: f32) {
    let a = a.clamp(0.0, 1.0);
    dst[i] = (dst[i] as f32 * (1.0 - a) + c.0 as f32 * a).round() as u8;
    dst[i + 1] = (dst[i + 1] as f32 * (1.0 - a) + c.1 as f32 * a).round() as u8;
    dst[i + 2] = (dst[i + 2] as f32 * (1.0 - a) + c.2 as f32 * a).round() as u8;
}

/// A horizontal arrow with a solid head, antialiased vertically.
fn arrow(rgba: &mut [u8], w: u32, x0: f32, x1: f32, y: f32, thick: f32, c: (u8, u8, u8)) {
    let head = thick * 5.0;
    let shaft_end = x1 - head;

    for px in x0 as u32..shaft_end as u32 {
        for py in (y - thick) as u32..=(y + thick) as u32 {
            let d = (py as f32 - y).abs();
            let a = (thick / 2.0 - d + 0.5).clamp(0.0, 1.0);
            if a > 0.0 {
                blend(rgba, ((py * w + px) * 4) as usize, c, a * 0.9);
            }
        }
    }
    // Head: a triangle whose half height shrinks to nothing at the tip.
    for px in shaft_end as u32..x1 as u32 {
        let t = (px as f32 - shaft_end) / head;
        let half = head * 0.42 * (1.0 - t);
        for py in (y - half - 1.0) as u32..=(y + half + 1.0) as u32 {
            let d = (py as f32 - y).abs();
            let a = (half - d + 0.5).clamp(0.0, 1.0);
            if a > 0.0 {
                blend(rgba, ((py * w + px) * 4) as usize, c, a * 0.9);
            }
        }
    }
}

/// Centred single line of text, baseline positioned by its ink box.
fn text(rgba: &mut [u8], w: u32, h: u32, s: &str, px: f32, centre_y: f32, c: (u8, u8, u8)) {
    let Ok(font) = FontRef::try_from_slice(FONT_DATA) else {
        return;
    };
    let scaled = font.as_scaled(PxScale::from(px));
    let mut pen = 0.0f32;
    let mut glyphs: Vec<Glyph> = Vec::new();
    for ch in s.chars() {
        let mut g: Glyph = font.glyph_id(ch).with_scale(PxScale::from(px));
        g.position = ab_glyph::point(pen, 0.0);
        pen += scaled.h_advance(font.glyph_id(ch));
        glyphs.push(g);
    }
    let outlines: Vec<_> = glyphs
        .into_iter()
        .filter_map(|g| font.outline_glyph(g))
        .collect();
    if outlines.is_empty() {
        return;
    }
    let (mut min_x, mut max_x) = (f32::MAX, f32::MIN);
    let (mut min_y, mut max_y) = (f32::MAX, f32::MIN);
    for o in &outlines {
        let b = o.px_bounds();
        min_x = min_x.min(b.min.x);
        max_x = max_x.max(b.max.x);
        min_y = min_y.min(b.min.y);
        max_y = max_y.max(b.max.y);
    }
    let dx = (w as f32 - (max_x - min_x)) / 2.0 - min_x;
    let dy = centre_y - (max_y - min_y) / 2.0 - min_y;

    for o in outlines {
        let b = o.px_bounds();
        o.draw(|gx, gy, cov| {
            if cov <= 0.0 {
                return;
            }
            let x = gx as f32 + b.min.x + dx;
            let y = gy as f32 + b.min.y + dy;
            if x < 0.0 || y < 0.0 || x >= w as f32 || y >= h as f32 {
                return;
            }
            blend(rgba, ((y as u32 * w + x as u32) * 4) as usize, c, cov);
        });
    }
}

fn write_png(path: &str, rgba: &[u8], width: u32, height: u32) {
    let file = std::fs::File::create(path).expect("could not create the file");
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .expect("could not write the png header")
        .write_image_data(rgba)
        .expect("could not write the png body");
}
