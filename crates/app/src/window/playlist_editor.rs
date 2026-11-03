//! A small owned editor; the main window keeps its route, scene and scroll.
use super::*;

struct Editor {
    draft: crate::playlist::Draft,
    title: HWND,
    description: HWND,
    font: HFONT,
    brush: HBRUSH,
    edit_brush: HBRUSH,
    scale: f32,
    labels: [String; 7],
    error: String,
    cover_name: String,
    done: bool,
    saved: bool,
    icon_font: HFONT,
    heading_font: HFONT,
    privacy_supported: bool,
    privacy_labels: [String; 4],
    confirm_labels: [String; 4],
    delete_label: String,
    foreground: u32,
    raised: u32,
    accent: u32,
    accent_text: u32,
    border: u32,
    radius: i32,
    preview: Option<std::sync::Arc<crate::catalog::Bitmap>>,
    privacy_receiver: Option<std::sync::mpsc::Receiver<Result<Option<bool>, String>>>,
}

impl Editor {
    fn footer_y(&self) -> i32 {
        340
    }
    fn toggle_privacy(&mut self) {
        if self.privacy_supported {
            if let Some(public) = self.draft.public {
                self.draft.public = Some(!public);
                self.draft.privacy_changed = true;
            }
        }
    }
    fn px(&self, n: i32) -> i32 {
        (n as f32 * self.scale).round() as i32
    }
    fn rect(&self, x: i32, y: i32, w: i32, h: i32) -> RECT {
        RECT {
            left: self.px(x),
            top: self.px(y),
            right: self.px(x + w),
            bottom: self.px(y + h),
        }
    }
    unsafe fn save(&mut self, hwnd: HWND) {
        let title = read(self.title).trim().to_owned();
        if title.is_empty() {
            self.error = self.labels[6].clone();
            SetFocus(self.title);
        } else {
            self.draft.title = title;
            let description = read(self.description);
            self.draft.description_changed |= description != self.draft.description;
            self.draft.description = description;
            if let Err(error) = self
                .draft
                .image
                .as_ref()
                .map(|_| self.draft.jpeg())
                .transpose()
            {
                self.error = error;
            } else {
                self.saved = true;
                self.done = true;
            }
        }
        InvalidateRect(hwnd, null(), 0);
    }
}

unsafe fn text(dc: HDC, font: HFONT, value: &str, mut rect: RECT, color: u32) {
    let old = SelectObject(dc, font);
    SetBkMode(dc, TRANSPARENT as i32);
    SetTextColor(dc, color);
    DrawTextW(
        dc,
        wide(value).as_ptr(),
        -1,
        &mut rect,
        DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS,
    );
    SelectObject(dc, old);
}

unsafe fn rounded(dc: HDC, rect: RECT, color: u32, radius: i32) {
    let brush = CreateSolidBrush(color);
    let old_brush = SelectObject(dc, brush);
    let old_pen = SelectObject(dc, GetStockObject(NULL_PEN));
    RoundRect(
        dc,
        rect.left,
        rect.top,
        rect.right,
        rect.bottom,
        radius * 2,
        radius * 2,
    );
    SelectObject(dc, old_pen);
    SelectObject(dc, old_brush);
    DeleteObject(brush);
}

unsafe extern "system" fn procedure(hwnd: HWND, msg: u32, wp: usize, lp: isize) -> isize {
    if msg == WM_NCCREATE {
        SetWindowLongPtrW(
            hwnd,
            GWLP_USERDATA,
            (*(lp as *const CREATESTRUCTW)).lpCreateParams as isize,
        );
    }
    let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Editor;
    if pointer.is_null() {
        return DefWindowProcW(hwnd, msg, wp, lp);
    }
    let state = &mut *pointer;
    match msg {
        WM_PAINT => {
            let mut ps: PAINTSTRUCT = zeroed();
            let dc = BeginPaint(hwnd, &mut ps);
            let mut bounds: RECT = zeroed();
            GetClientRect(hwnd, &mut bounds);
            FillRect(dc, &bounds, state.brush);
            let pen = CreatePen(PS_SOLID, 1, state.border);
            let old_pen = SelectObject(dc, pen);
            let old_brush = SelectObject(dc, state.brush);
            RoundRect(
                dc,
                0,
                0,
                bounds.right,
                bounds.bottom,
                state.px(20),
                state.px(20),
            );
            SelectObject(dc, old_pen);
            SelectObject(dc, old_brush);
            DeleteObject(pen);
            text(
                dc,
                state.heading_font,
                &state.labels[0],
                state.rect(24, 16, 510, 36),
                state.foreground,
            );
            let pen = CreatePen(PS_SOLID, state.px(2).max(1), state.foreground);
            let old = SelectObject(dc, pen);
            MoveToEx(dc, state.px(558), state.px(24), null_mut());
            LineTo(dc, state.px(570), state.px(36));
            MoveToEx(dc, state.px(570), state.px(24), null_mut());
            LineTo(dc, state.px(558), state.px(36));
            SelectObject(dc, old);
            DeleteObject(pen);
            rounded(
                dc,
                state.rect(260, 72, 316, 46),
                state.raised,
                state.px(state.radius),
            );
            rounded(
                dc,
                state.rect(260, 128, 316, 160),
                state.raised,
                state.px(state.radius),
            );
            let cover = state.rect(24, 72, 216, 216);
            rounded(dc, cover, state.raised, state.px(state.radius));
            if let Some(image) = &state.preview {
                bitmap(dc, cover, image, state.px(state.radius));
            } else {
                text(
                    dc,
                    state.icon_font,
                    "\u{e8d6}",
                    state.rect(112, 150, 40, 40),
                    state.foreground,
                );
                text(
                    dc,
                    state.font,
                    &state.labels[3],
                    state.rect(50, 245, 180, 30),
                    state.foreground,
                );
            }
            text(
                dc,
                state.font,
                &state.error,
                state.rect(260, 292, 316, 28),
                0x8888ff,
            );
            for (rect, label, color) in [
                (
                    state.rect(350, state.footer_y(), 100, 38),
                    &state.labels[4],
                    state.raised,
                ),
                (
                    state.rect(470, state.footer_y(), 106, 38),
                    &state.labels[5],
                    state.accent,
                ),
            ] {
                rounded(dc, rect, color, state.px(state.radius));
                let mut label_rect = rect;
                let old = SelectObject(dc, state.font);
                SetBkMode(dc, TRANSPARENT as i32);
                SetTextColor(
                    dc,
                    if color == state.accent {
                        state.accent_text
                    } else {
                        state.foreground
                    },
                );
                DrawTextW(
                    dc,
                    wide(label).as_ptr(),
                    -1,
                    &mut label_rect,
                    DT_CENTER | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS,
                );
                SelectObject(dc, old);
            }
            if state.privacy_supported {
                let (icon, label) = match state.draft.public {
                    Some(true) => ("\u{e785}", &state.privacy_labels[0]),
                    Some(false) => ("\u{e72e}", &state.privacy_labels[1]),
                    None => (
                        "",
                        &state.privacy_labels[if state.privacy_receiver.is_some() {
                            2
                        } else {
                            3
                        }],
                    ),
                };
                text(
                    dc,
                    state.icon_font,
                    icon,
                    state.rect(36, 294, 24, 34),
                    state.foreground,
                );
                text(
                    dc,
                    state.font,
                    label,
                    state.rect(70, 294, 170, 34),
                    state.foreground,
                );
            }
            if !state.draft.id.is_empty() {
                text(
                    dc,
                    state.icon_font,
                    "\u{e74d}",
                    state.rect(24, state.footer_y(), 24, 38),
                    0x9090ff,
                );
                text(
                    dc,
                    state.font,
                    &state.delete_label,
                    state.rect(54, state.footer_y(), 120, 38),
                    0x9090ff,
                );
            }
            EndPaint(hwnd, &ps);
            0
        }
        WM_ERASEBKGND => 1,
        WM_CTLCOLOREDIT => {
            SetTextColor(wp as HDC, state.foreground);
            SetBkColor(wp as HDC, state.raised);
            state.edit_brush as isize
        }
        WM_LBUTTONUP => {
            let point = POINT {
                x: lp as i16 as i32,
                y: (lp >> 16) as i16 as i32,
            };
            if PtInRect(&state.rect(24, 72, 216, 216), point) != 0 {
                if let Some(path) = open_image(hwnd) {
                    match state.draft.choose(&path) {
                        Ok(()) => {
                            state.cover_name = path
                                .file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .into_owned();
                            state.preview = state
                                .draft
                                .jpeg()
                                .ok()
                                .and_then(|bytes| crate::catalog::decode(&bytes, 500).ok())
                                .map(std::sync::Arc::new);
                            state.error.clear();
                        }
                        Err(error) => state.error = error,
                    }
                    InvalidateRect(hwnd, null(), 0);
                }
            } else if state.privacy_supported && PtInRect(&state.rect(24, 294, 216, 34), point) != 0
            {
                state.toggle_privacy();
                InvalidateRect(hwnd, null(), 0);
            } else if PtInRect(&state.rect(550, 12, 36, 40), point) != 0 {
                state.done = true;
            } else if !state.draft.id.is_empty()
                && PtInRect(&state.rect(24, state.footer_y(), 150, 38), point) != 0
            {
                if super::confirm::show(
                    hwnd,
                    [state.font, state.heading_font],
                    state.confirm_labels.clone(),
                ) {
                    state.draft.delete_requested = true;
                    state.saved = true;
                    state.done = true;
                }
            } else if PtInRect(&state.rect(470, state.footer_y(), 106, 38), point) != 0 {
                state.save(hwnd);
            } else if PtInRect(&state.rect(350, state.footer_y(), 100, 38), point) != 0 {
                state.done = true;
            }
            0
        }
        WM_TIMER => {
            if let Some(result) = state
                .privacy_receiver
                .as_ref()
                .and_then(|receiver| receiver.try_recv().ok())
            {
                state.privacy_receiver = None;
                match result {
                    Ok(public) => state.draft.public = public,
                    Err(error) => state.error = error,
                }
                InvalidateRect(hwnd, null(), 0);
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

pub(super) unsafe fn show(
    owner: HWND,
    app: &App,
    draft: crate::playlist::Draft,
) -> Option<crate::playlist::Draft> {
    show_initialized(owner, app, draft, |_| {})
}

pub(super) unsafe fn show_initialized(
    owner: HWND,
    app: &App,
    mut draft: crate::playlist::Draft,
    on_open: impl FnOnce(HWND),
) -> Option<crate::playlist::Draft> {
    let scale = GetDpiForWindow(owner) as f32 / 96.0;
    let privacy_supported = crate::playlist::supports_privacy(app.ui.provider);
    if privacy_supported && draft.id.is_empty() && draft.public.is_none() {
        draft.public = Some(false);
    }
    let privacy_receiver = if privacy_supported && !draft.id.is_empty() && draft.public.is_none() {
        let id = draft.id.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(crate::youtube::playlist_privacy(&id));
        });
        Some(rx)
    } else {
        None
    };
    let heading = if draft.id.is_empty() {
        "Create playlist"
    } else {
        "Edit playlist"
    };
    let labels = [
        heading,
        "Name",
        "Description",
        "Choose image",
        "Cancel",
        "Save",
        "Enter a playlist name",
    ]
    .map(|value| app.ui.t(value).to_owned());
    let confirm_labels = super::confirm::playlist_labels(&app.ui, &draft.title);
    let preview = app.images.get(&draft.preview).cloned();
    let mut state = Box::new(Editor {
        draft,
        title: null_mut(),
        description: null_mut(),
        font: app.fonts[0],
        brush: CreateSolidBrush(
            app.ui
                .color("Panel", if app.ui.light { 0xffffff } else { 0x202020 }),
        ),
        edit_brush: CreateSolidBrush(
            app.ui
                .color("Raised", if app.ui.light { 0xeeeeee } else { 0x303030 }),
        ),
        foreground: app
            .ui
            .color("Text", if app.ui.light { 0x202020 } else { 0xeeeeee }),
        raised: app
            .ui
            .color("Raised", if app.ui.light { 0xeeeeee } else { 0x303030 }),
        accent: app.ui.color("Accent", 0x684dba),
        accent_text: app.ui.color("AccentText", 0xffffff),
        border: app.ui.color("Border", 0x383838),
        radius: app.ui.corner_px(8),
        preview,
        scale,
        labels,
        error: String::new(),
        cover_name: String::new(),
        done: false,
        saved: false,
        icon_font: app.fonts[4],
        heading_font: app.fonts[3],
        privacy_supported,
        privacy_labels: [
            app.ui.t("Make private").into(),
            app.ui.t("Make public").into(),
            app.ui.t("Loading privacyâ€¦").into(),
            app.ui.t("Privacy unavailable").into(),
        ],
        confirm_labels,
        delete_label: app.ui.t("Delete").into(),
        privacy_receiver,
    });
    let class = wide("DeezNats.PlaylistEditor");
    let mut wc: WNDCLASSW = zeroed();
    wc.lpfnWndProc = Some(procedure);
    wc.hInstance = GetModuleHandleW(null());
    wc.lpszClassName = class.as_ptr();
    wc.hCursor = LoadCursorW(null_mut(), IDC_ARROW);
    wc.style = CS_DROPSHADOW;
    RegisterClassW(&wc);
    let mut parent: RECT = zeroed();
    GetWindowRect(owner, &mut parent);
    let mut monitor: MONITORINFO = zeroed();
    monitor.cbSize = size_of::<MONITORINFO>() as u32;
    GetMonitorInfoW(
        MonitorFromWindow(owner, MONITOR_DEFAULTTONEAREST),
        &mut monitor,
    );
    let width = state.px(600);
    let height = state.px(state.footer_y() + 62);
    let x = ((parent.left + parent.right - width) / 2)
        .max(monitor.rcWork.left)
        .min(monitor.rcWork.right - width);
    let y = ((parent.top + parent.bottom - height) / 2)
        .max(monitor.rcWork.top)
        .min(monitor.rcWork.bottom - height);
    let hwnd = CreateWindowExW(
        WS_EX_TOOLWINDOW | WS_EX_CONTROLPARENT,
        class.as_ptr(),
        wide(&state.labels[0]).as_ptr(),
        WS_POPUP | WS_CLIPCHILDREN,
        x,
        y,
        width,
        height,
        owner,
        null_mut(),
        wc.hInstance,
        &mut *state as *mut Editor as *const c_void,
    );
    if hwnd.is_null() {
        DeleteObject(state.brush);
        DeleteObject(state.edit_brush);
        return None;
    }
    SetWindowRgn(
        hwnd,
        CreateRoundRectRgn(0, 0, width + 1, height + 1, state.px(20), state.px(20)),
        0,
    );
    for (description, y, height, value) in [
        (false, 84, 24, state.draft.title.clone()),
        (true, 140, 136, state.draft.description.clone()),
    ] {
        let edit = CreateWindowExW(
            0,
            wide("EDIT").as_ptr(),
            wide(&value).as_ptr(),
            WS_CHILD
                | WS_VISIBLE
                | WS_TABSTOP
                | if description {
                    ES_MULTILINE as u32 | ES_AUTOVSCROLL as u32 | ES_WANTRETURN as u32
                } else {
                    ES_AUTOHSCROLL as u32
                },
            state.px(272),
            state.px(y),
            state.px(292),
            state.px(height),
            hwnd,
            null_mut(),
            wc.hInstance,
            null(),
        );
        SendMessageW(edit, WM_SETFONT, state.font as usize, 1);
        SendMessageW(
            edit,
            EM_SETCUEBANNER,
            1,
            wide(if description {
                &state.labels[2]
            } else {
                &state.labels[1]
            })
            .as_ptr() as isize,
        );
        SendMessageW(
            edit,
            EM_SETLIMITTEXT,
            if description { 20000 } else { 300 },
            0,
        );
        if description {
            state.description = edit;
        } else {
            state.title = edit;
        }
    }
    let old_focus = GetFocus();
    EnableWindow(owner, 0);
    ShowWindow(hwnd, SW_SHOW);
    SetForegroundWindow(hwnd);
    SetFocus(state.title);
    SetTimer(hwnd, 7, 50, None);
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
        if message.message == WM_KEYDOWN && message.wParam == 27 {
            state.done = true;
            continue;
        }
        if message.message == WM_KEYDOWN
            && message.wParam == 13
            && (GetFocus() == state.title
                || windows_sys::Win32::UI::Input::KeyboardAndMouse::GetKeyState(0x11) < 0)
        {
            state.save(hwnd);
            continue;
        }
        if IsDialogMessageW(hwnd, &message) == 0 {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    KillTimer(hwnd, 7);
    EnableWindow(owner, 1);
    DestroyWindow(hwnd);
    DeleteObject(state.brush);
    DeleteObject(state.edit_brush);
    SetForegroundWindow(owner);
    if IsWindow(old_focus) != 0 {
        SetFocus(old_focus);
    }
    state.saved.then(|| state.draft.clone())
}
