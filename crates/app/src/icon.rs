//! Draws the tray icon.
//!
//! The countdown is rendered into the bitmap on Windows and Linux because
//! neither platform gives a tray icon a text slot. macOS does, `NSStatusItem`
//! has a title beside the image, so there the icon stays a plain dot and the
//! text is set natively, which is what every other menu bar app does and what
//! the system expects when it truncates the bar.

use ab_glyph::{Font, FontRef, Glyph, PxScale, ScaleFont};
use gcloud_dot_core::status::Level;

/// Inconsolata Bold, SIL Open Font License 1.1. Monospaced on purpose: a
/// proportional face makes the icon visibly change width as "14h" becomes
/// "9h", which reads as flicker in the corner of the eye.
const FONT_DATA: &[u8] = include_bytes!("../../../assets/Inconsolata-Bold.ttf");

/// Rendered pixels, ready for `tray_icon::Icon::from_rgba`.
pub struct Bitmap {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// The size each platform wants.
///
/// Windows rasterises to a real HICON and shows it at 16, 24, or 32 device
/// pixels depending on DPI, so 32 is the largest useful source. macOS and the
/// Linux indicator both scale a larger source down cleanly.
pub const fn native_size() -> u32 {
    if cfg!(windows) {
        32
    } else {
        44
    }
}

/// Whether the countdown belongs inside the icon on this platform.
pub const fn text_goes_in_icon() -> bool {
    !cfg!(target_os = "macos")
}

/// Draw a filled disc, optionally with a short label centred on it.
pub fn render(label: Option<&str>, level: Level, size: u32) -> Bitmap {
    let mut rgba = vec![0u8; (size * size * 4) as usize];
    let (r, g, b) = level.rgb();

    // Leave a little breathing room so the disc is not clipped by the tray's
    // own padding, and so the antialiased edge has somewhere to fall off.
    let radius = size as f32 / 2.0 - size as f32 * 0.04;
    let centre = size as f32 / 2.0;

    for y in 0..size {
        for x in 0..size {
            let dx = x as f32 + 0.5 - centre;
            let dy = y as f32 + 0.5 - centre;
            let dist = (dx * dx + dy * dy).sqrt();
            // One pixel of linear falloff at the edge is enough to look smooth
            // at every size this is drawn at, and costs no supersampling.
            let coverage = (radius - dist + 0.5).clamp(0.0, 1.0);
            if coverage > 0.0 {
                let i = ((y * size + x) * 4) as usize;
                rgba[i] = r;
                rgba[i + 1] = g;
                rgba[i + 2] = b;
                rgba[i + 3] = (coverage * 255.0).round() as u8;
            }
        }
    }

    if let Some(text) = label {
        draw_centred_text(&mut rgba, size, text, level.ink());
    }

    Bitmap {
        rgba,
        width: size,
        height: size,
    }
}

/// Font size as a fraction of the icon, by label length.
///
/// Three characters is the widest the status model can produce, and these
/// fractions are what keep "100m" from ever being attempted: the label is
/// capped upstream, and this only has to look right for one, two, and three.
fn scale_for(len: usize, size: f32) -> f32 {
    match len {
        0 | 1 => size * 0.60,
        2 => size * 0.50,
        _ => size * 0.40,
    }
}

fn draw_centred_text(rgba: &mut [u8], size: u32, text: &str, ink: (u8, u8, u8)) {
    let Ok(font) = FontRef::try_from_slice(FONT_DATA) else {
        return;
    };
    let px = scale_for(text.chars().count(), size as f32);
    let scaled = font.as_scaled(PxScale::from(px));

    // Lay the glyphs out on an arbitrary baseline first, measure the ink they
    // actually cover, then translate that box to the centre of the icon.
    // Centring on font metrics instead would sit visibly high, because ascent
    // includes room for accents these glyphs do not have.
    let mut pen = 0.0f32;
    let mut glyphs: Vec<Glyph> = Vec::new();
    for ch in text.chars() {
        let mut glyph: Glyph = font.glyph_id(ch).with_scale(PxScale::from(px));
        glyph.position = ab_glyph::point(pen, 0.0);
        pen += scaled.h_advance(font.glyph_id(ch));
        glyphs.push(glyph);
    }

    let outlines: Vec<_> = glyphs
        .into_iter()
        .filter_map(|g| font.outline_glyph(g))
        .collect();
    if outlines.is_empty() {
        return;
    }

    let mut min_x = f32::MAX;
    let mut max_x = f32::MIN;
    let mut min_y = f32::MAX;
    let mut max_y = f32::MIN;
    for o in &outlines {
        let b = o.px_bounds();
        min_x = min_x.min(b.min.x);
        max_x = max_x.max(b.max.x);
        min_y = min_y.min(b.min.y);
        max_y = max_y.max(b.max.y);
    }

    let offset_x = (size as f32 - (max_x - min_x)) / 2.0 - min_x;
    let offset_y = (size as f32 - (max_y - min_y)) / 2.0 - min_y;

    for outline in outlines {
        let bounds = outline.px_bounds();
        outline.draw(|gx, gy, coverage| {
            if coverage <= 0.0 {
                return;
            }
            let px_x = gx as f32 + bounds.min.x + offset_x;
            let px_y = gy as f32 + bounds.min.y + offset_y;
            if px_x < 0.0 || px_y < 0.0 {
                return;
            }
            let (x, y) = (px_x as u32, px_y as u32);
            if x >= size || y >= size {
                return;
            }
            let i = ((y * size + x) * 4) as usize;
            let a = coverage.clamp(0.0, 1.0);
            // Source-over onto the disc. The disc is already opaque wherever
            // text lands, so this is a straight lerp of the colour channels.
            rgba[i] = blend(rgba[i], ink.0, a);
            rgba[i + 1] = blend(rgba[i + 1], ink.1, a);
            rgba[i + 2] = blend(rgba[i + 2], ink.2, a);
            rgba[i + 3] = rgba[i + 3].max((a * 255.0) as u8);
        });
    }
}

fn blend(dst: u8, src: u8, a: f32) -> u8 {
    (dst as f32 * (1.0 - a) + src as f32 * a)
        .round()
        .clamp(0.0, 255.0) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel(bm: &Bitmap, x: u32, y: u32) -> (u8, u8, u8, u8) {
        let i = ((y * bm.width + x) * 4) as usize;
        (bm.rgba[i], bm.rgba[i + 1], bm.rgba[i + 2], bm.rgba[i + 3])
    }

    #[test]
    fn produces_correctly_sized_rgba() {
        let bm = render(None, Level::Ok, 44);
        assert_eq!(bm.width, 44);
        assert_eq!(bm.height, 44);
        assert_eq!(bm.rgba.len(), 44 * 44 * 4);
    }

    #[test]
    fn corners_are_transparent_and_the_middle_is_not() {
        let bm = render(None, Level::Ok, 44);
        assert_eq!(pixel(&bm, 0, 0).3, 0, "corner should be outside the disc");
        assert_eq!(pixel(&bm, 22, 22).3, 255, "centre should be opaque");
    }

    #[test]
    fn the_disc_takes_the_level_colour() {
        let bm = render(None, Level::Expired, 44);
        let (r, g, b, _) = pixel(&bm, 22, 22);
        assert_eq!((r, g, b), Level::Expired.rgb());
    }

    #[test]
    fn the_edge_is_antialiased() {
        // Some pixel on the boundary must be partially transparent, or the
        // disc will look like a jagged blob at 18 points.
        let bm = render(None, Level::Ok, 44);
        let partial = (0..44)
            .flat_map(|y| (0..44).map(move |x| (x, y)))
            .any(|(x, y)| {
                let a = pixel(&bm, x, y).3;
                a > 0 && a < 255
            });
        assert!(partial);
    }

    #[test]
    fn text_marks_the_disc() {
        let plain = render(None, Level::Ok, 44);
        let labelled = render(Some("14h"), Level::Ok, 44);
        assert_ne!(plain.rgba, labelled.rgba, "the label should have drawn");
    }

    #[test]
    fn every_label_the_status_model_can_emit_renders() {
        // Guards against a glyph missing from the font, which would silently
        // draw a blank disc rather than fail.
        for label in ["!", "?", "ok", "0m", "5m", "99m", "14h", "3d"] {
            let plain = render(None, Level::Warn, 32);
            let bm = render(Some(label), Level::Warn, 32);
            assert_ne!(bm.rgba, plain.rgba, "nothing drawn for {label:?}");
        }
    }

    #[test]
    fn text_stays_inside_the_bitmap() {
        // Three characters at the smallest size is the tightest fit there is.
        let bm = render(Some("99m"), Level::Warn, 32);
        assert_eq!(bm.rgba.len(), 32 * 32 * 4);
        for x in 0..32 {
            assert_eq!(pixel(&bm, x, 0).3, 0, "row 0 should stay clear");
        }
    }

    #[test]
    fn light_levels_use_dark_ink_so_the_text_reads() {
        let bm = render(Some("45m"), Level::Warn, 44);
        // Somewhere in the middle band there must be a pixel close to the dark
        // ink colour rather than the yellow disc.
        let dark = (14..30)
            .flat_map(|y| (0..44).map(move |x| (x, y)))
            .any(|(x, y)| {
                let (r, g, b, _) = pixel(&bm, x, y);
                r < 80 && g < 80 && b < 80
            });
        assert!(dark, "dark ink not found on a light disc");
    }

    #[test]
    fn platform_size_is_sane() {
        assert!(native_size() >= 32);
    }
}
