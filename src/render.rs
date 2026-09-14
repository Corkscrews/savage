use resvg::tiny_skia::{Pixmap, Transform};
use resvg::usvg;

/// The grey behind everything: the letterbox bars, and the whole window when
/// there is nothing to draw.
const LETTERBOX_RGB: [u8; 3] = [0x12, 0x12, 0x14];
const LETTERBOX: u32 = pack(LETTERBOX_RGB);

/// Most of the artwork's own hue the page is allowed to keep; the rest is
/// plain black or white. Scaled by saturation, so grey artwork gets neither.
const MAX_TINT: f32 = 0.35;

/// Contrast ratio (WCAG AA) the page holds against the artwork in front of it.
/// The tint gives way to keep this; the black-or-white choice never does.
const MIN_CONTRAST: f32 = 4.5;

/// Owns the rasterization canvas so that it outlives a frame: it is the one
/// large allocation a frame needs — 33 MB at 4K — and dragging a window edge
/// asks for a new frame per event.
#[derive(Default)]
pub struct Renderer {
    canvas: Option<Pixmap>,
}

impl Renderer {
    pub const fn new() -> Self {
        Self { canvas: None }
    }

    /// Fit `tree` into `width`×`height` and write 0x00RRGGBB pixels into `out`.
    pub fn rasterize(&mut self, tree: &usvg::Tree, width: u32, height: u32, out: &mut [u32]) {
        let svg_w = tree.size().width();
        let svg_h = tree.size().height();
        if out.len() != width as usize * height as usize || !(svg_w > 0.0 && svg_h > 0.0) {
            fill_empty(out);
            return;
        }
        let Some(canvas) = self.canvas(width, height) else {
            fill_empty(out);
            return;
        };

        let scale = (width as f32 / svg_w).min(height as f32 / svg_h);
        let dx = (width as f32 - svg_w * scale) * 0.5;
        let dy = (height as f32 - svg_h * scale) * 0.5;

        // Draw onto the transparent canvas first: what the artwork leaves
        // uncovered is exactly what the page background has to fill in.
        let transform = Transform::from_row(scale, 0.0, 0.0, scale, dx, dy);
        resvg::render(tree, transform, &mut canvas.as_mut());

        let page = page_background(canvas, dx, dy, svg_w * scale, svg_h * scale);
        compose(canvas, page, out);
    }

    /// A transparent canvas of the requested size, reusing the last one when
    /// the window has not changed size — which is every frame but a resize.
    fn canvas(&mut self, width: u32, height: u32) -> Option<&mut Pixmap> {
        match &mut self.canvas {
            Some(canvas) if canvas.width() == width && canvas.height() == height => {
                // A fresh pixmap starts transparent; a reused one is cleared.
                canvas.data_mut().fill(0);
            }
            slot => {
                // Let go of the old canvas before asking for the new one, so a
                // resize drag reuses the same block instead of holding two.
                *slot = None;
                *slot = Pixmap::new(width, height);
            }
        }
        self.canvas.as_mut()
    }
}

pub fn fill_empty(out: &mut [u32]) {
    out.fill(LETTERBOX);
}

/// The page the artwork sits on: the pixels to paint, and the colour for them.
///
/// Returns `None` when the page came out opaque — the SVG brought a background
/// of its own and anything painted behind it would never be seen.
fn page_background(canvas: &Pixmap, x: f32, y: f32, w: f32, h: f32) -> Option<(Bounds, [u8; 3])> {
    let (width, height) = (canvas.width(), canvas.height());
    let color = page_color(canvas, Bounds::touched(x, y, w, h, width, height)?)?;
    Some((Bounds::filled(x, y, w, h, width, height)?, color))
}

/// Pick the page background for artwork that does not paint its own.
fn page_color(canvas: &Pixmap, area: Bounds) -> Option<[u8; 3]> {
    let stride = bytes(canvas.width() as usize);
    let data = canvas.data();

    let mut sums = [0u64; 4];
    let mut opaque = true;
    for y in area.y0 as usize..area.y1 as usize {
        let row = y * stride;
        let (span, all_opaque) =
            span_stats(&data[row + bytes(area.x0 as usize)..row + bytes(area.x1 as usize)]);
        for (sum, add) in sums.iter_mut().zip(span) {
            *sum += add;
        }
        opaque &= all_opaque;
    }

    if opaque {
        return None;
    }
    let alpha = sums[3];
    if alpha == 0 {
        // Nothing was drawn at all; a blank white page beats a blank black one.
        return Some([u8::MAX; 3]);
    }

    // tiny-skia stores premultiplied channels, so dividing these sums by the
    // alpha sum gives the alpha-weighted mean of the artwork — fully
    // transparent pixels contribute nothing, as they should.
    let alpha = alpha as f32;
    Some(contrasting_page([
        sums[0] as f32 / alpha,
        sums[1] as f32 / alpha,
        sums[2] as f32 / alpha,
    ]))
}

/// Black or white, whichever the artwork reads against, carrying as much of
/// the artwork's own hue as it can without spending the contrast that put it
/// there: white icons land on black, a bright blue one on a deep blue.
fn contrasting_page(art: [f32; 3]) -> [u8; 3] {
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

    tinted(tint).map(|c| (c * 255.0 + 0.5) as u8)
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

/// A half-open pixel rect, clipped to the canvas.
#[derive(Clone, Copy)]
struct Bounds {
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
}

impl Bounds {
    /// Every pixel the rect touches, however little of it: the artwork's own
    /// fringe is part of what the page has to be read against.
    fn touched(x: f32, y: f32, w: f32, h: f32, width: u32, height: u32) -> Option<Self> {
        Self::new(
            x.floor(),
            y.floor(),
            (x + w).ceil(),
            (y + h).ceil(),
            width,
            height,
        )
    }

    /// The pixels a tiny-skia `fill_rect` covers without anti-aliasing. The
    /// page is painted here rather than through tiny-skia, so it has to land on
    /// the same pixels: every edge rounds through `(edge.floor() + 0.5) as i32`
    /// — plain `floor` for the coordinates of a centred page — and a rect is
    /// never allowed to round away to nothing.
    fn filled(x: f32, y: f32, w: f32, h: f32, width: u32, height: u32) -> Option<Self> {
        let (x0, y0) = (x.floor(), y.floor());
        Self::new(
            x0,
            y0,
            (x + w).floor().max(x0 + 1.0),
            (y + h).floor().max(y0 + 1.0),
            width,
            height,
        )
    }

    fn new(x0: f32, y0: f32, x1: f32, y1: f32, width: u32, height: u32) -> Option<Self> {
        let (x0, x1) = (x0.clamp(0.0, width as f32), x1.clamp(0.0, width as f32));
        let (y0, y1) = (y0.clamp(0.0, height as f32), y1.clamp(0.0, height as f32));
        (x0 < x1 && y0 < y1).then_some(Self {
            x0: x0 as u32,
            y0: y0 as u32,
            x1: x1 as u32,
            y1: y1 as u32,
        })
    }
}

/// Copy the canvas out as 0x00RRGGBB, painting the page and the letterbox in
/// behind the artwork on the way. Both are opaque and both go *under* what is
/// already there, so they cost nothing beyond the copy that has to happen
/// anyway — and the canvas is read once instead of three times.
fn compose(canvas: &Pixmap, page: Option<(Bounds, [u8; 3])>, out: &mut [u32]) {
    let width = canvas.width() as usize;
    let data = canvas.data();

    let rows = out
        .chunks_exact_mut(width)
        .zip(data.chunks_exact(bytes(width)));
    for (y, (row, src)) in rows.enumerate() {
        let Some((area, color)) =
            page.filter(|(area, _)| y >= area.y0 as usize && y < area.y1 as usize)
        else {
            composite(src, row, LETTERBOX_RGB);
            continue;
        };

        let (x0, x1) = (area.x0 as usize, area.x1 as usize);
        let (left, rest) = row.split_at_mut(x0);
        let (middle, right) = rest.split_at_mut(x1 - x0);
        composite(&src[..bytes(x0)], left, LETTERBOX_RGB);
        composite(&src[bytes(x0)..bytes(x1)], middle, color);
        composite(&src[bytes(x1)..], right, LETTERBOX_RGB);
    }
}

/// Byte offset of a pixel index, the canvas being 4 bytes per pixel.
const fn bytes(pixels: usize) -> usize {
    pixels * 4
}

const fn pack(rgb: [u8; 3]) -> u32 {
    ((rgb[0] as u32) << 16) | ((rgb[1] as u32) << 8) | rgb[2] as u32
}

/// Premultiplied per-channel sums over a span of RGBA pixels, and whether
/// every one of them was opaque.
#[inline]
fn span_stats(span: &[u8]) -> ([u64; 4], bool) {
    #[cfg(target_arch = "aarch64")]
    {
        // SAFETY: NEON is part of the aarch64 baseline, so the intrinsics
        // `span_stats` is built from are always available here.
        unsafe { neon::span_stats(span) }
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        span_stats_scalar(span)
    }
}

fn span_stats_scalar(span: &[u8]) -> ([u64; 4], bool) {
    let mut sums = [0u64; 4];
    let mut opaque = u8::MAX;
    // 32-bit lanes are enough for a block this size and keep the inner loop
    // narrow enough to vectorise; they are widened once per block.
    for block in span.chunks(bytes(4096)) {
        let mut acc = [0u32; 4];
        for px in block.chunks_exact(4) {
            for (channel, &value) in acc.iter_mut().zip(px) {
                *channel += u32::from(value);
            }
            opaque &= px[3];
        }
        for (sum, add) in sums.iter_mut().zip(acc) {
            *sum += u64::from(add);
        }
    }
    (sums, opaque == u8::MAX)
}

/// Paint `color` behind a span of premultiplied RGBA pixels and write the
/// result out as 0x00RRGGBB.
#[inline]
fn composite(src: &[u8], dst: &mut [u32], color: [u8; 3]) {
    debug_assert_eq!(src.len(), bytes(dst.len()));
    #[cfg(target_arch = "aarch64")]
    {
        // SAFETY: as above — NEON is unconditional on aarch64.
        unsafe { neon::composite(src, dst, color) }
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        composite_scalar(src, dst, color)
    }
}

fn composite_scalar(src: &[u8], dst: &mut [u32], color: [u8; 3]) {
    for (out, px) in dst.iter_mut().zip(src.chunks_exact(4)) {
        let inverse = 255 - u32::from(px[3]);
        let mut rgb = 0;
        for (channel, &source) in px.iter().zip(&color) {
            // Destination-over, in the integer form tiny-skia's own lowp
            // pipeline uses: d + div255(s × (1 - dₐ)).
            rgb = (rgb << 8) | (u32::from(*channel) + ((u32::from(source) * inverse + 255) >> 8));
        }
        *out = rgb;
    }
}

#[cfg(target_arch = "aarch64")]
mod neon {
    use std::arch::aarch64::*;

    /// 16 pixels at a time; see [`super::span_stats`].
    #[target_feature(enable = "neon")]
    pub unsafe fn span_stats(span: &[u8]) -> ([u64; 4], bool) {
        let vectors = span.len() / 64;
        let mut source = span.as_ptr();
        let mut sums = [0u64; 4];
        let mut opaque = vdupq_n_u8(u8::MAX);

        let mut done = 0;
        while done < vectors {
            // A pairwise-widening add puts two bytes in each 16-bit lane, so a
            // lane can take 128 vectors before it could overflow.
            let block = (vectors - done).min(128);
            let mut acc = [vdupq_n_u16(0); 4];
            for _ in 0..block {
                let px = vld4q_u8(source);
                acc[0] = vpadalq_u8(acc[0], px.0);
                acc[1] = vpadalq_u8(acc[1], px.1);
                acc[2] = vpadalq_u8(acc[2], px.2);
                acc[3] = vpadalq_u8(acc[3], px.3);
                opaque = vandq_u8(opaque, px.3);
                source = source.add(64);
            }
            for (sum, acc) in sums.iter_mut().zip(acc) {
                *sum += u64::from(vaddlvq_u16(acc));
            }
            done += block;
        }

        let (tail, tail_opaque) = super::span_stats_scalar(&span[vectors * 64..]);
        for (sum, add) in sums.iter_mut().zip(tail) {
            *sum += add;
        }
        (sums, tail_opaque && vminvq_u8(opaque) == u8::MAX)
    }

    /// 16 pixels at a time; see [`super::composite`].
    #[target_feature(enable = "neon")]
    pub unsafe fn composite(src: &[u8], dst: &mut [u32], color: [u8; 3]) {
        let vectors = src.len().min(super::bytes(dst.len())) / 64;
        let mut source = src.as_ptr();
        let mut out = dst.as_mut_ptr();
        let zero = vdupq_n_u8(0);

        for _ in 0..vectors {
            let px = vld4q_u8(source);
            let inverse = vsubq_u8(vdupq_n_u8(u8::MAX), px.3);
            let r = over(px.0, inverse, color[0]);
            let g = over(px.1, inverse, color[1]);
            let b = over(px.2, inverse, color[2]);
            // 0x00RRGGBB is B, G, R, 0 in memory on a little-endian target.
            #[cfg(target_endian = "little")]
            let pixels = uint8x16x4_t(b, g, r, zero);
            #[cfg(target_endian = "big")]
            let pixels = uint8x16x4_t(zero, r, g, b);
            vst4q_u8(out.cast::<u8>(), pixels);
            source = source.add(64);
            out = out.add(16);
        }

        let done = vectors * 16;
        super::composite_scalar(&src[super::bytes(done)..], &mut dst[done..], color);
    }

    /// Destination-over for one channel: `d + div255(s × inverse)`.
    #[target_feature(enable = "neon")]
    unsafe fn over(d: uint8x16_t, inverse: uint8x16_t, s: u8) -> uint8x16_t {
        let s = vdupq_n_u8(s);
        let bias = vdupq_n_u16(255);
        let low = vshrn_n_u16::<8>(vaddq_u16(
            vmull_u8(vget_low_u8(inverse), vget_low_u8(s)),
            bias,
        ));
        let high = vshrn_n_u16::<8>(vaddq_u16(vmull_high_u8(inverse, s), bias));
        vaddq_u8(d, vcombine_u8(low, high))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document;
    use resvg::tiny_skia::{BlendMode, Color, Paint, Rect};

    /// Render a 64-unit square SVG into a 64×64 buffer, so the page covers the
    /// whole buffer and pixel 0 is a page corner the artwork never touches.
    fn render_icon(body: &str) -> Vec<u32> {
        let svg = format!(
            "<svg xmlns='http://www.w3.org/2000/svg' width='64' height='64' \
             viewBox='0 0 64 64'>{body}</svg>"
        );
        let tree = usvg::Tree::from_str(&svg, &usvg::Options::default()).unwrap();
        let mut pixels = vec![0u32; 64 * 64];
        Renderer::new().rasterize(&tree, 64, 64, &mut pixels);
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
        Renderer::new().rasterize(&doc.tree, 64, 64, &mut pixels);
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

    #[test]
    fn a_buffer_of_the_wrong_size_is_all_letterbox() {
        let svg = "<svg xmlns='http://www.w3.org/2000/svg' width='64' height='64'/>";
        let tree = usvg::Tree::from_str(svg, &usvg::Options::default()).unwrap();
        let mut pixels = vec![0u32; 16 * 9];
        Renderer::new().rasterize(&tree, 32, 32, &mut pixels);
        assert!(pixels.iter().all(|&p| p == LETTERBOX));
    }

    #[test]
    fn a_reused_canvas_renders_the_same_frame() {
        let tree = usvg::Tree::from_str(
            &format!(
                "<svg xmlns='http://www.w3.org/2000/svg' width='64' height='64' \
                 viewBox='0 0 64 64'>{}</svg>",
                circle("rgb(33,150,243)")
            ),
            &usvg::Options::default(),
        )
        .unwrap();

        let mut renderer = Renderer::new();
        let mut first = vec![0u32; 40 * 24];
        renderer.rasterize(&tree, 40, 24, &mut first);
        // A different size forces a new canvas, then back to a reused one.
        let mut other = vec![0u32; 24 * 40];
        renderer.rasterize(&tree, 24, 40, &mut other);
        let mut again = vec![0u32; 40 * 24];
        renderer.rasterize(&tree, 40, 24, &mut again);

        assert_eq!(first, again, "a reused canvas must be cleared first");
    }

    /// The old two-pass path: fill the page and the letterbox through
    /// tiny-skia, then copy the canvas out.
    fn reference_compose(
        canvas: &mut Pixmap,
        page: Option<(f32, f32, f32, f32, [u8; 3])>,
    ) -> Vec<u32> {
        let fill = |canvas: &mut Pixmap, rect: Rect, [r, g, b]: [u8; 3]| {
            let mut paint = Paint::default();
            paint.set_color(Color::from_rgba8(r, g, b, 0xFF));
            paint.anti_alias = false;
            paint.blend_mode = BlendMode::DestinationOver;
            canvas.fill_rect(rect, &paint, Transform::identity(), None);
        };

        if let Some((x, y, w, h, color)) = page {
            fill(canvas, Rect::from_xywh(x, y, w, h).unwrap(), color);
        }
        let whole =
            Rect::from_xywh(0.0, 0.0, canvas.width() as f32, canvas.height() as f32).unwrap();
        fill(canvas, whole, LETTERBOX_RGB);

        canvas
            .data()
            .chunks_exact(4)
            .map(|rgba| pack([rgba[0], rgba[1], rgba[2]]))
            .collect()
    }

    /// The composite pass has to land on the same pixels tiny-skia's own
    /// destination-over fills did, seams and rounding included.
    #[test]
    fn compositing_matches_tiny_skia() {
        let tree = usvg::Tree::from_str(
            "<svg xmlns='http://www.w3.org/2000/svg' width='256' height='256' \
             viewBox='0 0 256 256'><circle cx='128' cy='128' r='84' fill='#e85d04'/>\
             <path d='M128 68 L176 180 H80 Z' fill='#f4f1ea' opacity='0.6'/></svg>",
            &usvg::Options::default(),
        )
        .unwrap();

        // Sizes whose fitted page lands on fractional pixel boundaries, so the
        // page rect's rounding and both letterbox seams get exercised.
        for (width, height) in [(101u32, 63u32), (63, 101), (64, 64), (1, 1), (200, 199)] {
            let mut mine = vec![0u32; (width * height) as usize];
            Renderer::new().rasterize(&tree, width, height, &mut mine);

            let scale = (width as f32 / 256.0).min(height as f32 / 256.0);
            let (w, h) = (256.0 * scale, 256.0 * scale);
            let (dx, dy) = ((width as f32 - w) * 0.5, (height as f32 - h) * 0.5);
            let mut canvas = Pixmap::new(width, height).unwrap();
            resvg::render(
                &tree,
                Transform::from_row(scale, 0.0, 0.0, scale, dx, dy),
                &mut canvas.as_mut(),
            );
            let page =
                page_background(&canvas, dx, dy, w, h).map(|(_, color)| (dx, dy, w, h, color));
            let reference = reference_compose(&mut canvas, page);

            assert_eq!(mine, reference, "{width}x{height}");
        }
    }

    /// Pixel data with every kind of run the fast paths care about: opaque,
    /// transparent, and everything between, at every alignment.
    fn sample_pixels(len: usize) -> Vec<u8> {
        let mut state = 0x9E37_79B9u32;
        (0..len * 4)
            .map(|i| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                match (i / 4) % 5 {
                    0 => 0,
                    1 => u8::MAX,
                    _ => (state >> 24) as u8,
                }
            })
            .collect()
    }

    #[test]
    fn scalar_and_simd_agree() {
        for len in [0usize, 1, 3, 15, 16, 17, 64, 129, 1000, 4097] {
            // Premultiplied data: no channel may exceed its own alpha.
            let mut pixels = sample_pixels(len);
            for px in pixels.chunks_exact_mut(4) {
                let alpha = px[3];
                for channel in &mut px[..3] {
                    *channel = (*channel).min(alpha);
                }
            }

            assert_eq!(
                span_stats(&pixels),
                span_stats_scalar(&pixels),
                "stats disagree over {len} pixels"
            );

            for color in [[0, 0, 0], [0x12, 0x12, 0x14], [0xFF, 0x80, 0x01]] {
                let mut fast = vec![0u32; len];
                let mut slow = vec![0u32; len];
                composite(&pixels, &mut fast, color);
                composite_scalar(&pixels, &mut slow, color);
                assert_eq!(fast, slow, "composite disagrees over {len} pixels");
            }
        }
    }

    #[test]
    fn stats_stay_exact_over_a_large_span() {
        // 16 Mpx of pure white would overflow a 32-bit per-channel sum.
        let pixels = vec![u8::MAX; 4 * 4096 * 4096];
        let (sums, opaque) = span_stats(&pixels);
        assert!(opaque);
        assert_eq!(sums, [255 * 4096 * 4096; 4]);
    }
}
