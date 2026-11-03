use std::result::Result::Ok;
use windows_numerics::{Matrix3x2, Vector2};
// Direct3D rasterization and alpha-preserving DirectComposition presentation.
use std::{
    cell::RefCell,
    collections::VecDeque,
    ffi::c_void,
    time::{Duration, Instant},
};
use windows::{
    core::Interface,
    Win32::{
        Foundation::{HMODULE, HWND},
        Graphics::{
            Direct2D::{Common::*, *},
            Direct3D::*,
            Direct3D11::*,
            DirectComposition::*,
            Dxgi::{Common::*, *},
        },
    },
};
use windows_sys::Win32::{Foundation::RECT, Graphics::GdiPlus::*};
type Result<T> = windows::core::Result<T>;
pub(crate) const UPLOAD_WAKE: u32 = 0x8000 + 11;
#[path = "retained.rs"]
mod retained;
pub(crate) use retained::{
    configure_layers, expanded_clip, invalidate_layers, invalidate_main_layer,
    invalidate_queue_layer, is_recording, select_layer, Layer, LayerRequest, RetainedCanvas,
};
pub(super) unsafe fn region_rects(
    region: windows_sys::Win32::Graphics::Gdi::HRGN,
) -> Option<Vec<RECT>> {
    use windows_sys::Win32::Graphics::Gdi::{GetRegionData, RGNDATA, RGNDATAHEADER};
    let bytes = GetRegionData(region, 0, std::ptr::null_mut());
    if bytes < std::mem::size_of::<RGNDATAHEADER>() as u32 {
        return None;
    }
    let mut buffer = vec![0usize; (bytes as usize).div_ceil(std::mem::size_of::<usize>())];
    let data = buffer.as_mut_ptr().cast::<RGNDATA>();
    if GetRegionData(region, bytes, data) != bytes {
        return None;
    }
    let count = (*data).rdh.nCount as usize;
    if count > (bytes as usize - std::mem::size_of::<RGNDATAHEADER>()) / std::mem::size_of::<RECT>()
    {
        return None;
    }
    Some(
        std::slice::from_raw_parts(
            buffer
                .as_ptr()
                .cast::<u8>()
                .add(std::mem::size_of::<RGNDATAHEADER>())
                .cast::<RECT>(),
            count,
        )
        .to_vec(),
    )
}
pub(super) struct Surface {
    hwnd: HWND,
    width: i32,
    height: i32,
    factory: ID2D1Factory1,
    context: ID2D1DeviceContext,
    backing: ID2D1Bitmap1,
    output: ID2D1Bitmap1,
    swap: IDXGISwapChain1,
    composition: IDCompositionDevice,
    _target: IDCompositionTarget,
    _visual: IDCompositionVisual,
    base: ID2D1Bitmap,
    images: VecDeque<(u64, ID2D1Bitmap, usize)>,
    active: bool,
    failed: bool,
    geometries: VecDeque<(usize, ID2D1PathGeometry, usize)>,
    damage: Vec<RECT>,
    brush: ID2D1SolidColorBrush,
    stroke_style: ID2D1StrokeStyle,
    image_brush: ID2D1BitmapBrush,
    gdi_matrix: *mut Matrix,
    text_rasters: VecDeque<((usize, i32, i32), ID2D1Bitmap1, usize)>,
    current_clip: Option<D2D_RECT_F>,
    retained: retained::Retention,
    uploads: usize,
    upload_bytes: usize,
    deferred_uploads: bool,
    deferred_layers: u8,
    wake_posted: bool,
}
impl Drop for Surface {
    fn drop(&mut self) {
        unsafe {
            if !self.gdi_matrix.is_null() {
                GdipDeleteMatrix(self.gdi_matrix);
            }
            if self.active {
                let _ = self.context.EndDraw(None, None);
            }
            let _ = self._target.SetRoot(None::<&IDCompositionVisual>);
            let _ = self.composition.Commit();
        }
    }
}
thread_local! {
    static SURFACE: RefCell<Option<Surface>> = const { RefCell::new(None) };
    static RETRY: RefCell<Option<Instant>> = const { RefCell::new(None) };
}
pub(crate) fn hide() {
    SURFACE.with(|s| {
        s.borrow_mut().take();
    });
}
pub(super) fn active() -> bool {
    SURFACE.with(|s| s.borrow().as_ref().is_some_and(|s| s.active))
}
pub(crate) fn available() -> bool {
    SURFACE.with(|s| s.borrow().is_some())
}
fn retry_later() {
    RETRY.with(|r| *r.borrow_mut() = Some(Instant::now() + Duration::from_secs(5)));
}
fn identity() -> Matrix3x2 {
    Matrix3x2 {
        M11: 1.,
        M22: 1.,
        ..Default::default()
    }
}

unsafe fn create(hwnd: HWND, width: i32, height: i32, caption: i32) -> Result<Surface> {
    let mut device = None;
    D3D11CreateDevice(
        None,
        D3D_DRIVER_TYPE_HARDWARE,
        HMODULE::default(),
        D3D11_CREATE_DEVICE_BGRA_SUPPORT,
        None,
        D3D11_SDK_VERSION,
        Some(&mut device),
        None,
        None,
    )?;
    let device = device.unwrap();
    let dxgi: IDXGIDevice = device.cast()?;
    let factory: ID2D1Factory1 = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
    let context = factory
        .CreateDevice(&dxgi)?
        .CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)?;
    let dxgi_factory: IDXGIFactory2 = CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS(0))?;
    let swap = dxgi_factory.CreateSwapChainForComposition(
        &device,
        &DXGI_SWAP_CHAIN_DESC1 {
            Width: width as u32,
            Height: height as u32,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            BufferCount: 2,
            Scaling: DXGI_SCALING_STRETCH,
            SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
            AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED,
            ..Default::default()
        },
        None,
    )?;
    let buffer: IDXGISurface = swap.GetBuffer(0)?;
    let props = D2D1_BITMAP_PROPERTIES1 {
        pixelFormat: D2D1_PIXEL_FORMAT {
            format: DXGI_FORMAT_B8G8R8A8_UNORM,
            alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
        },
        dpiX: 96.,
        dpiY: 96.,
        bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET,
        ..Default::default()
    };
    let output_props = D2D1_BITMAP_PROPERTIES1 {
        bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
        ..props.clone()
    };
    let output = context.CreateBitmapFromDxgiSurface(&buffer, Some(&output_props))?;
    let size = D2D_SIZE_U {
        width: width as u32,
        height: height as u32,
    };
    let backing = context.CreateBitmap(size, None, 0, &props)?;
    let base = ID2D1RenderTarget::CreateBitmap(
        &context,
        size,
        None,
        0,
        &D2D1_BITMAP_PROPERTIES {
            pixelFormat: props.pixelFormat,
            dpiX: 96.,
            dpiY: 96.,
        },
    )?;
    let composition: IDCompositionDevice = DCompositionCreateDevice(&dxgi)?;
    let target = composition.CreateTargetForHwnd(hwnd, false)?;
    let visual = composition.CreateVisual()?;
    visual.SetContent(&swap)?;
    visual.SetOffsetY2(caption as f32)?;
    target.SetRoot(&visual)?;
    composition.Commit()?;
    let brush = context.CreateSolidColorBrush(&D2D1_COLOR_F::default(), None)?;
    let stroke_style = ID2D1Factory::CreateStrokeStyle(
        &factory,
        &D2D1_STROKE_STYLE_PROPERTIES {
            startCap: D2D1_CAP_STYLE_ROUND,
            endCap: D2D1_CAP_STYLE_ROUND,
            lineJoin: D2D1_LINE_JOIN_ROUND,
            ..Default::default()
        },
        None,
    )?;
    let image_brush = ID2D1RenderTarget::CreateBitmapBrush(
        &context,
        None::<&ID2D1Bitmap>,
        Some(&D2D1_BITMAP_BRUSH_PROPERTIES {
            extendModeX: D2D1_EXTEND_MODE_CLAMP,
            extendModeY: D2D1_EXTEND_MODE_CLAMP,
            interpolationMode: D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
        }),
        None,
    )?;
    Ok(Surface {
        hwnd,
        width,
        height,
        factory,
        context,
        backing,
        output,
        swap,
        composition,
        _target: target,
        _visual: visual,
        base,
        images: VecDeque::new(),
        geometries: VecDeque::new(),
        damage: Vec::new(),
        brush,
        stroke_style,
        image_brush,
        gdi_matrix: std::ptr::null_mut(),
        text_rasters: VecDeque::new(),
        current_clip: None,
        retained: retained::Retention::default(),
        uploads: 0,
        upload_bytes: 0,
        deferred_uploads: false,
        deferred_layers: 0,
        wake_posted: false,
        active: false,
        failed: false,
    })
}

unsafe fn resize_surface(s: &mut Surface, width: i32, height: i32, caption: i32) -> Result<()> {
    let props = D2D1_BITMAP_PROPERTIES1 {
        pixelFormat: D2D1_PIXEL_FORMAT {
            format: DXGI_FORMAT_B8G8R8A8_UNORM,
            alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
        },
        dpiX: 96.,
        dpiY: 96.,
        bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET,
        ..Default::default()
    };
    let size = D2D_SIZE_U { width: width as u32, height: height as u32 };
    let backing = s.context.CreateBitmap(size, None, 0, &props)?;
    let base = ID2D1RenderTarget::CreateBitmap(&s.context, size, None, 0, &D2D1_BITMAP_PROPERTIES {
        pixelFormat: props.pixelFormat,
        dpiX: 96.,
        dpiY: 96.,
    })?;
    s.context.SetTarget(None::<&ID2D1Image>);
    s.output = backing.clone();
    s.swap.ResizeBuffers(0, width as u32, height as u32, DXGI_FORMAT_UNKNOWN, DXGI_SWAP_CHAIN_FLAG(0))?;
    let buffer: IDXGISurface = s.swap.GetBuffer(0)?;
    let output_props = D2D1_BITMAP_PROPERTIES1 {
        bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
        ..props
    };
    s.output = s.context.CreateBitmapFromDxgiSurface(&buffer, Some(&output_props))?;
    s.backing = backing;
    s.base = base;
    s.width = width;
    s.height = height;
    s._visual.SetOffsetY2(caption as f32)?;
    s.retained = retained::Retention::default();
    Ok(())
}
pub(crate) unsafe fn begin(
    hwnd: windows_sys::Win32::Foundation::HWND,
    bits: *const c_void,
    stride: i32,
    width: i32,
    height: i32,
    caption: i32,
    dirty: Option<RECT>,
) -> bool {
    begin_with_region(
        hwnd,
        bits,
        stride,
        width,
        height,
        caption,
        dirty,
        std::ptr::null_mut(),
    )
}
pub(crate) unsafe fn begin_with_region(
    hwnd: windows_sys::Win32::Foundation::HWND,
    bits: *const c_void,
    stride: i32,
    width: i32,
    height: i32,
    caption: i32,
    dirty: Option<RECT>,
    region: windows_sys::Win32::Graphics::Gdi::HRGN,
) -> bool {
    use windows_sys::Win32::Graphics::Gdi::{GetRegionData, RGNDATA, RGNDATAHEADER};
    let damage = if !region.is_null() && dirty.is_some() {
        let bytes = GetRegionData(region, 0, std::ptr::null_mut());
        if bytes < std::mem::size_of::<RGNDATAHEADER>() as u32 {
            return false;
        }
        let mut buffer = vec![0usize; (bytes as usize).div_ceil(std::mem::size_of::<usize>())];
        let data = buffer.as_mut_ptr().cast::<RGNDATA>();
        if GetRegionData(region, bytes, data) != bytes {
            return false;
        }
        let count = (*data).rdh.nCount as usize;
        if count
            > (bytes as usize - std::mem::size_of::<RGNDATAHEADER>()) / std::mem::size_of::<RECT>()
        {
            return false;
        }
        let bounds = dirty.unwrap();
        std::slice::from_raw_parts(
            buffer
                .as_ptr()
                .cast::<u8>()
                .add(std::mem::size_of::<RGNDATAHEADER>())
                .cast::<RECT>(),
            count,
        )
        .iter()
        .filter_map(|r| {
            let r = RECT {
                left: r.left.max(bounds.left),
                top: (r.top - caption).max(bounds.top),
                right: r.right.min(bounds.right),
                bottom: (r.bottom - caption).min(bounds.bottom),
            };
            (r.left < r.right && r.top < r.bottom).then_some(r)
        })
        .collect::<Vec<_>>()
    } else {
        vec![dirty.unwrap_or(RECT {
            left: 0,
            top: 0,
            right: width,
            bottom: height,
        })]
    };
    if (!bits.is_null() && stride < width) || width <= 0 || height <= 0 {
        return false;
    }
    if RETRY.with(|r| r.borrow().is_some_and(|t| Instant::now() < t)) {
        return false;
    }
    SURFACE.with(|slot| {
        let mut slot = slot.borrow_mut();
        let hwnd = HWND(hwnd);
        if slot.as_ref().is_some_and(|s| s.hwnd == hwnd && (s.width != width || s.height != height)) {
            if dirty.is_some() {
                return false;
            }
            if resize_surface(slot.as_mut().unwrap(), width, height, caption).is_err() {
                slot.take();
            }
        }
        if slot.as_ref().is_none_or(|s| s.hwnd != hwnd) {
            slot.take();
            if dirty.is_some() {
                return false;
            }
            *slot = match create(hwnd, width, height, caption) {
                Ok(surface) => Some(surface),
                Err(_error) => {
                    #[cfg(test)]
                    eprintln!("GPU initialization failed: {_error}");
                    None
                }
            };
        }
        let Some(s) = slot.as_mut() else {
            retry_later();
            return false;
        };
        let upload = dirty.map(|r| D2D_RECT_U {
            left: r.left.clamp(0, width) as u32,
            top: r.top.clamp(0, height) as u32,
            right: r.right.clamp(0, width) as u32,
            bottom: r.bottom.clamp(0, height) as u32,
        });
        let source = if bits.is_null() {
            bits
        } else {
            upload.map_or(bits, |r| {
                bits.cast::<u8>()
                    .add((r.top as usize * stride as usize + r.left as usize) * 4)
                    .cast()
            })
        };
        if !bits.is_null()
            && s.base
                .CopyFromMemory(
                    upload.as_ref().map(|r| r as *const _),
                    source,
                    stride as u32 * 4,
                )
                .is_err()
        {
            slot.take();
            retry_later();
            return false;
        }
        s.context.SetTarget(&s.backing);
        s.uploads = 0;
        s.upload_bytes = 0;
        s.deferred_layers = 0;
        s.context.SetTransform(&identity());
        s.context.BeginDraw();
        s.damage = damage;
        for r in &s.damage {
            let rect = D2D_RECT_F {
                left: r.left as f32,
                top: r.top as f32,
                right: r.right as f32,
                bottom: r.bottom as f32,
            };
            s.context
                .PushAxisAlignedClip(&rect, D2D1_ANTIALIAS_MODE_ALIASED);
            s.context.Clear(Some(&D2D1_COLOR_F::default()));
            if !bits.is_null() {
                ID2D1RenderTarget::DrawBitmap(
                    &s.context,
                    &s.base,
                    None,
                    1.,
                    D2D1_BITMAP_INTERPOLATION_MODE_NEAREST_NEIGHBOR,
                    None,
                );
            }
            s.context.PopAxisAlignedClip();
        }
        s.active = true;
        s.failed = false;
        true
    })
}
/// Background crop, transition, tint and brightness are GPU operations too.
pub(crate) unsafe fn background(
    image: Option<&crate::catalog::Bitmap>,
    alpha: u32,
    tint: u32,
    opacity: u32,
    brightness: u32,
    caption: i32,
    _dirty: Option<RECT>,
    solid: Option<u32>,
) {
    SURFACE.with(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(s) = slot.as_mut().filter(|s| s.active) else {
            return;
        };
        for r in s.damage.clone() {
            s.context.PushAxisAlignedClip(
                &D2D_RECT_F {
                    left: r.left as f32,
                    top: r.top as f32,
                    right: r.right as f32,
                    bottom: r.bottom as f32,
                },
                D2D1_ANTIALIAS_MODE_ALIASED,
            );
            let result = (|| -> Result<()> {
                if let Some(solid) = solid {
                    s.context.Clear(Some(&color(solid)));
                    return Ok(());
                }
                let gain = brightness as f32 / 100.;
                let tint = color(tint | 0xff000000);
                let base = D2D1_COLOR_F {
                    r: (tint.r * gain).min(1.),
                    g: (tint.g * gain).min(1.),
                    b: (tint.b * gain).min(1.),
                    a: 1.,
                };
                s.context.Clear(Some(&base));
                let Some(image) = image.filter(|i| i.width > 0 && i.height > 0) else {
                    return Ok(());
                };
                let scale = (s.width as f32 / image.width as f32)
                    .max((s.height + caption) as f32 / image.height as f32);
                let width = (image.width as f32 * scale).ceil() as i32;
                let height = (image.height as f32 * scale).ceil() as i32;
                let left = (s.width - width) / 2;
                let top = (s.height + caption - height) / 2 - caption;
                let list = s.context.CreateCommandList()?;
                s.context.SetTarget(&list);
                let result = s.image(
                    (image.id << 1) | 1,
                    image.pixels.as_ptr().cast(),
                    image.width,
                    image.height,
                    image.width * 4,
                    RECT {
                        left,
                        top,
                        right: left + width,
                        bottom: top + height,
                    },
                    0.,
                    255,
                );
                s.context.SetTarget(&s.backing);
                result?;
                list.Close()?;
                let effect = s.context.CreateEffect(&CLSID_D2D1ColorMatrix)?;
                effect.SetInput(0, &list, false);
                let weight = alpha.min(255) as f32 / 255. * (1. - opacity.min(255) as f32 / 255.);
                let mut matrix = [0f32; 20];
                matrix[0] = weight * gain;
                matrix[5] = weight * gain;
                matrix[10] = weight * gain;
                matrix[15] = 1.;
                matrix[16] = tint.r * (1. - weight) * gain;
                matrix[17] = tint.g * (1. - weight) * gain;
                matrix[18] = tint.b * (1. - weight) * gain;
                let bytes = std::slice::from_raw_parts(
                    matrix.as_ptr().cast::<u8>(),
                    std::mem::size_of_val(&matrix),
                );
                effect.SetValue(
                    D2D1_COLORMATRIX_PROP_COLOR_MATRIX.0 as u32,
                    D2D1_PROPERTY_TYPE_MATRIX_5X4,
                    bytes,
                )?;
                s.context.DrawImage(
                    &effect.GetOutput()?,
                    None,
                    None,
                    D2D1_INTERPOLATION_MODE_NEAREST_NEIGHBOR,
                    D2D1_COMPOSITE_MODE_SOURCE_OVER,
                );
                Ok(())
            })();
            if result.is_err() {
                s.failed = true;
            }
            s.context.PopAxisAlignedClip();
        }
    });
}
pub(crate) unsafe fn finish() -> bool {
    SURFACE.with(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(s) = slot.as_mut().filter(|s| s.active) else {
            return false;
        };
        s.active = false;
        let result = (|| -> Result<()> {
            s.composite_layers()?;
            s.context.EndDraw(None, None)?;
            if s.failed {
                return Err(windows::core::Error::empty());
            }
            s.context.SetTarget(&s.output);
            s.context.SetTransform(&identity());
            s.context.BeginDraw();
            s.context.Clear(Some(&D2D1_COLOR_F::default()));
            ID2D1RenderTarget::DrawBitmap(
                &s.context,
                &s.backing,
                None,
                1.,
                D2D1_BITMAP_INTERPOLATION_MODE_NEAREST_NEIGHBOR,
                None,
            );
            s.context.EndDraw(None, None)?;
            s.swap.Present(0, DXGI_PRESENT(0)).ok()?;
            if s.deferred_uploads {
                s.retained.invalidate_deferred(s.deferred_layers);
                if !s.wake_posted {
                    s.wake_posted = windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW(
                        s.hwnd.0,
                        UPLOAD_WAKE,
                        0,
                        0,
                    ) != 0;
                }
            }
            s.composition.Commit()
        })();
        if result.is_err() {
            slot.take();
            retry_later();
            return false;
        }
        true
    })
}
/// Read back only the native search editor's small background snapshot.
pub(crate) unsafe fn snapshot(rect: RECT) -> Option<Vec<u32>> {
    SURFACE.with(|slot| {
        let slot = slot.borrow();
        let s = slot.as_ref()?;
        if rect.left < 0 || rect.top < 0 || rect.right > s.width || rect.bottom > s.height {
            return None;
        }
        let (width, height) = (rect.right - rect.left, rect.bottom - rect.top);
        if width <= 0 || height <= 0 {
            return None;
        }
        let bitmap = s
            .context
            .CreateBitmap(
                D2D_SIZE_U {
                    width: width as u32,
                    height: height as u32,
                },
                None,
                0,
                &D2D1_BITMAP_PROPERTIES1 {
                    pixelFormat: D2D1_PIXEL_FORMAT {
                        format: DXGI_FORMAT_B8G8R8A8_UNORM,
                        alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                    },
                    dpiX: 96.,
                    dpiY: 96.,
                    bitmapOptions: D2D1_BITMAP_OPTIONS_CPU_READ | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
                    ..Default::default()
                },
            )
            .ok()?;
        bitmap
            .CopyFromBitmap(
                None,
                &s.backing,
                Some(&D2D_RECT_U {
                    left: rect.left as u32,
                    top: rect.top as u32,
                    right: rect.right as u32,
                    bottom: rect.bottom as u32,
                }),
            )
            .ok()?;
        let mapped = bitmap.Map(D2D1_MAP_OPTIONS_READ).ok()?;
        let mut pixels = Vec::with_capacity((width * height) as usize);
        for y in 0..height {
            pixels.extend_from_slice(std::slice::from_raw_parts(
                mapped
                    .bits
                    .add(y as usize * mapped.pitch as usize)
                    .cast::<u32>(),
                width as usize,
            ));
        }
        let _ = bitmap.Unmap();
        Some(pixels)
    })
}
pub(super) unsafe fn forget_image(key: u64) {
    let _ = SURFACE.try_with(|s| {
        if let Some(s) = s.borrow_mut().as_mut() {
            s.images.retain(|i| i.0 != key);
        }
    });
}
pub(super) fn forget_path(path: usize) {
    let _ = SURFACE.try_with(|s| {
        if let Some(s) = s.borrow_mut().as_mut() {
            s.geometries.retain(|g| g.0 != path);
            s.text_rasters.retain(|g| g.0 .0 != path);
        }
    });
}
pub(crate) fn abandon() {
    hide();
    retry_later();
}
pub(crate) fn take_deferred_uploads() -> bool {
    SURFACE.with(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(s) = slot.as_mut() else {
            return false;
        };
        s.wake_posted = false;
        std::mem::take(&mut s.deferred_uploads)
    })
}
/// Preserve the exact GDI+ clip (including holes) and transform for every draw.
pub(super) unsafe fn draw(
    g: *mut GpGraphics,
    operation: impl Fn(&mut Surface) -> Result<()>,
) -> bool {
    if !super::is_current_graphics(g) {
        return false;
    }
    super::GPU_DC_CLIPS.with(|dc_clips| {
        let dc_clips = dc_clips.borrow();
        SURFACE.with(|slot| {
            let mut slot = slot.borrow_mut();
            let Some(s) = slot.as_mut().filter(|s| s.active) else {
                return false;
            };
            if s.retained.reusing() {
                return true;
            }
            let result = (|| -> Result<()> {
                if s.gdi_matrix.is_null() {
                    GdipCreateMatrix(&mut s.gdi_matrix);
                }
                GdipGetWorldTransform(g, s.gdi_matrix);
                let mut v = [0f32; 6];
                GdipGetMatrixElements(s.gdi_matrix, v.as_mut_ptr());
                let transform = Matrix3x2 {
                    M11: v[0],
                    M12: v[1],
                    M21: v[2],
                    M22: v[3],
                    M31: v[4],
                    M32: v[5],
                };
                // Region scans round outward to pixels and can let text spill
                // beyond its layout rectangle. Use exact world bounds plus the
                // original Win32 rectangles for holes instead of scan conversion.
                let mut bounds = RectF::default();
                GdipGetClipBounds(g, &mut bounds);
                let scans = [bounds];
                let actual = 1;
                s.context.SetTransform(&transform);
                for r in scans.iter().take(actual.max(0) as usize) {
                    // GDI+ region scans may include a boundary pixel beyond an
                    // HRGN edge. Never allow a draw to modify retained pixels
                    // outside the actual Windows update region.
                    let corners = [
                        (r.X, r.Y),
                        (r.X + r.Width, r.Y),
                        (r.X, r.Y + r.Height),
                        (r.X + r.Width, r.Y + r.Height),
                    ]
                    .map(|(x, y)| (x * v[0] + y * v[2] + v[4], x * v[1] + y * v[3] + v[5]));
                    let left = corners.iter().map(|p| p.0).fold(f32::INFINITY, f32::min);
                    let top = corners.iter().map(|p| p.1).fold(f32::INFINITY, f32::min);
                    let right = corners
                        .iter()
                        .map(|p| p.0)
                        .fold(f32::NEG_INFINITY, f32::max);
                    let bottom = corners
                        .iter()
                        .map(|p| p.1)
                        .fold(f32::NEG_INFINITY, f32::max);
                    let recording = s.retained.recording_bounds();
                    for index in 0..if recording.is_some() {
                        1
                    } else {
                        s.damage.len()
                    } {
                        let damage = recording.unwrap_or_else(|| s.damage[index]);
                        for dc_clip in dc_clips.iter() {
                            let clip = D2D_RECT_F {
                                left: left.max(damage.left.max(dc_clip.left) as f32),
                                top: top.max(damage.top.max(dc_clip.top) as f32),
                                right: right.min(damage.right.min(dc_clip.right) as f32),
                                bottom: bottom.min(damage.bottom.min(dc_clip.bottom) as f32),
                            };
                            if clip.left >= clip.right || clip.top >= clip.bottom {
                                continue;
                            }
                            s.context.SetTransform(&identity());
                            s.context
                                .PushAxisAlignedClip(&clip, D2D1_ANTIALIAS_MODE_ALIASED);
                            s.context.SetTransform(&transform);
                            s.current_clip = Some(clip);
                            let result = operation(s);
                            s.context.PopAxisAlignedClip();
                            s.current_clip = None;
                            result?;
                        }
                    }
                }
                Ok(())
            })();
            if result.is_err() {
                s.failed = true;
            }
            true
        })
    })
}
fn color(c: u32) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: ((c >> 16) & 255) as f32 / 255.,
        g: ((c >> 8) & 255) as f32 / 255.,
        b: (c & 255) as f32 / 255.,
        a: (c >> 24) as f32 / 255.,
    }
}
impl Surface {
    pub(super) unsafe fn text(
        &mut self,
        path: *mut GpPath,
        value: u32,
        width: i32,
        height: i32,
    ) -> Result<()> {
        if width <= 0 || height <= 0 {
            return Ok(());
        }
        if width > 4096 || height > 1024 {
            self.solid(path, (value & 0xffffff) | 0x30000000, Some(0.3))?;
            return self.solid(path, value, None);
        }
        // Cache coverage, not color: hover/theme changes reuse the same mask.
        let key = (path as usize, width, height);
        let bitmap = if let Some(index) = self.text_rasters.iter().position(|entry| entry.0 == key)
        {
            let entry = self.text_rasters.remove(index).unwrap();
            let bitmap = entry.1.clone();
            self.text_rasters.push_back(entry);
            bitmap
        } else {
            let bitmap = self.context.CreateBitmap(
                D2D_SIZE_U {
                    width: width as u32,
                    height: height as u32,
                },
                None,
                0,
                &D2D1_BITMAP_PROPERTIES1 {
                    pixelFormat: D2D1_PIXEL_FORMAT {
                        format: DXGI_FORMAT_B8G8R8A8_UNORM,
                        alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                    },
                    dpiX: 96.,
                    dpiY: 96.,
                    bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET,
                    ..Default::default()
                },
            )?;
            let previous = self.context.GetTarget()?;
            let mut transform = Matrix3x2::default();
            self.context.GetTransform(&mut transform);
            if self.current_clip.is_some() {
                self.context.PopAxisAlignedClip();
            }
            self.context.SetTarget(&bitmap);
            self.context.SetTransform(&identity());
            self.context.Clear(Some(&D2D1_COLOR_F::default()));
            let result = (|| -> Result<()> {
                self.solid(path, 0x30ffffff, Some(0.3))?;
                self.solid(path, 0xffffffff, None)
            })();
            self.context.SetTarget(&previous);
            if let Some(clip) = self.current_clip {
                self.context
                    .PushAxisAlignedClip(&clip, D2D1_ANTIALIAS_MODE_ALIASED);
            }
            self.context.SetTransform(&transform);
            result?;
            let bytes = width as usize * height as usize * 4;
            while self.text_rasters.len() >= 256
                || self.text_rasters.iter().map(|e| e.2).sum::<usize>() + bytes > 16 * 1024 * 1024
            {
                if self.text_rasters.pop_front().is_none() {
                    break;
                }
            }
            self.text_rasters.push_back((key, bitmap.clone(), bytes));
            bitmap
        };
        self.brush.SetColor(&color(value));
        let mode = self.context.GetAntialiasMode();
        self.context.SetAntialiasMode(D2D1_ANTIALIAS_MODE_ALIASED);
        ID2D1RenderTarget::FillOpacityMask(
            &self.context,
            &bitmap,
            &self.brush,
            D2D1_OPACITY_MASK_CONTENT_GRAPHICS,
            None,
            None,
        );
        self.context.SetAntialiasMode(mode);
        Ok(())
    }
    pub(super) unsafe fn rounded(
        &mut self,
        r: RECT,
        value: u32,
        radius: f32,
        stroke: Option<f32>,
    ) -> Result<()> {
        self.rounded_offset(r, value, radius, stroke, 0.)
    }
    pub(super) unsafe fn rounded_offset(
        &mut self,
        r: RECT,
        value: u32,
        radius: f32,
        stroke: Option<f32>,
        y: f32,
    ) -> Result<()> {
        self.brush.SetColor(&color(value));
        let rounded = D2D1_ROUNDED_RECT {
            rect: D2D_RECT_F {
                left: r.left as f32,
                top: r.top as f32 + y,
                right: r.right as f32,
                bottom: r.bottom as f32 + y,
            },
            radiusX: radius
                .max(0.)
                .min((r.right - r.left).min(r.bottom - r.top) as f32 / 2.),
            radiusY: radius
                .max(0.)
                .min((r.right - r.left).min(r.bottom - r.top) as f32 / 2.),
        };
        if let Some(width) = stroke {
            self.context
                .DrawRoundedRectangle(&rounded, &self.brush, width, &self.stroke_style);
        } else {
            self.context.FillRoundedRectangle(&rounded, &self.brush);
        }
        Ok(())
    }
    pub(super) unsafe fn line(
        &mut self,
        x: f32,
        y: f32,
        u: f32,
        v: f32,
        value: u32,
        width: f32,
    ) -> Result<()> {
        self.brush.SetColor(&color(value));
        self.context.DrawLine(
            Vector2 { X: x, Y: y },
            Vector2 { X: u, Y: v },
            &self.brush,
            width,
            &self.stroke_style,
        );
        Ok(())
    }
    pub(super) unsafe fn blur(
        &mut self,
        path: *mut GpPath,
        value: u32,
        sigma: f32,
        stroke: Option<f32>,
    ) -> Result<()> {
        let list = self.context.CreateCommandList()?;
        let previous = self.context.GetTarget()?;
        let mut transform = Matrix3x2::default();
        self.context.GetTransform(&mut transform);
        self.context.SetTarget(&list);
        let result = self.solid(path, value, stroke);
        self.context.SetTarget(&previous);
        result?;
        list.Close()?;
        let effect = self.context.CreateEffect(&CLSID_D2D1GaussianBlur)?;
        effect.SetInput(0, &list, false);
        effect.SetValue(
            D2D1_GAUSSIANBLUR_PROP_STANDARD_DEVIATION.0 as u32,
            D2D1_PROPERTY_TYPE_FLOAT,
            &sigma.to_ne_bytes(),
        )?;
        self.context.SetTransform(&identity());
        self.context.DrawImage(
            &effect.GetOutput()?,
            None,
            None,
            D2D1_INTERPOLATION_MODE_LINEAR,
            D2D1_COMPOSITE_MODE_SOURCE_OVER,
        );
        self.context.SetTransform(&transform);
        Ok(())
    }
    pub(super) unsafe fn path(&mut self, path: *mut GpPath) -> Result<ID2D1PathGeometry> {
        if let Some(index) = self.geometries.iter().position(|g| g.0 == path as usize) {
            let entry = self.geometries.remove(index).unwrap();
            let geometry = entry.1.clone();
            self.geometries.push_back(entry);
            return Ok(geometry);
        }
        let mut count = 0;
        GdipGetPointCount(path, &mut count);
        let mut points = vec![PointF::default(); count.max(0) as usize];
        let mut types = vec![0u8; points.len()];
        GdipGetPathPoints(path, points.as_mut_ptr(), count);
        GdipGetPathTypes(path, types.as_mut_ptr(), count);
        let geometry = self.factory.CreatePathGeometry()?;
        let sink = geometry.Open()?;
        let mut mode = 0;
        GdipGetPathFillMode(path, &mut mode);
        sink.SetFillMode(if mode == FillModeAlternate {
            D2D1_FILL_MODE_ALTERNATE
        } else {
            D2D1_FILL_MODE_WINDING
        });
        let point = |i: usize| Vector2 {
            X: points[i].X,
            Y: points[i].Y,
        };
        let mut i = 0;
        let mut open = false;
        while i < points.len() {
            match types[i] & 7 {
                0 => {
                    if open {
                        sink.EndFigure(D2D1_FIGURE_END_OPEN);
                    }
                    sink.BeginFigure(point(i), D2D1_FIGURE_BEGIN_FILLED);
                    open = true;
                }
                3 if i + 2 < points.len() => {
                    sink.AddBezier(&D2D1_BEZIER_SEGMENT {
                        point1: point(i),
                        point2: point(i + 1),
                        point3: point(i + 2),
                    });
                    i += 2;
                }
                _ => sink.AddLine(point(i)),
            }
            if types[i] & 0x80 != 0 && open {
                sink.EndFigure(D2D1_FIGURE_END_CLOSED);
                open = false;
            }
            i += 1;
        }
        if open {
            sink.EndFigure(D2D1_FIGURE_END_OPEN);
        }
        sink.Close()?;
        let geometry: ID2D1PathGeometry = geometry.into();
        let bytes = points.len() * std::mem::size_of::<PointF>();
        while self.geometries.len() >= 512
            || self.geometries.iter().map(|g| g.2).sum::<usize>() + bytes > 16 * 1024 * 1024
        {
            if self.geometries.pop_front().is_none() {
                break;
            }
        }
        self.geometries
            .push_back((path as usize, geometry.clone(), bytes));
        Ok(geometry)
    }
    pub(super) unsafe fn solid(
        &mut self,
        path: *mut GpPath,
        value: u32,
        stroke: Option<f32>,
    ) -> Result<()> {
        let geometry = self.path(path)?;
        let brush = &self.brush;
        brush.SetColor(&color(value));
        if let Some(width) = stroke {
            self.context
                .DrawGeometry(&geometry, brush, width, &self.stroke_style);
        } else {
            self.context.FillGeometry(&geometry, brush, None);
        }
        Ok(())
    }
    pub(super) unsafe fn gradient(
        &mut self,
        path: *mut GpPath,
        colors: [u32; 2],
        r: RectF,
        angle: f32,
        stroke: Option<f32>,
    ) -> Result<()> {
        let geometry = self.path(path)?;
        let a = angle.to_radians();
        let (dx, dy) = (a.cos(), a.sin());
        let extent = (dx.abs() * r.Width + dy.abs() * r.Height) / 2.;
        let (x, y) = (r.X + r.Width / 2., r.Y + r.Height / 2.);
        let stops = ID2D1RenderTarget::CreateGradientStopCollection(
            &self.context,
            &[
                D2D1_GRADIENT_STOP {
                    position: 0.,
                    color: color(colors[0]),
                },
                D2D1_GRADIENT_STOP {
                    position: 1.,
                    color: color(colors[1]),
                },
            ],
            D2D1_GAMMA_2_2,
            D2D1_EXTEND_MODE_MIRROR,
        )?;
        let brush = self.context.CreateLinearGradientBrush(
            &D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
                startPoint: Vector2 {
                    X: x - dx * extent,
                    Y: y - dy * extent,
                },
                endPoint: Vector2 {
                    X: x + dx * extent,
                    Y: y + dy * extent,
                },
            },
            None,
            &stops,
        )?;
        if let Some(width) = stroke {
            self.context.DrawGeometry(&geometry, &brush, width, None);
        } else {
            self.context.FillGeometry(&geometry, &brush, None);
        }
        Ok(())
    }
    pub(super) unsafe fn image(
        &mut self,
        key: u64,
        pixels: *const c_void,
        width: u32,
        height: u32,
        stride: u32,
        r: RECT,
        radius: f32,
        alpha: u8,
    ) -> Result<()> {
        if width == 0 || height == 0 || r.right <= r.left || r.bottom <= r.top {
            return Ok(());
        }
        let bitmap = if let Some(i) = self.images.iter().position(|i| i.0 == key) {
            let entry = self.images.remove(i).unwrap();
            let bitmap = entry.1.clone();
            self.images.push_back(entry);
            bitmap
        } else {
            let bytes = width as usize * height as usize * 4;
            if self.uploads >= 2 || self.uploads > 0 && self.upload_bytes + bytes > 4 * 1024 * 1024
            {
                self.deferred_uploads = true;
                self.deferred_layers |= self.retained.layer_mask();
                // Decoding has finished, but GPU admission can still wait. Keep
                // a static tile visible instead of recording a blank image.
                self.rounded(
                    r,
                    ((alpha as u32 * 128 / 255) << 24) | 0x006b6b73,
                    radius,
                    None,
                )?;
                return Ok(());
            }
            let bitmap = ID2D1RenderTarget::CreateBitmap(
                &self.context,
                D2D_SIZE_U { width, height },
                Some(pixels),
                stride,
                &D2D1_BITMAP_PROPERTIES {
                    pixelFormat: D2D1_PIXEL_FORMAT {
                        format: DXGI_FORMAT_B8G8R8A8_UNORM,
                        alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                    },
                    dpiX: 96.,
                    dpiY: 96.,
                },
            )?;
            self.uploads += 1;
            self.upload_bytes += bytes;
            let bytes = width as usize * height as usize * 4;
            while self.images.len() >= 256
                || self.images.iter().map(|i| i.2).sum::<usize>() + bytes > 64 * 1024 * 1024
            {
                if self.images.pop_front().is_none() {
                    break;
                }
            }
            if bytes <= 64 * 1024 * 1024 {
                self.images.push_back((key, bitmap.clone(), bytes));
            }
            bitmap
        };
        let brush = &self.image_brush;
        brush.SetBitmap(&bitmap);
        brush.SetTransform(&Matrix3x2 {
            M11: (r.right - r.left) as f32 / width as f32,
            M22: (r.bottom - r.top) as f32 / height as f32,
            M31: r.left as f32,
            M32: r.top as f32,
            ..Default::default()
        });
        brush.SetOpacity(alpha as f32 / 255.);
        self.context.FillRoundedRectangle(
            &D2D1_ROUNDED_RECT {
                rect: D2D_RECT_F {
                    left: r.left as f32,
                    top: r.top as f32,
                    right: r.right as f32,
                    bottom: r.bottom as f32,
                },
                radiusX: radius,
                radiusY: radius,
            },
            brush,
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::{Graphics::Gdi::*, UI::WindowsAndMessaging::*};

    #[test]
    #[ignore = "Requires Windows hardware Direct3D and DirectComposition"]
    fn gpu_upload_budget_defers_misses_and_reuses_cached_artwork() {
        unsafe {
            let hwnd = CreateWindowExW(
                0,
                windows::core::w!("STATIC").as_ptr(),
                std::ptr::null(),
                WS_POPUP,
                0,
                0,
                64,
                64,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
            );
            assert!(!hwnd.is_null());
            let pixel = 0xff123456u32;
            let rect = RECT {
                left: 0,
                top: 0,
                right: 16,
                bottom: 16,
            };
            assert!(begin(hwnd, std::ptr::null(), 64, 64, 64, 0, None));
            SURFACE.with(|slot| {
                let mut slot = slot.borrow_mut();
                let surface = slot.as_mut().unwrap();
                for key in 1..=3 {
                    surface
                        .image(
                            key,
                            (&pixel as *const u32).cast(),
                            1,
                            1,
                            4,
                            if key == 3 {
                                RECT {
                                    left: 32,
                                    right: 48,
                                    ..rect
                                }
                            } else {
                                rect
                            },
                            0.,
                            255,
                        )
                        .unwrap();
                }
                assert_eq!(surface.uploads, 2);
                assert_eq!(surface.images.len(), 2);
            });
            assert!(finish());
            let placeholder = snapshot(RECT {
                left: 36,
                top: 4,
                right: 37,
                bottom: 5,
            })
            .unwrap()[0];
            assert!(
                placeholder >> 24 > 0,
                "upload-deferred artwork must have a visible placeholder"
            );
            assert_ne!(placeholder, pixel);
            assert!(take_deferred_uploads());
            assert!(begin(hwnd, std::ptr::null(), 64, 64, 64, 0, None));
            SURFACE.with(|slot| {
                let mut slot = slot.borrow_mut();
                let surface = slot.as_mut().unwrap();
                for key in 1..=3 {
                    surface
                        .image(
                            key,
                            (&pixel as *const u32).cast(),
                            1,
                            1,
                            4,
                            if key == 3 {
                                RECT {
                                    left: 32,
                                    right: 48,
                                    ..rect
                                }
                            } else {
                                rect
                            },
                            0.,
                            255,
                        )
                        .unwrap();
                }
                assert_eq!(surface.uploads, 1);
                assert_eq!(surface.images.len(), 3);
            });
            assert!(finish());
            assert!(!take_deferred_uploads());
            assert_eq!(
                snapshot(RECT {
                    left: 36,
                    top: 4,
                    right: 37,
                    bottom: 5
                })
                .unwrap()[0],
                pixel,
                "the admitted texture must replace the placeholder"
            );
            assert_eq!(
                snapshot(RECT {
                    left: 4,
                    top: 4,
                    right: 5,
                    bottom: 5
                })
                .unwrap()[0],
                pixel
            );
            SURFACE.with(|slot| *slot.borrow_mut() = None);
            DestroyWindow(hwnd);
        }
    }

    #[test]
    #[ignore = "Requires Windows hardware Direct3D and DirectComposition"]
    fn gpu_rasterizes_alpha_clips_text_artwork_and_blur_without_cpu_foreground() {
        unsafe {
            let hwnd = CreateWindowExW(
                0,
                windows::core::w!("STATIC").as_ptr(),
                std::ptr::null(),
                WS_POPUP,
                0,
                0,
                160,
                160,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
            );
            assert!(!hwnd.is_null());
            let dc = CreateCompatibleDC(std::ptr::null_mut());
            let dc_bitmap = CreateBitmap(160, 160, 1, 32, std::ptr::null());
            let old_bitmap = SelectObject(dc, dc_bitmap);
            let mut pixels = vec![0u32; 160 * 160];
            assert!(
                begin(hwnd, pixels.as_ptr().cast(), 160, 160, 160, 0, None),
                "hardware backend unavailable"
            );
            let frame =
                crate::drawing::Frame::begin(dc, pixels.as_mut_ptr().cast(), 160, 160, 160, false)
                    .unwrap();
            IntersectClipRect(dc, 0, 0, 160, 160);
            assert!(crate::drawing::fill(
                dc,
                RECT {
                    left: 8,
                    top: 8,
                    right: 48,
                    bottom: 48
                },
                0x80ff0000,
                0.
            ));
            let saved = SaveDC(dc);
            IntersectClipRect(dc, 64, 8, 80, 24);
            crate::drawing::fill(
                dc,
                RECT {
                    left: 56,
                    top: 0,
                    right: 96,
                    bottom: 40,
                },
                0xff00ff00,
                0.,
            );
            RestoreDC(dc, saved);
            let font = CreateFontW(
                -18,
                0,
                0,
                0,
                400,
                0,
                0,
                0,
                1,
                0,
                0,
                0,
                0,
                windows::core::w!("Segoe UI").as_ptr(),
            );
            crate::drawing::text(
                dc,
                font,
                "GPU",
                RECT {
                    left: 8,
                    top: 60,
                    right: 80,
                    bottom: 88,
                },
                0xffffff,
                0,
            );
            let masks = SURFACE.with(|s| s.borrow().as_ref().unwrap().text_rasters.len());
            crate::drawing::text(
                dc,
                font,
                "GPU",
                RECT {
                    left: 8,
                    top: 130,
                    right: 80,
                    bottom: 158,
                },
                0x00ff00,
                0,
            );
            assert_eq!(
                masks,
                SURFACE.with(|s| s.borrow().as_ref().unwrap().text_rasters.len()),
                "text color changes must reuse the GPU glyph mask"
            );
            crate::drawing::blurred_text(
                dc,
                font,
                "Blur",
                RECT {
                    left: 8,
                    top: 100,
                    right: 90,
                    bottom: 130,
                },
                0xffffff,
                0,
                2.,
            );
            crate::drawing::spotify_logo(
                dc,
                RECT {
                    left: 100,
                    top: 60,
                    right: 140,
                    bottom: 100,
                },
            );
            crate::drawing::fill_offset_y(
                dc,
                RECT {
                    left: 148,
                    top: 80,
                    right: 158,
                    bottom: 90,
                },
                0xffffffff,
                0.,
                0.5,
            );
            crate::drawing::line(dc, 20., 52., 40., 52., 0xffff0000, 4.);
            crate::drawing::line(dc, 60., 52., 80., 52., 0xff00ff00, 4.);
            crate::drawing::gradient(
                dc,
                RECT {
                    left: 100,
                    top: 8,
                    right: 140,
                    bottom: 48,
                },
                0xff0000ff,
                0xffffffff,
                4.,
            );
            let artwork = crate::catalog::Bitmap::new(2, 2, vec![0xff112233; 4]);
            crate::drawing::image(
                dc,
                RECT {
                    left: 100,
                    top: 110,
                    right: 140,
                    bottom: 150,
                },
                &artwork,
                0.,
                255,
            );
            assert!(
                pixels.iter().all(|p| *p == 0),
                "foreground must stay on the GPU"
            );
            drop(frame);
            assert!(finish(), "GPU drawing or composition failed");
            let rendered = snapshot(RECT {
                left: 0,
                top: 0,
                right: 160,
                bottom: 160,
            })
            .unwrap();
            let red = rendered[20 * 160 + 20];
            let fractional = rendered[80 * 160 + 152] >> 24;
            assert!(
                (120..=136).contains(&fractional),
                "fractional scrollbar coverage: {fractional}"
            );
            assert!(
                (red >> 24).abs_diff(128) <= 1 && ((red >> 16) & 255).abs_diff(128) <= 1,
                "{red:08x}"
            );
            assert_eq!(rendered[12 * 160 + 68], 0xff00ff00);
            assert_eq!(rendered[4 * 160 + 60], 0);
            assert_eq!(rendered[52 * 160 + 30], 0xffff0000);
            assert_eq!(
                rendered[52 * 160 + 70],
                0xff00ff00,
                "temporary path geometry must not be reused after deletion"
            );
            assert!(snapshot(RECT {
                left: 0,
                top: 60,
                right: 80,
                bottom: 88
            })
            .unwrap()
            .iter()
            .any(|p| *p != 0));
            assert!(
                snapshot(RECT {
                    left: 0,
                    top: 100,
                    right: 90,
                    bottom: 128
                })
                .unwrap()
                .iter()
                .any(|p| *p != 0),
                "GPU blur output missing"
            );
            assert!(
                snapshot(RECT {
                    left: 0,
                    top: 130,
                    right: 80,
                    bottom: 158
                })
                .unwrap()
                .iter()
                .any(|p| p & 0xff00ff == 0 && p & 0x00ff00 != 0),
                "cached glyph mask must accept a new color"
            );
            assert_eq!(rendered[120 * 160 + 120], 0xff112233);
            let dirty = RECT {
                left: 100,
                top: 110,
                right: 140,
                bottom: 150,
            };
            assert!(begin(
                hwnd,
                pixels.as_ptr().cast(),
                160,
                160,
                160,
                0,
                Some(dirty)
            ));
            let frame =
                crate::drawing::Frame::begin(dc, pixels.as_mut_ptr().cast(), 160, 160, 160, false)
                    .unwrap();
            let saved = SaveDC(dc);
            IntersectClipRect(dc, dirty.left, dirty.top, dirty.right, dirty.bottom);
            ExcludeClipRect(dc, 112, 122, 128, 138);
            crate::drawing::fill(dc, dirty, 0xffabcdef, 0.);
            RestoreDC(dc, saved);
            drop(frame);
            assert!(finish());
            let partial = snapshot(RECT {
                left: 0,
                top: 0,
                right: 160,
                bottom: 160,
            })
            .unwrap();
            assert_eq!(
                partial[20 * 160 + 20],
                red,
                "partial paint must preserve untouched pixels"
            );
            assert_eq!(partial[120 * 160 + 104], 0xffabcdef);
            assert_eq!(
                partial[128 * 160 + 120],
                0,
                "excluded clip must remain clear"
            );
            assert!(begin(hwnd, std::ptr::null(), 160, 160, 160, 0, None));
            background(Some(&artwork), 255, 0, 0, 100, 0, None, None);
            assert!(finish());
            assert_eq!(
                snapshot(RECT {
                    left: 0,
                    top: 0,
                    right: 1,
                    bottom: 1
                })
                .unwrap()[0],
                0xff112233
            );
            assert!(begin(hwnd, std::ptr::null(), 160, 160, 160, 0, None));
            background(Some(&artwork), 0, 0xff224466, 0, 100, 0, None, None);
            assert!(finish());
            assert_eq!(
                snapshot(RECT {
                    left: 0,
                    top: 0,
                    right: 1,
                    bottom: 1
                })
                .unwrap()[0],
                0xff224466
            );
            assert!(begin(hwnd, std::ptr::null(), 160, 160, 160, 0, None));
            SURFACE.with(|s| s.borrow_mut().as_mut().unwrap().failed = true);
            assert!(
                !finish(),
                "failed device frame must discard hardware resources"
            );
            assert!(snapshot(RECT {
                left: 0,
                top: 0,
                right: 1,
                bottom: 1
            })
            .is_none());
            assert!(
                !begin(hwnd, std::ptr::null(), 160, 160, 160, 0, None),
                "failed hardware must back off"
            );
            hide();
            DeleteObject(font);
            SelectObject(dc, old_bitmap);
            DeleteObject(dc_bitmap);
            DeleteDC(dc);
            DestroyWindow(hwnd);
        }
    }
}
