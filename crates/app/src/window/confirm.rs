//! App-rendered destructive confirmation, with a dimmed owner and safe default.
use super::*;
const STATIC_BLACK_RECT: u32 = 0x00000004; // Win32 SS_BLACKRECT style.

pub(super) fn playlist_labels(ui: &Ui, title: &str) -> [String; 4] {
    let title = if title.chars().count() > 70 {
        format!("{}…", title.chars().take(70).collect::<String>())
    } else {
        title.to_owned()
    };
    let prompt = if ui.provider == crate::catalog::Provider::Spotify {
        "Remove “{0}” from your library? Other people may still access it."
    } else {
        "Delete “{0}”? This cannot be undone."
    };
    [
        ui.t("Delete playlist").into(),
        ui.t(prompt).replace("{0}", &title),
        ui.t("Cancel").into(),
        ui.t("Delete").into(),
    ]
}

struct Confirm {
    labels: [String; 4],
    fonts: [HFONT; 2],
    scale: f32,
    done: bool,
    accepted: bool,
    delete_focused: bool,
    hover_delete: bool,
}

impl Confirm {
    fn px(&self, value: i32) -> i32 {
        (value as f32 * self.scale).round() as i32
    }
    fn rect(&self, x: i32, y: i32, w: i32, h: i32) -> RECT {
        RECT {
            left: self.px(x),
            top: self.px(y),
            right: self.px(x + w),
            bottom: self.px(y + h),
        }
    }
}

unsafe fn text(dc: HDC, font: HFONT, value: &str, mut rect: RECT, color: u32, flags: u32) {
    let old = SelectObject(dc, font);
    SetBkMode(dc, TRANSPARENT as i32);
    SetTextColor(dc, color);
    DrawTextW(dc, wide(value).as_ptr(), -1, &mut rect, flags);
    SelectObject(dc, old);
}

unsafe extern "system" fn procedure(hwnd: HWND, msg: u32, wp: usize, lp: isize) -> isize {
    if msg == WM_NCCREATE {
        SetWindowLongPtrW(
            hwnd,
            GWLP_USERDATA,
            (*(lp as *const CREATESTRUCTW)).lpCreateParams as isize,
        );
    }
    let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Confirm;
    if pointer.is_null() {
        return DefWindowProcW(hwnd, msg, wp, lp);
    }
    let state = &mut *pointer;
    match msg {
        WM_PAINT => {
            let mut ps: PAINTSTRUCT = zeroed();
            let dc = BeginPaint(hwnd, &mut ps);
            let brush = CreateSolidBrush(0x141414);
            let pen = CreatePen(PS_SOLID, 1, 0x303030);
            let old_brush = SelectObject(dc, brush);
            let old_pen = SelectObject(dc, pen);
            RoundRect(
                dc,
                0,
                0,
                state.px(488),
                state.px(176),
                state.px(24),
                state.px(24),
            );
            SelectObject(dc, old_pen);
            SelectObject(dc, old_brush);
            DeleteObject(pen);
            DeleteObject(brush);
            text(
                dc,
                state.fonts[1],
                &state.labels[0],
                state.rect(20, 18, 448, 38),
                0xffffff,
                DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS,
            );
            text(
                dc,
                state.fonts[0],
                &state.labels[1],
                state.rect(20, 62, 448, 46),
                0x919191,
                DT_LEFT | DT_WORDBREAK | DT_END_ELLIPSIS,
            );
            text(
                dc,
                state.fonts[0],
                &state.labels[2],
                state.rect(306, 120, 92, 40),
                0xffffff,
                DT_CENTER | DT_VCENTER | DT_SINGLELINE,
            );
            let brush = CreateSolidBrush(if state.hover_delete {
                0x3030a0
            } else {
                0x202088
            });
            let old_brush = SelectObject(dc, brush);
            let old_pen = SelectObject(dc, GetStockObject(NULL_PEN));
            RoundRect(
                dc,
                state.px(416),
                state.px(120),
                state.px(468),
                state.px(160),
                state.px(40),
                state.px(40),
            );
            SelectObject(dc, old_pen);
            SelectObject(dc, old_brush);
            DeleteObject(brush);
            text(
                dc,
                state.fonts[0],
                &state.labels[3],
                state.rect(416, 120, 52, 40),
                0xffffff,
                DT_CENTER | DT_VCENTER | DT_SINGLELINE,
            );
            if state.delete_focused {
                let r = state.rect(413, 117, 58, 46);
                DrawFocusRect(dc, &r);
            }
            EndPaint(hwnd, &ps);
            0
        }
        WM_ERASEBKGND => 1,
        WM_MOUSEMOVE => {
            let p = POINT {
                x: lp as i16 as i32,
                y: (lp >> 16) as i16 as i32,
            };
            let hover = PtInRect(&state.rect(416, 120, 52, 40), p) != 0;
            if hover != state.hover_delete {
                state.hover_delete = hover;
                InvalidateRect(hwnd, null(), 0);
            }
            0
        }
        WM_LBUTTONUP => {
            let p = POINT {
                x: lp as i16 as i32,
                y: (lp >> 16) as i16 as i32,
            };
            if PtInRect(&state.rect(416, 120, 52, 40), p) != 0 {
                state.accepted = true;
                state.done = true;
            } else if PtInRect(&state.rect(306, 120, 92, 40), p) != 0 {
                state.done = true;
            }
            0
        }
        WM_KEYDOWN => {
            match wp as u16 {
                27 => state.done = true,
                9 | VK_LEFT | VK_RIGHT => {
                    state.delete_focused = !state.delete_focused;
                    InvalidateRect(hwnd, null(), 0);
                }
                13 => {
                    state.accepted = state.delete_focused;
                    state.done = true;
                }
                _ => {}
            }
            0
        }
        WM_CLOSE => {
            state.done = true;
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

pub(super) unsafe fn show(owner: HWND, fonts: [HFONT; 2], labels: [String; 4]) -> bool {
    show_initialized(owner, fonts, labels, |_| {})
}

pub(super) unsafe fn show_initialized(
    owner: HWND,
    fonts: [HFONT; 2],
    labels: [String; 4],
    on_open: impl FnOnce(HWND),
) -> bool {
    let mut state = Box::new(Confirm {
        labels,
        fonts,
        scale: GetDpiForWindow(owner).max(96) as f32 / 96.0,
        done: false,
        accepted: false,
        delete_focused: false,
        hover_delete: false,
    });
    let instance = GetModuleHandleW(null());
    let class = wide("DeezNats.Confirm");
    let mut wc: WNDCLASSW = zeroed();
    wc.lpfnWndProc = Some(procedure);
    wc.hInstance = instance;
    wc.lpszClassName = class.as_ptr();
    wc.hCursor = LoadCursorW(null_mut(), IDC_ARROW);
    wc.style = CS_DROPSHADOW;
    RegisterClassW(&wc);
    let root = owner;
    let mut bounds: RECT = zeroed();
    GetWindowRect(root, &mut bounds);
    let overlay = CreateWindowExW(
        WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_LAYERED,
        wide("STATIC").as_ptr(),
        wide("").as_ptr(),
        WS_POPUP | STATIC_BLACK_RECT,
        bounds.left,
        bounds.top,
        bounds.right - bounds.left,
        bounds.bottom - bounds.top,
        root,
        null_mut(),
        instance,
        null(),
    );
    if !overlay.is_null() {
        SetLayeredWindowAttributes(overlay, 0, 150, LWA_ALPHA);
        ShowWindow(overlay, SW_SHOWNOACTIVATE);
    }
    let width = state.px(488);
    let height = state.px(176);
    let x = (bounds.left + bounds.right - width) / 2;
    let y = (bounds.top + bounds.bottom - height) / 2;
    let hwnd = CreateWindowExW(
        WS_EX_TOOLWINDOW,
        class.as_ptr(),
        wide(&state.labels[0]).as_ptr(),
        WS_POPUP,
        x,
        y,
        width,
        height,
        if overlay.is_null() { owner } else { overlay },
        null_mut(),
        instance,
        &mut *state as *mut Confirm as *const c_void,
    );
    if hwnd.is_null() {
        if !overlay.is_null() {
            DestroyWindow(overlay);
        }
        return false;
    }
    SetWindowRgn(
        hwnd,
        CreateRoundRectRgn(0, 0, width + 1, height + 1, state.px(24), state.px(24)),
        0,
    );
    let enabled = IsWindowEnabled(owner);
    let focus = GetFocus();
    EnableWindow(owner, 0);
    ShowWindow(hwnd, SW_SHOW);
    SetForegroundWindow(hwnd);
    SetFocus(hwnd);
    on_open(hwnd);
    let mut message: MSG = zeroed();
    while !state.done && IsWindow(hwnd) != 0 {
        let result = GetMessageW(&mut message, null_mut(), 0, 0);
        if result <= 0 {
            if result == 0 {
                PostQuitMessage(message.wParam as i32);
            }
            break;
        }
        TranslateMessage(&message);
        DispatchMessageW(&message);
    }
    DestroyWindow(hwnd);
    if !overlay.is_null() {
        DestroyWindow(overlay);
    }
    EnableWindow(owner, enabled);
    SetForegroundWindow(owner);
    if IsWindow(focus) != 0 {
        SetFocus(focus);
    }
    state.accepted
}
