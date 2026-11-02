//! Keep GDI+ geometry/layout, dispatch raster operations to Direct2D.
#![allow(non_snake_case)]
use super::{gpu, *};
use windows_sys::Win32::Graphics::GdiPlus as gp;
pub(super) unsafe fn GdipDeletePath(path: *mut GpPath) -> Status {
    gpu::forget_path(path as usize);
    gp::GdipDeletePath(path)
}
thread_local! { static ANGLES: std::cell::RefCell<std::collections::HashMap<usize, f32>> = std::cell::RefCell::new(std::collections::HashMap::new()); }

pub(super) unsafe fn GdipCreateLineBrushFromRectWithAngle(
    r: *const RectF,
    a: u32,
    b: u32,
    angle: f32,
    scalable: i32,
    wrap: WrapMode,
    brush: *mut *mut GpLineGradient,
) -> Status {
    let status = gp::GdipCreateLineBrushFromRectWithAngle(r, a, b, angle, scalable, wrap, brush);
    if status == 0 {
        ANGLES.with(|m| m.borrow_mut().insert(*brush as usize, angle));
    }
    status
}
pub(super) unsafe fn GdipDeleteBrush(brush: *mut GpBrush) -> Status {
    let _ = ANGLES.try_with(|m| m.borrow_mut().remove(&(brush as usize)));
    gp::GdipDeleteBrush(brush)
}
unsafe fn painted(
    g: *mut GpGraphics,
    brush: *mut GpBrush,
    path: *mut GpPath,
    stroke: Option<f32>,
) -> bool {
    gpu::draw(g, |s| {
        let mut kind = 0;
        gp::GdipGetBrushType(brush, &mut kind);
        if kind == BrushTypeLinearGradient {
            let mut colors = [0u32; 2];
            let mut rect = RectF::default();
            gp::GdipGetLineColors(brush.cast(), colors.as_mut_ptr());
            gp::GdipGetLineRect(brush.cast(), &mut rect);
            let angle = ANGLES.with(|m| m.borrow().get(&(brush as usize)).copied().unwrap_or(90.));
            s.gradient(path, colors, rect, angle, stroke)
        } else {
            let mut color = 0;
            gp::GdipGetSolidFillColor(brush.cast(), &mut color);
            s.solid(path, color, stroke)
        }
    })
}
pub(super) unsafe fn GdipFillPath(
    g: *mut GpGraphics,
    brush: *mut GpBrush,
    path: *mut GpPath,
) -> Status {
    if painted(g, brush, path, None) {
        0
    } else {
        gp::GdipFillPath(g, brush, path)
    }
}
pub(super) unsafe fn GdipDrawPath(
    g: *mut GpGraphics,
    pen: *mut GpPen,
    path: *mut GpPath,
) -> Status {
    if gpu::active() && is_current_graphics(g) {
        let mut brush = null_mut();
        let mut width = 1.;
        gp::GdipGetPenWidth(pen, &mut width);
        gp::GdipGetPenBrushFill(pen, &mut brush);
        painted(g, brush, path, Some(width));
        gp::GdipDeleteBrush(brush);
        0
    } else {
        gp::GdipDrawPath(g, pen, path)
    }
}
unsafe fn shape(
    g: *mut GpGraphics,
    pen: *mut GpPen,
    brush: *mut GpBrush,
    build: impl FnOnce(*mut GpPath),
) -> bool {
    if !gpu::active() || !is_current_graphics(g) {
        return false;
    }
    let mut path = null_mut();
    gp::GdipCreatePath(FillModeAlternate, &mut path);
    build(path);
    if !pen.is_null() {
        GdipDrawPath(g, pen, path);
    } else {
        GdipFillPath(g, brush, path);
    }
    GdipDeletePath(path);
    true
}
pub(super) unsafe fn GdipFillEllipse(
    g: *mut GpGraphics,
    b: *mut GpBrush,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
) -> Status {
    if shape(g, null_mut(), b, |p| {
        gp::GdipAddPathEllipse(p, x, y, w, h);
    }) {
        0
    } else {
        gp::GdipFillEllipse(g, b, x, y, w, h)
    }
}
pub(super) unsafe fn GdipDrawEllipse(
    g: *mut GpGraphics,
    p: *mut GpPen,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
) -> Status {
    if shape(g, p, null_mut(), |p| {
        gp::GdipAddPathEllipse(p, x, y, w, h);
    }) {
        0
    } else {
        gp::GdipDrawEllipse(g, p, x, y, w, h)
    }
}
pub(super) unsafe fn GdipDrawArc(
    g: *mut GpGraphics,
    p: *mut GpPen,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    a: f32,
    b: f32,
) -> Status {
    if shape(g, p, null_mut(), |p| {
        gp::GdipAddPathArc(p, x, y, w, h, a, b);
    }) {
        0
    } else {
        gp::GdipDrawArc(g, p, x, y, w, h, a, b)
    }
}
pub(super) unsafe fn GdipDrawLine(
    g: *mut GpGraphics,
    p: *mut GpPen,
    x: f32,
    y: f32,
    u: f32,
    v: f32,
) -> Status {
    if shape(g, p, null_mut(), |p| {
        gp::GdipAddPathLine(p, x, y, u, v);
    }) {
        0
    } else {
        gp::GdipDrawLine(g, p, x, y, u, v)
    }
}
pub(super) unsafe fn GdipDrawBezier(
    g: *mut GpGraphics,
    p: *mut GpPen,
    x: f32,
    y: f32,
    a: f32,
    b: f32,
    c: f32,
    d: f32,
    u: f32,
    v: f32,
) -> Status {
    if shape(g, p, null_mut(), |p| {
        gp::GdipAddPathBezier(p, x, y, a, b, c, d, u, v);
    }) {
        0
    } else {
        gp::GdipDrawBezier(g, p, x, y, a, b, c, d, u, v)
    }
}
pub(super) unsafe fn GdipDrawCurve(
    g: *mut GpGraphics,
    p: *mut GpPen,
    points: *const PointF,
    count: i32,
) -> Status {
    GdipDrawCurve2(g, p, points, count, 0.5)
}
pub(super) unsafe fn GdipDrawCurve2(
    g: *mut GpGraphics,
    p: *mut GpPen,
    points: *const PointF,
    count: i32,
    tension: f32,
) -> Status {
    if shape(g, p, null_mut(), |p| {
        gp::GdipAddPathCurve2(p, points, count, tension);
    }) {
        0
    } else {
        gp::GdipDrawCurve2(g, p, points, count, tension)
    }
}
pub(super) unsafe fn GdipFillPolygon(
    g: *mut GpGraphics,
    b: *mut GpBrush,
    points: *const PointF,
    count: i32,
    mode: FillMode,
) -> Status {
    if shape(g, null_mut(), b, |p| {
        gp::GdipSetPathFillMode(p, mode);
        gp::GdipAddPathPolygon(p, points, count);
    }) {
        0
    } else {
        gp::GdipFillPolygon(g, b, points, count, mode)
    }
}
pub(super) unsafe fn GdipDrawImageRectI(
    g: *mut GpGraphics,
    image: *mut GpImage,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
) -> Status {
    if !gpu::active() || !is_current_graphics(g) {
        return gp::GdipDrawImageRectI(g, image, x, y, w, h);
    }
    let mut width = 0;
    let mut height = 0;
    gp::GdipGetImageWidth(image, &mut width);
    gp::GdipGetImageHeight(image, &mut height);
    let rect = Rect {
        X: 0,
        Y: 0,
        Width: width as i32,
        Height: height as i32,
    };
    let mut data = BitmapData::default();
    if gp::GdipBitmapLockBits(
        image.cast(),
        &rect,
        ImageLockModeRead as u32,
        0xe200b,
        &mut data,
    ) != 0
    {
        return 1;
    }
    gpu::draw(g, |s| {
        s.image(
            (image as u64) << 1,
            data.Scan0,
            width,
            height,
            data.Stride as u32,
            RECT {
                left: x,
                top: y,
                right: x + w,
                bottom: y + h,
            },
            0.,
            255,
        )
    });
    gp::GdipBitmapUnlockBits(image.cast(), &mut data);
    0
}
pub(super) unsafe fn GdipDisposeImage(image: *mut GpImage) -> Status {
    gpu::forget_image((image as u64) << 1);
    gp::GdipDisposeImage(image)
}
