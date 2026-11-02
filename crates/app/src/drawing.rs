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
// Rounding is baked into the cached pixels rather than clipped every frame.
type ImageKey = (u64, i32, i32, u32);
#[path = "drawing/composite.rs"]
mod composite;
pub(crate) mod gpu;
#[path = "drawing/gpu_dispatch.rs"]
mod gpu_dispatch;
#[path = "provider_logos.rs"]
mod provider_logos;
pub unsafe fn spotify_logo(dc: HDC, r: RECT) {
    let Some(g) = graphics(dc) else {
        return;
    };
    let size = (r.right - r.left).min(r.bottom - r.top).max(0) as f32;
    let x = r.left as f32 + ((r.right - r.left) as f32 - size) / 2.0;
    let y = r.top as f32 + ((r.bottom - r.top) as f32 - size) / 2.0;
    let mut brush = null_mut();
    GdipCreateSolidFill(0xff1ed760, &mut brush);
    gpu_dispatch::GdipFillEllipse(g, brush.cast(), x, y, size, size);
    gpu_dispatch::GdipDeleteBrush(brush.cast());
    let mut pen = null_mut();
    GdipCreatePen1(0xff121212, size * 0.065, UnitPixel, &mut pen);
    GdipSetPenStartCap(pen, LineCapRound);
    GdipSetPenEndCap(pen, LineCapRound);
    for (left, top, right, bottom) in [
        (0.25, 0.36, 0.76, 0.43),
        (0.29, 0.49, 0.70, 0.55),
        (0.33, 0.61, 0.65, 0.66),
    ] {
        gpu_dispatch::GdipDrawBezier(
            g, pen, x + size * left, y + size * top,
            x + size * 0.42, y + size * (top - 0.06),
            x + size * 0.62, y + size * (top - 0.03),
            x + size * right, y + size * bottom,
        );
    }
    GdipDeletePen(pen);
}

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
    gpu_dispatch::GdipFillPath(g, brush.cast(), path);
    gpu_dispatch::GdipDeleteBrush(brush.cast());
    gpu_dispatch::GdipDeletePath(path);
    true
}
type CachedImage = (ImageKey, *mut GpBitmap);
const RESIZED_IMAGE_BYTES: usize = 8 * 1024 * 1024;
fn resized_bytes(key: ImageKey) -> usize {
    (key.1.max(0) as usize)
        .saturating_mul(key.2.max(0) as usize)
        .saturating_mul(4)
}
thread_local! {static RESIZED_IMAGES:std::cell::RefCell<std::collections::VecDeque<CachedImage>>=const {std::cell::RefCell::new(std::collections::VecDeque::new())};}
pub(crate) fn resized_artwork_stats() -> (usize, usize) {
    RESIZED_IMAGES.with(|cache| {
        let cache = cache.borrow();
        (
            cache.len(),
            cache.iter().map(|(key, _)| resized_bytes(*key)).sum(),
        )
    })
}
pub(crate) fn evict_resized_artwork(ids: &std::collections::HashSet<u64>) {
    if ids.is_empty() {
        return;
    }
    ARTWORK_PREPARATION.with(|state| {
        if let Some(state) = state.borrow_mut().as_mut() {
            state.pending.retain(|key| !ids.contains(&key.0));
        }
    });
    RESIZED_IMAGES.with(|cache| {
        let mut cache = cache.borrow_mut();
        cache.retain(|(key, bitmap)| {
            if ids.contains(&key.0) {
                unsafe {
                    gpu_dispatch::GdipDisposeImage((*bitmap).cast());
                }
                false
            } else {
                true
            }
        });
    });
}
thread_local! {static CURRENT:Cell<(HDC,*mut GpGraphics)>=const {Cell::new((null_mut(),null_mut()))};}
fn is_current_graphics(g: *mut GpGraphics) -> bool { CURRENT.get().1 == g && !g.is_null() }
thread_local! {static CURRENT_CLIP:Cell<Option<(i32,i32,i32,i32)>>=const {Cell::new(None)};}
thread_local! {static COMPLEX_CLIP:Cell<bool>=const {Cell::new(false)};}
thread_local! {static GPU_DC_CLIPS:std::cell::RefCell<Vec<RECT>>=const {std::cell::RefCell::new(Vec::new())};}
struct DcRegions { current:HRGN,scratch:HRGN }
impl Drop for DcRegions {fn drop(&mut self){unsafe{DeleteObject(self.current);DeleteObject(self.scratch);}}}
thread_local! {static DC_REGIONS:std::cell::RefCell<DcRegions>=std::cell::RefCell::new(DcRegions {
    current:unsafe{CreateRectRgn(0,0,0,0)},scratch:unsafe{CreateRectRgn(0,0,0,0)}
});}
thread_local! {static TEXT_PATHS:std::cell::RefCell<std::collections::VecDeque<(String,*mut GpPath)>>=const {std::cell::RefCell::new(std::collections::VecDeque::new())};}
thread_local! {static LIGHT:Cell<bool>=const {Cell::new(false)};}
thread_local! {static FRAME_FONTS:std::cell::RefCell<std::collections::HashMap<usize,LOGFONTW>>=std::cell::RefCell::new(std::collections::HashMap::new());}
unsafe fn font_properties(font:HFONT)->LOGFONTW {
    let read=|| {let mut lf:LOGFONTW=zeroed();GetObjectW(font,size_of::<LOGFONTW>() as i32,(&mut lf as *mut LOGFONTW).cast());lf};
    if CURRENT.get().1.is_null(){return read();}
    FRAME_FONTS.with(|fonts|*fonts.borrow_mut().entry(font as usize).or_insert_with(read))
}
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
#[derive(Clone, Copy)]
struct FramePixels {
    bits: *mut u32,
    stride: usize,
    width: i32,
    height: i32,
}
thread_local! { static FRAME_PIXELS: Cell<Option<FramePixels>> = const { Cell::new(None) }; }

// All rasters and the destination use the same premultiplied BGRA format.
// Flush GDI+ before writing and intersect the current DC clip with the frame.
unsafe fn composite_raster(
    g: *mut GpGraphics,
    pixels: &[u32],
    width: i32,
    height: i32,
    x: i32,
    y: i32,
) -> bool {
    // The direct pixel blender only supports rectangular clips. GDI+ handles
    // disjoint update regions without painting into the gaps between them.
    if COMPLEX_CLIP.get() {
        return false;
    }
    let Some(frame) = FRAME_PIXELS.get() else {
        return false;
    };
    let Some((clip_left, clip_top, clip_right, clip_bottom)) = CURRENT_CLIP.get() else {
        return false;
    };
    if width <= 0 || height <= 0 || pixels.len() < width as usize * height as usize {
        return false;
    }
    let left = x.max(clip_left).max(0);
    let top = y.max(clip_top).max(0);
    let right = x.saturating_add(width).min(clip_right).min(frame.width);
    let bottom = y.saturating_add(height).min(clip_bottom).min(frame.height);
    if left >= right || top >= bottom {
        return true;
    }
    GdipFlush(g, FlushIntentionSync);
    let count = (right - left) as usize;
    for row in top..bottom {
        let start = (row - y) as usize * width as usize + (left - x) as usize;
        let destination = std::slice::from_raw_parts_mut(
            frame.bits.add(row as usize * frame.stride + left as usize),
            count,
        );
        composite::blend_row(destination, &pixels[start..start + count]);
    }
    true
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
            gpu_dispatch::GdipDisposeImage(image.cast());
            return None;
        }
        GdipSetSmoothingMode(graphics, SmoothingModeAntiAlias);
        GdipSetInterpolationMode(graphics, InterpolationModeHighQualityBicubic);
        GdipSetPixelOffsetMode(graphics, PixelOffsetModeHighQuality);
        FRAME_FONTS.with(|fonts|fonts.borrow_mut().clear());
        CURRENT.set((dc, graphics));
        FRAME_PIXELS.set(
            (!bits.is_null() && stride >= width && width > 0 && height > 0).then_some(
                FramePixels {
                    bits: bits.cast(),
                    stride: stride as usize,
                    width,
                    height,
                },
            ),
        );
        CURRENT_CLIP.set(None);
        COMPLEX_CLIP.set(false);
        LIGHT.set(light);
        Some(Self { image, graphics })
    }
}
impl Drop for Frame {
    fn drop(&mut self) {
        unsafe {
            CURRENT.set((null_mut(), null_mut()));
            FRAME_PIXELS.set(None);
            GdipFlush(self.graphics, FlushIntentionSync);
            GdipDeleteGraphics(self.graphics);
            gpu_dispatch::GdipDisposeImage(self.image.cast());
        }
    }
}
unsafe fn graphics(dc: HDC) -> Option<*mut GpGraphics> {
    let (target, g) = CURRENT.get();
    if target != dc || g.is_null() {
        return None;
    }
    let mut clip: RECT = zeroed();
    let complex = GetClipBox(dc, &mut clip) == COMPLEXREGION;
    let key = (clip.left, clip.top, clip.right, clip.bottom);
    let was_complex = COMPLEX_CLIP.replace(complex);
    if complex {
        DC_REGIONS.with(|regions| {let regions=regions.borrow_mut();
            let has_region=GetClipRgn(dc,regions.scratch)==1;
            if has_region&&gpu::active(){let mut window:windows_sys::Win32::Foundation::POINT=zeroed();let mut viewport=zeroed();GetWindowOrgEx(dc,&mut window);GetViewportOrgEx(dc,&mut viewport);OffsetRgn(regions.scratch,window.x-viewport.x,window.y-viewport.y);}
            if has_region && (!was_complex||CURRENT_CLIP.get()!=Some(key)||EqualRgn(regions.current,regions.scratch)==0) {
                GdipSetClipHrgn(g,regions.scratch,CombineModeReplace);
                if gpu::active() {GPU_DC_CLIPS.with(|clips|*clips.borrow_mut()=gpu::region_rects(regions.scratch).unwrap_or_default());}
                CombineRgn(regions.current,regions.scratch,null_mut(),RGN_COPY);
            }
        });
        CURRENT_CLIP.set(Some(key));
    } else if was_complex || CURRENT_CLIP.get() != Some(key) {
        GPU_DC_CLIPS.with(|clips| {let mut clips=clips.borrow_mut();clips.clear();clips.push(clip);});
        GdipSetClipRectI(
            g,
            clip.left,
            clip.top,
            clip.right - clip.left,
            clip.bottom - clip.top,
            CombineModeReplace,
        );
        CURRENT_CLIP.set(Some(key));
    }
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
type SurfaceKey = (i32, i32, u32, u32, u32);
const SURFACE_CACHE_BYTES: usize = 16 * 1024 * 1024;
thread_local! {
    static RASTER_SURFACES: std::cell::RefCell<std::collections::VecDeque<(SurfaceKey, BlurredText)>> = const {std::cell::RefCell::new(std::collections::VecDeque::new())};
}

// Rounded panels and button borders retain their geometry while scrolling.
// Cache their antialiased pixels instead of rasterizing paths every frame.
unsafe fn raster_surface(
    g: *mut GpGraphics,
    r: RECT,
    color: u32,
    radius: f32,
    stroke: f32,
) -> bool {
    if gpu::active() && is_current_graphics(g) { return false; }
    let pad = if stroke > 0.0 {
        stroke.ceil() as i32 + 1
    } else {
        0
    };
    let width = r.right - r.left;
    let height = r.bottom - r.top;
    let bitmap_width = width.saturating_add(pad * 2);
    let bitmap_height = height.saturating_add(pad * 2);
    let bytes = (bitmap_width.max(0) as usize)
        .saturating_mul(bitmap_height.max(0) as usize)
        .saturating_mul(4);
    if width <= 0 || height <= 0 || bytes > SURFACE_CACHE_BYTES / 2 {
        return false;
    }
    let key = (width, height, color, radius.to_bits(), stroke.to_bits());
    RASTER_SURFACES.with(|cache| {
        let mut cache = cache.borrow_mut();
        let entry = if let Some(index) = cache.iter().position(|(old, _)| *old == key) {
            cache.remove(index).unwrap()
        } else {
            let mut pixels = vec![0u32; bytes / 4];
            let mut bitmap = null_mut();
            if GdipCreateBitmapFromScan0(
                bitmap_width,
                bitmap_height,
                bitmap_width * 4,
                0xe200b,
                pixels.as_mut_ptr().cast(),
                &mut bitmap,
            ) != 0
            {
                return false;
            }
            let mut canvas = null_mut();
            if GdipGetImageGraphicsContext(bitmap.cast(), &mut canvas) != 0 {
                gpu_dispatch::GdipDisposeImage(bitmap.cast());
                return false;
            }
            GdipSetSmoothingMode(canvas, SmoothingModeAntiAlias);
            GdipSetPixelOffsetMode(canvas, PixelOffsetModeHighQuality);
            let shape = path(
                RECT {
                    left: pad,
                    top: pad,
                    right: pad + width,
                    bottom: pad + height,
                },
                radius,
            );
            if stroke > 0.0 {
                let mut pen = null_mut();
                GdipCreatePen1(color, stroke, UnitPixel, &mut pen);
                gpu_dispatch::GdipDrawPath(canvas, pen, shape);
                GdipDeletePen(pen);
            } else {
                let mut brush = null_mut();
                GdipCreateSolidFill(color, &mut brush);
                gpu_dispatch::GdipFillPath(canvas, brush.cast(), shape);
                gpu_dispatch::GdipDeleteBrush(brush.cast());
            }
            gpu_dispatch::GdipDeletePath(shape);
            GdipDeleteGraphics(canvas);
            let mut resident: usize = cache.iter().map(|(_, image)| image.pixels.len() * 4).sum();
            while cache.len() >= 128 || resident + bytes > SURFACE_CACHE_BYTES {
                if let Some((_, old)) = cache.pop_front() {
                    resident = resident.saturating_sub(old.pixels.len() * 4);
                } else {
                    break;
                }
            }
            (key, BlurredText { bitmap, pixels })
        };
        if cfg!(debug_assertions)
            || !composite_raster(
                g,
                &entry.1.pixels,
                bitmap_width,
                bitmap_height,
                r.left - pad,
                r.top - pad,
            )
        {
            gpu_dispatch::GdipDrawImageRectI(
                g,
                entry.1.bitmap.cast(),
                r.left - pad,
                r.top - pad,
                bitmap_width,
                bitmap_height,
            );
        }
        cache.push_back(entry);
        true
    })
}

pub unsafe fn fill(dc: HDC, r: RECT, color: u32, radius: f32) -> bool {
    let color = surface_color(color);
    let Some(g) = graphics(dc) else {
        return false;
    };
    if RectVisible(dc, &r) == 0 {
        return true;
    }
    if r.right <= r.left || r.bottom <= r.top {
        return true;
    }
    if gpu::draw(g,|s|s.rounded(r,color,radius,None)) {return true;}
    if raster_surface(g, r, color, radius, 0.0) {
        return true;
    }
    let p = path(r, radius);
    let mut b = null_mut();
    GdipCreateSolidFill(color, &mut b);
    gpu_dispatch::GdipFillPath(g, b.cast(), p);
    gpu_dispatch::GdipDeleteBrush(b.cast());
    gpu_dispatch::GdipDeletePath(p);
    true
}
/// Keep scrollbar movement fractional instead of rounding it to layout pixels.
pub unsafe fn fill_offset_y(dc: HDC, r: RECT, color: u32, radius: f32, y: f32) -> bool {
    let Some(g) = graphics(dc) else { return false; };
    let color = surface_color(color);
    if gpu::draw(g, |s| s.rounded_offset(r, color, radius, None, y)) { return true; }
    let p = path(r, radius);
    let mut matrix = null_mut();
    GdipCreateMatrix2(1., 0., 0., 1., 0., y, &mut matrix);
    GdipTransformPath(p, matrix);
    GdipDeleteMatrix(matrix);
    let mut brush = null_mut();
    GdipCreateSolidFill(color, &mut brush);
    gpu_dispatch::GdipFillPath(g, brush.cast(), p);
    gpu_dispatch::GdipDeleteBrush(brush.cast());
    gpu_dispatch::GdipDeletePath(p);
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
    if gpu::draw(g,|s|s.rounded(r,color,radius,Some(width))) {return true;}
    if width > 0.0 && raster_surface(g, r, color, radius, width) {
        return true;
    }
    let p = path(r, radius);
    let mut pen = null_mut();
    GdipCreatePen1(color, width, UnitPixel, &mut pen);
    gpu_dispatch::GdipDrawPath(g, pen, p);
    GdipDeletePen(pen);
    gpu_dispatch::GdipDeletePath(p);
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
    gpu_dispatch::GdipCreateLineBrushFromRectWithAngle(
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
    gpu_dispatch::GdipDrawPath(g, pen, p);
    GdipDeletePen(pen);
    gpu_dispatch::GdipDeleteBrush(brush.cast());
    gpu_dispatch::GdipDeletePath(p);
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
    gpu_dispatch::GdipCreateLineBrushFromRectWithAngle(
        &bounds,
        top,
        bottom,
        angle,
        0,
        WrapModeTileFlipXY,
        &mut brush,
    );
    gpu_dispatch::GdipFillPath(g, brush.cast(), p);
    gpu_dispatch::GdipDeleteBrush(brush.cast());
    gpu_dispatch::GdipDeletePath(p);
    true
}
pub unsafe fn line(dc: HDC, x1: f32, y1: f32, x2: f32, y2: f32, color: u32, width: f32) -> bool {
    let Some(g) = graphics(dc) else {
        return false;
    };
    if gpu::draw(g,|s|s.line(x1,y1,x2,y2,color,width)) {return true;}
    let mut pen = null_mut();
    GdipCreatePen1(color, width, UnitPixel, &mut pen);
    gpu_dispatch::GdipDrawLine(g, pen, x1, y1, x2, y2);
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
    gpu_dispatch::GdipDrawCurve(g, pen, pts.as_ptr(), pts.len() as i32);
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
    gpu_dispatch::GdipFillPolygon(
        g,
        brush.cast(),
        pts.as_ptr(),
        pts.len() as i32,
        FillModeAlternate,
    );
    gpu_dispatch::GdipDeleteBrush(brush.cast());
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
    gpu_dispatch::GdipDrawEllipse(g, pen, x + w * 0.34, y + h * 0.22, w * 0.32, h * 0.32);
    gpu_dispatch::GdipDrawArc(
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
    let lf=font_properties(font);
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
        let mut cache = cache.borrow_mut();
        let index = cache.iter().position(|(k, _)| *k == key)?;
        let entry = cache.remove(index)?;
        let path = entry.1;
        cache.push_back(entry);
        Some(path)
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
                gpu_dispatch::GdipDeletePath(old);
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
    if RectVisible(dc, &r) == 0 {
        return true;
    }
    if value.is_empty() || r.right <= r.left || r.bottom <= r.top {
        return true;
    }
    // Reuse rasterized glyphs while scrolling instead of stroking and filling
    // the same vector paths at every new screen position.
    if !gpu::active() && raster_text(g, font, value, r, color, flags) {
        return true;
    }
    let Some(p) = cached_text_path(font, value, r, flags) else {
        return true;
    };
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
    if gpu::draw(g,|s|s.text(p,argb(color,255),r.right-r.left,r.bottom-r.top)) {
        GdipRestoreGraphics(g,state);return true;
    }
    let _ = LIGHT.get();
    let mut pen = null_mut();
    GdipCreatePen1(argb(color, 48), 0.3, UnitPixel, &mut pen);
    GdipSetPenLineJoin(pen, LineJoinRound);
    gpu_dispatch::GdipDrawPath(g, pen, p);
    GdipDeletePen(pen);
    let mut brush = null_mut();
    GdipCreateSolidFill(argb(color, 255), &mut brush);
    gpu_dispatch::GdipFillPath(g, brush.cast(), p);
    gpu_dispatch::GdipDeleteBrush(brush.cast());
    GdipRestoreGraphics(g, state);
    true
}
// A cached glyph-shaped shadow, never a rectangular lyric backpanel.
thread_local! {static LYRIC_SHADOWS:std::cell::RefCell<std::collections::VecDeque<(String,BlurredText)>>=const {std::cell::RefCell::new(std::collections::VecDeque::new())};}
fn lyric_cache_budget(default: usize, _name: &str, entry_bytes: usize) -> usize {
    #[cfg(test)]
    let default = std::env::var(_name)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(default);
    // Very large text needs enough entries to keep a visible lyric window hot.
    default
        .max(entry_bytes.saturating_mul(16))
        .min(default.saturating_mul(2))
}
pub unsafe fn lyric_shadow(dc: HDC, font: HFONT, value: &str, r: RECT, flags: u32) -> bool {
    let Some(g) = graphics(dc) else { return false };
    if gpu::active() {
        if let Some(p) = cached_text_path(font,value,r,flags | LYRICS_UNTRIMMED) {
            let mut state=0; GdipSaveGraphics(g,&mut state);
            GdipTranslateWorldTransform(g,r.left as f32,r.top as f32+1.,MatrixOrderPrepend);
            let drawn=gpu::draw(g,|s| s.blur(p,0xa0000000,2.,Some(3.)));
            GdipRestoreGraphics(g,state); if drawn { return true; }
        }
    }
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
                gpu_dispatch::GdipDisposeImage(bitmap.cast());
                return false;
            }
            GdipSetSmoothingMode(canvas, SmoothingModeAntiAlias);
            GdipSetPixelOffsetMode(canvas, PixelOffsetModeHighQuality);
            GdipTranslateWorldTransform(canvas, pad as f32, pad as f32 + 1., MatrixOrderPrepend);
            for (stroke, alpha) in [(8., 45), (3., 190)] {
                let mut pen = null_mut();
                GdipCreatePen1((alpha as u32) << 24, stroke, UnitPixel, &mut pen);
                GdipSetPenLineJoin(pen, LineJoinRound);
                gpu_dispatch::GdipDrawPath(canvas, pen, path);
                GdipDeletePen(pen);
            }
            GdipDeleteGraphics(canvas);
            while cache.len() >= 96
                || cache
                    .iter()
                    .map(|(_, image)| image.pixels.len() * 4)
                    .sum::<usize>()
                    + pixels.len() * 4
                    > lyric_cache_budget(
                        2 * 1024 * 1024,
                        "DEEZNATS_TEST_SHADOW_CACHE_BYTES",
                        pixels.len() * 4,
                    )
            {
                if cache.pop_front().is_none() {
                    break;
                }
            }
            (key, BlurredText { bitmap, pixels })
        };
        gpu_dispatch::GdipDrawImageRectI(
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
struct TextRaster {
    image: BlurredText,
    bounds: RECT,
}
thread_local! {static RASTER_LYRICS:std::cell::RefCell<std::collections::VecDeque<(String,TextRaster)>>=const {std::cell::RefCell::new(std::collections::VecDeque::new())};}

// Keep alignment and ellipsis based on the original layout rectangle, but
// retain and blend only the pixels occupied by glyphs.
fn ink_bounds(pixels: &[u32], width: usize, height: usize) -> Option<RECT> {
    let mut bounds = RECT {
        left: width as i32,
        top: height as i32,
        right: 0,
        bottom: 0,
    };
    for (y, row) in pixels.chunks_exact(width).take(height).enumerate() {
        if let Some(left) = row.iter().position(|pixel| pixel >> 24 != 0) {
            let right = row.iter().rposition(|pixel| pixel >> 24 != 0).unwrap() + 1;
            bounds.left = bounds.left.min(left as i32);
            bounds.right = bounds.right.max(right as i32);
            bounds.top = bounds.top.min(y as i32);
            bounds.bottom = y as i32 + 1;
        }
    }
    (bounds.right > bounds.left).then_some(bounds)
}
unsafe fn raster_text(
    g: *mut GpGraphics,
    font: HFONT,
    value: &str,
    r: RECT,
    color: u32,
    flags: u32,
) -> bool {
    let width = r.right - r.left;
    let height = r.bottom - r.top;
    if width <= 0 || height <= 0 || width as i64 * height as i64 > 2_000_000 {
        return false;
    }
    let lf=font_properties(font);
    let key = format!(
        "{:p}:{}:{}:{width}:{height}:{color}:{flags}:{value}",
        font, lf.lfHeight, lf.lfWeight
    );
    RASTER_LYRICS.with(|cache| {
        let mut cache = cache.borrow_mut();
        let entry = if let Some(index) = cache.iter().position(|(old, _)| *old == key) {
            cache.remove(index).unwrap()
        } else {
            let Some(path) = cached_text_path(font, value, r, flags) else {
                return true;
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
                gpu_dispatch::GdipDisposeImage(bitmap.cast());
                return false;
            }
            GdipSetSmoothingMode(canvas, SmoothingModeAntiAlias);
            GdipSetPixelOffsetMode(canvas, PixelOffsetModeHighQuality);
            let mut pen = null_mut();
            GdipCreatePen1(argb(color, 48), 0.3, UnitPixel, &mut pen);
            GdipSetPenLineJoin(pen, LineJoinRound);
            gpu_dispatch::GdipDrawPath(canvas, pen, path);
            GdipDeletePen(pen);
            let mut brush = null_mut();
            GdipCreateSolidFill(argb(color, 255), &mut brush);
            gpu_dispatch::GdipFillPath(canvas, brush.cast(), path);
            gpu_dispatch::GdipDeleteBrush(brush.cast());
            GdipDeleteGraphics(canvas);
            let ink = ink_bounds(&pixels, width as usize, height as usize).unwrap_or(RECT {
                left: 0,
                top: 0,
                right: 1,
                bottom: 1,
            });
            let cropped_width = ink.right - ink.left;
            let cropped_height = ink.bottom - ink.top;
            let mut cropped = Vec::with_capacity((cropped_width * cropped_height) as usize);
            for y in ink.top..ink.bottom {
                let start = (y * width + ink.left) as usize;
                cropped.extend_from_slice(&pixels[start..start + cropped_width as usize]);
            }
            gpu_dispatch::GdipDisposeImage(bitmap.cast());
            bitmap = null_mut();
            if GdipCreateBitmapFromScan0(
                cropped_width,
                cropped_height,
                cropped_width * 4,
                0xe200b,
                cropped.as_mut_ptr().cast(),
                &mut bitmap,
            ) != 0
            {
                return false;
            }
            let pixels = cropped;
            while cache.len() >= 512
                || cache
                    .iter()
                    .map(|(_, raster)| raster.image.pixels.len() * 4)
                    .sum::<usize>()
                    + pixels.len() * 4
                    > lyric_cache_budget(
                        4 * 1024 * 1024,
                        "DEEZNATS_TEST_TEXT_CACHE_BYTES",
                        pixels.len() * 4,
                    )
            {
                if cache.pop_front().is_none() {
                    break;
                }
            }
            (
                key,
                TextRaster {
                    image: BlurredText { bitmap, pixels },
                    bounds: ink,
                },
            )
        };
        let ink = entry.1.bounds;
        let width = ink.right - ink.left;
        let height = ink.bottom - ink.top;
        let x = r.left + ink.left;
        let y = r.top + ink.top;
        if cfg!(debug_assertions)
            || !composite_raster(g, &entry.1.image.pixels, width, height, x, y)
        {
            gpu_dispatch::GdipDrawImageRectI(g, entry.1.image.bitmap.cast(), x, y, width, height);
        }
        cache.push_back(entry);
        true
    })
}
impl Drop for BlurredText {
    fn drop(&mut self) {
        unsafe {
            gpu_dispatch::GdipDisposeImage(self.bitmap.cast());
        }
    }
}
thread_local! {
    static BLURRED_TEXT: std::cell::RefCell<std::collections::VecDeque<(String,BlurredText)>> = const {std::cell::RefCell::new(std::collections::VecDeque::new())};
    static LYRIC_WIDTHS: std::cell::RefCell<std::collections::VecDeque<(String,i32)>> = const {std::cell::RefCell::new(std::collections::VecDeque::new())};
    static LYRIC_CACHE_SCOPE: std::cell::RefCell<Option<(String, String)>> = const {std::cell::RefCell::new(None)};
}
pub(crate) fn lyric_raster_bytes() -> (usize, usize, usize) {
    let bytes = |items: &std::collections::VecDeque<(String, BlurredText)>| {
        items.iter().map(|(_, image)| image.pixels.len() * 4).sum()
    };
    (
        LYRIC_SHADOWS.with(|cache| bytes(&cache.borrow())),
        RASTER_LYRICS.with(|cache| {
            cache
                .borrow()
                .iter()
                .map(|(_, raster)| raster.image.pixels.len() * 4)
                .sum()
        }),
        BLURRED_TEXT.with(|cache| bytes(&cache.borrow())),
    )
}
pub(crate) fn clear_lyric_rasters() {
    LYRIC_CACHE_SCOPE.with(|scope| *scope.borrow_mut() = None);
    LYRIC_SHADOWS.with(|cache| cache.borrow_mut().clear());
    RASTER_LYRICS.with(|cache| cache.borrow_mut().clear());
    BLURRED_TEXT.with(|cache| cache.borrow_mut().clear());
    LYRIC_WIDTHS.with(|cache| cache.borrow_mut().clear());
}
pub(crate) fn set_lyric_cache_scope(scope: Option<(&str, &str)>) {
    let changed = LYRIC_CACHE_SCOPE.with(|current| {
        let mut current = current.borrow_mut();
        if current
            .as_ref()
            .map(|(provider, track)| (provider.as_str(), track.as_str()))
            == scope
        {
            return false;
        }
        *current = scope.map(|(provider, track)| (provider.to_owned(), track.to_owned()));
        true
    });
    if changed {
        LYRIC_SHADOWS.with(|cache| *cache.borrow_mut() = Default::default());
        RASTER_LYRICS.with(|cache| *cache.borrow_mut() = Default::default());
        BLURRED_TEXT.with(|cache| *cache.borrow_mut() = Default::default());
        LYRIC_WIDTHS.with(|cache| *cache.borrow_mut() = Default::default());
    }
}
pub unsafe fn lyric_text_width(dc: HDC, font: HFONT, value: &str) -> i32 {
    if value.is_empty() {
        return 0;
    }
    let lf=font_properties(font);
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
    if gpu::active() {
        if let Some(p) = cached_text_path(font,value,r,flags | LYRICS_UNTRIMMED) {
            let mut state=0; GdipSaveGraphics(g,&mut state);
            GdipTranslateWorldTransform(g,r.left as f32,r.top as f32,MatrixOrderPrepend);
            let drawn=gpu::draw(g,|s| s.blur(p,argb(color,255),blur_px.clamp(0.5,6.),None));
            GdipRestoreGraphics(g,state); if drawn { return true; }
        }
    }
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
    let lf=font_properties(font);
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
                gpu_dispatch::GdipDisposeImage(bitmap.cast());
                return false;
            }
            GdipSetSmoothingMode(mask_graphics, SmoothingModeAntiAlias);
            GdipTranslateWorldTransform(mask_graphics, pad as f32, pad as f32, MatrixOrderPrepend);
            let mut brush = null_mut();
            GdipCreateSolidFill(0xffffffff, &mut brush);
            gpu_dispatch::GdipFillPath(mask_graphics, brush.cast(), path);
            gpu_dispatch::GdipDeleteBrush(brush.cast());
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
                    > lyric_cache_budget(
                        2 * 1024 * 1024,
                        "DEEZNATS_TEST_BLUR_CACHE_BYTES",
                        pixels.len() * 4,
                    )
            {
                cache.pop_front();
            }
            cache.push_back((key, BlurredText { bitmap, pixels }));
        }
        let bitmap = cache.back().unwrap().1.bitmap;
        gpu_dispatch::GdipDrawImageRectI(g, bitmap.cast(), r.left - pad, r.top - pad, width, height) == 0
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
    let Some(g) = graphics(dc) else { return false };
    if value.is_empty() || r.right <= r.left || r.bottom <= r.top {
        return true;
    }
    let Some(p) = cached_text_path(font, value, r, flags | LYRICS_UNTRIMMED) else {
        return true;
    };
    let mut bounds = RectF {
        X: 0.0,
        Y: 0.0,
        Width: (r.right - r.left) as f32,
        Height: (r.bottom - r.top) as f32,
    };
    GdipGetPathWorldBounds(p, &mut bounds, null_mut(), null_mut());
    let phase = progress.clamp(0.0, 1.0);
    let lift = (phase * std::f32::consts::PI).sin() * (r.bottom - r.top) as f32 * 0.055;
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
    GdipTranslateWorldTransform(g, r.left as f32, r.top as f32 - lift, MatrixOrderPrepend);
    // Keep the normal lyric outline/opacity while each sung word lifts smoothly.
    let mut pen = null_mut();
    GdipCreatePen1(argb(unlit_color, 48), 0.3, UnitPixel, &mut pen);
    GdipSetPenLineJoin(pen, LineJoinRound);
    gpu_dispatch::GdipDrawPath(g, pen, p);
    GdipDeletePen(pen);
    let mut brush = null_mut();
    GdipCreateSolidFill(argb(unlit_color, 255), &mut brush);
    gpu_dispatch::GdipFillPath(g, brush.cast(), p);
    gpu_dispatch::GdipDeleteBrush(brush.cast());
    let eased = phase * phase * (3.0 - 2.0 * phase);
    if eased > 0.005 {
        let lit_right = (bounds.X + (bounds.Width + 6.0) * eased).ceil() as i32;
        GdipSetClipRectI(
            g,
            -8,
            -8,
            (lit_right + 14).max(1),
            r.bottom - r.top + 16,
            CombineModeIntersect,
        );
        if phase < 1.0 {
            let mut glow = null_mut();
            GdipCreatePen1(
                argb(
                    glow_color,
                    (70.0 * (phase * std::f32::consts::PI).sin()) as u32,
                ),
                3.0,
                UnitPixel,
                &mut glow,
            );
            GdipSetPenLineJoin(glow, LineJoinRound);
            gpu_dispatch::GdipDrawPath(g, glow, p);
            GdipDeletePen(glow);
        }
        let layers: &[(i32, u32)] = if phase >= 1.0 {
            &[(8, 255)]
        } else {
            &[(6, 45), (2, 95), (-3, 255)]
        };
        for &(feather, alpha) in layers {
            GdipSetClipRectI(
                g,
                -8,
                -8,
                (lit_right + 8 + feather).max(1),
                r.bottom - r.top + 16,
                CombineModeIntersect,
            );
            let mut lit = null_mut();
            GdipCreateSolidFill(argb(lit_color, alpha), &mut lit);
            gpu_dispatch::GdipFillPath(g, lit.cast(), p);
            gpu_dispatch::GdipDeleteBrush(lit.cast());
        }
    }
    GdipRestoreGraphics(g, state);
    true
}

// Each GDI+ bitmap has one owner. Workers finish drawing before transferring
// ownership through the channel; only the UI touches a completed bitmap.
struct PreparedArtwork(*mut GpBitmap);
unsafe impl Send for PreparedArtwork {}
impl Drop for PreparedArtwork {
    fn drop(&mut self) {
        unsafe {
            gpu_dispatch::GdipDisposeImage(self.0.cast());
        }
    }
}

struct ArtworkPreparation {
    requests: std::sync::mpsc::SyncSender<(ImageKey, crate::catalog::Bitmap)>,
    ready: std::sync::mpsc::Receiver<(ImageKey, Option<PreparedArtwork>)>,
    pending: std::collections::HashSet<ImageKey>,
}

type ArtworkJob = (ImageKey, crate::catalog::Bitmap);
type ArtworkPreparer = std::sync::Arc<
    dyn Fn(ImageKey, crate::catalog::Bitmap) -> Option<PreparedArtwork> + Send + Sync,
>;

fn spawn_artwork_workers(
    work: std::sync::mpsc::Receiver<ArtworkJob>,
    finished: std::sync::mpsc::SyncSender<(ImageKey, Option<PreparedArtwork>)>,
    count: usize,
    prepare: ArtworkPreparer,
) {
    let work = std::sync::Arc::new(std::sync::Mutex::new(work));
    for index in 0..count {
        let work = work.clone();
        let finished = finished.clone();
        let prepare = prepare.clone();
        std::thread::Builder::new()
            .name(format!("artwork-resize-{index}"))
            .spawn(move || {crate::catalog::artwork_background_priority();loop {
                let result = match work.lock() {
                    std::result::Result::Ok(receiver) => receiver.recv(),
                    Err(_) => return,
                };
                let std::result::Result::Ok((key, image)) = result else {
                    break;
                };
                let bitmap = prepare(key, image);
                if finished.send((key, bitmap)).is_err() {
                    break;
                }
            }})
            .expect("start artwork preparation worker");
    }
}

pub(crate) fn progressive_artwork_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("DEEZNATS_LEGACY_ARTWORK").is_none())
}

impl ArtworkPreparation {
    fn new() -> Self {
        let (requests, work) =
            std::sync::mpsc::sync_channel::<(ImageKey, crate::catalog::Bitmap)>(8);
        let (finished, ready) = std::sync::mpsc::sync_channel(8);
        let prepare = std::sync::Arc::new(|key, image| unsafe {
            prepare_artwork(&image, key).map(PreparedArtwork)
        });
        spawn_artwork_workers(
            work,
            finished,
            if progressive_artwork_enabled() { 3 } else { 1 },
            prepare,
        );
        Self {
            requests,
            ready,
            pending: Default::default(),
        }
    }
}

thread_local! {
    static ARTWORK_PREPARATION: std::cell::RefCell<Option<ArtworkPreparation>> = const { std::cell::RefCell::new(None) };
}

fn cache_prepared_artwork(key: ImageKey, bitmap: *mut GpBitmap) {
    RESIZED_IMAGES.with(|cache| {
        let mut cache = cache.borrow_mut();
        let mut bytes: usize = cache.iter().map(|(key, _)| resized_bytes(*key)).sum();
        while cache.len() >= 512 || bytes + resized_bytes(key) > RESIZED_IMAGE_BYTES {
            if let Some((old_key, old)) = cache.pop_front() {
                bytes = bytes.saturating_sub(resized_bytes(old_key));
                unsafe {
                    gpu_dispatch::GdipDisposeImage(old.cast());
                }
            } else {
                break;
            }
        }
        cache.push_back((key, bitmap));
    });
}

// Bounded polling leaves time for input even when a catalog delivers many covers.
pub(crate) fn poll_prepared_artwork() -> bool {
    ARTWORK_PREPARATION.with(|state| {
        let mut state = state.borrow_mut();
        let Some(state) = state.as_mut() else {
            return false;
        };
        let mut changed = false;
        for _ in 0..4 {
            let std::result::Result::Ok((key, bitmap)) = state.ready.try_recv() else {
                break;
            };
            if state.pending.remove(&key) {
                if let Some(bitmap) = bitmap {
                    cache_prepared_artwork(key, bitmap.0);
                    std::mem::forget(bitmap);
                    changed = true;
                }
            }
        }
        changed
    })
}

pub(crate) fn artwork_preparation_pending() -> bool {
    ARTWORK_PREPARATION.with(|state| {
        state
            .borrow()
            .as_ref()
            .is_some_and(|state| !state.pending.is_empty())
    })
}

unsafe fn prepare_artwork(image: &crate::catalog::Bitmap, key: ImageKey) -> Option<*mut GpBitmap> {
    let (_, width, height, radius) = key;
    let radius = f32::from_bits(radius);
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
        return None;
    }
    if GdipCreateBitmapFromScan0(width, height, 0, 0xe200b, null(), &mut bitmap) != 0 {
        gpu_dispatch::GdipDisposeImage(source.cast());
        return None;
    }
    GdipGetImageGraphicsContext(bitmap.cast(), &mut resize);
    GdipGraphicsClear(resize, 0);
    let rounded = path(
        RECT {
            left: 0,
            top: 0,
            right: width,
            bottom: height,
        },
        radius,
    );
    GdipSetClipPath(resize, rounded, CombineModeIntersect);
    gpu_dispatch::GdipDeletePath(rounded);
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
    gpu_dispatch::GdipDisposeImage(source.cast());
    Some(bitmap)
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
    if gpu::draw(g, |s| s.image((image.id << 1) | 1, image.pixels.as_ptr().cast(), image.width,
        image.height, image.width * 4, r, radius.max(0.).min(((r.right-r.left).min(r.bottom-r.top)) as f32 / 2.), alpha)) {
        return true;
    }
    if r.right <= r.left || r.bottom <= r.top {
        return true;
    }
    let width = r.right - r.left;
    let height = r.bottom - r.top;
    let radius = radius.max(0.0).min(width.min(height) as f32 / 2.0);
    let key = (image.id, width, height, radius.to_bits());
    let cacheable = resized_bytes(key) <= RESIZED_IMAGE_BYTES;
    let cached = RESIZED_IMAGES.with(|cache| {
        let mut cache = cache.borrow_mut();
        let index = cache.iter().position(|(k, _)| *k == key)?;
        let entry = cache.remove(index)?;
        let bitmap = entry.1;
        cache.push_back(entry);
        Some(bitmap)
    });
    let (bitmap, source_width, source_height, direct) = if let Some(bitmap) = cached {
        (bitmap, width, height, false)
    } else {
        if cacheable {
            #[cfg(test)]
            if std::env::var_os("DEEZNATS_BENCH_SYNC_ARTWORK").is_some() {
                if let Some(bitmap) = prepare_artwork(image, key) {
                    cache_prepared_artwork(key, bitmap);
                    return self::image(dc, r, image, radius, alpha);
                }
                return true;
            }
            ARTWORK_PREPARATION.with(|state| {
                let mut state = state.borrow_mut();
                let state = state.get_or_insert_with(ArtworkPreparation::new);
                if !state.pending.contains(&key) && state.pending.len() < 8 {
                    if state.requests.try_send((key, image.clone())).is_ok() {
                        state.pending.insert(key);
                    }
                }
            });
            if !progressive_artwork_enabled() {
                return true;
            }
            // Display the decoded pixels immediately while the bounded worker
            // queue prepares the rounded, device-sized cached version.
            let mut source = null_mut();
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
            (source, image.width as i32, image.height as i32, true)
        } else {
            // Oversized, uncached one-off surfaces retain their existing behavior.
            let Some(bitmap) = prepare_artwork(image, key) else {
                return true;
            };
            (bitmap, width, height, false)
        }
    };
    let mut state = 0;
    GdipSaveGraphics(g, &mut state);
    if direct {
        let rounded = path(r, radius);
        GdipSetClipPath(g, rounded, CombineModeIntersect);
        gpu_dispatch::GdipDeletePath(rounded);
    }
    let mut attributes = null_mut();
    if alpha < 255 {
        GdipCreateImageAttributes(&mut attributes);
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
    GdipSetInterpolationMode(
        g,
        if direct {
            InterpolationModeBilinear
        } else {
            InterpolationModeNearestNeighbor
        },
    );
    if alpha == 255 && !direct {
        // The cached bitmap is already at device size with transparent corners.
        gpu_dispatch::GdipDrawImageRectI(g, bitmap.cast(), r.left, r.top, width, height);
    } else {
        GdipDrawImageRectRectI(
            g,
            bitmap.cast(),
            r.left,
            r.top,
            r.right - r.left,
            r.bottom - r.top,
            0,
            0,
            source_width,
            source_height,
            UnitPixel,
            attributes,
            0,
            null_mut(),
        );
        if !attributes.is_null() {
            GdipDisposeImageAttributes(attributes);
        }
    }
    GdipRestoreGraphics(g, state);

    if direct || !cacheable {
        gpu_dispatch::GdipDisposeImage(bitmap.cast());
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
    gpu_dispatch::GdipCreateLineBrushFromRectWithAngle(
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

    gpu_dispatch::GdipFillPath(g, brush.cast(), path);
    gpu_dispatch::GdipDeleteBrush(brush.cast());
    gpu_dispatch::GdipDeletePath(path);

    if glow != 0 {
        let mut glow_pen = null_mut();
        GdipCreatePen1(glow, stroke_width * 2.8, UnitPixel, &mut glow_pen);
        gpu_dispatch::GdipDrawCurve2(g, glow_pen, pts.as_ptr(), pts.len() as i32, 0.45);
        GdipDeletePen(glow_pen);
    }

    let mut pen = null_mut();
    GdipCreatePen1(stroke, stroke_width, UnitPixel, &mut pen);
    gpu_dispatch::GdipDrawCurve2(g, pen, pts.as_ptr(), pts.len() as i32, 0.45);
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
        gpu_dispatch::GdipCreateLineBrushFromRectWithAngle(
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

        gpu_dispatch::GdipFillPath(g, brush.cast(), path);
        gpu_dispatch::GdipDeleteBrush(brush.cast());
        gpu_dispatch::GdipDeletePath(path);

        // Glowing cap at the top of each active bar
        if glow != 0 {
            let mut cap_pen = null_mut();
            GdipCreatePen1(glow, 2.0, UnitPixel, &mut cap_pen);
            gpu_dispatch::GdipDrawLine(g, cap_pen, bx + radius, by, bx + bar_w - radius, by);
            GdipDeletePen(cap_pen);
        }
    }

    true
}

pub unsafe fn trap_nation_rectangle(
    dc: HDC,
    cover: RECT,
    bass_pump: f32,
    layers: &[Vec<f32>],
    accent: u32,
) -> bool {
    let Some(g) = graphics(dc) else {
        return false;
    };
    let width = (cover.right - cover.left) as f32;
    let height = (cover.bottom - cover.top) as f32;
    let perimeter = 2. * (width + height);
    for (layer, spikes) in layers.iter().enumerate().rev() {
        let points: Vec<PointF> = spikes
            .iter()
            .enumerate()
            .map(|(k, spike)| {
                let distance = k as f32 / spikes.len() as f32 * perimeter;
                let offset = spike + layer as f32 * 1.2 + bass_pump.clamp(0., 1.) * 8. + 2.;
                let (x, y) = if distance < width {
                    (distance, -offset)
                } else if distance < width + height {
                    (width + offset, distance - width)
                } else if distance < 2. * width + height {
                    (2. * width + height - distance, height + offset)
                } else {
                    (-offset, perimeter - distance)
                };
                PointF {
                    X: cover.left as f32 + x,
                    Y: cover.top as f32 + y,
                }
            })
            .collect();
        if points.len() < 3 {
            continue;
        }
        let mut path = null_mut();
        GdipCreatePath(FillModeAlternate, &mut path);
        GdipAddPathPolygon(path, points.as_ptr(), points.len() as i32);
        let mut pen = null_mut();
        GdipCreatePen1(
            argb(accent, if layer == 0 { 200 } else { 45 }),
            if layer == 0 { 2. } else { 1.2 },
            UnitPixel,
            &mut pen,
        );
        gpu_dispatch::GdipDrawPath(g, pen, path);
        GdipDeletePen(pen);
        gpu_dispatch::GdipDeletePath(path);
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
        gpu_dispatch::GdipFillPath(g, brush.cast(), halo);
        gpu_dispatch::GdipDeleteBrush(brush.cast());
        gpu_dispatch::GdipDeletePath(halo);
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
        gpu_dispatch::GdipDrawPath(g, pen, edge);
        GdipDeletePen(pen);
        gpu_dispatch::GdipDeletePath(edge);
    }
    let _ = glow_col;

    true
}

#[cfg(test)]
mod tests {
    #[test]
    fn cropped_text_raster_reconstructs_full_gdiplus_rendering() {
        unsafe {
            let dc = CreateCompatibleDC(null_mut());
            let mut output = vec![0u32; 240 * 100];
            let frame = Frame::begin(dc, output.as_mut_ptr().cast(), 240, 240, 100, false).unwrap();
            let (_, graphics) = CURRENT.get();
            let mut lf: LOGFONTW = zeroed();
            lf.lfHeight = -24;
            lf.lfWeight = 700;
            for (slot, ch) in lf.lfFaceName.iter_mut().zip("Segoe UI".encode_utf16()) {
                *slot = ch;
            }
            let font = CreateFontIndirectW(&lf);
            let cases = [
                ("Left text", DT_LEFT, 10, 8),
                ("Centered text", DT_CENTER, -30, 5),
                ("Right text", DT_RIGHT, 190, -12),
                (
                    "A wrapped line of text with several words",
                    DT_LEFT | DT_WORDBREAK,
                    -15,
                    70,
                ),
                (
                    "H\u{e9}llo \u{4e16}\u{754c} \u{1f3b5}",
                    DT_CENTER | DT_WORDBREAK,
                    20,
                    10,
                ),
                ("   ", DT_RIGHT, 0, 0),
            ];
            for (value, flags, x, y) in cases {
                let width = 180;
                let height = 80;
                let rect = RECT {
                    left: x,
                    top: y,
                    right: x + width,
                    bottom: y + height,
                };
                let color = 0x009a52d3;
                let mut reference = vec![0u32; (width * height) as usize];
                let mut bitmap = null_mut();
                assert_eq!(
                    GdipCreateBitmapFromScan0(
                        width,
                        height,
                        width * 4,
                        0xe200b,
                        reference.as_mut_ptr().cast(),
                        &mut bitmap
                    ),
                    0
                );
                let mut canvas = null_mut();
                assert_eq!(GdipGetImageGraphicsContext(bitmap.cast(), &mut canvas), 0);
                GdipSetSmoothingMode(canvas, SmoothingModeAntiAlias);
                GdipSetPixelOffsetMode(canvas, PixelOffsetModeHighQuality);
                let path = cached_text_path(font, value, rect, flags).unwrap();
                let mut pen = null_mut();
                GdipCreatePen1(argb(color, 48), 0.3, UnitPixel, &mut pen);
                GdipSetPenLineJoin(pen, LineJoinRound);
                gpu_dispatch::GdipDrawPath(canvas, pen, path);
                GdipDeletePen(pen);
                let mut brush = null_mut();
                GdipCreateSolidFill(argb(color, 255), &mut brush);
                gpu_dispatch::GdipFillPath(canvas, brush.cast(), path);
                gpu_dispatch::GdipDeleteBrush(brush.cast());
                GdipDeleteGraphics(canvas);
                gpu_dispatch::GdipDisposeImage(bitmap.cast());

                RASTER_LYRICS.with(|cache| cache.borrow_mut().clear());
                assert!(raster_text(graphics, font, value, rect, color, flags));
                RASTER_LYRICS.with(|cache| {
                    let cache = cache.borrow();
                    let raster = &cache.back().unwrap().1;
                    let bounds = raster.bounds;
                    let crop_width = (bounds.right - bounds.left) as usize;
                    let mut reconstructed = vec![0u32; reference.len()];
                    for row in bounds.top..bounds.bottom {
                        let src = (row - bounds.top) as usize * crop_width;
                        let dst = row as usize * width as usize + bounds.left as usize;
                        reconstructed[dst..dst + crop_width]
                            .copy_from_slice(&raster.image.pixels[src..src + crop_width]);
                    }
                    assert_eq!(
                        reconstructed, reference,
                        "text {value:?}, flags {flags}, position ({x}, {y})"
                    );
                });
            }
            RASTER_LYRICS.with(|cache| cache.borrow_mut().clear());
            drop(frame);
            DeleteObject(font);
            DeleteDC(dc);
        }
    }

    #[test]
    fn lyric_budget_grows_for_large_entries_but_has_a_ceiling() {
        let base = 4 * 1024 * 1024;
        assert_eq!(
            super::lyric_cache_budget(base, "DEEZNATS_TEST_UNUSED_BUDGET", 1024),
            base
        );
        assert_eq!(
            super::lyric_cache_budget(base, "DEEZNATS_TEST_UNUSED_BUDGET", 384 * 1024),
            6 * 1024 * 1024
        );
        assert_eq!(
            super::lyric_cache_budget(base, "DEEZNATS_TEST_UNUSED_BUDGET", 2 * 1024 * 1024),
            2 * base
        );
    }

    unsafe fn prepared_test_image(
        dc: HDC,
        rect: RECT,
        source: &crate::catalog::Bitmap,
        radius: f32,
        alpha: u8,
    ) -> bool {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            poll_prepared_artwork();
            let result = image(dc, rect, source, radius, alpha);
            if !artwork_preparation_pending() {
                return result;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "artwork preparation timed out"
            );
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }
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
            set_lyric_cache_scope(Some(("Spotify", "track-a")));
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
            assert!(text(
                dc,
                font,
                "All your lies",
                rect,
                0xffffff,
                DT_LEFT | LYRICS_UNTRIMMED
            ));
            assert!(lyric_shadow(dc, font, "All your lies", rect, DT_LEFT));
            assert!(lyric_text_width(dc, font, "All your lies") > 0);
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
            set_lyric_cache_scope(Some(("Spotify", "track-a")));
            assert!(!BLURRED_TEXT.with(|c| c.borrow().is_empty()));
            set_lyric_cache_scope(Some(("Spotify", "track-b")));
            assert!(BLURRED_TEXT.with(|c| c.borrow().is_empty()));
            assert!(RASTER_LYRICS.with(|c| c.borrow().is_empty()));
            assert!(LYRIC_SHADOWS.with(|c| c.borrow().is_empty()));
            assert!(LYRIC_WIDTHS.with(|c| c.borrow().is_empty()));
            assert!(blurred_text(
                dc,
                font,
                "All your lies",
                rect,
                0xffffff,
                DT_LEFT,
                2.0
            ));
            set_lyric_cache_scope(None);
            assert!(BLURRED_TEXT.with(|c| c.borrow().is_empty()));
            drop(frame);
            DeleteObject(font);
            DeleteDC(dc);
        }
    }

    use super::*;
    #[test]
    fn spotify_logo_preserves_the_frame_for_following_draws() {
        unsafe {
            let dc = CreateCompatibleDC(null_mut());
            let bitmap = CreateBitmap(64, 64, 1, 32, null());
            let previous = SelectObject(dc, bitmap);
            let mut pixels = vec![0u32; 64 * 64];
            {
                let _frame = Frame::begin(dc, pixels.as_mut_ptr().cast(), 64, 64, 64, false).unwrap();
                spotify_logo(dc, RECT { left: 0, top: 0, right: 32, bottom: 32 });
                fill(dc, RECT { left: 40, top: 40, right: 60, bottom: 60 }, 0xffffffff, 0.0);
            }
            assert_ne!(pixels[50 * 64 + 50], 0, "drawing after the logo must remain visible");
            SelectObject(dc, previous);
            DeleteObject(bitmap);
            DeleteDC(dc);
        }
    }

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
                    gpu_dispatch::GdipDisposeImage(bitmap.cast());
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
                assert!(prepared_test_image(dc, rect, source, 0.0, 255));
                RESIZED_IMAGES.with(|cache| {
                    let bytes: usize = cache
                        .borrow()
                        .iter()
                        .map(|(key, _)| resized_bytes(*key))
                        .sum();
                    assert!(bytes <= RESIZED_IMAGE_BYTES);
                });
            }
            assert!(prepared_test_image(dc, rect, &sources[2], 0.0, 255));
            assert!(prepared_test_image(dc, rect, &sources[8], 0.0, 255));
            evict_resized_artwork(&std::collections::HashSet::from([sources[2].id]));
            RESIZED_IMAGES.with(|cache| {
                let mut cache = cache.borrow_mut();
                assert!(!cache.iter().any(|(key, _)| key.0 == sources[2].id));
                assert!(cache.iter().any(|(key, _)| key.0 == sources[8].id));
                assert!(!cache.iter().any(|(key, _)| key.0 == sources[3].id));
                for (_, bitmap) in cache.drain(..) {
                    gpu_dispatch::GdipDisposeImage(bitmap.cast());
                }
            });
            drop(frame);
            DeleteDC(dc);
        }
    }

    #[test]
    fn artwork_cache_retains_dense_lists_and_distinguishes_corner_shapes() {
        unsafe {
            let dc = CreateCompatibleDC(null_mut());
            let target = CreateBitmap(64, 64, 1, 32, null());
            let previous = SelectObject(dc, target);
            let mut pixels = vec![0u32; 64 * 64];
            let frame = Frame::begin(dc, pixels.as_mut_ptr().cast(), 64, 64, 64, false).unwrap();
            RESIZED_IMAGES.with(|cache| {
                for (_, bitmap) in cache.borrow_mut().drain(..) {
                    gpu_dispatch::GdipDisposeImage(bitmap.cast());
                }
            });
            let sources: Vec<_> = (0..80)
                .map(|_| crate::catalog::Bitmap::new(1, 1, vec![0xffffffff]))
                .collect();
            let rect = RECT {
                left: 0,
                top: 0,
                right: 48,
                bottom: 48,
            };
            for source in &sources {
                assert!(prepared_test_image(dc, rect, source, 8.0, 255));
            }
            let cached = || {
                RESIZED_IMAGES.with(|cache| {
                    cache
                        .borrow()
                        .iter()
                        .map(|(key, _)| *key)
                        .collect::<std::collections::HashSet<_>>()
                })
            };
            let first = cached();
            assert_eq!(first.len(), sources.len());
            for source in &sources {
                assert!(prepared_test_image(dc, rect, source, 8.0, 255));
            }
            assert_eq!(
                cached(),
                first,
                "scrolling a dense list must reuse resized covers"
            );

            // One source can appear as both an artist circle and square cover.
            let source = &sources[0];
            let mut corner_pixels = |radius, alpha| {
                GdipGraphicsClear(frame.graphics, 0);
                assert!(prepared_test_image(dc, rect, source, radius, alpha));
                GdipFlush(frame.graphics, FlushIntentionSync);
                (pixels[0], pixels[24 * 64 + 24])
            };
            let (corner, center) = corner_pixels(0.0, 255);
            assert_eq!(corner, 0xffffffff);
            assert_eq!(center, 0xffffffff);
            let (corner, center) = corner_pixels(24.0, 255);
            assert_eq!(corner, 0);
            assert_eq!(center, 0xffffffff);
            let (corner, center) = corner_pixels(24.0, 128);
            assert_eq!(corner, 0);
            assert!(
                (127..=129).contains(&(center >> 24)),
                "fade opacity changed: {center:#x}"
            );
            RESIZED_IMAGES.with(|cache| {
                for (_, bitmap) in cache.borrow_mut().drain(..) {
                    gpu_dispatch::GdipDisposeImage(bitmap.cast());
                }
            });
            drop(frame);
            SelectObject(dc, previous);
            DeleteObject(target);
            DeleteDC(dc);
        }
    }

    #[test]
    fn cached_surfaces_preserve_translucent_corners_borders_and_scroll_positions() {
        unsafe {
            let dc = CreateCompatibleDC(null_mut());
            let target = CreateBitmap(192, 80, 1, 32, null());
            let previous = SelectObject(dc, target);
            let mut pixels = vec![0xff123456u32; 192 * 80];
            let frame = Frame::begin(dc, pixels.as_mut_ptr().cast(), 192, 192, 80, false).unwrap();
            RASTER_SURFACES.with(|cache| cache.borrow_mut().clear());
            let _opacity = SurfaceOpacity::set(100);
            let color = 0xb4a060d0;
            let border = 0xd0ffffff;
            for left in [8, 104] {
                let rect = RECT {
                    left,
                    top: 8,
                    right: left + 64,
                    bottom: 64,
                };
                assert!(fill(dc, rect, color, 12.0));
                assert!(stroke(dc, rect, border, 12.0, 1.2));
            }
            GdipFlush(frame.graphics, FlushIntentionSync);
            assert_eq!(
                pixels[8 * 192 + 8],
                0xff123456,
                "rounded corner must retain the backdrop"
            );
            assert_ne!(
                pixels[32 * 192 + 32],
                0xff123456,
                "translucent panel did not render"
            );
            for y in 4..68 {
                for x in 4..76 {
                    assert_eq!(
                        pixels[y * 192 + x],
                        pixels[y * 192 + x + 96],
                        "moving a cached surface changed its pixels"
                    );
                }
            }
            RASTER_SURFACES.with(|cache| {
                let cache = cache.borrow();
                assert_eq!(
                    cache.len(),
                    2,
                    "fills and borders should each reuse their raster at a new position"
                );
                assert!(
                    cache
                        .iter()
                        .map(|(_, surface)| surface.pixels.len() * 4)
                        .sum::<usize>()
                        <= SURFACE_CACHE_BYTES
                );
            });
            drop(frame);
            SelectObject(dc, previous);
            DeleteObject(target);
            DeleteDC(dc);
        }
    }

    #[test]
    fn disjoint_clips_preserve_gaps_and_restore_rectangular_compositing() {
        unsafe {
            let dc = CreateCompatibleDC(null_mut());
            let target = CreateBitmap(32, 16, 1, 32, null());
            let previous = SelectObject(dc, target);
            let mut pixels = vec![0xff000000u32; 32 * 16];
            let frame = Frame::begin(dc, pixels.as_mut_ptr().cast(), 32, 32, 16, false).unwrap();
            let region = CreateRectRgn(0, 0, 8, 16);
            let other = CreateRectRgn(24, 0, 32, 16);
            CombineRgn(region, region, other, RGN_OR);
            SelectClipRgn(dc, region);
            let rect = RECT {
                left: 0,
                top: 0,
                right: 32,
                bottom: 16,
            };
            assert!(fill(dc, rect, 0xffffffff, 0.0));
            GdipFlush(frame.graphics, FlushIntentionSync);
            assert_ne!(pixels[8 * 32 + 4], 0xff000000);
            assert_eq!(pixels[8 * 32 + 16], 0xff000000);
            assert_ne!(pixels[8 * 32 + 28], 0xff000000);
            assert!(!composite_raster(
                graphics(dc).unwrap(),
                &[0xffffffff; 512],
                32,
                16,
                0,
                0
            ));
            SelectClipRgn(dc, null_mut());
            assert!(composite_raster(
                graphics(dc).unwrap(),
                &[0xffffffff; 512],
                32,
                16,
                0,
                0
            ));
            assert_eq!(pixels[8 * 32 + 16], 0xffffffff);
            drop(frame);
            DeleteObject(region);
            DeleteObject(other);
            SelectObject(dc, previous);
            DeleteObject(target);
            DeleteDC(dc);
        }
    }

    #[test]
    fn direct_compositing_respects_clipping_negative_origins_and_padded_stride() {
        unsafe {
            let dc = CreateCompatibleDC(null_mut());
            let target = CreateBitmap(4, 4, 1, 32, null());
            let previous = SelectObject(dc, target);
            let mut pixels = vec![0xff0000ffu32; 6 * 4];
            let frame = Frame::begin(dc, pixels.as_mut_ptr().cast(), 6, 4, 4, false).unwrap();
            IntersectClipRect(dc, 1, 1, 2, 3);
            let graphics = graphics(dc).unwrap();
            let source = vec![0x80800000; 4 * 4];
            assert!(composite_raster(graphics, &source, 4, 4, -1, -1));
            GdipFlush(graphics, FlushIntentionSync);
            for (index, &pixel) in pixels.iter().enumerate() {
                let expected = if index == 7 || index == 13 {
                    0xff80007f
                } else {
                    0xff0000ff
                };
                assert_eq!(
                    pixel, expected,
                    "compositing touched a clipped pixel or row padding at {index}"
                );
            }
            drop(frame);
            SelectObject(dc, previous);
            DeleteObject(target);
            DeleteDC(dc);
        }
    }

    #[test]
    fn artwork_workers_do_not_block_other_covers_behind_a_slow_resize() {
        use std::sync::{mpsc, Arc, Mutex};
        use std::time::Duration;
        let (requests, work) = mpsc::sync_channel(8);
        let (finished, ready) = mpsc::sync_channel(8);
        let (started, first_started) = mpsc::sync_channel(1);
        let (release, blocked) = mpsc::sync_channel(1);
        let blocked = Mutex::new(blocked);
        let prepare: ArtworkPreparer = Arc::new(move |key, _image| {
            if key.0 == 1 {
                let _ = started.send(());
                let _ = blocked.lock().unwrap().recv();
            }
            None
        });
        spawn_artwork_workers(work, finished, 3, prepare);
        let bitmap = crate::catalog::Bitmap::new(1, 1, vec![0xffffffff]);
        requests.send(((1, 1, 1, 0), bitmap.clone())).unwrap();
        first_started
            .recv_timeout(Duration::from_secs(2))
            .expect("first resize did not start");
        requests.send(((2, 1, 1, 0), bitmap)).unwrap();
        let completed = ready.recv_timeout(Duration::from_secs(2));
        let _ = release.send(());
        let (key, _) = completed.expect("later cover was blocked by an earlier resize");
        assert_eq!(key.0, 2);
    }

    #[test]
    fn artwork_cache_misses_queue_bounded_work_without_resizing_in_paint() {
        unsafe {
            let dc = CreateCompatibleDC(null_mut());
            let target = CreateBitmap(64, 64, 1, 32, null());
            let previous = SelectObject(dc, target);
            let mut pixels = vec![0u32; 64 * 64];
            let frame = Frame::begin(dc, pixels.as_mut_ptr().cast(), 64, 64, 64, false).unwrap();
            let sources: Vec<_> = (0..20)
                .map(|_| crate::catalog::Bitmap::new(192, 192, vec![0xffffffff; 192 * 192]))
                .collect();
            let rect = RECT {
                left: 0,
                top: 0,
                right: 64,
                bottom: 64,
            };
            assert!(image(dc, rect, &sources[0], 8.0, 255));
            GdipFlush(frame.graphics, FlushIntentionSync);
            assert_ne!(
                pixels[32 * 64 + 32] & 0x00ffffff,
                0,
                "a decoded cover must appear before resizing completes"
            );
            assert_eq!(
                pixels[0] & 0x00ffffff,
                0,
                "the immediate cover must preserve rounded corners"
            );
            pixels.fill(0);
            assert!(image(dc, rect, &sources[0], 8.0, 128));
            GdipFlush(frame.graphics, FlushIntentionSync);
            let alpha = pixels[32 * 64 + 32] >> 24;
            assert!(
                (120..=136).contains(&alpha),
                "immediate cover must preserve fade opacity, got {alpha}"
            );
            for source in &sources {
                assert!(image(dc, rect, source, 8.0, 255));
                assert!(image(dc, rect, source, 8.0, 255));
            }
            RESIZED_IMAGES.with(|cache| {
                assert!(
                    cache.borrow().is_empty(),
                    "paint must not resize cache misses"
                )
            });
            ARTWORK_PREPARATION.with(|state| {
                let state = state.borrow();
                assert_eq!(
                    state.as_ref().unwrap().pending.len(),
                    8,
                    "bound work and deduplicate repeated paints"
                );
            });
            assert!(artwork_preparation_pending());
            evict_resized_artwork(&sources.iter().map(|source| source.id).collect());
            assert!(!artwork_preparation_pending());
            // Release both channel ends; the worker discards its in-flight result.
            ARTWORK_PREPARATION.with(|state| {
                state.borrow_mut().take();
            });
            drop(frame);
            SelectObject(dc, previous);
            DeleteObject(target);
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
