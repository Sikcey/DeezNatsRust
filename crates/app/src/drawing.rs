//! The original application's GDI+ paths, text and glass gradients, rendered
//! directly into one premultiplied frame instead of independently painted HWNDs.
use std::{
    cell::Cell,
    mem::{size_of, zeroed},
    ptr::{null, null_mut},
    sync::OnceLock,
};
pub const LYRICS_UNTRIMMED: u32 = 0x80000000;

use windows_sys::Win32::{
    Foundation::RECT,
    Graphics::{Gdi::*, GdiPlus::*},
};
type ImageKey = (u64, i32, i32);
#[path = "provider_logos.rs"]
mod provider_logos;
pub unsafe fn deezer_logo(dc: HDC, r: RECT) -> bool {
    let Some(g) = graphics(dc) else {
        return false;
    };
    let size = (r.right - r.left).min(r.bottom - r.top).max(0) as f32;
    let x = r.left as f32 + ((r.right - r.left) as f32 - size) / 2.0;
    let y = r.top as f32 + ((r.bottom - r.top) as f32 - size) / 2.0;
    let scale = size / 24.0;
    let mut path = null_mut();
    GdipCreatePath(FillModeAlternate, &mut path);
    let (figures, color) = (provider_logos::DEEZER, 0xffa238ff);
    for figure in figures {
        GdipStartPathFigure(path);
        for p in *figure {
            GdipAddPathBezier(
                path,
                x + p[0] * scale,
                y + p[1] * scale,
                x + p[2] * scale,
                y + p[3] * scale,
                x + p[4] * scale,
                y + p[5] * scale,
                x + p[6] * scale,
                y + p[7] * scale,
            );
        }
        GdipClosePathFigure(path);
    }
    let mut brush = null_mut();
    GdipCreateSolidFill(color, &mut brush);
    GdipFillPath(g, brush.cast(), path);
    GdipDeleteBrush(brush.cast());
    GdipDeletePath(path);
    true
}
type CachedImage = (ImageKey, *mut GpBitmap);
const RESIZED_IMAGE_BYTES: usize = 24 * 1024 * 1024;
fn resized_bytes(key: ImageKey) -> usize {
    (key.1.max(0) as usize)
        .saturating_mul(key.2.max(0) as usize)
        .saturating_mul(4)
}
thread_local! {static RESIZED_IMAGES:std::cell::RefCell<std::collections::VecDeque<CachedImage>>=const {std::cell::RefCell::new(std::collections::VecDeque::new())};}
thread_local! {static CURRENT:Cell<(HDC,*mut GpGraphics)>=const {Cell::new((null_mut(),null_mut()))};}
thread_local! {static TEXT_PATHS:std::cell::RefCell<std::collections::VecDeque<(String,*mut GpPath)>>=const {std::cell::RefCell::new(std::collections::VecDeque::new())};}
thread_local! {static LIGHT:Cell<bool>=const {Cell::new(false)};}
pub fn argb(color: u32, alpha: u32) -> u32 {
    (alpha << 24) | ((color & 255) << 16) | (color & 0xff00) | ((color >> 16) & 255)
}
thread_local! {static SURFACE_OPACITY:Cell<u32>=const {Cell::new(100)};}
pub struct SurfaceOpacity(u32);
impl SurfaceOpacity {
    pub fn set(percent: u32) -> Self {
        Self(SURFACE_OPACITY.with(|value| value.replace(percent.min(100))))
    }
}
impl Drop for SurfaceOpacity {
    fn drop(&mut self) {
        SURFACE_OPACITY.with(|value| value.set(self.0));
    }
}
fn surface_color(color: u32) -> u32 {
    (color & 0xffffff) | ((((color >> 24) * SURFACE_OPACITY.with(Cell::get) + 50) / 100) << 24)
}
pub struct Frame {
    image: *mut GpBitmap,
    graphics: *mut GpGraphics,
}
impl Frame {
    pub unsafe fn begin(
        dc: HDC,
        bits: *mut u8,
        stride: i32,
        width: i32,
        height: i32,
        light: bool,
    ) -> Option<Self> {
        static TOKEN: OnceLock<usize> = OnceLock::new();
        if *TOKEN.get_or_init(|| {
            let mut token = 0;
            let input = GdiplusStartupInput {
                GdiplusVersion: 1,
                ..zeroed()
            };
            if GdiplusStartup(&mut token, &input, null_mut()) == 0 {
                token
            } else {
                0
            }
        }) == 0
        {
            return None;
        }
        let mut image = null_mut();
        let mut graphics = null_mut();
        // PixelFormat32bppPARGB = 0xE200B.
        if GdipCreateBitmapFromScan0(width, height, stride * 4, 0xe200b, bits, &mut image) != 0 {
            return None;
        }
        if GdipGetImageGraphicsContext(image.cast(), &mut graphics) != 0 {
            GdipDisposeImage(image.cast());
            return None;
        }
        GdipSetSmoothingMode(graphics, SmoothingModeAntiAlias);
        GdipSetInterpolationMode(graphics, InterpolationModeHighQualityBicubic);
        GdipSetPixelOffsetMode(graphics, PixelOffsetModeHighQuality);
        CURRENT.set((dc, graphics));
        LIGHT.set(light);
        Some(Self { image, graphics })
    }
}
impl Drop for Frame {
    fn drop(&mut self) {
        unsafe {
            CURRENT.set((null_mut(), null_mut()));
            GdipFlush(self.graphics, FlushIntentionSync);
            GdipDeleteGraphics(self.graphics);
            GdipDisposeImage(self.image.cast());
        }
    }
}
unsafe fn graphics(dc: HDC) -> Option<*mut GpGraphics> {
    let (target, g) = CURRENT.get();
    if target != dc || g.is_null() {
        return None;
    }
    let mut clip: RECT = zeroed();
    GetClipBox(dc, &mut clip);
    GdipResetClip(g);
    GdipSetClipRectI(
        g,
        clip.left,
        clip.top,
        clip.right - clip.left,
        clip.bottom - clip.top,
        CombineModeReplace,
    );
    Some(g)
}
unsafe fn path(r: RECT, radius: f32) -> *mut GpPath {
    let mut p = null_mut();
    GdipCreatePath(FillModeAlternate, &mut p);
    let x = r.left as f32;
    let y = r.top as f32;
    let w = (r.right - r.left).max(1) as f32;
    let h = (r.bottom - r.top).max(1) as f32;
    if radius <= 0.0 {
        GdipAddPathRectangle(p, x, y, w, h);
        GdipClosePathFigure(p);
        return p;
    }
    let d = (radius * 2.0).min(w).min(h).max(1.0);
    for (xx, yy, angle) in [
        (x, y, 180.0),
        (x + w - d, y, 270.0),
        (x + w - d, y + h - d, 0.0),
        (x, y + h - d, 90.0),
    ] {
        GdipAddPathArc(p, xx, yy, d, d, angle, 90.0);
    }
    GdipClosePathFigure(p);
    p
}
pub unsafe fn fill(dc: HDC, r: RECT, color: u32, radius: f32) -> bool {
    let color = surface_color(color);
    let Some(g) = graphics(dc) else {
        return false;
    };
    if r.right <= r.left || r.bottom <= r.top {
        return true;
    }
    let p = path(r, radius);
    let mut b = null_mut();
    GdipCreateSolidFill(color, &mut b);
    GdipFillPath(g, b.cast(), p);
    GdipDeleteBrush(b.cast());
    GdipDeletePath(p);
    true
}
pub unsafe fn stroke(dc: HDC, r: RECT, color: u32, radius: f32, width: f32) -> bool {
    let color = surface_color(color);
    let Some(g) = graphics(dc) else {
        return false;
    };
    if r.right <= r.left || r.bottom <= r.top {
        return true;
    }
    let p = path(r, radius);
    let mut pen = null_mut();
    GdipCreatePen1(color, width, UnitPixel, &mut pen);
    GdipDrawPath(g, pen, p);
    GdipDeletePen(pen);
    GdipDeletePath(p);
    true
}
pub unsafe fn stroke_gradient(
    dc: HDC,
    r: RECT,
    top: u32,
    bottom: u32,
    radius: f32,
    width: f32,
) -> bool {
    let (top, bottom) = (surface_color(top), surface_color(bottom));
    let Some(g) = graphics(dc) else {
        return false;
    };
    if r.right <= r.left || r.bottom <= r.top {
        return true;
    }
    let p = path(r, radius);
    let mut brush = null_mut();
    let bounds = RectF {
        X: r.left as f32,
        Y: r.top as f32,
        Width: (r.right - r.left) as f32,
        Height: (r.bottom - r.top) as f32,
    };
    GdipCreateLineBrushFromRectWithAngle(
        &bounds,
        top,
        bottom,
        90.0,
        0,
        WrapModeTileFlipXY,
        &mut brush,
    );
    let mut pen = null_mut();
    GdipCreatePen2(brush.cast(), width, UnitPixel, &mut pen);
    GdipDrawPath(g, pen, p);
    GdipDeletePen(pen);
    GdipDeleteBrush(brush.cast());
    GdipDeletePath(p);
    true
}
pub unsafe fn gradient(dc: HDC, r: RECT, top: u32, bottom: u32, radius: f32) -> bool {
    gradient_angle(dc, r, top, bottom, radius, 100.0)
}
pub unsafe fn gradient_angle(
    dc: HDC,
    r: RECT,
    top: u32,
    bottom: u32,
    radius: f32,
    angle: f32,
) -> bool {
    let (top, bottom) = (surface_color(top), surface_color(bottom));
    let Some(g) = graphics(dc) else {
        return false;
    };
    if r.right <= r.left || r.bottom <= r.top {
        return true;
    }
    let p = path(r, radius);
    let mut brush = null_mut();
    let bounds = RectF {
        X: r.left as f32,
        Y: r.top as f32,
        Width: (r.right - r.left) as f32,
        Height: (r.bottom - r.top) as f32,
    };
    GdipCreateLineBrushFromRectWithAngle(
        &bounds,
        top,
        bottom,
        angle,
        0,
        WrapModeTileFlipXY,
        &mut brush,
    );
    GdipFillPath(g, brush.cast(), p);
    GdipDeleteBrush(brush.cast());
    GdipDeletePath(p);
    true
}
pub unsafe fn line(dc: HDC, x1: f32, y1: f32, x2: f32, y2: f32, color: u32, width: f32) -> bool {
    let Some(g) = graphics(dc) else {
        return false;
    };
    let mut pen = null_mut();
    GdipCreatePen1(color, width, UnitPixel, &mut pen);
    GdipDrawLine(g, pen, x1, y1, x2, y2);
    GdipDeletePen(pen);
    true
}
pub unsafe fn curve(dc: HDC, points: &[(f32, f32)], color: u32, width: f32) -> bool {
    let Some(g) = graphics(dc) else {
        return false;
    };
    if points.len() < 2 {
        return true;
    }
    let pts: Vec<PointF> = points.iter().map(|&(x, y)| PointF { X: x, Y: y }).collect();
    let mut pen = null_mut();
    GdipCreatePen1(color, width, UnitPixel, &mut pen);
    GdipDrawCurve(g, pen, pts.as_ptr(), pts.len() as i32);
    GdipDeletePen(pen);
    true
}
pub unsafe fn polygon(dc: HDC, points: &[(f32, f32)], color: u32) -> bool {
    let Some(g) = graphics(dc) else {
        return false;
    };
    if points.len() < 3 {
        return true;
    }
    let pts: Vec<PointF> = points.iter().map(|&(x, y)| PointF { X: x, Y: y }).collect();
    let mut brush = null_mut();
    GdipCreateSolidFill(color, &mut brush);
    GdipFillPolygon(
        g,
        brush.cast(),
        pts.as_ptr(),
        pts.len() as i32,
        FillModeAlternate,
    );
    GdipDeleteBrush(brush.cast());
    true
}
pub unsafe fn profile_silhouette(dc: HDC, r: RECT, color: u32) -> bool {
    let Some(g) = graphics(dc) else {
        return false;
    };
    let w = (r.right - r.left).max(1) as f32;
    let h = (r.bottom - r.top).max(1) as f32;
    let x = r.left as f32;
    let y = r.top as f32;
    let mut pen = null_mut();
    GdipCreatePen1(color, 1.8, UnitPixel, &mut pen);
    GdipDrawEllipse(g, pen, x + w * 0.34, y + h * 0.22, w * 0.32, h * 0.32);
    GdipDrawArc(
        g,
        pen,
        x + w * 0.20,
        y + h * 0.50,
        w * 0.60,
        h * 0.44,
        200.0,
        140.0,
    );
    GdipDeletePen(pen);
    true
}
unsafe fn cached_text_path(font: HFONT, value: &str, r: RECT, flags: u32) -> Option<*mut GpPath> {
    let mut lf: LOGFONTW = zeroed();
    GetObjectW(
        font,
        size_of::<LOGFONTW>() as i32,
        (&mut lf as *mut LOGFONTW).cast(),
    );
    let key = format!(
        "{:p}:{}:{}:{}:{}:{}:{}",
        font,
        lf.lfHeight,
        lf.lfWeight,
        r.right - r.left,
        r.bottom - r.top,
        flags,
        value
    );
    let cached = TEXT_PATHS.with(|cache| {
        cache
            .borrow()
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, path)| *path)
    });
    if let Some(path) = cached {
        return Some(path);
    }
    let mut family = null_mut();
    GdipCreateFontFamilyFromName(lf.lfFaceName.as_ptr(), null_mut(), &mut family);
    if family.is_null() {
        return None;
    }
    let mut format = null_mut();
    GdipCreateStringFormat(
        if flags & DT_WORDBREAK != 0 {
            0
        } else {
            StringFormatFlagsNoWrap
        },
        0,
        &mut format,
    );
    if flags & LYRICS_UNTRIMMED != 0 {
        GdipDeleteStringFormat(format);
        GdipStringFormatGetGenericTypographic(&mut format);
        GdipSetStringFormatFlags(
            format,
            StringFormatFlagsNoWrap
                | StringFormatFlagsNoClip
                | StringFormatFlagsMeasureTrailingSpaces,
        );
    }
    GdipSetStringFormatTrimming(
        format,
        if flags & LYRICS_UNTRIMMED != 0 {
            StringTrimmingNone
        } else {
            StringTrimmingEllipsisCharacter
        },
    );
    GdipSetStringFormatAlign(
        format,
        if flags & DT_CENTER != 0 {
            StringAlignmentCenter
        } else if flags & DT_RIGHT != 0 {
            StringAlignmentFar
        } else {
            StringAlignmentNear
        },
    );
    GdipSetStringFormatLineAlign(
        format,
        if flags & DT_VCENTER != 0 {
            StringAlignmentCenter
        } else {
            StringAlignmentNear
        },
    );
    let bounds = RectF {
        X: 0.0,
        Y: 0.0,
        Width: (r.right - r.left) as f32,
        Height: (r.bottom - r.top) as f32,
    };
    let utf16: Vec<_> = value.encode_utf16().collect();
    let mut p = null_mut();
    GdipCreatePath(FillModeAlternate, &mut p);
    GdipAddPathString(
        p,
        utf16.as_ptr(),
        utf16.len() as i32,
        family,
        if lf.lfWeight >= 600 {
            FontStyleBold
        } else {
            FontStyleRegular
        },
        lf.lfHeight.unsigned_abs() as f32,
        &bounds,
        format,
    );
    GdipDeleteStringFormat(format);
    GdipDeleteFontFamily(family);
    TEXT_PATHS.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache.len() >= 256 {
            if let Some((_, old)) = cache.pop_front() {
                GdipDeletePath(old);
            }
        }
        cache.push_back((key, p));
    });
    Some(p)
}
pub unsafe fn text(dc: HDC, font: HFONT, value: &str, r: RECT, color: u32, flags: u32) -> bool {
    let Some(g) = graphics(dc) else {
        return false;
    };
    if value.is_empty() || r.right <= r.left || r.bottom <= r.top {
        return true;
    }
    let Some(p) = cached_text_path(font, value, r, flags) else {
        return true;
    };
    if flags & LYRICS_UNTRIMMED != 0 && raster_lyric_text(g, font, value, r, color, flags, p) {
        return true;
    }
    let mut state = 0;
    GdipSaveGraphics(g, &mut state);
    GdipSetClipRectI(
        g,
        r.left,
        r.top,
        r.right - r.left,
        r.bottom - r.top,
        CombineModeIntersect,
    );
    GdipTranslateWorldTransform(g, r.left as f32, r.top as f32, MatrixOrderPrepend);
    let _ = LIGHT.get();
    let mut pen = null_mut();
    GdipCreatePen1(argb(color, 48), 0.3, UnitPixel, &mut pen);
    GdipSetPenLineJoin(pen, LineJoinRound);
    GdipDrawPath(g, pen, p);
    GdipDeletePen(pen);
    let mut brush = null_mut();
    GdipCreateSolidFill(argb(color, 255), &mut brush);
    GdipFillPath(g, brush.cast(), p);
    GdipDeleteBrush(brush.cast());
    GdipRestoreGraphics(g, state);
    true
}
// A cached glyph-shaped shadow, never a rectangular lyric backpanel.
thread_local! {static LYRIC_SHADOWS:std::cell::RefCell<std::collections::VecDeque<(String,BlurredText)>>=const {std::cell::RefCell::new(std::collections::VecDeque::new())};}
pub unsafe fn lyric_shadow(dc: HDC, font: HFONT, value: &str, r: RECT, flags: u32) -> bool {
    let Some(g) = graphics(dc) else { return false };
    let key = format!(
        "{:p}:{}:{}:{}:{}",
        font,
        r.right - r.left,
        r.bottom - r.top,
        flags,
        value
    );
    let pad = 12;
    let width = r.right - r.left + pad * 2;
    let height = r.bottom - r.top + pad * 2;
    if width <= 0 || height <= 0 || width as i64 * height as i64 > 2_000_000 {
        return false;
    }
    LYRIC_SHADOWS.with(|cache| {
        let mut cache = cache.borrow_mut();
        let index = cache.iter().position(|(old, _)| *old == key);
        let entry = if let Some(index) = index {
            cache.remove(index).unwrap()
        } else {
            let Some(path) = cached_text_path(font, value, r, flags | LYRICS_UNTRIMMED) else {
                return false;
            };
            let mut pixels = vec![0u32; (width * height) as usize];
            let mut bitmap = null_mut();
            if GdipCreateBitmapFromScan0(
                width,
                height,
                width * 4,
                0xe200b,
                pixels.as_mut_ptr().cast(),
                &mut bitmap,
            ) != 0
            {
                return false;
            }
            let mut canvas = null_mut();
            if GdipGetImageGraphicsContext(bitmap.cast(), &mut canvas) != 0 {
                GdipDisposeImage(bitmap.cast());
                return false;
            }
            GdipSetSmoothingMode(canvas, SmoothingModeAntiAlias);
            GdipSetPixelOffsetMode(canvas, PixelOffsetModeHighQuality);
            GdipTranslateWorldTransform(canvas, pad as f32, pad as f32 + 1., MatrixOrderPrepend);
            for (stroke, alpha) in [(8., 45), (3., 190)] {
                let mut pen = null_mut();
                GdipCreatePen1((alpha as u32) << 24, stroke, UnitPixel, &mut pen);
                GdipSetPenLineJoin(pen, LineJoinRound);
                GdipDrawPath(canvas, pen, path);
                GdipDeletePen(pen);
            }
            GdipDeleteGraphics(canvas);
            while cache.len() >= 96
                || cache
                    .iter()
                    .map(|(_, image)| image.pixels.len() * 4)
                    .sum::<usize>()
                    + pixels.len() * 4
                    > 16 * 1024 * 1024
            {
                if cache.pop_front().is_none() {
                    break;
                }
            }
            (key, BlurredText { bitmap, pixels })
        };
        GdipDrawImageRectI(
            g,
            entry.1.bitmap.cast(),
            r.left - pad,
            r.top - pad,
            width,
            height,
        );
        cache.push_back(entry);
        true
    })
}

struct BlurredText {
    bitmap: *mut GpBitmap,
    pixels: Vec<u32>,
}
thread_local! {static RASTER_LYRICS:std::cell::RefCell<std::collections::VecDeque<(String,BlurredText)>>=const {std::cell::RefCell::new(std::collections::VecDeque::new())};}
unsafe fn raster_lyric_text(
    g: *mut GpGraphics,
    font: HFONT,
    value: &str,
    r: RECT,
    color: u32,
    flags: u32,
    path: *mut GpPath,
) -> bool {
    let width = r.right - r.left;
    let height = r.bottom - r.top;
    if width <= 0 || height <= 0 || width as i64 * height as i64 > 2_000_000 {
        return false;
    }
    let mut lf: LOGFONTW = zeroed();
    GetObjectW(
        font,
        size_of::<LOGFONTW>() as i32,
        (&mut lf as *mut LOGFONTW).cast(),
    );
    let key = format!(
        "{:p}:{}:{}:{width}:{height}:{color}:{flags}:{value}",
        font, lf.lfHeight, lf.lfWeight
    );
    RASTER_LYRICS.with(|cache| {
        let mut cache = cache.borrow_mut();
        let entry = if let Some(index) = cache.iter().position(|(old, _)| *old == key) {
            cache.remove(index).unwrap()
        } else {
            let mut pixels = vec![0u32; (width * height) as usize];
            let mut bitmap = null_mut();
            if GdipCreateBitmapFromScan0(
                width,
                height,
                width * 4,
                0xe200b,
                pixels.as_mut_ptr().cast(),
                &mut bitmap,
            ) != 0
            {
                return false;
            }
            let mut canvas = null_mut();
            if GdipGetImageGraphicsContext(bitmap.cast(), &mut canvas) != 0 {
                GdipDisposeImage(bitmap.cast());
                return false;
            }
            GdipSetSmoothingMode(canvas, SmoothingModeAntiAlias);
            GdipSetPixelOffsetMode(canvas, PixelOffsetModeHighQuality);
            let mut pen = null_mut();
            GdipCreatePen1(argb(color, 48), 0.3, UnitPixel, &mut pen);
            GdipSetPenLineJoin(pen, LineJoinRound);
            GdipDrawPath(canvas, pen, path);
            GdipDeletePen(pen);
            let mut brush = null_mut();
            GdipCreateSolidFill(argb(color, 255), &mut brush);
            GdipFillPath(canvas, brush.cast(), path);
            GdipDeleteBrush(brush.cast());
            GdipDeleteGraphics(canvas);
            while cache.len() >= 192
                || cache
                    .iter()
                    .map(|(_, image)| image.pixels.len() * 4)
                    .sum::<usize>()
                    + pixels.len() * 4
                    > 24 * 1024 * 1024
            {
                if cache.pop_front().is_none() {
                    break;
                }
            }
            (key, BlurredText { bitmap, pixels })
        };
        GdipDrawImageRectI(g, entry.1.bitmap.cast(), r.left, r.top, width, height);
        cache.push_back(entry);
        true
    })
}
impl Drop for BlurredText {
    fn drop(&mut self) {
        unsafe {
            GdipDisposeImage(self.bitmap.cast());
        }
    }
}
thread_local! {
    static BLURRED_TEXT: std::cell::RefCell<std::collections::VecDeque<(String,BlurredText)>> = const {std::cell::RefCell::new(std::collections::VecDeque::new())};
    static LYRIC_WIDTHS: std::cell::RefCell<std::collections::VecDeque<(String,i32)>> = const {std::cell::RefCell::new(std::collections::VecDeque::new())};
}
pub unsafe fn lyric_text_width(dc: HDC, font: HFONT, value: &str) -> i32 {
    if value.is_empty() {
        return 0;
    }
    let mut lf: LOGFONTW = zeroed();
    GetObjectW(
        font,
        size_of::<LOGFONTW>() as i32,
        (&mut lf as *mut LOGFONTW).cast(),
    );
    let key = format!("{:p}:{}:{}:{}", font, lf.lfHeight, lf.lfWeight, value);
    if let Some(width) =
        LYRIC_WIDTHS.with(|c| c.borrow().iter().find(|(k, _)| *k == key).map(|(_, w)| *w))
    {
        return width;
    }
    let Some(g) = graphics(dc) else {
        return 0;
    };
    let mut family = null_mut();
    let mut gp_font = null_mut();
    let mut format = null_mut();
    GdipCreateFontFamilyFromName(lf.lfFaceName.as_ptr(), null_mut(), &mut family);
    if family.is_null() {
        return 0;
    }
    GdipCreateFont(
        family,
        lf.lfHeight.unsigned_abs() as f32,
        if lf.lfWeight >= 600 {
            FontStyleBold
        } else {
            FontStyleRegular
        },
        UnitPixel,
        &mut gp_font,
    );
    GdipStringFormatGetGenericTypographic(&mut format);
    GdipSetStringFormatFlags(
        format,
        StringFormatFlagsNoWrap | StringFormatFlagsNoClip | StringFormatFlagsMeasureTrailingSpaces,
    );
    let text: Vec<_> = value.encode_utf16().collect();
    let layout = RectF {
        X: 0.0,
        Y: 0.0,
        Width: 100000.0,
        Height: 10000.0,
    };
    let mut bounds: RectF = zeroed();
    let status = GdipMeasureString(
        g,
        text.as_ptr(),
        text.len() as i32,
        gp_font,
        &layout,
        format,
        &mut bounds,
        null_mut(),
        null_mut(),
    );
    GdipDeleteStringFormat(format);
    GdipDeleteFont(gp_font);
    GdipDeleteFontFamily(family);
    let width = if status == 0 {
        bounds.Width.ceil() as i32
    } else {
        0
    };
    LYRIC_WIDTHS.with(|c| {
        let mut c = c.borrow_mut();
        if c.len() >= 512 {
            c.pop_front();
        }
        c.push_back((key, width));
    });
    width
}
pub unsafe fn blurred_text(
    dc: HDC,
    font: HFONT,
    value: &str,
    r: RECT,
    color: u32,
    flags: u32,
    blur_px: f32,
) -> bool {
    let Some(g) = graphics(dc) else {
        return false;
    };
    if value.is_empty() || r.right <= r.left || r.bottom <= r.top {
        return true;
    }
    let radius = blur_px.clamp(0.5, 6.0);
    let pad = (radius * 3.0).ceil() as i32 + 2;
    let width = r.right - r.left + pad * 2;
    let height = r.bottom - r.top + pad * 2;
    if width as usize * height as usize * 4 > 8 * 1024 * 1024 {
        return false;
    }
    let mut lf: LOGFONTW = zeroed();
    GetObjectW(
        font,
        size_of::<LOGFONTW>() as i32,
        (&mut lf as *mut LOGFONTW).cast(),
    );
    let key = format!(
        "{:p}:{}:{}:{}:{}:{}:{}:{}:{}",
        font, lf.lfHeight, lf.lfWeight, width, height, color, flags, radius, value
    );
    BLURRED_TEXT.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some(index) = cache.iter().position(|(k, _)| *k == key) {
            let entry = cache.remove(index).unwrap();
            cache.push_back(entry);
        } else {
            let Some(path) = cached_text_path(font, value, r, flags | LYRICS_UNTRIMMED) else {
                return false;
            };
            let mut pixels = vec![0u32; width as usize * height as usize];
            let mut bitmap = null_mut();
            let mut mask_graphics = null_mut();
            if GdipCreateBitmapFromScan0(
                width,
                height,
                width * 4,
                0xe200b,
                pixels.as_mut_ptr().cast(),
                &mut bitmap,
            ) != 0
            {
                return false;
            }
            if GdipGetImageGraphicsContext(bitmap.cast(), &mut mask_graphics) != 0 {
                GdipDisposeImage(bitmap.cast());
                return false;
            }
            GdipSetSmoothingMode(mask_graphics, SmoothingModeAntiAlias);
            GdipTranslateWorldTransform(mask_graphics, pad as f32, pad as f32, MatrixOrderPrepend);
            let mut brush = null_mut();
            GdipCreateSolidFill(0xffffffff, &mut brush);
            GdipFillPath(mask_graphics, brush.cast(), path);
            GdipDeleteBrush(brush.cast());
            GdipDeleteGraphics(mask_graphics);
            let mask = image::GrayImage::from_vec(
                width as u32,
                height as u32,
                pixels.iter().map(|p| (p >> 24) as u8).collect(),
            )
            .unwrap();
            let blurred = image::imageops::blur(&mask, radius);
            let red = color & 255;
            let green = (color >> 8) & 255;
            let blue = (color >> 16) & 255;
            for (pixel, alpha) in pixels.iter_mut().zip(blurred.as_raw()) {
                let alpha = *alpha as u32;
                *pixel = (alpha << 24)
                    | ((red * alpha / 255) << 16)
                    | ((green * alpha / 255) << 8)
                    | (blue * alpha / 255);
            }
            while cache.len() >= 64
                || cache.iter().map(|(_, b)| b.pixels.len() * 4).sum::<usize>() + pixels.len() * 4
                    > 8 * 1024 * 1024
            {
                cache.pop_front();
            }
            cache.push_back((key, BlurredText { bitmap, pixels }));
        }
        let bitmap = cache.back().unwrap().1.bitmap;
        GdipDrawImageRectI(g, bitmap.cast(), r.left - pad, r.top - pad, width, height) == 0
    })
}

pub unsafe fn karaoke_text(
    dc: HDC,
    font: HFONT,
    value: &str,
    progress: f32,
    r: RECT,
    lit_color: u32,
    unlit_color: u32,
    glow_color: u32,
    flags: u32,
) -> bool {
    let Some(g) = graphics(dc) else {
        return false;
    };
    if value.is_empty() || r.right <= r.left || r.bottom <= r.top {
        return true;
    }
    let Some(p) = cached_text_path(font, value, r, flags | LYRICS_UNTRIMMED) else {
        return true;
    };
    let mut path_bounds = RectF {
        X: 0.0,
        Y: 0.0,
        Width: (r.right - r.left) as f32,
        Height: (r.bottom - r.top) as f32,
    };
    let _ = GdipGetPathWorldBounds(p, &mut path_bounds, null_mut(), null_mut());
    let mut state = 0;
    GdipSaveGraphics(g, &mut state);
    GdipSetClipRectI(
        g,
        r.left - 8,
        r.top - 8,
        r.right - r.left + 16,
        r.bottom - r.top + 16,
        CombineModeIntersect,
    );
    let phase = progress.clamp(0.0, 1.0);
    let lift = (phase * std::f32::consts::PI).sin() * (r.bottom - r.top) as f32 * 0.055;
    GdipTranslateWorldTransform(g, r.left as f32, r.top as f32 - lift, MatrixOrderPrepend);
    // Each timed word rises slightly as it is sung, then settles on the baseline.
    let mut base_brush = null_mut();
    GdipCreateSolidFill(argb(unlit_color, 175), &mut base_brush);
    GdipFillPath(g, base_brush.cast(), p);
    GdipDeleteBrush(base_brush.cast());
    // Lit progressive syllable layer
    let clamped = phase * phase * (3.0 - 2.0 * phase);
    if clamped > 0.005 {
        // The world transform already places the path at r.left/r.top.
        // Subsequent clip coordinates must remain local to that transform.
        let lit_right = (path_bounds.X + (path_bounds.Width + 6.0) * clamped).ceil() as i32;
        GdipSetClipRectI(
            g,
            -8,
            -8,
            (lit_right + 14).max(1),
            r.bottom - r.top + 16,
            CombineModeIntersect,
        );
        if phase < 1.0 {
            let mut glow_pen = null_mut();
            GdipCreatePen1(
                argb(
                    glow_color,
                    (70.0 * (phase * std::f32::consts::PI).sin()) as u32,
                ),
                3.0,
                UnitPixel,
                &mut glow_pen,
            );
            GdipSetPenLineJoin(glow_pen, LineJoinRound);
            GdipDrawPath(g, glow_pen, p);
            GdipDeletePen(glow_pen);
        }
        // A narrow feather follows the sung word instead of a hard rectangular wipe.
        let layers: &[(i32, u32)] = if phase >= 1.0 {
            &[(8, 255)]
        } else {
            &[(6, 45), (2, 95), (-3, 255)]
        };
        for (feather, alpha) in layers {
            GdipSetClipRectI(
                g,
                -8,
                -8,
                (lit_right + 8 + feather).max(1),
                r.bottom - r.top + 16,
                CombineModeIntersect,
            );
            let mut lit_brush = null_mut();
            GdipCreateSolidFill(argb(lit_color, *alpha), &mut lit_brush);
            GdipFillPath(g, lit_brush.cast(), p);
            GdipDeleteBrush(lit_brush.cast());
        }
    }
    GdipRestoreGraphics(g, state);
    true
}
pub unsafe fn image(
    dc: HDC,
    r: RECT,
    image: &crate::catalog::Bitmap,
    radius: f32,
    alpha: u8,
) -> bool {
    let Some(g) = graphics(dc) else {
        return false;
    };
    if r.right <= r.left || r.bottom <= r.top {
        return true;
    }
    let width = r.right - r.left;
    let height = r.bottom - r.top;
    let key = (image.id, width, height);
    let cacheable = resized_bytes(key) <= RESIZED_IMAGE_BYTES;
    let cached = RESIZED_IMAGES.with(|cache| {
        let mut cache = cache.borrow_mut();
        let index = cache.iter().position(|(k, _)| *k == key)?;
        let entry = cache.remove(index)?;
        let bitmap = entry.1;
        cache.push_back(entry);
        Some(bitmap)
    });
    let bitmap = if let Some(bitmap) = cached {
        bitmap
    } else {
        let mut source = null_mut();
        let mut bitmap = null_mut();
        let mut resize = null_mut();
        let mut attributes = null_mut();
        if GdipCreateBitmapFromScan0(
            image.width as i32,
            image.height as i32,
            image.width as i32 * 4,
            0xe200b,
            image.pixels.as_ptr().cast(),
            &mut source,
        ) != 0
        {
            return true;
        }
        if GdipCreateBitmapFromScan0(width, height, 0, 0xe200b, null(), &mut bitmap) != 0 {
            GdipDisposeImage(source.cast());
            return true;
        }
        GdipGetImageGraphicsContext(bitmap.cast(), &mut resize);
        GdipSetInterpolationMode(resize, InterpolationModeHighQualityBicubic);
        GdipSetPixelOffsetMode(resize, PixelOffsetModeHighQuality);
        GdipCreateImageAttributes(&mut attributes);
        GdipSetImageAttributesWrapMode(attributes, WrapModeTileFlipXY, 0, 0);
        GdipDrawImageRectRectI(
            resize,
            source.cast(),
            0,
            0,
            width,
            height,
            0,
            0,
            image.width as i32,
            image.height as i32,
            UnitPixel,
            attributes,
            0,
            null_mut(),
        );
        GdipDisposeImageAttributes(attributes);
        GdipDeleteGraphics(resize);
        GdipDisposeImage(source.cast());
        if cacheable {
            RESIZED_IMAGES.with(|cache| {
                let mut cache = cache.borrow_mut();
                let mut bytes: usize = cache.iter().map(|(key, _)| resized_bytes(*key)).sum();
                while cache.len() >= 48 || bytes + resized_bytes(key) > RESIZED_IMAGE_BYTES {
                    if let Some((old_key, old)) = cache.pop_front() {
                        bytes = bytes.saturating_sub(resized_bytes(old_key));
                        GdipDisposeImage(old.cast());
                    } else {
                        break;
                    }
                }
                cache.push_back((key, bitmap));
            });
        }
        bitmap
    };
    let mut state = 0;
    GdipSaveGraphics(g, &mut state);
    let p = path(r, radius);
    GdipSetClipPath(g, p, CombineModeIntersect);
    let mut attributes = null_mut();
    GdipCreateImageAttributes(&mut attributes);
    GdipSetImageAttributesWrapMode(attributes, WrapModeTileFlipXY, 0, 0);
    if alpha < 255 {
        let mut matrix = ColorMatrix::default();
        for i in 0..5 {
            matrix.m[i * 5 + i] = 1.0;
        }
        matrix.m[18] = alpha as f32 / 255.0;
        GdipSetImageAttributesColorMatrix(
            attributes,
            ColorAdjustTypeBitmap,
            1,
            &matrix,
            null(),
            ColorMatrixFlagsDefault,
        );
    }
    GdipSetInterpolationMode(g, InterpolationModeNearestNeighbor);
    GdipDrawImageRectRectI(
        g,
        bitmap.cast(),
        r.left,
        r.top,
        r.right - r.left,
        r.bottom - r.top,
        0,
        0,
        width,
        height,
        UnitPixel,
        attributes,
        0,
        null_mut(),
    );
    GdipDisposeImageAttributes(attributes);
    GdipRestoreGraphics(g, state);
    GdipDeletePath(p);

    if !cacheable {
        GdipDisposeImage(bitmap.cast());
    }

    true
}

// Draw at device resolution: no intermediate downsampling or bilinear blur.
pub unsafe fn recessed_visualizer(_dc: HDC, width: i32, height: i32, draw: impl FnOnce()) {
    if width > 0 && height > 0 {
        draw();
    }
}
pub unsafe fn wave_landscape(
    dc: HDC,
    points: &[(f32, f32)],
    baseline_y: f32,
    fill_top: u32,
    fill_bottom: u32,
    stroke: u32,
    stroke_width: f32,
    glow: u32,
) -> bool {
    let Some(g) = graphics(dc) else {
        return false;
    };
    if points.len() < 2 {
        return true;
    }
    let pts: Vec<PointF> = points.iter().map(|&(x, y)| PointF { X: x, Y: y }).collect();
    let min_y = pts.iter().map(|p| p.Y).fold(baseline_y, f32::min);

    let bounds = RectF {
        X: pts[0].X,
        Y: min_y,
        Width: (pts[pts.len() - 1].X - pts[0].X).max(1.0),
        Height: (baseline_y - min_y).max(1.0),
    };

    let mut brush = null_mut();
    GdipCreateLineBrushFromRectWithAngle(
        &bounds,
        fill_top,
        fill_bottom,
        90.0,
        0,
        WrapModeTileFlipXY,
        &mut brush,
    );

    let mut path = null_mut();
    GdipCreatePath(FillModeAlternate, &mut path);
    GdipAddPathCurve2(path, pts.as_ptr(), pts.len() as i32, 0.45);
    GdipAddPathLine(path, pts[pts.len() - 1].X, baseline_y, pts[0].X, baseline_y);
    GdipClosePathFigure(path);

    GdipFillPath(g, brush.cast(), path);
    GdipDeleteBrush(brush.cast());
    GdipDeletePath(path);

    if glow != 0 {
        let mut glow_pen = null_mut();
        GdipCreatePen1(glow, stroke_width * 2.8, UnitPixel, &mut glow_pen);
        GdipDrawCurve2(g, glow_pen, pts.as_ptr(), pts.len() as i32, 0.45);
        GdipDeletePen(glow_pen);
    }

    let mut pen = null_mut();
    GdipCreatePen1(stroke, stroke_width, UnitPixel, &mut pen);
    GdipDrawCurve2(g, pen, pts.as_ptr(), pts.len() as i32, 0.45);
    GdipDeletePen(pen);

    true
}

pub unsafe fn spectrum_bars(
    dc: HDC,
    bars: &[f32],
    x: f32,
    y: f32,
    width: f32,
    max_height: f32,
    accent: u32,
    glow: u32,
) -> bool {
    let Some(g) = graphics(dc) else {
        return false;
    };
    if bars.is_empty() {
        return true;
    }
    let count = bars.len();
    let spacing = 3.0f32;
    let total_spacing = (count - 1) as f32 * spacing;
    let bar_w = ((width - total_spacing) / count as f32).max(2.0);

    for (i, &mag) in bars.iter().enumerate() {
        let bh = (mag * max_height).clamp(4.0, max_height);
        let bx = x + i as f32 * (bar_w + spacing);
        let by = y + max_height - bh;

        let bounds = RectF {
            X: bx,
            Y: by,
            Width: bar_w,
            Height: bh,
        };

        // Linear gradient: vibrant accent at top fading downwards
        let mut brush = null_mut();
        GdipCreateLineBrushFromRectWithAngle(
            &bounds,
            argb(accent, 225),
            argb(accent, 35),
            90.0,
            0,
            WrapModeTileFlipXY,
            &mut brush,
        );

        let mut path = null_mut();
        let radius = (bar_w * 0.45).clamp(1.0, 4.0);
        GdipCreatePath(FillModeAlternate, &mut path);
        let d = radius * 2.0;
        GdipAddPathArc(path, bx + bar_w - d, by, d, d, 270.0, 90.0);
        GdipAddPathLine(path, bx + bar_w, by + bh, bx, by + bh);
        GdipAddPathArc(path, bx, by, d, d, 180.0, 90.0);
        GdipClosePathFigure(path);

        GdipFillPath(g, brush.cast(), path);
        GdipDeleteBrush(brush.cast());
        GdipDeletePath(path);

        // Glowing cap at the top of each active bar
        if glow != 0 {
            let mut cap_pen = null_mut();
            GdipCreatePen1(glow, 2.0, UnitPixel, &mut cap_pen);
            GdipDrawLine(g, cap_pen, bx + radius, by, bx + bar_w - radius, by);
            GdipDeletePen(cap_pen);
        }
    }

    true
}

pub unsafe fn trap_nation_circle(
    dc: HDC,
    cx: f32,
    cy: f32,
    base_radius: f32,
    bass_pump: f32,
    layers: &[Vec<f32>],
    accent: u32,
    glow_col: u32,
) -> bool {
    let Some(g) = graphics(dc) else {
        return false;
    };
    if layers.is_empty() {
        return true;
    }

    let inner = base_radius * (1. + bass_pump.clamp(0., 1.) * 0.25);
    for (layer, spikes) in layers.iter().enumerate().rev() {
        let count = spikes.len();
        let alpha = if layer == 0 { 48 } else { 12 };
        let points: Vec<PointF> = spikes
            .iter()
            .enumerate()
            .map(|(k, spike)| {
                let theta =
                    k as f32 * std::f32::consts::TAU / count as f32 - std::f32::consts::FRAC_PI_2;
                let radius = inner + spike + layer as f32 * 1.2;
                PointF {
                    X: cx + radius * theta.cos(),
                    Y: cy + radius * theta.sin(),
                }
            })
            .collect();
        if points.len() < 3 {
            continue;
        }
        let mut halo = null_mut();
        GdipCreatePath(FillModeAlternate, &mut halo);
        GdipAddPathClosedCurve2(halo, points.as_ptr(), points.len() as i32, 0.5);
        GdipAddPathEllipse(halo, cx - inner, cy - inner, inner * 2., inner * 2.);
        let color = if layer == 1 {
            let r = (accent & 255) * 3 / 4 + 63;
            let g = ((accent >> 8) & 255) * 3 / 4 + 63;
            let b = ((accent >> 16) & 255) * 3 / 4 + 63;
            r | (g << 8) | (b << 16)
        } else {
            accent
        };
        let mut brush = null_mut();
        GdipCreateSolidFill(argb(color, alpha), &mut brush);
        GdipFillPath(g, brush.cast(), halo);
        GdipDeleteBrush(brush.cast());
        GdipDeletePath(halo);
        let mut edge = null_mut();
        GdipCreatePath(FillModeAlternate, &mut edge);
        GdipAddPathClosedCurve2(edge, points.as_ptr(), points.len() as i32, 0.5);
        let mut pen = null_mut();
        GdipCreatePen1(
            argb(color, if layer == 0 { 155 } else { alpha * 2 }),
            if layer == 0 { 1.8 } else { 1.2 },
            UnitPixel,
            &mut pen,
        );
        GdipDrawPath(g, pen, edge);
        GdipDeletePen(pen);
        GdipDeletePath(edge);
    }
    let _ = glow_col;

    true
}

#[cfg(test)]
mod tests {
    #[test]
    fn surface_opacity_scales_backgrounds_once_and_restores_nested_scopes() {
        assert_eq!(surface_color(0xffffffff), 0xffffffff);
        {
            let _surface = SurfaceOpacity::set(40);
            assert_eq!(surface_color(0xffffffff), 0x66ffffff);
            assert_eq!(surface_color(0x80aabbcc), 0x33aabbcc);
            {
                let _surface = SurfaceOpacity::set(0);
                assert_eq!(surface_color(0xffabcdef), 0x00abcdef);
            }
            assert_eq!(surface_color(0xffffffff), 0x66ffffff);
        }
        assert_eq!(surface_color(0xffffffff), 0xffffffff);
    }
    #[test]
    fn visualizer_preserves_device_resolution_and_foreground_layer() {
        unsafe {
            let dc = CreateCompatibleDC(null_mut());
            let bitmap = CreateBitmap(320, 120, 1, 32, null());
            let previous = SelectObject(dc, bitmap);
            let mut pixels = vec![0u32; 320 * 120];
            let frame = Frame::begin(dc, pixels.as_mut_ptr().cast(), 320, 320, 120, false).unwrap();
            let before = CURRENT.get();
            recessed_visualizer(dc, 320, 120, || {
                fill(
                    dc,
                    RECT {
                        left: 100,
                        top: 30,
                        right: 140,
                        bottom: 80,
                    },
                    0xffffffff,
                    0.0,
                );
            });
            assert_eq!(CURRENT.get(), before);
            let mut foreground: RECT = zeroed();
            foreground.left = 200;
            foreground.top = 30;
            foreground.right = 240;
            foreground.bottom = 80;
            fill(dc, foreground, 0xffffffff, 0.0);
            drop(frame);
            let alpha = |x: usize, y: usize| pixels[y * 320 + x] >> 24;
            assert!(
                alpha(120, 50) == 255,
                "visualizer must preserve crisp device-resolution coverage"
            );
            assert_eq!(
                alpha(80, 50),
                0,
                "depth must not bleed into unrelated content"
            );
            assert_eq!(alpha(220, 50), 255, "foreground must stay sharp and opaque");
            SelectObject(dc, previous);
            DeleteObject(bitmap);
            DeleteDC(dc);
        }
    }

    #[test]
    fn lyric_paths_never_ellipsize_and_blur_is_cached_filled_coverage() {
        unsafe {
            let dc = CreateCompatibleDC(null_mut());
            let mut lf: LOGFONTW = zeroed();
            lf.lfHeight = -36;
            lf.lfWeight = 700;
            for (target, value) in lf.lfFaceName.iter_mut().zip("Segoe UI".encode_utf16()) {
                *target = value;
            }
            let font = CreateFontIndirectW(&lf);
            let mut pixels = vec![0u32; 400 * 80];
            let frame = Frame::begin(dc, pixels.as_mut_ptr().cast(), 400, 400, 80, false).unwrap();
            let small = RECT {
                left: 10,
                top: 10,
                right: 25,
                bottom: 65,
            };
            let path = cached_text_path(font, "Have", small, DT_LEFT | LYRICS_UNTRIMMED).unwrap();
            let mut bounds: RectF = zeroed();
            GdipGetPathWorldBounds(path, &mut bounds, null_mut(), null_mut());
            assert!(bounds.Width > 65.0, "full word was replaced by an ellipsis");
            let rect = RECT {
                left: 10,
                top: 10,
                right: 390,
                bottom: 65,
            };
            assert!(blurred_text(
                dc,
                font,
                "All your lies",
                rect,
                0xffffff,
                DT_LEFT,
                2.0
            ));
            let cached = BLURRED_TEXT.with(|c| {
                let c = c.borrow();
                let entry = &c.back().unwrap().1;
                assert!(
                    entry.pixels.iter().any(|p| p >> 24 > 150),
                    "glyph interiors must remain filled"
                );
                assert!(
                    entry.pixels.iter().any(|p| {
                        let a = p >> 24;
                        a > 0 && a < 100
                    }),
                    "Gaussian blur must produce a soft fringe"
                );
                entry.bitmap
            });
            assert!(blurred_text(
                dc,
                font,
                "All your lies",
                rect,
                0xffffff,
                DT_LEFT,
                2.0
            ));
            assert_eq!(
                cached,
                BLURRED_TEXT.with(|c| c.borrow().back().unwrap().1.bitmap)
            );
            drop(frame);
            DeleteObject(font);
            DeleteDC(dc);
        }
    }

    use super::*;
    #[test]
    fn deezer_logo_draws_visible_shape_inside_its_bounds() {
        unsafe {
            let dc = CreateCompatibleDC(null_mut());
            let bitmap = CreateBitmap(64, 64, 1, 32, null());
            let previous = SelectObject(dc, bitmap);
            let mut outlines = Vec::new();
            for _ in 0..1 {
                let mut pixels = vec![0u32; 64 * 64];
                let frame =
                    Frame::begin(dc, pixels.as_mut_ptr().cast(), 64, 64, 64, false).unwrap();
                assert!(deezer_logo(
                    dc,
                    RECT {
                        left: 8,
                        top: 8,
                        right: 56,
                        bottom: 56
                    }
                ));
                drop(frame);
                assert!(pixels.iter().filter(|pixel| (**pixel >> 24) > 100).count() > 150);
                assert!(pixels[..8 * 64].iter().all(|pixel| *pixel == 0));
                outlines.push(pixels);
            }
            assert_eq!(outlines.len(), 1);
            SelectObject(dc, previous);
            DeleteObject(bitmap);
            DeleteDC(dc);
        }
    }
    #[test]
    fn surface_corner_styles_render_square_through_extra_rounded() {
        unsafe {
            let dc = CreateCompatibleDC(null_mut());
            let bitmap = CreateBitmap(120, 48, 1, 32, null());
            let previous = SelectObject(dc, bitmap);
            let mut ui = crate::ui::Ui::default();
            let mut top_edges = Vec::new();
            for style in 0..4 {
                ui.corner_style = style;
                let mut pixels = vec![0u32; 120 * 48];
                let frame =
                    Frame::begin(dc, pixels.as_mut_ptr().cast(), 120, 120, 48, false).unwrap();
                assert!(fill(
                    dc,
                    RECT {
                        left: 0,
                        top: 0,
                        right: 120,
                        bottom: 48
                    },
                    0xffffffff,
                    ui.corner_px(16) as f32
                ));
                drop(frame);
                top_edges.push(pixels[..120].iter().filter(|p| (**p >> 24) > 200).count());
                assert_eq!(pixels[24 * 120 + 60], 0xffffffff);
                if style == 0 {
                    assert_eq!(
                        pixels[0], 0xffffffff,
                        "square style still rounded its corner"
                    );
                } else {
                    assert_eq!(pixels[0], 0, "rounded surface lost its corner cutout");
                }
            }
            assert!(
                top_edges.windows(2).all(|pair| pair[0] > pair[1]),
                "corner styles must visibly increase rounding: {top_edges:?}"
            );
            SelectObject(dc, previous);
            DeleteObject(bitmap);
            DeleteDC(dc);
        }
    }

    #[test]
    fn resized_cover_cache_obeys_byte_budget_and_retains_recent_covers() {
        unsafe {
            let dc = CreateCompatibleDC(null_mut());
            let mut pixels = vec![0xff000000u32; 16];
            let frame = Frame::begin(dc, pixels.as_mut_ptr().cast(), 4, 4, 4, false).unwrap();
            RESIZED_IMAGES.with(|cache| {
                for (_, bitmap) in cache.borrow_mut().drain(..) {
                    GdipDisposeImage(bitmap.cast());
                }
            });
            let sources: Vec<_> = (0..9)
                .map(|_| crate::catalog::Bitmap::new(1, 1, vec![0xffffffff]))
                .collect();
            let rect = RECT {
                left: 0,
                top: 0,
                right: 1024,
                bottom: 1024,
            };
            for source in sources.iter().take(8) {
                assert!(image(dc, rect, source, 0.0, 255));
                RESIZED_IMAGES.with(|cache| {
                    let bytes: usize = cache
                        .borrow()
                        .iter()
                        .map(|(key, _)| resized_bytes(*key))
                        .sum();
                    assert!(bytes <= RESIZED_IMAGE_BYTES);
                });
            }
            assert!(image(dc, rect, &sources[2], 0.0, 255));
            assert!(image(dc, rect, &sources[8], 0.0, 255));
            RESIZED_IMAGES.with(|cache| {
                let mut cache = cache.borrow_mut();
                assert!(cache.iter().any(|(key, _)| key.0 == sources[2].id));
                assert!(!cache.iter().any(|(key, _)| key.0 == sources[3].id));
                for (_, bitmap) in cache.drain(..) {
                    GdipDisposeImage(bitmap.cast());
                }
            });
            drop(frame);
            DeleteDC(dc);
        }
    }
    #[test]
    fn karaoke_lights_progressively_at_nonzero_screen_coordinates() {
        unsafe {
            let dc = CreateCompatibleDC(null_mut());
            let bitmap = CreateBitmap(640, 100, 1, 32, null());
            let previous = SelectObject(dc, bitmap);
            let mut description: LOGFONTW = zeroed();
            description.lfHeight = -32;
            description.lfWeight = 700;
            for (target, value) in description
                .lfFaceName
                .iter_mut()
                .zip("Segoe UI".encode_utf16())
            {
                *target = value;
            }
            let font = CreateFontIndirectW(&description);
            let render = |progress| {
                let mut pixels = vec![0xff000000u32; 640 * 100];
                let frame =
                    Frame::begin(dc, pixels.as_mut_ptr().cast(), 640, 640, 100, false).unwrap();
                assert!(karaoke_text(
                    dc,
                    font,
                    "Wide words here",
                    progress,
                    RECT {
                        left: 220,
                        top: 30,
                        right: 620,
                        bottom: 90
                    },
                    0x000000ff,
                    0x00808080,
                    0x000000ff,
                    DT_LEFT
                ));
                drop(frame);
                pixels
                    .iter()
                    .filter(|pixel| {
                        let red = (**pixel >> 16) & 255;
                        let green = (**pixel >> 8) & 255;
                        red > green + 20
                    })
                    .count()
            };
            assert_eq!(render(0.0), 0);
            let half = render(0.5);
            let full = render(1.0);
            assert!(
                half > 20,
                "progressive text was clipped away at its screen offset"
            );
            assert!(full > half + 20, "word highlighting did not advance");
            SelectObject(dc, previous);
            DeleteObject(font);
            DeleteObject(bitmap);
            DeleteDC(dc);
        }
    }
    #[test]
    fn premultiplied_canvas_preserves_backdrop_and_blends_foreground() {
        unsafe {
            let dc = CreateCompatibleDC(null_mut());
            let mut pixels = vec![0xff80c040u32; 16];
            let frame = Frame::begin(dc, pixels.as_mut_ptr().cast(), 4, 4, 4, false).unwrap();
            assert!(fill(
                dc,
                RECT {
                    left: 0,
                    top: 0,
                    right: 4,
                    bottom: 4
                },
                0x800000ff,
                0.0
            ));
            drop(frame);
            assert_eq!(pixels[15], 0xff80c040);
            assert_ne!(pixels[0], 0xff80c040);
        }
    }
}
