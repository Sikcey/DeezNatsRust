//! Retain independently clipped GPU paint lists across scroll presentation frames.
use super::*;
use windows_sys::Win32::Graphics::Gdi::*;
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Layer {
    #[default]
    Direct,
    Main,
    Queue,
    Overlay,
}
#[derive(Clone, Copy)]
pub(crate) struct LayerRequest {
    pub viewport: RECT,
    pub layout_offset: i32,
    pub shown_offset: f64,
    pub overscan: i32,
}
struct Saved {
    list: ID2D1CommandList,
    origin: i32,
    viewport: RECT,
}
#[derive(Default)]
struct Slot {
    saved: Option<Saved>,
    recording: Option<ID2D1CommandList>,
    request: Option<LayerRequest>,
    reuse: bool,
}
#[derive(Default)]
pub(super) struct Retention {
    main: Slot,
    queue: Slot,
    overlay: Option<ID2D1CommandList>,
    current: Layer,
    visual_key: u64,
}
impl Slot {
    unsafe fn configure(
        &mut self,
        context: &ID2D1DeviceContext,
        request: Option<LayerRequest>,
    ) -> Result<()> {
        self.request = request;
        self.recording = None;
        self.reuse = false;
        let Some(request) = request else {
            self.saved = None;
            return Ok(());
        };
        self.reuse = self.saved.as_ref().is_some_and(|s| {
            s.viewport.left == request.viewport.left
                && s.viewport.top == request.viewport.top
                && s.viewport.right == request.viewport.right
                && s.viewport.bottom == request.viewport.bottom
                && (request.shown_offset - s.origin as f64).abs()
                    < (request.overscan - 16).max(1) as f64
        });
        if !self.reuse {
            self.saved = None;
            self.recording = Some(context.CreateCommandList()?);
        }
        Ok(())
    }
    fn recording_bounds(&self) -> Option<RECT> {
        self.recording.as_ref()?;
        let request = self.request?;
        Some(RECT {
            top: request.viewport.top - request.overscan,
            bottom: request.viewport.bottom + request.overscan,
            ..request.viewport
        })
    }
    unsafe fn finish(&mut self) -> Result<()> {
        if let Some(list) = self.recording.take() {
            list.Close()?;
            let request = self.request.unwrap();
            self.saved = Some(Saved {
                list,
                origin: request.layout_offset,
                viewport: request.viewport,
            });
        }
        Ok(())
    }
}
impl Retention {
    pub(super) fn layer_mask(&self) -> u8 {
        match self.current {
            Layer::Main => 1,
            Layer::Queue => 2,
            _ => 0,
        }
    }
    pub(super) fn invalidate_deferred(&mut self, mask: u8) {
        if mask & 1 != 0 {
            self.main.saved = None;
        }
        if mask & 2 != 0 {
            self.queue.saved = None;
        }
    }
    pub(super) fn recording_bounds(&self) -> Option<RECT> {
        match self.current {
            Layer::Main => self.main.recording_bounds(),
            Layer::Queue => self.queue.recording_bounds(),
            _ => None,
        }
    }
    pub(super) fn reusing(&self) -> bool {
        match self.current {
            Layer::Main => self.main.reuse,
            Layer::Queue => self.queue.reuse,
            _ => false,
        }
    }
}
pub(crate) fn invalidate_layers() {
    let _ = SURFACE.try_with(|s| {
        if let Some(s) = s.borrow_mut().as_mut() {
            s.retained.main.saved = None;
            s.retained.queue.saved = None;
        }
    });
}
pub(crate) fn invalidate_queue_layer() {
    let _ = SURFACE.try_with(|s| {
        if let Some(s) = s.borrow_mut().as_mut() {
            s.retained.queue.saved = None;
        }
    });
}
pub(crate) fn invalidate_main_layer() {
    let _ = SURFACE.try_with(|s| {
        if let Some(s) = s.borrow_mut().as_mut() {
            s.retained.main.saved = None;
        }
    });
}
pub(crate) unsafe fn configure_layers(
    main: Option<LayerRequest>,
    queue: Option<LayerRequest>,
    visual_key: u64,
) -> bool {
    SURFACE.with(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(s) = slot.as_mut().filter(|s| s.active) else {
            return false;
        };
        if s.retained.visual_key != visual_key {
            s.retained.main.saved = None;
            s.retained.queue.saved = None;
            s.retained.visual_key = visual_key;
        }
        s.retained.current = Layer::Direct;
        s.retained.overlay = None;
        let result = s
            .retained
            .main
            .configure(&s.context, main)
            .and_then(|_| s.retained.queue.configure(&s.context, queue));
        if result.is_err() {
            s.failed = true;
        }
        s.retained.main.recording.is_some() || s.retained.queue.recording.is_some()
    })
}
pub(crate) fn is_recording(layer: Layer) -> bool {
    SURFACE.with(|s| {
        s.borrow().as_ref().is_some_and(|s| match layer {
            Layer::Main => s.retained.main.recording.is_some(),
            Layer::Queue => s.retained.queue.recording.is_some(),
            _ => false,
        })
    })
}
pub(crate) unsafe fn select_layer(layer: Layer) -> bool {
    SURFACE.with(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(s) = slot.as_mut().filter(|s| s.active) else {
            return true;
        };
        if s.retained.current != layer {
            s.retained.current = layer;
            match layer {
                Layer::Main => {
                    if let Some(list) = &s.retained.main.recording {
                        s.context.SetTarget(list);
                    } else {
                        s.context.SetTarget(&s.backing);
                    }
                }
                Layer::Queue => {
                    if let Some(list) = &s.retained.queue.recording {
                        s.context.SetTarget(list);
                    } else {
                        s.context.SetTarget(&s.backing);
                    }
                }
                Layer::Overlay
                    if s.retained.main.request.is_some() || s.retained.queue.request.is_some() =>
                {
                    if s.retained.overlay.is_none() {
                        match s.context.CreateCommandList() {
                            Ok(list) => s.retained.overlay = Some(list),
                            Err(_) => s.failed = true,
                        }
                    }
                    if let Some(list) = &s.retained.overlay {
                        s.context.SetTarget(list);
                    }
                }
                _ => s.context.SetTarget(&s.backing),
            }
        }
        !s.retained.reusing()
    })
}
pub(crate) fn expanded_clip(mut rect: RECT, layer: Layer) -> RECT {
    let margin = SURFACE.with(|s| {
        s.borrow()
            .as_ref()
            .and_then(|s| match layer {
                Layer::Main => s
                    .retained
                    .main
                    .recording
                    .as_ref()
                    .and(s.retained.main.request),
                Layer::Queue => s
                    .retained
                    .queue
                    .recording
                    .as_ref()
                    .and(s.retained.queue.request),
                _ => None,
            })
            .map_or(0, |r| r.overscan)
    });
    rect.top -= margin;
    rect.bottom += margin;
    rect
}
impl Surface {
    pub(super) unsafe fn composite_layers(&mut self) -> Result<()> {
        self.context.SetTarget(&self.backing);
        self.retained.main.finish()?;
        self.retained.queue.finish()?;
        if let Some(overlay) = &self.retained.overlay {
            overlay.Close()?;
        }
        for slot in [&self.retained.main, &self.retained.queue] {
            let (Some(saved), Some(request)) = (&slot.saved, slot.request) else {
                continue;
            };
            for damage in &self.damage {
                let clip = D2D_RECT_F {
                    left: request.viewport.left.max(damage.left) as f32,
                    top: request.viewport.top.max(damage.top) as f32,
                    right: request.viewport.right.min(damage.right) as f32,
                    bottom: request.viewport.bottom.min(damage.bottom) as f32,
                };
                if clip.left >= clip.right || clip.top >= clip.bottom {
                    continue;
                }
                self.context.SetTransform(&identity());
                self.context
                    .PushAxisAlignedClip(&clip, D2D1_ANTIALIAS_MODE_ALIASED);
                let mut transform = identity();
                transform.M32 = (saved.origin as f64 - request.shown_offset) as f32;
                self.context.SetTransform(&transform);
                self.context.DrawImage(
                    &saved.list,
                    None,
                    None,
                    D2D1_INTERPOLATION_MODE_LINEAR,
                    D2D1_COMPOSITE_MODE_SOURCE_OVER,
                );
                self.context.PopAxisAlignedClip();
            }
        }
        if let Some(overlay) = self.retained.overlay.take() {
            self.context.SetTransform(&identity());
            self.context.DrawImage(
                &overlay,
                None,
                None,
                D2D1_INTERPOLATION_MODE_LINEAR,
                D2D1_COMPOSITE_MODE_SOURCE_OVER,
            );
        }
        self.context.SetTransform(&identity());
        self.retained.current = Layer::Direct;
        Ok(())
    }
}

/// A small monochrome measurement canvas lets GDI clip queries see overscan.
/// All actual pixels are generated by Direct2D; the native paint bitmap stays intact.
pub(crate) struct RetainedCanvas {
    dc: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
}
impl RetainedCanvas {
    pub(crate) unsafe fn new(width: i32, height: i32, margin: i32) -> Option<Self> {
        let dc = CreateCompatibleDC(std::ptr::null_mut());
        if dc.is_null() {
            return None;
        }
        let bitmap = CreateBitmap(width, height + 2 * margin, 1, 1, std::ptr::null());
        if bitmap.is_null() {
            DeleteDC(dc);
            return None;
        }
        let previous = SelectObject(dc, bitmap);
        SetWindowOrgEx(dc, 0, -margin, std::ptr::null_mut());
        Some(Self {
            dc,
            bitmap,
            previous,
        })
    }
    pub(crate) fn dc(&self) -> HDC {
        self.dc
    }
}
impl Drop for RetainedCanvas {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.previous);
            DeleteObject(self.bitmap);
            DeleteDC(self.dc);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::UI::WindowsAndMessaging::*;
    #[test]
    #[ignore = "Requires Windows hardware Direct3D and DirectComposition"]
    fn gpu_retained_layer_replays_overscan_without_submitting_elements() {
        unsafe {
            let hwnd = CreateWindowExW(
                0,
                windows::core::w!("STATIC").as_ptr(),
                std::ptr::null(),
                WS_POPUP,
                0,
                0,
                128,
                128,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
            );
            assert!(!hwnd.is_null());
            let mut pixels = vec![0u32; 128 * 128];
            assert!(begin(hwnd, std::ptr::null(), 128, 128, 128, 0, None));
            background(None, 0, 0, 0, 100, 0, None, Some(0xff111111));
            let viewport = RECT {
                left: 10,
                top: 10,
                right: 118,
                bottom: 100,
            };
            let queue = Some(LayerRequest {
                viewport: RECT {
                    left: 90,
                    right: 118,
                    ..viewport
                },
                layout_offset: 0,
                shown_offset: 0.,
                overscan: 96,
            });
            assert!(configure_layers(
                Some(LayerRequest {
                    viewport,
                    layout_offset: 0,
                    shown_offset: 0.,
                    overscan: 96
                }),
                queue,
                1
            ));
            let canvas = RetainedCanvas::new(128, 128, 96).unwrap();
            let frame = crate::drawing::Frame::begin(
                canvas.dc(),
                pixels.as_mut_ptr().cast(),
                128,
                128,
                128,
                false,
            )
            .unwrap();
            assert!(select_layer(Layer::Main));
            crate::drawing::fill(
                canvas.dc(),
                RECT {
                    left: 20,
                    top: 140,
                    right: 40,
                    bottom: 160,
                },
                0xffff0000,
                0.,
            );
            assert!(select_layer(Layer::Queue));
            crate::drawing::fill(
                canvas.dc(),
                RECT {
                    left: 94,
                    top: 20,
                    right: 110,
                    bottom: 40,
                },
                0xff00ff00,
                0.,
            );
            select_layer(Layer::Direct);
            drop(frame);
            assert!(finish());
            assert!(begin(hwnd, std::ptr::null(), 128, 128, 128, 0, None));
            background(None, 0, 0, 0, 100, 0, None, Some(0xff111111));
            assert!(!configure_layers(
                Some(LayerRequest {
                    viewport,
                    layout_offset: 70,
                    shown_offset: 70.,
                    overscan: 96
                }),
                queue,
                1
            ));
            assert!(
                !select_layer(Layer::Main),
                "replay must bypass element submission"
            );
            select_layer(Layer::Direct);
            assert!(finish());
            assert_eq!(
                snapshot(RECT {
                    left: 25,
                    top: 75,
                    right: 26,
                    bottom: 76
                })
                .unwrap()[0],
                0xffff0000,
                "offscreen content must survive until it scrolls into view"
            );
            assert_eq!(
                snapshot(RECT {
                    left: 25,
                    top: 5,
                    right: 26,
                    bottom: 6
                })
                .unwrap()[0],
                0xff111111,
                "presentation must stay clipped to its own viewport"
            );
            SURFACE.with(|slot| {
                slot.borrow_mut()
                    .as_mut()
                    .unwrap()
                    .retained
                    .invalidate_deferred(1)
            });
            assert!(begin(hwnd, std::ptr::null(), 128, 128, 128, 0, None));
            assert!(configure_layers(
                Some(LayerRequest {
                    viewport,
                    layout_offset: 70,
                    shown_offset: 70.,
                    overscan: 96
                }),
                queue,
                1
            ));
            select_layer(Layer::Main);
            assert!(
                !select_layer(Layer::Queue),
                "a deferred main upload must preserve the queue paint cache"
            );
            select_layer(Layer::Direct);
            assert!(finish());
            hide();
            DestroyWindow(hwnd);
        }
    }
}
