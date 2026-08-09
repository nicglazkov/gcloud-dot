//! Writes the full set of tray icons to PNG.
//!
//! Two jobs: it is how the icon renderer gets looked at rather than only unit
//! tested, and it produces the images the website and README use, so those can
//! never drift from what the app actually draws.
//!
//! ```sh
//! cargo run -p gcloud-dot-app --example render_icons -- site/img
//! ```

// Compiled directly rather than reached through the binary crate, which an
// example cannot import. The platform-size helpers go unused here because this
// renders at a fixed documentation size, not at whatever the host tray wants.
#[path = "../src/icon.rs"]
#[allow(dead_code)]
mod icon;

use gcloud_dot_core::status::Level;

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "site/img".to_string());
    std::fs::create_dir_all(&out).expect("could not create the output directory");

    // Every state the status model can produce, at the size each platform uses.
    let cases: &[(&str, Option<&str>, Level)] = &[
        ("ok", Some("14h"), Level::Ok),
        ("ok-hours", Some("5h"), Level::Ok),
        ("warn", Some("99m"), Level::Warn),
        ("soon", Some("22m"), Level::Soon),
        ("expired", Some("!"), Level::Expired),
        ("unknown", Some("?"), Level::Unknown),
        ("days", Some("3d"), Level::Ok),
        ("dot-ok", None, Level::Ok),
        ("dot-warn", None, Level::Warn),
        ("dot-soon", None, Level::Soon),
        ("dot-expired", None, Level::Expired),
        ("dot-unknown", None, Level::Unknown),
    ];

    for (name, label, level) in cases {
        // Rendered at 4× the macOS display size so the PNGs are usable in
        // documentation without looking soft.
        let bitmap = icon::render(*label, *level, 176);
        let path = format!("{out}/icon-{name}.png");
        write_png(&path, &bitmap.rgba, bitmap.width, bitmap.height);
        println!("wrote {path}");
    }

    let app = app_icon(1024);
    let path = format!("{out}/appicon.png");
    write_png(&path, &app, 1024, 1024);
    println!("wrote {path}");
}

/// The application icon: a dark rounded square carrying the same green dot the
/// menu bar shows.
///
/// No text. An app icon is seen at 32 points in a Finder list as often as at
/// 512 in a DMG, and a countdown rendered at 32 points is an illegible smudge.
/// The dot alone survives every size, which is the whole test.
fn app_icon(size: u32) -> Vec<u8> {
    let mut rgba = vec![0u8; (size * size * 4) as usize];
    let s = size as f32;
    // Apple's icon grid insets the shape; matching it keeps this the same
    // visual weight as its neighbours in the Dock.
    let inset = s * 0.0586;
    let radius = s * 0.1856;
    let (left, top, right, bottom) = (inset, inset, s - inset, s - inset);

    for y in 0..size {
        for x in 0..size {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let coverage = rounded_rect_coverage(px, py, left, top, right, bottom, radius);
            if coverage <= 0.0 {
                continue;
            }
            // A vertical gradient, light at the top, the way a physical object
            // lit from above actually behaves.
            let t = (py - top) / (bottom - top);
            let bg = (
                lerp(38.0, 17.0, t) as u8,
                lerp(43.0, 20.0, t) as u8,
                lerp(52.0, 25.0, t) as u8,
            );
            let i = ((y * size + x) * 4) as usize;
            rgba[i] = bg.0;
            rgba[i + 1] = bg.1;
            rgba[i + 2] = bg.2;
            rgba[i + 3] = (coverage * 255.0) as u8;
        }
    }

    // The dot, with a soft halo so it reads as lit rather than pasted on.
    let centre = s / 2.0;
    let dot_r = s * 0.235;
    let (dr, dg, db) = (22.0, 163.0, 74.0);
    for y in 0..size {
        for x in 0..size {
            let dx = x as f32 + 0.5 - centre;
            let dy = y as f32 + 0.5 - centre;
            let dist = (dx * dx + dy * dy).sqrt();
            let core = (dot_r - dist + 0.5).clamp(0.0, 1.0);
            let halo = if dist > dot_r {
                (1.0 - (dist - dot_r) / (s * 0.10))
                    .clamp(0.0, 1.0)
                    .powf(2.4)
                    * 0.30
            } else {
                0.0
            };
            let a = core.max(halo);
            if a <= 0.0 {
                continue;
            }
            let i = ((y * size + x) * 4) as usize;
            if rgba[i + 3] == 0 {
                continue; // never paint outside the rounded square
            }
            rgba[i] = mix(rgba[i], dr, a);
            rgba[i + 1] = mix(rgba[i + 1], dg, a);
            rgba[i + 2] = mix(rgba[i + 2], db, a);
        }
    }
    rgba
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t.clamp(0.0, 1.0)
}

fn mix(dst: u8, src: f32, a: f32) -> u8 {
    (dst as f32 * (1.0 - a) + src * a).round().clamp(0.0, 255.0) as u8
}

/// Antialiased coverage of a rounded rectangle at one pixel.
fn rounded_rect_coverage(
    px: f32,
    py: f32,
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
    radius: f32,
) -> f32 {
    // Signed distance to the rounded rect, then a one-pixel falloff.
    let cx = (left + radius).max(px.min(right - radius));
    let cy = (top + radius).max(py.min(bottom - radius));
    let dx = px - cx;
    let dy = py - cy;
    let dist = (dx * dx + dy * dy).sqrt();
    let inside_box = px >= left && px <= right && py >= top && py <= bottom;
    if dist <= 0.0001 {
        return if inside_box { 1.0 } else { 0.0 };
    }
    (radius - dist + 0.5).clamp(0.0, 1.0)
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
