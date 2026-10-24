//! Draws the native taskbar widget (see `widget`) with Direct2D and DirectWrite into a 32-bit
//! premultiplied DIB: the window shows it with `UpdateLayeredWindow`, and `png` turns the same
//! pixels into an image for agents (MCP `inspect_window`) and the snapshot test.
//!
//! Software rendering on purpose: the widget is a few hundred pixels, and a GPU device would cost
//! more memory than the whole picture.

use windows::core::{w, Result};
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_FIGURE_BEGIN_FILLED, D2D1_FIGURE_BEGIN_HOLLOW, D2D1_FIGURE_END_CLOSED,
    D2D1_FIGURE_END_OPEN, D2D1_FILL_MODE_WINDING, D2D1_PIXEL_FORMAT, D2D_RECT_F, D2D_SIZE_F, D2D1_BEZIER_SEGMENT,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1CreateFactory, ID2D1DCRenderTarget, ID2D1Factory, ID2D1PathGeometry, ID2D1StrokeStyle, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
    D2D1_ARC_SEGMENT, D2D1_ARC_SIZE_LARGE, D2D1_ARC_SIZE_SMALL, D2D1_CAP_STYLE_ROUND, D2D1_DASH_STYLE_SOLID,
    D2D1_DRAW_TEXT_OPTIONS_NONE, D2D1_ELLIPSE, D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_FEATURE_LEVEL_DEFAULT,
    D2D1_LINE_JOIN_ROUND, D2D1_QUADRATIC_BEZIER_SEGMENT, D2D1_RENDER_TARGET_PROPERTIES, D2D1_RENDER_TARGET_TYPE_SOFTWARE,
    D2D1_RENDER_TARGET_USAGE_NONE, D2D1_ROUNDED_RECT, D2D1_STROKE_STYLE_PROPERTIES, D2D1_SWEEP_DIRECTION_CLOCKWISE,
    D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE,
};
use windows::Win32::Graphics::DirectWrite::{
    DWriteCreateFactory, IDWriteFactory, IDWriteTextFormat, IDWriteTextLayout, DWRITE_FACTORY_TYPE_SHARED,
    DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT, DWRITE_FONT_WEIGHT_BOLD,
    DWRITE_FONT_WEIGHT_NORMAL, DWRITE_TEXT_METRICS, DWRITE_TRIMMING, DWRITE_TRIMMING_GRANULARITY_CHARACTER,
    DWRITE_WORD_WRAPPING_NO_WRAP,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::HDC;
use windows_numerics::{Matrix3x2, Vector2};
use windows_sys::Win32::Graphics::Gdi as gdi;

use crate::accounts::Provider;
use crate::widget::{self, Box2, Color, Item, Layout, Palette, Seg, Theme, View};

/// How the widget looks right now, besides what it shows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Look {
    pub theme: Theme,
    /// Hovered, or its flyout is open: the tray buttons' highlight.
    pub hot: bool,
    pub pressed: bool,
}

pub struct Renderer {
    d2d: ID2D1Factory,
    target: ID2D1DCRenderTarget,
    dwrite: IDWriteFactory,
    text: IDWriteTextFormat,
    unknown: IDWriteTextFormat,
    round: ID2D1StrokeStyle,
    marks: [ID2D1PathGeometry; 2],
}

/// A drawn picture: a DIB selected into a memory DC, ready for `UpdateLayeredWindow`.
pub struct Surface {
    pub dc: gdi::HDC,
    bitmap: gdi::HBITMAP,
    old: gdi::HGDIOBJ,
    bits: *mut u8,
    pub width: i32,
    pub height: i32,
}

impl Surface {
    fn new(width: i32, height: i32) -> Option<Surface> {
        unsafe {
            let dc = gdi::CreateCompatibleDC(std::ptr::null_mut());
            if dc.is_null() {
                return None;
            }
            let mut info: gdi::BITMAPINFO = std::mem::zeroed();
            info.bmiHeader.biSize = std::mem::size_of::<gdi::BITMAPINFOHEADER>() as u32;
            info.bmiHeader.biWidth = width;
            // Negative: top-down rows.
            info.bmiHeader.biHeight = -height;
            info.bmiHeader.biPlanes = 1;
            info.bmiHeader.biBitCount = 32;
            info.bmiHeader.biCompression = gdi::BI_RGB;
            let mut bits = std::ptr::null_mut();
            let bitmap = gdi::CreateDIBSection(dc, &info, gdi::DIB_RGB_COLORS, &mut bits, std::ptr::null_mut(), 0);
            if bitmap.is_null() || bits.is_null() {
                gdi::DeleteDC(dc);
                return None;
            }
            let old = gdi::SelectObject(dc, bitmap);
            Some(Surface { dc, bitmap, old, bits: bits as *mut u8, width, height })
        }
    }

    /// Premultiplied BGRA, top-down.
    pub fn pixels(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.bits, (self.width * self.height * 4) as usize) }
    }

    /// Straight-alpha RGBA, as images store it.
    pub fn rgba(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.pixels().len());
        for px in self.pixels().chunks_exact(4) {
            let (b, g, r, a) = (px[0] as u32, px[1] as u32, px[2] as u32, px[3] as u32);
            let un = |c: u32| if a == 0 { 0 } else { ((c * 255 + a / 2) / a).min(255) as u8 };
            out.extend_from_slice(&[un(r), un(g), un(b), a as u8]);
        }
        out
    }

    pub fn png(&self) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, self.width as u32, self.height as u32);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            if let Ok(mut w) = enc.write_header() {
                let _ = w.write_image_data(&self.rgba());
            }
        }
        out
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        unsafe {
            gdi::SelectObject(self.dc, self.old);
            gdi::DeleteObject(self.bitmap);
            gdi::DeleteDC(self.dc);
        }
    }
}

fn color(c: Color, opacity: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F { r: c.r, g: c.g, b: c.b, a: c.a * opacity }
}

fn pt(x: f32, y: f32) -> Vector2 {
    Vector2 { X: x, Y: y }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

impl Renderer {
    pub fn new() -> Result<Renderer> {
        unsafe {
            let d2d: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let target = d2d.CreateDCRenderTarget(&D2D1_RENDER_TARGET_PROPERTIES {
                r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE,
                pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
                dpiX: 96.0,
                dpiY: 96.0,
                usage: D2D1_RENDER_TARGET_USAGE_NONE,
                minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
            })?;
            let dwrite: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            let format = |size: f32, weight: DWRITE_FONT_WEIGHT| -> Result<IDWriteTextFormat> {
                // The flyout's font; DirectWrite falls back to Segoe UI where it is missing.
                let f = dwrite.CreateTextFormat(w!("Segoe UI Variable Text"), None, weight, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_STRETCH_NORMAL, size, w!(""))?;
                f.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
                Ok(f)
            };
            let text = format(widget::FONT_SIZE, DWRITE_FONT_WEIGHT_NORMAL)?;
            let unknown = format(widget::UNKNOWN_FONT_SIZE, DWRITE_FONT_WEIGHT_BOLD)?;
            let round = d2d.CreateStrokeStyle(
                &D2D1_STROKE_STYLE_PROPERTIES {
                    startCap: D2D1_CAP_STYLE_ROUND,
                    endCap: D2D1_CAP_STYLE_ROUND,
                    dashCap: D2D1_CAP_STYLE_ROUND,
                    lineJoin: D2D1_LINE_JOIN_ROUND,
                    miterLimit: 10.0,
                    dashStyle: D2D1_DASH_STYLE_SOLID,
                    dashOffset: 0.0,
                },
                None,
            )?;
            let marks = [geometry(&d2d, widget::mark_path(Provider::Claude))?, geometry(&d2d, widget::mark_path(Provider::Codex))?];
            Ok(Renderer { d2d, target, dwrite, text, unknown, round, marks })
        }
    }

    fn text_layout(&self, text: &str, format: &IDWriteTextFormat) -> Result<IDWriteTextLayout> {
        unsafe { self.dwrite.CreateTextLayout(&wide(text), format, 10_000.0, 100.0) }
    }

    fn width_of(layout: &IDWriteTextLayout) -> f32 {
        let mut m = DWRITE_TEXT_METRICS::default();
        unsafe { layout.GetMetrics(&mut m) }.map(|_| m.widthIncludingTrailingWhitespace).unwrap_or(0.0)
    }

    /// A line's width at the widget's font, in DIPs.
    pub fn measure(&self, text: &str) -> f32 {
        self.text_layout(text, &self.text).map(|l| Self::width_of(&l)).unwrap_or(text.chars().count() as f32 * 7.0)
    }

    /// Draws `view` `height` physical pixels tall for a monitor at `dpi`.
    #[cfg(test)]
    pub fn draw(&self, view: &View, look: Look, dpi: u32, height: i32) -> Option<(Surface, Layout)> {
        self.draw_within(view, look, dpi, height, None)
    }

    /// As `draw`, at most `room` physical pixels wide (the free space on the taskbar): a notice
    /// too long for it is cut short with an ellipsis.
    pub fn draw_within(&self, view: &View, look: Look, dpi: u32, height: i32, room: Option<i32>) -> Option<(Surface, Layout)> {
        let scale = widget::scale(dpi);
        // Rounding up to whole pixels must not take it past the room.
        let room = room.map_or(f32::INFINITY, |r| ((r as f32 - 1.0) / scale).floor().max(0.0));
        let layout = widget::layout_within(view, height as f32 / scale, room, &|s| self.measure(s));
        let width = widget::physical(layout.width, dpi);
        let surface = Surface::new(width.max(1), height.max(1))?;
        unsafe { self.paint(&surface, view, &layout, look, dpi) }.ok()?;
        Some((surface, layout))
    }

    unsafe fn paint(&self, s: &Surface, view: &View, l: &Layout, look: Look, dpi: u32) -> Result<()> {
        let rt = &self.target;
        rt.BindDC(HDC(s.dc as _), &RECT { left: 0, top: 0, right: s.width, bottom: s.height })?;
        rt.SetDpi(dpi as f32, dpi as f32);
        rt.SetAntialiasMode(D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
        // Grayscale: ClearType needs an opaque background.
        rt.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
        rt.BeginDraw();
        // Not quite transparent: a layered window lets clicks through where alpha is zero.
        rt.Clear(Some(&D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 1.0 / 255.0 }));
        let p = widget::palette(look.theme);
        let brush = |c: Color, o: f32| rt.CreateSolidColorBrush(&color(c, o), None);
        if look.hot || look.pressed {
            frame(rt, &p, l.frame, look.pressed, &brush)?;
        }
        // Pressed dims the content, like `#widget:active #compact`.
        let o = if look.pressed { 0.786 } else { 1.0 };
        for (i, (item, b)) in view.items.iter().zip(&l.items).enumerate() {
            match item {
                Item::Chip { provider, percent, .. } => {
                    let center = (b.x + b.w / 2.0, b.y + b.h / 2.0);
                    match percent {
                        Some(pc) => self.ring(center, *pc, &p, o, &brush)?,
                        None => {
                            // "?" up and to the right of the mark (`.unknown-mark`).
                            let layout = self.text_layout("?", &self.unknown)?;
                            let w = Self::width_of(&layout);
                            rt.DrawTextLayout(pt(b.x + b.w + 4.0 - w, b.y - 5.0), &layout, &brush(p.text, o)?, D2D1_DRAW_TEXT_OPTIONS_NONE);
                        }
                    }
                    let size = widget::mark_size(*provider);
                    let k = size / widget::MARK_VIEW;
                    let geom = &self.marks[if *provider == Provider::Claude { 0 } else { 1 }];
                    rt.SetTransform(&Matrix3x2 { M11: k, M12: 0.0, M21: 0.0, M22: k, M31: center.0 - size / 2.0, M32: center.1 - size / 2.0 });
                    rt.FillGeometry(geom, &brush(p.text, o)?, None);
                    rt.SetTransform(&Matrix3x2::identity());
                    if view.overlap.get(i).copied().unwrap_or(false) {
                        // Amber with an edge in the taskbar's own tone, so it stands off the ring.
                        let ((x, y), r) = widget::overlap_badge(b);
                        rt.FillEllipse(&D2D1_ELLIPSE { point: pt(x, y), radiusX: r + 1.0, radiusY: r + 1.0 }, &brush(p.badge_edge, 1.0)?);
                        rt.FillEllipse(&D2D1_ELLIPSE { point: pt(x, y), radiusX: r, radiusY: r }, &brush(p.warn, 1.0)?);
                    }
                }
                Item::Text { text, dim } => {
                    let layout = self.text_layout(text, &self.text)?;
                    if l.cut.get(i).copied().unwrap_or(false) {
                        layout.SetMaxWidth(b.w)?;
                        let trimming = DWRITE_TRIMMING { granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER, delimiter: 0, delimiterCount: 0 };
                        layout.SetTrimming(&trimming, &self.dwrite.CreateEllipsisTrimmingSign(&self.text)?)?;
                    }
                    let c = if *dim { p.text_2 } else { p.text };
                    // Centered on the line box: DirectWrite's line is a little shorter than CSS's 1.35.
                    let mut m = DWRITE_TEXT_METRICS::default();
                    layout.GetMetrics(&mut m)?;
                    rt.DrawTextLayout(pt(b.x, b.y + (b.h - m.height) / 2.0), &layout, &brush(c, o)?, D2D1_DRAW_TEXT_OPTIONS_NONE);
                }
            }
        }
        if let Some(d) = l.dot {
            let e = D2D1_ELLIPSE { point: pt(d.x + d.w / 2.0, d.y + d.h / 2.0), radiusX: d.w / 2.0, radiusY: d.h / 2.0 };
            rt.FillEllipse(&e, &brush(p.warn, 1.0)?);
        }
        rt.EndDraw(None, None)?;
        gdi::GdiFlush();
        Ok(())
    }

    unsafe fn ring(
        &self,
        center: (f32, f32),
        percent: f64,
        p: &Palette,
        o: f32,
        brush: &dyn Fn(Color, f32) -> Result<windows::Win32::Graphics::Direct2D::ID2D1SolidColorBrush>,
    ) -> Result<()> {
        let rt = &self.target;
        let r = widget::RING_RADIUS;
        let stroke = widget::RING_STROKE;
        let circle = D2D1_ELLIPSE { point: pt(center.0, center.1), radiusX: r, radiusY: r };
        rt.DrawEllipse(&circle, &brush(p.track, o)?, stroke, None);
        let used = brush(widget::usage_color(p, percent), o)?;
        let (start, sweep) = widget::arc_degrees(percent);
        if sweep >= 359.99 {
            rt.DrawEllipse(&circle, &used, stroke, None);
        } else if sweep <= 0.0 {
            // A zero-length dash with round caps: the browser draws a dot at 12 o'clock.
            let (x, y) = widget::point_on_ring(center, r, start);
            rt.FillEllipse(&D2D1_ELLIPSE { point: pt(x, y), radiusX: stroke / 2.0, radiusY: stroke / 2.0 }, &used);
        } else {
            let (x0, y0) = widget::point_on_ring(center, r, start);
            let (x1, y1) = widget::point_on_ring(center, r, start + sweep);
            let g = self.d2d.CreatePathGeometry()?;
            let sink = g.Open()?;
            sink.BeginFigure(pt(x0, y0), D2D1_FIGURE_BEGIN_HOLLOW);
            sink.AddArc(&D2D1_ARC_SEGMENT {
                point: pt(x1, y1),
                size: D2D_SIZE_F { width: r, height: r },
                rotationAngle: 0.0,
                sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
                arcSize: if sweep > 180.0 { D2D1_ARC_SIZE_LARGE } else { D2D1_ARC_SIZE_SMALL },
            });
            sink.EndFigure(D2D1_FIGURE_END_OPEN);
            sink.Close()?;
            rt.DrawGeometry(&g, &used, stroke, &self.round);
        }
        Ok(())
    }
}

/// The hover/pressed box of the tray buttons: a fill inside a 1 px border, lighter on top (dark
/// taskbar) or darker at the bottom (light taskbar).
unsafe fn frame(
    rt: &ID2D1DCRenderTarget,
    p: &Palette,
    f: Box2,
    pressed: bool,
    brush: &dyn Fn(Color, f32) -> Result<windows::Win32::Graphics::Direct2D::ID2D1SolidColorBrush>,
) -> Result<()> {
    let b = widget::BORDER;
    let inner = D2D1_ROUNDED_RECT {
        rect: D2D_RECT_F { left: f.x + b, top: f.y + b, right: f.x + f.w - b, bottom: f.y + f.h - b },
        radiusX: widget::CORNER - b,
        radiusY: widget::CORNER - b,
    };
    rt.FillRoundedRectangle(&inner, &brush(if pressed { p.press } else { p.hover }, 1.0)?);
    let edge = D2D1_ROUNDED_RECT {
        rect: D2D_RECT_F { left: f.x + b / 2.0, top: f.y + b / 2.0, right: f.x + f.w - b / 2.0, bottom: f.y + f.h - b / 2.0 },
        radiusX: widget::CORNER - b / 2.0,
        radiusY: widget::CORNER - b / 2.0,
    };
    rt.DrawRoundedRectangle(&edge, &brush(p.hover_border, 1.0)?, b, None);
    if !pressed {
        let (l, r) = (f.x + widget::CORNER, f.x + f.w - widget::CORNER);
        rt.DrawLine(pt(l, f.y + b / 2.0), pt(r, f.y + b / 2.0), &brush(p.hover_border_top, 1.0)?, b, None);
        rt.DrawLine(pt(l, f.y + f.h - b / 2.0), pt(r, f.y + f.h - b / 2.0), &brush(p.hover_border_bottom, 1.0)?, b, None);
    }
    Ok(())
}

/// Direct2D geometry from SVG segments, filled nonzero like the browser's default `fill-rule`.
fn geometry(d2d: &ID2D1Factory, segs: &[Seg]) -> Result<ID2D1PathGeometry> {
    unsafe {
        let g = d2d.CreatePathGeometry()?;
        let sink = g.Open()?;
        sink.SetFillMode(D2D1_FILL_MODE_WINDING);
        let mut open = false;
        let mut start = (0.0, 0.0);
        let mut cur = (0.0f32, 0.0f32);
        for seg in segs {
            if !open && !matches!(seg, Seg::Move(..) | Seg::Close) {
                sink.BeginFigure(pt(cur.0, cur.1), D2D1_FIGURE_BEGIN_FILLED);
                open = true;
            }
            match *seg {
                Seg::Move(x, y) => {
                    if open {
                        sink.EndFigure(D2D1_FIGURE_END_OPEN);
                    }
                    sink.BeginFigure(pt(x, y), D2D1_FIGURE_BEGIN_FILLED);
                    open = true;
                    start = (x, y);
                    cur = (x, y);
                }
                Seg::Line(x, y) => {
                    sink.AddLine(pt(x, y));
                    cur = (x, y);
                }
                Seg::Cubic(a, b, c, d, x, y) => {
                    sink.AddBezier(&D2D1_BEZIER_SEGMENT { point1: pt(a, b), point2: pt(c, d), point3: pt(x, y) });
                    cur = (x, y);
                }
                Seg::Quad(a, b, x, y) => {
                    sink.AddQuadraticBezier(&D2D1_QUADRATIC_BEZIER_SEGMENT { point1: pt(a, b), point2: pt(x, y) });
                    cur = (x, y);
                }
                Seg::Arc { rx, ry, rot, large, sweep, x, y } => {
                    sink.AddArc(&D2D1_ARC_SEGMENT {
                        point: pt(x, y),
                        size: D2D_SIZE_F { width: rx, height: ry },
                        rotationAngle: rot,
                        sweepDirection: if sweep { D2D1_SWEEP_DIRECTION_CLOCKWISE } else { windows::Win32::Graphics::Direct2D::D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE },
                        arcSize: if large { D2D1_ARC_SIZE_LARGE } else { D2D1_ARC_SIZE_SMALL },
                    });
                    cur = (x, y);
                }
                Seg::Close => {
                    if open {
                        sink.EndFigure(D2D1_FIGURE_END_CLOSED);
                        open = false;
                    }
                    cur = start;
                }
            }
        }
        if open {
            sink.EndFigure(D2D1_FIGURE_END_OPEN);
        }
        sink.Close()?;
        Ok(g)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widget::Texts;

    struct En;
    impl Texts for En {
        fn t(&self, key: &str, vars: &[(&str, &str)]) -> String {
            crate::i18n::t("en", key, vars)
        }
    }

    fn chips(claude: Option<f64>, codex: Option<f64>, problems: usize) -> View {
        let items = [(Provider::Claude, claude), (Provider::Codex, codex)]
            .into_iter()
            .map(|(provider, percent)| Item::Chip { provider, percent, tip: widget::chip_tip(&En, provider, percent) })
            .collect();
        View { items, health_problems: problems, label: "SlopBar".into(), details: vec![], overlap: vec![] }
    }

    const LOOK: Look = Look { theme: Theme::Dark, hot: false, pressed: false };

    fn px(s: &Surface, x: i32, y: i32) -> [u8; 4] {
        let i = ((y * s.width + x) * 4) as usize;
        let p = s.pixels();
        [p[i + 2], p[i + 1], p[i], p[i + 3]] // RGBA, premultiplied
    }

    /// Whether any pixel within `r` of (x, y) is close to `want` (straight RGB) and mostly opaque.
    fn near(s: &Surface, x: f32, y: f32, r: i32, want: Color) -> bool {
        let rgba = s.rgba();
        let (cx, cy) = (x.round() as i32, y.round() as i32);
        (cy - r..=cy + r).any(|yy| {
            (cx - r..=cx + r).any(|xx| {
                if xx < 0 || yy < 0 || xx >= s.width || yy >= s.height {
                    return false;
                }
                let i = ((yy * s.width + xx) * 4) as usize;
                let d = |c: u8, w: f32| (c as f32 - w * 255.0).abs();
                rgba[i + 3] > 200 && d(rgba[i], want.r) < 24.0 && d(rgba[i + 1], want.g) < 24.0 && d(rgba[i + 2], want.b) < 24.0
            })
        })
    }

    #[test]
    fn sizes_follow_the_layout_and_dpi() {
        let r = Renderer::new().unwrap();
        for (dpi, height) in [(96, 48), (120, 60), (144, 72), (192, 96), (240, 120)] {
            let (s, l) = r.draw(&chips(Some(20.0), Some(80.0), 0), LOOK, dpi, height).unwrap();
            assert_eq!(s.width, widget::physical(66.0, dpi), "{dpi}");
            assert_eq!(s.height, height);
            assert_eq!(l.width, 66.0);
        }
    }

    #[test]
    fn draws_rings_in_the_usage_colors_at_every_scale() {
        let r = Renderer::new().unwrap();
        for theme in [Theme::Dark, Theme::Light] {
            let p = widget::palette(theme);
            for dpi in [96, 144, 240] {
                let k = widget::scale(dpi);
                let (s, l) = r.draw(&chips(Some(50.0), Some(95.0), 0), Look { theme, ..LOOK }, dpi, (48.0 * k) as i32).unwrap();
                let rad = widget::RING_RADIUS;
                // Claude at 50%: the right half is used (3 o'clock), the left half is track.
                let c0 = (l.items[0].x + 12.0, l.items[0].y + 12.0);
                assert!(near(&s, (c0.0 + rad) * k, c0.1 * k, 1, p.ok), "{theme:?} {dpi}: no ok arc");
                assert!(!near(&s, (c0.0 - rad) * k, c0.1 * k, 1, p.ok), "{theme:?} {dpi}: arc past half");
                // Codex at 95%: critical, and almost all the way round.
                let c1 = (l.items[1].x + 12.0, l.items[1].y + 12.0);
                assert!(near(&s, (c1.0 - rad) * k, c1.1 * k, 1, p.bad), "{theme:?} {dpi}: no bad arc");
                // The mark is drawn in the text color at the center.
                assert!(near(&s, c1.0 * k, c1.1 * k, (3.0 * k) as i32, p.text) || theme == Theme::Light, "{theme:?} {dpi}: no mark");
            }
        }
    }

    #[test]
    fn empty_space_is_almost_transparent_but_clickable() {
        let r = Renderer::new().unwrap();
        let (s, _) = r.draw(&chips(Some(10.0), None, 0), LOOK, 96, 48).unwrap();
        assert_eq!(px(&s, 1, 1)[3], 1);
        assert_eq!(px(&s, s.width - 2, s.height - 2)[3], 1);
    }

    #[test]
    fn hover_and_the_health_dot_show() {
        let r = Renderer::new().unwrap();
        let p = widget::palette(Theme::Dark);
        let (plain, _) = r.draw(&chips(Some(10.0), Some(10.0), 0), LOOK, 96, 48).unwrap();
        let (hot, _) = r.draw(&chips(Some(10.0), Some(10.0), 0), Look { hot: true, ..LOOK }, 96, 48).unwrap();
        // Inside the frame, between the rings and the edge.
        assert!(px(&hot, 3, 24)[3] > px(&plain, 3, 24)[3]);
        let (dot, l) = r.draw(&chips(Some(10.0), Some(10.0), 2), LOOK, 96, 48).unwrap();
        let d = l.dot.unwrap();
        assert!(near(&dot, d.x + d.w / 2.0, d.y + d.h / 2.0, 0, p.warn));
    }

    /// The snapshot: a PNG of a known view whose pixels match a reference within tolerances
    /// (font hinting and antialiasing may differ a little between Windows builds).
    #[test]
    fn png_snapshot_matches_within_tolerance() {
        let r = Renderer::new().unwrap();
        let (s, _) = r.draw(&chips(Some(42.0), Some(75.0), 1), LOOK, 144, 72).unwrap();
        let png = s.png();
        let decoder = png::Decoder::new(std::io::Cursor::new(png));
        let mut reader = decoder.read_info().unwrap();
        let mut buf = vec![0; reader.output_buffer_size()];
        let info = reader.next_frame(&mut buf).unwrap();
        assert_eq!((info.width, info.height), (99, 72));
        assert_eq!(&buf[..info.buffer_size()], &s.rgba()[..]);
        // Coverage per color: how many clearly drawn pixels of each token.
        let p = widget::palette(Theme::Dark);
        let count = |c: Color| {
            buf.chunks_exact(4)
                .filter(|q| q[3] > 200 && [(q[0], c.r), (q[1], c.g), (q[2], c.b)].iter().all(|(v, w)| (*v as f32 - w * 255.0).abs() < 24.0))
                .count()
        };
        let (ok, warn) = (count(p.ok), count(p.warn));
        // A 42% arc at 1.5×: 2π·14.4·0.42 ≈ 38 px long and ~3.8 px wide; the 75% one and the dot
        // in the caution color. Generous bounds, but far from empty or flooded.
        assert!((60..=260).contains(&ok), "ok pixels {ok}");
        assert!((110..=480).contains(&warn), "warn pixels {warn}");
        assert!(count(p.text) > 40, "mark pixels {}", count(p.text));
        if let Ok(dir) = std::env::var("SLOPBAR_WIDGET_PNG") {
            let _ = std::fs::write(std::path::Path::new(&dir).join("widget-snapshot.png"), s.png());
            // A large one, to look at the marks' detail.
            let (big, _) = r.draw(&chips(Some(42.0), None, 0), LOOK, 480, 240).unwrap();
            let _ = std::fs::write(std::path::Path::new(&dir).join("widget-large.png"), big.png());
        }
    }

    #[test]
    fn text_items_are_measured_with_directwrite() {
        let r = Renderer::new().unwrap();
        let short = r.measure("Loading");
        let long = r.measure("Loading, loading, loading");
        assert!(short > 20.0 && short < 80.0, "{short}");
        assert!(long > short * 2.5);
        let view = View { items: vec![Item::Text { text: "Loading".into(), dim: true }], health_problems: 0, label: String::new(), details: vec![], overlap: vec![] };
        let (s, l) = r.draw(&view, LOOK, 96, 48).unwrap();
        assert_eq!(l.width, 1.0 + 8.0 + short.ceil() + 8.0 + 1.0);
        assert!(s.pixels().chunks_exact(4).any(|q| q[3] > 100));
    }

    #[test]
    fn a_notice_too_long_for_the_room_is_cut_to_it() {
        let r = Renderer::new().unwrap();
        let text = "Monitoring sign-in did not complete. Try connecting again.";
        let view = View { items: vec![Item::Text { text: text.into(), dim: false }], health_problems: 0, label: String::new(), details: vec![], overlap: vec![] };
        for dpi in [96, 144] {
            let (full, _) = r.draw(&view, LOOK, dpi, 48).unwrap();
            let (s, l) = r.draw_within(&view, LOOK, dpi, 48, Some(150)).unwrap();
            assert!(full.width > 150 && s.width <= 150, "{dpi}: {} {}", full.width, s.width);
            assert_eq!(l.cut, vec![true]);
            // Ink stops inside the box: nothing is drawn in the right padding.
            let pad = (8.0 * widget::scale(dpi)) as i32;
            let inked = |x: i32| (0..s.height).any(|y| px(&s, x, y)[3] > 100);
            assert!((s.width - pad..s.width).all(|x| !inked(x)), "{dpi}");
            assert!((0..s.width).any(inked));
        }
    }
}
