//! The icons' artwork (the approach comes from Carlita). Icons are SVG
//! path data in `ui/icons.slint`; this wraps them in SVG documents that Slint rasterises on the CPU (resvg) at
//! the size they are shown. Skia's GPU backends draw `Path` elements without
//! proper edge smoothing, which left the icons jagged; images come out
//! smooth on every renderer.

use slint::{Color, ComponentHandle, Image, SharedString};
use std::cell::RefCell;
use std::collections::HashMap;

use crate::{IconArt, MainWindow};

type Key = (SharedString, SharedString, u32, u32);

thread_local! {
    // Bindings call `draw` often; parse each icon only once.
    static CACHE: RefCell<HashMap<Key, Image>> = RefCell::new(HashMap::new());
    static LOGOS: RefCell<HashMap<[u32; 3], Image>> = RefCell::new(HashMap::new());
}

pub fn install(ui: &MainWindow) {
    ui.global::<IconArt>().on_draw(draw);
    ui.global::<IconArt>().on_logo(logo);
}

/// The logo as shipped (the desktop entry's icon, the launcher's).
const LOGO: &str = include_str!("../assets/yoink-icon.svg");

fn logo(a: Color, b: Color, c: Color) -> Image {
    let key = [a, b, c].map(|c| u32::from_be_bytes([c.red(), c.green(), c.blue(), c.alpha()]));
    if let Some(image) = LOGOS.with(|c| c.borrow().get(&key).cloned()) {
        return image;
    }
    let image = Image::load_from_svg_data(logo_svg(rgb(a), rgb(b), rgb(c)).as_bytes())
        .unwrap_or_else(|e| {
            log::warn!("logo could not be drawn: {e:?}");
            Image::default()
        });
    LOGOS.with(|c| c.borrow_mut().insert(key, image.clone()));
    image
}

type Rgb = [u8; 3];

fn rgb(c: Color) -> Rgb {
    [c.red(), c.green(), c.blue()]
}

/// `a` over `b`, `t` of the way from `b` to `a`.
fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    [0, 1, 2].map(|i| (a[i] as f32 * t + b[i] as f32 * (1.0 - t)).round() as u8)
}

fn hex(c: Rgb) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

/// The logo's colours for a theme's paint `a`, `b` and `c`, in the order of
/// `IRIS`: the paint itself, then the tile (`a` sunk nearly to black), the
/// poles (`a` lifted nearly to white), the field lines and carton outline
/// (between `a` and `b`, lightened) and the carton's fill (between `a` and
/// `c`, darkened). Iris gives back the colours the SVG is drawn in.
fn logo_colours(a: Rgb, b: Rgb, c: Rgb) -> [Rgb; 9] {
    let night = [8, 7, 26];
    let white = [255, 255, 255];
    let carton = mix(a, c, 0.5);
    [
        a,
        b,
        c,
        mix(a, night, 0.15),
        mix(a, [6, 6, 12], 0.05),
        mix(a, white, 0.15),
        mix(mix(a, b, 0.5), white, 0.7),
        mix(carton, night, 0.35),
        mix(carton, night, 0.15),
    ]
}

/// The colours assets/yoink-icon.svg is drawn in: `logo_colours` of Iris.
const IRIS: [&str; 9] = [
    "#7278fe", "#c688d6", "#a85eea", "#18183c", "#0b0c18", "#eaebff", "#baa6f0", "#372a66", "#1c163b",
];

/// The logo with its Iris colours swapped for a theme's, in one pass so a
/// colour put in is never swapped again.
fn logo_svg(a: Rgb, b: Rgb, c: Rgb) -> String {
    let to = logo_colours(a, b, c).map(hex);
    let mut out = String::with_capacity(LOGO.len());
    let mut rest = LOGO;
    while let Some(i) = rest.find('#') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        match IRIS.iter().position(|k| rest.get(..7) == Some(*k)) {
            Some(n) => {
                out.push_str(&to[n]);
                rest = &rest[7..];
            }
            None => {
                out.push('#');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn draw(line: SharedString, solid: SharedString, weight: f32, grid: f32) -> Image {
    let key = (
        line.clone(),
        solid.clone(),
        weight.to_bits(),
        grid.to_bits(),
    );
    if let Some(image) = CACHE.with(|c| c.borrow().get(&key).cloned()) {
        return image;
    }
    let image = Image::load_from_svg_data(svg(&line, &solid, weight, grid).as_bytes())
        .unwrap_or_else(|e| {
            log::warn!("icon could not be drawn: {e:?}");
            Image::default()
        });
    CACHE.with(|c| c.borrow_mut().insert(key, image.clone()));
    image
}

/// An app's icon made ready to be shown `px` physical pixels wide: a raster
/// icon larger than that (an AppImage's 512 px `.DirIcon`) is scaled down
/// here, averaging every source pixel. The renderers sample images without
/// mipmaps, so drawing a large one small skips most of its pixels and comes
/// out jagged. SVGs are left alone: Slint renders them at the size shown.
pub fn smooth(image: Image, px: u32) -> Image {
    let px = px.max(1);
    let size = image.size();
    if size.width <= px && size.height <= px {
        return image;
    }
    // Vector images have no pixels to give.
    let Some(buf) = image.to_rgba8_premultiplied() else {
        return image;
    };
    let (w, h) = (buf.width() as usize, buf.height() as usize);
    if w == 0 || h == 0 {
        return image;
    }
    let scale = px as f64 / w.max(h) as f64;
    let nw = ((w as f64 * scale).round() as usize).max(1);
    let nh = ((h as f64 * scale).round() as usize).max(1);
    let src = buf.as_bytes();
    let out = downscale(src, w, h, nw, nh);
    let mut dst = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::new(nw as u32, nh as u32);
    dst.make_mut_bytes().copy_from_slice(&out);
    Image::from_rgba8(dst)
}

/// Area-averaging downscale of premultiplied RGBA (`w`×`h` to `nw`×`nh`,
/// each no larger), returned un-premultiplied.
fn downscale(src: &[u8], w: usize, h: usize, nw: usize, nh: usize) -> Vec<u8> {
    // How much of each source column (row) falls in each target column (row).
    fn spans(n: usize, nn: usize) -> Vec<Vec<(usize, f64)>> {
        let ratio = n as f64 / nn as f64;
        (0..nn)
            .map(|i| {
                let (a, b) = (i as f64 * ratio, (i + 1) as f64 * ratio);
                let mut v = Vec::new();
                let mut s = a.floor() as usize;
                while (s as f64) < b && s < n {
                    let cover = (b.min((s + 1) as f64) - a.max(s as f64)).max(0.0);
                    if cover > 0.0 {
                        v.push((s, cover / ratio));
                    }
                    s += 1;
                }
                v
            })
            .collect()
    }
    let xs = spans(w, nw);
    let ys = spans(h, nh);
    let mut out = vec![0u8; nw * nh * 4];
    for (ty, yspan) in ys.iter().enumerate() {
        for (tx, xspan) in xs.iter().enumerate() {
            let mut acc = [0f64; 4];
            for &(sy, wy) in yspan {
                for &(sx, wx) in xspan {
                    let i = (sy * w + sx) * 4;
                    let k = wx * wy;
                    for c in 0..4 {
                        acc[c] += src[i + c] as f64 * k;
                    }
                }
            }
            let o = (ty * nw + tx) * 4;
            let a = acc[3];
            for c in 0..3 {
                let v = if a > 0.0 { acc[c] * 255.0 / a } else { 0.0 };
                out[o + c] = v.round().clamp(0.0, 255.0) as u8;
            }
            out[o + 3] = a.round().clamp(0.0, 255.0) as u8;
        }
    }
    out
}

/// An SVG document `grid` units square: `solid` filled (even-odd) and `line`
/// stroked `weight` wide with round ends, both white so `colorize` can tint
/// them.
fn svg(line: &str, solid: &str, weight: f32, grid: f32) -> String {
    let mut out = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{grid}" height="{grid}" viewBox="0 0 {grid} {grid}">"#
    );
    if !solid.is_empty() {
        out += &format!(
            r#"<path d="{}" fill="white" fill-rule="evenodd"/>"#,
            attr(solid)
        );
    }
    if !line.is_empty() {
        out += &format!(
            r#"<path d="{}" fill="none" stroke="white" stroke-width="{weight}" stroke-linecap="round" stroke-linejoin="round"/>"#,
            attr(line)
        );
    }
    out + "</svg>"
}

/// Path data holds only commands and numbers; drop anything that could end
/// the attribute.
fn attr(s: &str) -> String {
    s.chars()
        .filter(|c| !matches!(c, '"' | '<' | '>' | '&'))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn svg_has_both_paths() {
        let s = svg("M 1 1 L 2 2", "M 0 0 Z", 2.0, 24.0);
        assert!(s.starts_with(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24">"#
        ));
        assert!(s.contains(r#"<path d="M 0 0 Z" fill="white" fill-rule="evenodd"/>"#));
        assert!(s.contains(r#"<path d="M 1 1 L 2 2" fill="none" stroke="white" stroke-width="2""#));
        assert!(s.ends_with("</svg>"));
    }

    #[test]
    fn empty_paths_are_left_out() {
        assert_eq!(svg("", "", 2.0, 14.0).matches("<path").count(), 0);
        assert_eq!(svg("M 1 1", "", 1.4, 14.0).matches("<path").count(), 1);
    }

    #[test]
    fn downscale_averages() {
        // 2×2 → 1×1: two opaque white pixels and two transparent ones give
        // half-transparent white (premultiplied in, straight out).
        let src = [255, 255, 255, 255, 0, 0, 0, 0, 0, 0, 0, 0, 255, 255, 255, 255];
        assert_eq!(downscale(&src, 2, 2, 1, 1), vec![255, 255, 255, 128]);
        // 3 → 2 columns: the middle pixel is shared.
        let src = [0, 0, 0, 255, 90, 90, 90, 255, 180, 180, 180, 255];
        assert_eq!(downscale(&src, 3, 1, 2, 1), vec![30, 30, 30, 255, 150, 150, 150, 255]);
    }

    #[test]
    fn smooth_shrinks_large_rasters_only() {
        let big = Image::from_rgba8(slint::SharedPixelBuffer::new(512, 256));
        let s = smooth(big, 48).size();
        assert_eq!((s.width, s.height), (48, 24));
        let small = Image::from_rgba8(slint::SharedPixelBuffer::new(32, 32));
        assert_eq!(smooth(small, 48).size().width, 32);
    }

    #[test]
    fn logo_is_drawn_in_iris() {
        let iris = logo_colours([0x72, 0x78, 0xfe], [0xc6, 0x88, 0xd6], [0xa8, 0x5e, 0xea]);
        assert_eq!(iris.map(hex), IRIS.map(String::from));
        for k in IRIS {
            assert!(LOGO.contains(k), "{k} not in the SVG");
        }
        assert_eq!(logo_svg([0x72, 0x78, 0xfe], [0xc6, 0x88, 0xd6], [0xa8, 0x5e, 0xea]), LOGO);
    }

    #[test]
    fn logo_takes_the_theme_colours() {
        // Abyss: paint b is Iris's paint a, so a one-pass swap matters.
        let s = logo_svg([0x7f, 0xf5, 0xe0], [0x72, 0x78, 0xfe], [0x3a, 0x7b, 0xff]);
        assert!(s.contains("#7ff5e0") && s.contains("#3a7bff"));
        assert!(!s.contains("#c688d6") && !s.contains("#a85eea"));
        let image = Image::load_from_svg_data(s.as_bytes()).unwrap();
        assert_eq!((image.size().width, image.size().height), (256, 256));
    }

    #[test]
    fn svg_parses() {
        let s = svg(
            "M 17.66 6.6 A 8 8 0 1 1 6.34 6.6 M 12 2.8 L 12 11.5",
            "",
            2.2,
            24.0,
        );
        let image = Image::load_from_svg_data(s.as_bytes()).unwrap();
        assert_eq!((image.size().width, image.size().height), (24, 24));
    }
}
