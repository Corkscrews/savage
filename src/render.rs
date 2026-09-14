use resvg::tiny_skia::{BlendMode, Color, Paint, Pixmap, Rect, Transform};
use resvg::usvg;

const LETTERBOX: u32 = 0x0012_1214;

/// Most of the artwork's own hue the page is allowed to keep; the rest is
/// plain black or white. Scaled by saturation, so grey artwork gets neither.
const MAX_TINT: f32 = 0.35;

/// Contrast ratio (WCAG AA) the page holds against the artwork in front of it.
/// The tint gives way to keep this; the black-or-white choice never does.
const MIN_CONTRAST: f32 = 4.5;

/// Fit `tree` into `width`×`height` and write 0x00RRGGBB pixels into `out`.
pub fn rasterize(tree: &usvg::Tree, width: u32, height: u32, out: &mut [u32]) {
    if out.len() != width as usize * height as usize {
        fill_empty(out);
        return;
    }

    let Some(mut pixmap) = Pixmap::new(width, height) else {
        fill_empty(out);
        return;
    };

    let svg_w = tree.size().width();
    let svg_h = tree.size().height();
    if svg_w > 0.0 && svg_h > 0.0 {
        let scale = (width as f32 / svg_w).min(height as f32 / svg_h);
        let dx = (width as f32 - svg_w * scale) * 0.5;
        let dy = (height as f32 - svg_h * scale) * 0.5;

        // Draw onto the transparent canvas first: what the artwork leaves
        // uncovered is exactly what the page background has to fill in.
        let transform = Transform::from_row(scale, 0.0, 0.0, scale, dx, dy);
        resvg::render(tree, transform, &mut pixmap.as_mut());

        if let Some(page) = Rect::from_xywh(dx, dy, svg_w * scale, svg_h * scale) {
            if let Some(color) = page_backdrop(&pixmap, page) {
                fill_behind(&mut pixmap, page, color);
            }
        }
    }

    if let Some(canvas) = Rect::from_xywh(0.0, 0.0, width as f32, height as f32) {
        fill_behind(
            &mut pixmap,
            canvas,
            Color::from_rgba8(0x12, 0x12, 0x14, 0xFF),
        );
    }

    blit(&pixmap, out);
}

pub fn fill_empty(out: &mut [u32]) {
    out.fill(LETTERBOX);
}

/// Pick the page background for artwork that does not paint its own.
///
/// Returns `None` when the page came out opaque — the SVG brought a background
/// of its own and anything painted behind it would never be seen.
fn page_backdrop(pixmap: &Pixmap, page: Rect) -> Option<Color> {
    let (x0, y0, x1, y1) = clamp_to_pixels(page, pixmap.width(), pixmap.height())?;

    let mut sum = [0.0f32; 3];
    let mut alpha = 0.0f32;
    let mut opaque = true;
    for y in y0..y1 {
        let row = y as usize * pixmap.width() as usize;
        for px in &pixmap.pixels()[row + x0 as usize..row + x1 as usize] {
            // tiny-skia stores premultiplied channels, so dividing these sums
            // by the alpha sum gives the alpha-weighted mean of the artwork —
            // fully transparent pixels contribute nothing, as they should.
            sum[0] += f32::from(px.red());
            sum[1] += f32::from(px.green());
            sum[2] += f32::from(px.blue());
            alpha += f32::from(px.alpha());
            opaque &= px.alpha() == u8::MAX;
        }
    }

    if opaque {
        return None;
    }
    if alpha <= 0.0 {
        // Nothing was drawn at all; a blank white page beats a blank black one.
        return Some(Color::WHITE);
    }

    Some(contrasting_page([
        sum[0] / alpha,
        sum[1] / alpha,
        sum[2] / alpha,
    ]))
}

/// Black or white, whichever the artwork reads against, carrying as much of
/// the artwork's own hue as it can without spending the contrast that put it
/// there: white icons land on black, a bright blue one on a deep blue.
fn contrasting_page(art: [f32; 3]) -> Color {
    let art_luminance = relative_luminance(art);
    let base = if contrast_ratio(art_luminance, 0.0) >= contrast_ratio(art_luminance, 1.0) {
        0.0
    } else {
        1.0
    };

    let tinted = |t: f32| art.map(|c| (base + (c - base) * t).clamp(0.0, 1.0));
    let reads =
        |t: f32| contrast_ratio(art_luminance, relative_luminance(tinted(t))) >= MIN_CONTRAST;

    // Grey artwork has no hue worth borrowing, saturated artwork has plenty.
    let chroma =
        art.iter().fold(0.0f32, |a, &b| a.max(b)) - art.iter().fold(1.0f32, |a, &b| a.min(b));
    let mut tint = MAX_TINT * chroma;
    if !reads(tint) {
        // The page slides monotonically from the base towards the artwork as
        // the tint grows, so the strongest tint that still reads is a bisection
        // away. If even the bare base falls short, `lo` never moves off zero.
        let (mut lo, mut hi) = (0.0, tint);
        for _ in 0..12 {
            let mid = 0.5 * (lo + hi);
            if reads(mid) {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        tint = lo;
    }

    let [r, g, b] = tinted(tint);
    Color::from_rgba(r, g, b, 1.0).unwrap_or(Color::WHITE)
}

/// WCAG relative luminance: sRGB channels linearised, then Rec. 709 weights.
fn relative_luminance(color: [f32; 3]) -> f32 {
    let linear = |c: f32| {
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(color[0]) + 0.7152 * linear(color[1]) + 0.0722 * linear(color[2])
}

/// WCAG contrast ratio between two luminances: 1.0 identical, 21.0 at most.
fn contrast_ratio(a: f32, b: f32) -> f32 {
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

fn clamp_to_pixels(rect: Rect, width: u32, height: u32) -> Option<(u32, u32, u32, u32)> {
    let x0 = rect.left().floor().clamp(0.0, width as f32) as u32;
    let y0 = rect.top().floor().clamp(0.0, height as f32) as u32;
    let x1 = rect.right().ceil().clamp(0.0, width as f32) as u32;
    let y1 = rect.bottom().ceil().clamp(0.0, height as f32) as u32;
    (x0 < x1 && y0 < y1).then_some((x0, y0, x1, y1))
}

fn fill_behind(pixmap: &mut Pixmap, rect: Rect, color: Color) {
    let mut paint = Paint::default();
    paint.set_color(color);
    paint.anti_alias = false;
    paint.blend_mode = BlendMode::DestinationOver;
    pixmap.fill_rect(rect, &paint, Transform::identity(), None);
}

fn blit(pixmap: &Pixmap, out: &mut [u32]) {
    for (dest, rgba) in out.iter_mut().zip(pixmap.data().chunks_exact(4)) {
        *dest = (u32::from(rgba[0]) << 16) | (u32::from(rgba[1]) << 8) | u32::from(rgba[2]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document;

    /// Render a 64-unit square SVG into a 64×64 buffer, so the page covers the
    /// whole buffer and pixel 0 is a page corner the artwork never touches.
    fn render_icon(body: &str) -> Vec<u32> {
        let svg = format!(
            "<svg xmlns='http://www.w3.org/2000/svg' width='64' height='64' \
             viewBox='0 0 64 64'>{body}</svg>"
        );
        let tree = usvg::Tree::from_str(&svg, &usvg::Options::default()).unwrap();
        let mut pixels = vec![0u32; 64 * 64];
        rasterize(&tree, 64, 64, &mut pixels);
        pixels
    }

    fn circle(fill: &str) -> String {
        format!("<circle cx='32' cy='32' r='20' fill='{fill}'/>")
    }

    fn channels(pixel: u32) -> [f32; 3] {
        [
            ((pixel >> 16) & 0xFF) as f32 / 255.0,
            ((pixel >> 8) & 0xFF) as f32 / 255.0,
            (pixel & 0xFF) as f32 / 255.0,
        ]
    }

    /// The page colour chosen for a flat icon of `fill`, as 0xRRGGBB.
    fn page_for(fill: &str) -> u32 {
        render_icon(&circle(fill))[0] & 0x00FF_FFFF
    }

    #[test]
    fn rasterizes_sample() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/sample.svg");
        let doc = document::load(&path).unwrap();
        let mut pixels = vec![0u32; 64 * 64];
        rasterize(&doc.tree, 64, 64, &mut pixels);
        assert!(
            pixels.iter().any(|&p| p != LETTERBOX),
            "fitted SVG should paint over the letterbox"
        );
    }

    #[test]
    fn light_artwork_lands_on_black() {
        assert_eq!(page_for("white"), 0x0000_0000);
        assert_eq!(page_for("rgb(128,128,128)"), 0x0000_0000);
    }

    #[test]
    fn dark_artwork_lands_on_white() {
        assert_eq!(page_for("black"), 0x00FF_FFFF);
        let navy = page_for("rgb(20,24,40)");
        assert!(
            relative_luminance(channels(navy)) > 0.9,
            "near-grey navy should sit on an all-but-white page, got {navy:#08x}"
        );
    }

    #[test]
    fn pure_blue_lands_on_a_light_blue() {
        // The tint works towards white as readily as towards black.
        let page = page_for("rgb(0,0,255)");
        let [r, g, b] = channels(page);
        assert!(
            b > r && b > g && relative_luminance([r, g, b]) > 0.4,
            "page should be a light blue, got {page:#08x}"
        );
    }

    #[test]
    fn bright_blue_lands_on_deep_blue() {
        let page = channels(page_for("rgb(33,150,243)"));
        assert!(
            page[2] > page[0] && page[2] > page[1],
            "page should keep the artwork's blue cast, got {:#08x}",
            page_for("rgb(33,150,243)")
        );
        assert!(
            relative_luminance(page) < 0.1,
            "page should still be a deep blue, got {:#08x}",
            page_for("rgb(33,150,243)")
        );
    }

    #[test]
    fn grey_artwork_gets_no_tint() {
        // Nothing to borrow from an unsaturated icon: plain black, not a cast.
        assert_eq!(page_for("rgb(200,200,200)"), 0x0000_0000);
    }

    #[test]
    fn the_tint_never_eats_the_contrast() {
        for fill in [
            "white",
            "black",
            "rgb(33,150,243)",
            "rgb(232,93,4)",
            "rgb(0,0,255)",
            "rgb(0,255,0)",
            "rgb(244,241,234)",
            "rgb(16,24,40)",
            "rgb(128,128,128)",
        ] {
            let art = channels(render_icon(&circle(fill))[32 * 64 + 32]);
            let page = channels(page_for(fill));
            let ratio = contrast_ratio(relative_luminance(art), relative_luminance(page));
            // A tint is only ever spent down to plain black or white; some
            // mid-tones cannot reach AA against either, and that is the floor.
            let untinted = page.iter().all(|&c| c <= 0.004) || page.iter().all(|&c| c >= 0.996);
            assert!(
                ratio >= MIN_CONTRAST - 0.01 || untinted,
                "{fill}: page {page:?} only reaches {ratio:.2}:1"
            );
        }
    }

    #[test]
    fn opaque_svg_keeps_its_own_background() {
        let body = format!(
            "<rect width='64' height='64' fill='rgb(20,20,22)'/>{}",
            circle("white")
        );
        let pixels = render_icon(&body);
        assert_eq!(
            pixels[0] & 0x00FF_FFFF,
            0x0014_1416,
            "an SVG that paints its own background must not be recoloured"
        );
    }

    #[test]
    fn empty_svg_falls_back_to_white() {
        let pixels = render_icon("<rect width='0' height='0' fill='white'/>");
        assert_eq!(pixels[0] & 0x00FF_FFFF, 0x00FF_FFFF);
    }
}
