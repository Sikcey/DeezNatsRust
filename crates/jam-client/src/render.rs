use crate::model::{Color, ImageData, Rect, BLACK};
use std::{cell::Cell, collections::HashMap, sync::Arc};
use windows::{
    core::{Interface, PCWSTR},
    Win32::{
        Foundation::HWND,
        Graphics::{
            Direct2D::{Common::*, *},
            DirectWrite::*,
            Dxgi::Common::*,
            Imaging::*,
        },
        System::Com::*,
    },
};
use windows_numerics::Vector2;

fn color(c: Color) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: c.0,
        g: c.1,
        b: c.2,
        a: c.3,
    }
}
fn rect(r: Rect) -> D2D_RECT_F {
    D2D_RECT_F {
        left: r.x,
        top: r.y,
        right: r.x + r.w,
        bottom: r.y + r.h,
    }
}
fn err(e: windows::core::Error) -> String {
    e.to_string()
}
pub struct Renderer {
    pub failed: Cell<bool>,
    factory: ID2D1Factory,
    target: ID2D1RenderTarget,
    hwnd_target: Option<ID2D1HwndRenderTarget>,
    wic: Option<IWICBitmap>,
    write: IDWriteFactory,
    brush: ID2D1SolidColorBrush,
    formats: HashMap<(u32, u32, bool), IDWriteTextFormat>,
    layouts: HashMap<(String, u32, u32, bool, u32, u32, bool), IDWriteTextLayout>,
    bitmaps: HashMap<String, (usize, ID2D1Bitmap, usize)>,
    background: Option<(String, u32, u32, u32, Arc<ImageData>)>,
    pub synchronous_lyrics: bool,
    glyph_revision: u64,
    pub width: f32,
    pub height: f32,
}
impl Renderer {
    pub unsafe fn window(
        hwnd: windows_sys::Win32::Foundation::HWND,
        w: u32,
        h: u32,
        dpi: f32,
    ) -> Result<Self, String> {
        let factory: ID2D1Factory =
            D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None).map_err(err)?;
        let target = factory
            .CreateHwndRenderTarget(
                &D2D1_RENDER_TARGET_PROPERTIES {
                    r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
                    pixelFormat: D2D1_PIXEL_FORMAT {
                        format: DXGI_FORMAT_B8G8R8A8_UNORM,
                        alphaMode: D2D1_ALPHA_MODE_IGNORE,
                    },
                    dpiX: dpi,
                    dpiY: dpi,
                    usage: D2D1_RENDER_TARGET_USAGE_NONE,
                    minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
                },
                &D2D1_HWND_RENDER_TARGET_PROPERTIES {
                    hwnd: HWND(hwnd),
                    pixelSize: D2D_SIZE_U {
                        width: w,
                        height: h,
                    },
                    presentOptions: D2D1_PRESENT_OPTIONS_NONE,
                },
            )
            .map_err(err)?;
        Self::new(
            factory,
            target.cast().map_err(err)?,
            Some(target),
            None,
            w as f32 * 96. / dpi,
            h as f32 * 96. / dpi,
        )
    }
    pub unsafe fn offscreen(w: u32, h: u32) -> Result<Self, String> {
        let factory: ID2D1Factory =
            D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None).map_err(err)?;
        let imaging: IWICImagingFactory =
            CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).map_err(err)?;
        let bitmap = imaging
            .CreateBitmap(w, h, &GUID_WICPixelFormat32bppPBGRA, WICBitmapCacheOnLoad)
            .map_err(err)?;
        let target = factory
            .CreateWicBitmapRenderTarget(
                &bitmap,
                &D2D1_RENDER_TARGET_PROPERTIES {
                    r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE,
                    pixelFormat: D2D1_PIXEL_FORMAT {
                        format: DXGI_FORMAT_B8G8R8A8_UNORM,
                        alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                    },
                    dpiX: 96.,
                    dpiY: 96.,
                    usage: D2D1_RENDER_TARGET_USAGE_NONE,
                    minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
                },
            )
            .map_err(err)?;
        Self::new(factory, target, None, Some(bitmap), w as f32, h as f32)
    }
    unsafe fn new(
        factory: ID2D1Factory,
        target: ID2D1RenderTarget,
        hwnd_target: Option<ID2D1HwndRenderTarget>,
        wic: Option<IWICBitmap>,
        width: f32,
        height: f32,
    ) -> Result<Self, String> {
        let write: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED).map_err(err)?;
        let brush = target
            .CreateSolidColorBrush(&color(BLACK), None)
            .map_err(err)?;
        target.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
        let synchronous_lyrics = hwnd_target.is_none();
        Ok(Self {
            failed: Cell::new(true),
            factory,
            target,
            hwnd_target,
            wic,
            write,
            brush,
            formats: HashMap::new(),
            layouts: HashMap::new(),
            bitmaps: HashMap::new(),
            background: None,
            synchronous_lyrics,
            glyph_revision: crate::glyphs::revision(),
            width,
            height,
        })
    }
    pub fn resize(&mut self, w: u32, h: u32, dpi: f32) {
        unsafe {
            if let Some(t) = &self.hwnd_target {
                if t.Resize(&D2D_SIZE_U {
                    width: w,
                    height: h,
                })
                .is_err()
                {
                    self.failed.set(true);
                }
                t.SetDpi(dpi, dpi);
            }
            self.width = w as f32 * 96. / dpi;
            self.height = h as f32 * 96. / dpi;
        }
    }
    pub fn begin(&mut self, c: Color) {
        unsafe {
            self.target.BeginDraw();
            self.target.Clear(Some(&color(c)));
        }
    }
    pub fn end(&self) -> Result<(), String> {
        let result = unsafe { self.target.EndDraw(None, None).map_err(err) };
        self.failed.set(result.is_err());
        result
    }
    fn ink(&self, c: Color) -> &ID2D1SolidColorBrush {
        unsafe {
            self.brush.SetColor(&color(c));
        }
        &self.brush
    }
    pub fn fill(&self, r: Rect, c: Color) {
        if r.w > 0. && r.h > 0. {
            unsafe {
                self.target.FillRectangle(&rect(r), self.ink(c));
            }
        }
    }
    pub fn round(&self, r: Rect, radius: f32, c: Color, border: Option<Color>) {
        let rr = D2D1_ROUNDED_RECT {
            rect: rect(r),
            radiusX: radius,
            radiusY: radius,
        };
        unsafe {
            self.target.FillRoundedRectangle(&rr, self.ink(c));
            if let Some(c) = border {
                self.target.DrawRoundedRectangle(&rr, self.ink(c), 1., None);
            }
        }
    }
    pub fn shadow(&self, r: Rect, radius: f32, c: Color, spread: f32) {
        for i in (1..=8).rev() {
            let s = spread * i as f32 / 8.;
            self.round(
                Rect::new(r.x - s, r.y - s + spread * 0.2, r.w + s * 2., r.h + s * 2.),
                radius + s,
                c.alpha(c.3 * 0.04),
                None,
            );
        }
    }
    pub fn line(&self, x1: f32, y1: f32, x2: f32, y2: f32, c: Color, width: f32) {
        unsafe {
            self.target.DrawLine(
                Vector2 { X: x1, Y: y1 },
                Vector2 { X: x2, Y: y2 },
                self.ink(c),
                width,
                None,
            )
        }
    }
    pub fn clip(&self, r: Rect) {
        unsafe {
            self.target
                .PushAxisAlignedClip(&rect(r), D2D1_ANTIALIAS_MODE_PER_PRIMITIVE)
        }
    }
    pub fn pop(&self) {
        unsafe { self.target.PopAxisAlignedClip() }
    }
    pub fn gradient(&self, r: Rect, top: Color, bottom: Color) {
        unsafe {
            if let Ok(stops) = self.target.CreateGradientStopCollection(
                &[
                    D2D1_GRADIENT_STOP {
                        position: 0.,
                        color: color(top),
                    },
                    D2D1_GRADIENT_STOP {
                        position: 1.,
                        color: color(bottom),
                    },
                ],
                D2D1_GAMMA_2_2,
                D2D1_EXTEND_MODE_CLAMP,
            ) {
                if let Ok(brush) = self.target.CreateLinearGradientBrush(
                    &D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
                        startPoint: Vector2 { X: r.x, Y: r.y },
                        endPoint: Vector2 {
                            X: r.x,
                            Y: r.bottom(),
                        },
                    },
                    None,
                    &stops,
                ) {
                    self.target.FillRectangle(&rect(r), &brush)
                }
            }
        }
    }
    fn layout(
        &mut self,
        s: &str,
        size: f32,
        weight: u32,
        icons: bool,
        w: f32,
        h: f32,
        wrap: bool,
    ) -> Option<IDWriteTextLayout> {
        let key = (
            s.to_owned(),
            size.to_bits(),
            weight,
            icons,
            (w.max(1.) * 4.) as u32,
            (h.max(1.) * 4.) as u32,
            wrap,
        );
        if let Some(l) = self.layouts.get(&key) {
            return Some(l.clone());
        }
        let fkey = (size.to_bits(), weight, icons);
        if !self.formats.contains_key(&fkey) {
            unsafe {
                let f = self
                    .write
                    .CreateTextFormat(
                        PCWSTR(
                            crate::model::wide(if icons {
                                "Segoe MDL2 Assets"
                            } else {
                                "Segoe UI"
                            })
                            .as_ptr(),
                        ),
                        None,
                        DWRITE_FONT_WEIGHT(weight as i32),
                        DWRITE_FONT_STYLE_NORMAL,
                        DWRITE_FONT_STRETCH_NORMAL,
                        size,
                        PCWSTR(crate::model::wide("en-us").as_ptr()),
                    )
                    .ok()?;
                self.formats.insert(fkey, f);
            }
        }
        unsafe {
            let u: Vec<_> = s.encode_utf16().collect();
            let l = self
                .write
                .CreateTextLayout(&u, &self.formats[&fkey], w.max(1.), h.max(1.))
                .ok()?;
            let _ = l.SetWordWrapping(if wrap {
                DWRITE_WORD_WRAPPING_WRAP
            } else {
                DWRITE_WORD_WRAPPING_NO_WRAP
            });
            if self.layouts.len() > 768 {
                self.layouts.clear()
            }
            self.layouts.insert(key, l.clone());
            Some(l)
        }
    }
    pub fn measure(&mut self, s: &str, size: f32, weight: u32, w: f32, wrap: bool) -> (f32, f32) {
        if let Some(l) = self.layout(s, size, weight, false, w, 10000., wrap) {
            let mut m = DWRITE_TEXT_METRICS::default();
            unsafe {
                let _ = l.GetMetrics(&mut m);
            }
            return (m.widthIncludingTrailingWhitespace, m.height);
        }
        (0., size * 1.4)
    }
    pub fn text(
        &mut self,
        s: &str,
        r: Rect,
        size: f32,
        weight: u32,
        c: Color,
        center: bool,
        wrap: bool,
    ) {
        self.draw_text(s, r, size, weight, c, center, wrap, false)
    }
    pub fn icon(&mut self, s: char, r: Rect, size: f32, c: Color) {
        self.draw_text(&s.to_string(), r, size, 400, c, true, false, true)
    }
    fn draw_text(
        &mut self,
        s: &str,
        r: Rect,
        size: f32,
        weight: u32,
        c: Color,
        center: bool,
        wrap: bool,
        icons: bool,
    ) {
        if s.is_empty() || r.w < 1. || r.h < 1. {
            return;
        }
        if let Some(l) = self.layout(s, size, weight, icons, r.w, r.h, wrap) {
            unsafe {
                let _ = l.SetTextAlignment(if center {
                    DWRITE_TEXT_ALIGNMENT_CENTER
                } else {
                    DWRITE_TEXT_ALIGNMENT_LEADING
                });
                let _ = l.SetParagraphAlignment(if wrap {
                    DWRITE_PARAGRAPH_ALIGNMENT_NEAR
                } else {
                    DWRITE_PARAGRAPH_ALIGNMENT_CENTER
                });
                self.target.DrawTextLayout(
                    Vector2 { X: r.x, Y: r.y },
                    &l,
                    self.ink(c),
                    D2D1_DRAW_TEXT_OPTIONS_CLIP,
                );
            }
        }
    }
    pub fn glyph_changed(&mut self) -> bool {
        let revision = crate::glyphs::revision();
        let changed = self.glyph_revision != revision;
        self.glyph_revision = revision;
        changed
    }
    pub(crate) fn rasterize_lyric(job: &crate::glyphs::Job) -> Result<Arc<ImageData>, String> {
        let s = job.text.as_str();
        let r = job.rect;
        let size = job.size;
        let c = job.color;
        let blur = job.blur;
        let pad = 56.;
        let width = (r.w + pad * 2.).ceil() as u32;
        let height = (r.h + pad * 2.).ceil() as u32;
        if width as u64 * height as u64 > 2_000_000 {
            return Err("Lyric mask exceeds the cache limit".into());
        }
        let mut temp = unsafe { Self::offscreen(width, height)? };
        temp.begin(Color(0., 0., 0., 0.));
        temp.text(
            s,
            Rect::new(pad, pad, r.w, r.h),
            size,
            800,
            crate::model::WHITE,
            false,
            true,
        );
        temp.end()?;
        let mut pixels = vec![0u8; (width * height * 4) as usize];
        unsafe {
            temp.wic
                .as_ref()
                .unwrap()
                .CopyPixels(std::ptr::null(), width * 4, &mut pixels)
                .map_err(err)?;
        }
        let original = image::GrayImage::from_raw(
            width,
            height,
            pixels.chunks_exact(4).map(|p| p[3]).collect(),
        )
        .ok_or("Invalid lyric mask")?;
        let glyph = if blur {
            image::imageops::blur(&original, 1.)
        } else {
            original
        };
        let near = image::imageops::blur(&glyph, 2.);
        let middle = image::imageops::blur(&glyph, 8.);
        let far = image::imageops::blur(&glyph, 16.);
        let mut rgba = Vec::with_capacity((width * height * 4) as usize);
        for y in 0..height {
            for x in 0..width {
                let t = glyph.get_pixel(x, y)[0] as f32 / 255.;
                let a = near.get_pixel(x, y.saturating_sub(1))[0] as f32 / 255.;
                let b = middle.get_pixel(x, y)[0] as f32 / 255. * 0.8;
                let d = far.get_pixel(x, y)[0] as f32 / 255. * 0.66;
                let shadow = 1. - (1. - a) * (1. - b) * (1. - d);
                let alpha = t + shadow * (1. - t);
                let ratio = if alpha > 0. { t / alpha } else { 0. };
                rgba.extend_from_slice(&[
                    (c.0 * ratio * 255.) as u8,
                    (c.1 * ratio * 255.) as u8,
                    (c.2 * ratio * 255.) as u8,
                    (alpha * 255.) as u8,
                ]);
            }
        }
        Ok(Arc::new(ImageData {
            width,
            height,
            rgba,
        }))
    }
    pub fn frosted_lyric(&mut self, s: &str, r: Rect, size: f32, c: Color, blur: bool) {
        let job = crate::glyphs::Job::new(s, r, size, c, blur);
        let key = job.key.clone();
        let pad = 56.;
        if let Some(mask) = crate::glyphs::lookup(job, self.synchronous_lyrics) {
            self.image(
                &key,
                &mask,
                Rect::new(r.x - pad, r.y - pad, mask.width as f32, mask.height as f32),
                0.,
                false,
                1.,
            );
        } else {
            for (dx, dy, a) in [(-1., 0., 0.3), (1., 0., 0.3), (0., 1., 0.8)] {
                self.text(
                    s,
                    Rect::new(r.x + dx, r.y + dy, r.w, r.h),
                    size,
                    800,
                    BLACK.alpha(a),
                    false,
                    true,
                );
            }
            self.text(s, r, size, 800, c, false, true);
        }
    }
    pub fn highlight_range(
        &mut self,
        s: &str,
        r: Rect,
        size: f32,
        start: u32,
        len: u32,
        fraction: f32,
    ) {
        if let Some(layout) = self.layout(s, size, 800, false, r.w, r.h, true) {
            unsafe {
                let mut count = 0;
                let _ = layout.HitTestTextRange(start, len, 0., 0., None, &mut count);
                if count == 0 || count > 1024 {
                    return;
                }
                let mut metrics = vec![DWRITE_HIT_TEST_METRICS::default(); count as usize];
                if layout
                    .HitTestTextRange(start, len, 0., 0., Some(&mut metrics), &mut count)
                    .is_err()
                {
                    return;
                }
                metrics.truncate(count as usize);
                let mut filled =
                    metrics.iter().map(|m| m.width).sum::<f32>() * fraction.clamp(0., 1.);
                for metric in metrics {
                    let rect = Rect::new(
                        r.x + metric.left,
                        r.y + metric.top,
                        metric.width,
                        metric.height,
                    );
                    self.clip(rect);
                    self.text(s, r, size, 800, Color::hex("#c5c5ca"), false, true);
                    self.pop();
                    if filled > 0. {
                        self.clip(Rect::new(rect.x, rect.y, filled.min(rect.w), rect.h));
                        self.text(s, r, size, 800, crate::model::WHITE, false, true);
                        self.pop();
                    }
                    filled -= rect.w;
                }
            }
        }
    }
    pub fn wave(&self, points: &[(f32, f32)], baseline: f32, accent: Color, alpha: f32) {
        if points.len() < 2 {
            return;
        }
        unsafe {
            let Ok(path) = self.factory.CreatePathGeometry() else {
                return;
            };
            let Ok(sink) = path.Open() else { return };
            sink.BeginFigure(
                Vector2 {
                    X: points[0].0,
                    Y: baseline,
                },
                D2D1_FIGURE_BEGIN_FILLED,
            );
            let mut vertices: Vec<_> = points
                .iter()
                .map(|&(x, y)| Vector2 { X: x, Y: y })
                .collect();
            vertices.push(Vector2 {
                X: points.last().unwrap().0,
                Y: baseline,
            });
            sink.AddLines(&vertices);
            sink.EndFigure(D2D1_FIGURE_END_CLOSED);
            if sink.Close().is_err() {
                return;
            }
            self.target
                .FillGeometry(&path, self.ink(accent.alpha(alpha * 0.1)), None);
            for pair in points.windows(2) {
                self.line(
                    pair[0].0,
                    pair[0].1,
                    pair[1].0,
                    pair[1].1,
                    accent.alpha(alpha),
                    1.2,
                );
            }
        }
    }
    pub fn background(&mut self, key: &str, data: &Arc<ImageData>, r: Rect, blur: f32) {
        if blur < 0.5 {
            self.image(key, data, r, 0., false, 1.);
            return;
        }
        let bucket = blur.round() as u32;
        let width = r.w.round() as u32;
        let height = r.h.round() as u32;
        if !self
            .background
            .as_ref()
            .is_some_and(|(k, b, w, h, _)| k == key && *b == bucket && *w == width && *h == height)
        {
            let sw = 256u32;
            let sh = (256. * r.h / r.w.max(1.)).round().clamp(1., 512.) as u32;
            if let Some(image) =
                image::RgbaImage::from_raw(data.width, data.height, data.rgba.clone())
            {
                let scaled = image::DynamicImage::ImageRgba8(image)
                    .resize_to_fill(sw, sh, image::imageops::FilterType::Triangle)
                    .into_rgba8();
                let sigma = (blur * 0.5 * sw as f32 / r.w.max(1.)).max(0.1);
                let blurred = image::imageops::blur(&scaled, sigma);
                self.background = Some((
                    key.into(),
                    bucket,
                    width,
                    height,
                    Arc::new(ImageData {
                        width: sw,
                        height: sh,
                        rgba: blurred.into_raw(),
                    }),
                ));
            }
        }
        if let Some((_, _, _, _, data)) = &self.background {
            let data = data.clone();
            self.image("::backdrop", &data, r, 0., false, 1.);
        }
    }
    pub fn image(
        &mut self,
        key: &str,
        data: &Arc<ImageData>,
        r: Rect,
        radius: f32,
        _blur: bool,
        opacity: f32,
    ) {
        let stamp = Arc::as_ptr(data) as usize;
        if !self.bitmaps.get(key).is_some_and(|(p, _, _)| *p == stamp) {
            unsafe {
                let properties = D2D1_BITMAP_PROPERTIES {
                    pixelFormat: D2D1_PIXEL_FORMAT {
                        format: DXGI_FORMAT_B8G8R8A8_UNORM,
                        alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                    },
                    dpiX: 96.,
                    dpiY: 96.,
                };
                let rgba: Vec<_> = data
                    .rgba
                    .chunks_exact(4)
                    .flat_map(|p| {
                        let a = p[3] as u16;
                        [
                            (p[2] as u16 * a / 255) as u8,
                            (p[1] as u16 * a / 255) as u8,
                            (p[0] as u16 * a / 255) as u8,
                            p[3],
                        ]
                    })
                    .collect();
                if let Ok(bitmap) = self.target.CreateBitmap(
                    D2D_SIZE_U {
                        width: data.width,
                        height: data.height,
                    },
                    Some(rgba.as_ptr().cast()),
                    data.width * 4,
                    &properties,
                ) {
                    if self.bitmaps.len() > 96
                        || self.bitmaps.values().map(|(_, _, n)| *n).sum::<usize>() + rgba.len()
                            > 24 * 1024 * 1024
                    {
                        self.bitmaps.clear();
                    }
                    self.bitmaps.insert(key.into(), (stamp, bitmap, rgba.len()));
                }
            }
        }
        if let Some((_, bitmap, _)) = self.bitmaps.get(key) {
            unsafe {
                let (size_w, size_h) = (data.width as f32, data.height as f32);
                let ratio = r.w / r.h.max(1.);
                let src = if size_w / size_h > ratio {
                    let width = size_h * ratio;
                    D2D_RECT_F {
                        left: (size_w - width) / 2.,
                        right: (size_w + width) / 2.,
                        top: 0.,
                        bottom: size_h,
                    }
                } else {
                    let height = size_w / ratio;
                    D2D_RECT_F {
                        left: 0.,
                        right: size_w,
                        top: (size_h - height) / 2.,
                        bottom: (size_h + height) / 2.,
                    }
                };
                if radius > 0. {
                    if let Ok(geo) =
                        self.factory
                            .CreateRoundedRectangleGeometry(&D2D1_ROUNDED_RECT {
                                rect: rect(r),
                                radiusX: radius,
                                radiusY: radius,
                            })
                    {
                        if let Ok(layer) = self.target.CreateLayer(None) {
                            let mut parameters = D2D1_LAYER_PARAMETERS {
                                contentBounds: rect(r),
                                geometricMask: std::mem::ManuallyDrop::new(Some(
                                    geo.cast().unwrap(),
                                )),
                                maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                                maskTransform: windows_numerics::Matrix3x2 {
                                    M11: 1.,
                                    M12: 0.,
                                    M21: 0.,
                                    M22: 1.,
                                    M31: 0.,
                                    M32: 0.,
                                },
                                opacity: 1.,
                                opacityBrush: std::mem::ManuallyDrop::new(None),
                                layerOptions: D2D1_LAYER_OPTIONS_NONE,
                            };
                            self.target.PushLayer(&parameters, &layer);
                            self.target.DrawBitmap(
                                bitmap,
                                Some(&rect(r)),
                                opacity,
                                D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
                                Some(&src),
                            );
                            self.target.PopLayer();
                            std::mem::ManuallyDrop::drop(&mut parameters.geometricMask);
                            return;
                        }
                    }
                }
                self.target.DrawBitmap(
                    bitmap,
                    Some(&rect(r)),
                    opacity,
                    D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
                    Some(&src),
                );
            }
        }
    }
    pub fn save(&self, path: &std::path::Path) -> Result<(), String> {
        let bitmap = self.wic.as_ref().ok_or("Not an offscreen target")?;
        let w = self.width as u32;
        let h = self.height as u32;
        let mut bytes = vec![0u8; (w * h * 4) as usize];
        unsafe {
            bitmap
                .CopyPixels(std::ptr::null(), w * 4, &mut bytes)
                .map_err(err)?;
        }
        for p in bytes.chunks_exact_mut(4) {
            p.swap(0, 2);
            if p[3] > 0 && p[3] < 255 {
                let a = p[3] as u16;
                for c in &mut p[..3] {
                    *c = (*c as u16 * 255 / a).min(255) as u8
                }
            }
        }
        image::save_buffer(path, &bytes, w, h, image::ColorType::Rgba8).map_err(|e| e.to_string())
    }
}
