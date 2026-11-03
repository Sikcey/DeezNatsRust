//! App-rendered popup surfaces. HMENU is only an action-description adapter;
//! Windows never measures, paints, or tracks these menus.
use super::*;
use std::sync::mpsc::Receiver;

type PlaylistResult = Result<Vec<crate::catalog::Entry>, String>;

struct Item {
    title: String,
    id: i32,
    disabled: bool,
    separator: bool,
    icon: &'static str,
    children: Vec<Item>,
}

unsafe fn items(menu: HMENU, context: bool, nested: bool) -> Vec<Item> {
    (0..GetMenuItemCount(menu).max(0) as u32)
        .map(|index| {
            let mut info: MENUITEMINFOW = zeroed();
            info.cbSize = size_of::<MENUITEMINFOW>() as u32;
            info.fMask = MIIM_FTYPE | MIIM_SUBMENU | MIIM_ID | MIIM_STATE;
            GetMenuItemInfoW(menu, index, 1, &mut info);
            let mut label =
                vec![0u16; GetMenuStringW(menu, index, null_mut(), 0, MF_BYPOSITION) as usize + 1];
            GetMenuStringW(
                menu,
                index,
                label.as_mut_ptr(),
                label.len() as i32,
                MF_BYPOSITION,
            );
            Item {
                title: String::from_utf16_lossy(&label[..label.len().saturating_sub(1)]),
                id: info.wID as i32,
                disabled: info.fState & (MFS_DISABLED | MFS_GRAYED) != 0,
                separator: info.fType & MFT_SEPARATOR != 0,
                icon: if !context {
                    ""
                } else if !info.hSubMenu.is_null() || (nested && info.wID == 13) {
                    "\u{e710}"
                } else if nested {
                    ""
                } else {
                    match info.wID {
                        1 => "\u{e8a7}",
                        3 => "\u{e8fd}",
                        4 => "\u{e946}",
                        5 => "\u{e93c}",
                        6 => "\u{e77b}",
                        7 => "\u{eb51}",
                        8 => "\u{e896}",
                        9 => "\u{e71b}",
                        11 => "\u{e70f}",
                        14 | 15 => "\u{e74d}",
                        16 => "\u{e718}",
                        17 => "\u{e768}",
                        18 => "\u{e946}",
                        19 => "\u{e787}",
                        _ => "",
                    }
                },
                children: if info.hSubMenu.is_null() {
                    Vec::new()
                } else {
                    items(info.hSubMenu, context, true)
                },
            }
        })
        .collect()
}

struct Popup {
    root: HWND,
    child: HWND,
    rows: Vec<Item>,
    branch: Option<usize>,
    hover: Option<(bool, usize)>,
    query: String,
    offset: usize,
    root_offset: usize,
    selected: i32,
    done: bool,
    scale: f32,
    fonts: [HFONT; 2],
    palette: [u32; 6],
    receiver: Option<Receiver<PlaylistResult>>,
    playlists: Vec<crate::catalog::Entry>,
    status: String,
    labels: [String; 3],
}

impl Popup {
    fn px(&self, n: i32) -> i32 {
        (n as f32 * self.scale).round() as i32
    }
    fn visible(&self) -> Vec<usize> {
        self.branch
            .map(|b| {
                self.rows[b]
                    .children
                    .iter()
                    .enumerate()
                    .filter(|(_, r)| {
                        r.id < 1000 || r.title.to_lowercase().contains(&self.query.to_lowercase())
                    })
                    .map(|(i, _)| i)
                    .collect()
            })
            .unwrap_or_default()
    }
    fn row_height(&self, row: &Item) -> i32 {
        self.px(if row.separator { 13 } else { 36 })
    }
    unsafe fn reveal(&mut self, child: bool, index: usize) {
        let indices = if child {
            self.visible()
        } else {
            (0..self.rows.len()).collect()
        };
        let Some(position) = indices.iter().position(|i| *i == index) else {
            return;
        };
        let rows = if child {
            &self.rows[self.branch.unwrap()].children
        } else {
            &self.rows
        };
        let mut rect: RECT = zeroed();
        GetClientRect(if child { self.child } else { self.root }, &mut rect);
        let available = rect.bottom - self.px(if child { 56 } else { 12 });
        let mut offset = if child { self.offset } else { self.root_offset }.min(position);
        while offset < position
            && indices[offset..=position]
                .iter()
                .map(|i| self.row_height(&rows[*i]))
                .sum::<i32>()
                > available
        {
            offset += 1;
        }
        if child {
            self.offset = offset;
        } else {
            self.root_offset = offset;
        }
    }
    unsafe fn close_child(&mut self) {
        if self.hover.is_some_and(|(child, _)| child) {
            self.hover = self.branch.map(|index| (false, index));
        }
        if !self.child.is_null() {
            DestroyWindow(self.child);
            self.child = null_mut();
        }
        self.branch = None;
    }
    unsafe fn open_child(&mut self, index: usize) {
        if self.branch == Some(index) {
            return;
        }
        self.close_child();
        self.branch = Some(index);
        self.query.clear();
        self.offset = 0;
        let mut root: RECT = zeroed();
        GetWindowRect(self.root, &mut root);
        let top = root.top
            + self.px(6)
            + self.rows[self.root_offset..index]
                .iter()
                .map(|r| self.row_height(r))
                .sum::<i32>();
        self.child = create(self.root, self, root.right - self.px(1), top, true);
        InvalidateRect(self.root, null(), 0);
    }
    unsafe fn choose(&mut self, child: bool, index: usize) {
        let row = if child {
            &self.rows[self.branch.unwrap()].children[index]
        } else {
            &self.rows[index]
        };
        if row.disabled || row.separator {
            return;
        }
        if !row.children.is_empty() {
            self.open_child(index);
        } else {
            self.selected = row.id;
            self.done = true;
        }
    }
    unsafe fn hit(&self, point: POINT) -> Option<(bool, usize)> {
        for (child, hwnd) in [(true, self.child), (false, self.root)] {
            if hwnd.is_null() {
                continue;
            }
            let mut rect: RECT = zeroed();
            GetWindowRect(hwnd, &mut rect);
            if PtInRect(&rect, point) == 0 {
                continue;
            }
            let mut y = rect.top + self.px(if child { 50 } else { 6 });
            let indices = if child {
                self.visible().into_iter().skip(self.offset).collect()
            } else {
                (self.root_offset..self.rows.len()).collect::<Vec<_>>()
            };
            for index in indices {
                let row = if child {
                    &self.rows[self.branch.unwrap()].children[index]
                } else {
                    &self.rows[index]
                };
                let height = self.row_height(row);
                if point.y >= y && point.y < y + height && point.y < rect.bottom - self.px(6) {
                    return Some((child, index));
                }
                y += height;
            }
        }
        None
    }
}

unsafe fn create(owner: HWND, state: &mut Popup, mut x: i32, mut y: i32, child: bool) -> HWND {
    let class = wide("DeezNats.CustomPopup");
    let mut wc: WNDCLASSW = zeroed();
    wc.lpfnWndProc = Some(procedure);
    wc.style = CS_DROPSHADOW;
    wc.hInstance = GetModuleHandleW(null());
    wc.lpszClassName = class.as_ptr();
    wc.hCursor = LoadCursorW(null_mut(), IDC_ARROW);
    RegisterClassW(&wc);
    let width = state.px(if child { 320 } else { 290 });
    let content = if child {
        state.px(50)
            + state.rows[state.branch.unwrap()]
                .children
                .iter()
                .map(|r| state.row_height(r))
                .sum::<i32>()
    } else {
        state.px(6) + state.rows.iter().map(|r| state.row_height(r)).sum::<i32>()
    };
    let mut monitor: MONITORINFO = zeroed();
    monitor.cbSize = size_of::<MONITORINFO>() as u32;
    GetMonitorInfoW(
        MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST),
        &mut monitor,
    );
    let work = monitor.rcWork;
    let height = (content + state.px(6))
        .max(state.px(if child { 150 } else { 12 }))
        .min(work.bottom - work.top - state.px(12));
    if child && x + width > work.right {
        let mut r: RECT = zeroed();
        GetWindowRect(owner, &mut r);
        x = r.left - width + state.px(1);
    }
    x = x.min(work.right - width).max(work.left);
    y = y.min(work.bottom - height).max(work.top);
    let hwnd = CreateWindowExW(
        // Shell tray overflow is topmost; transient menus must appear above it.
        WS_EX_TOOLWINDOW | WS_EX_TOPMOST | if child { WS_EX_NOACTIVATE } else { 0 },
        class.as_ptr(),
        wide("").as_ptr(),
        WS_POPUP,
        x,
        y,
        width,
        height,
        owner,
        null_mut(),
        wc.hInstance,
        state as *mut Popup as *const c_void,
    );
    let region = CreateRoundRectRgn(0, 0, width + 1, height + 1, state.px(18), state.px(18));
    SetWindowRgn(hwnd, region, 0);
    ShowWindow(hwnd, SW_SHOWNOACTIVATE);
    hwnd
}

unsafe fn draw_text(dc: HDC, font: HFONT, value: &str, mut rect: RECT, color: u32, flags: u32) {
    let old = SelectObject(dc, font);
    SetBkMode(dc, TRANSPARENT as i32);
    SetTextColor(dc, color);
    DrawTextW(dc, wide(value).as_ptr(), -1, &mut rect, flags);
    SelectObject(dc, old);
}

unsafe fn paint_surface(hwnd: HWND, state: &Popup) {
    let mut ps: PAINTSTRUCT = zeroed();
    let dc = BeginPaint(hwnd, &mut ps);
    let mut rect: RECT = zeroed();
    GetClientRect(hwnd, &mut rect);
    let buffer = CreateCompatibleDC(dc);
    let bitmap = CreateCompatibleBitmap(dc, rect.right, rect.bottom);
    let old = SelectObject(buffer, bitmap);
    let background = CreateSolidBrush(state.palette[1]);
    FillRect(buffer, &rect, background);
    let pen = CreatePen(PS_SOLID, state.px(1), state.palette[3]);
    let old_pen = SelectObject(buffer, pen);
    let old_brush = SelectObject(buffer, GetStockObject(HOLLOW_BRUSH));
    RoundRect(
        buffer,
        0,
        0,
        rect.right,
        rect.bottom,
        state.px(18),
        state.px(18),
    );
    SelectObject(buffer, old_pen);
    SelectObject(buffer, old_brush);
    DeleteObject(pen);
    let child = hwnd == state.child;
    let mut y = state.px(if child { 50 } else { 6 });
    if child {
        let search = RECT {
            left: state.px(9),
            top: state.px(9),
            right: rect.right - state.px(9),
            bottom: state.px(43),
        };
        let brush = CreateSolidBrush(state.palette[2]);
        FillRect(buffer, &search, brush);
        DeleteObject(brush);
        let mut label = search;
        label.left += state.px(10);
        let text = if state.query.is_empty() {
            &state.labels[0]
        } else {
            &state.query
        };
        draw_text(
            buffer,
            state.fonts[0],
            text,
            label,
            state.palette[5],
            DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS,
        );
    }
    let indices = if child {
        state.visible().into_iter().skip(state.offset).collect()
    } else {
        (state.root_offset..state.rows.len()).collect::<Vec<_>>()
    };
    for index in indices {
        let row = if child {
            &state.rows[state.branch.unwrap()].children[index]
        } else {
            &state.rows[index]
        };
        let height = state.row_height(row);
        if y + height > rect.bottom - state.px(6) {
            break;
        }
        let mut r = RECT {
            left: state.px(6),
            top: y,
            right: rect.right - state.px(6),
            bottom: y + height,
        };
        if row.separator {
            r.left += state.px(8);
            r.right -= state.px(8);
            r.top += height / 2;
            r.bottom = r.top + 1;
            let brush = CreateSolidBrush(state.palette[3]);
            FillRect(buffer, &r, brush);
            DeleteObject(brush);
        } else {
            if state.hover == Some((child, index)) || (!child && state.branch == Some(index)) {
                let brush = CreateSolidBrush(state.palette[2]);
                FillRect(buffer, &r, brush);
                DeleteObject(brush);
            }
            let color = if row.disabled {
                state.palette[5]
            } else {
                state.palette[4]
            };
            r.left += state.px(9);
            r.right = r.left + state.px(22);
            draw_text(
                buffer,
                state.fonts[1],
                row.icon,
                r,
                color,
                DT_CENTER | DT_VCENTER | DT_SINGLELINE,
            );
            r.left += state.px(30);
            r.right = rect.right - state.px(30);
            draw_text(
                buffer,
                state.fonts[0],
                &row.title,
                r,
                color,
                DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS,
            );
            if !row.children.is_empty() {
                r.left = rect.right - state.px(26);
                r.right = rect.right - state.px(10);
                draw_text(
                    buffer,
                    state.fonts[0],
                    "\u{203a}",
                    r,
                    color,
                    DT_CENTER | DT_VCENTER | DT_SINGLELINE,
                );
            }
        }
        y += height;
    }
    let no_matches = child
        && state.receiver.is_none()
        && !state.query.is_empty()
        && !state
            .visible()
            .iter()
            .any(|i| state.rows[state.branch.unwrap()].children[*i].id >= 1000);
    if child && (!state.status.is_empty() || no_matches) {
        let r = RECT {
            left: state.px(16),
            top: y,
            right: rect.right - state.px(12),
            bottom: y + state.px(36),
        };
        draw_text(
            buffer,
            state.fonts[0],
            if no_matches {
                &state.labels[2]
            } else {
                &state.status
            },
            r,
            state.palette[5],
            DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS,
        );
    }
    if child {
        let visible = state.visible();
        let rows = &state.rows[state.branch.unwrap()].children;
        let total: i32 = visible.iter().map(|i| state.row_height(&rows[*i])).sum();
        let top = state.px(50);
        let available = rect.bottom - top - state.px(8);
        if total > available && available > 0 {
            let height = (available * available / total).max(state.px(20));
            let before: i32 = visible
                .iter()
                .take(state.offset)
                .map(|i| state.row_height(&rows[*i]))
                .sum();
            let y = top + ((available - height) * before / total).min(available - height);
            let thumb = RECT {
                left: rect.right - state.px(5),
                top: y,
                right: rect.right - state.px(2),
                bottom: y + height,
            };
            let brush = CreateSolidBrush(state.palette[5]);
            FillRect(buffer, &thumb, brush);
            DeleteObject(brush);
        }
    }
    BitBlt(dc, 0, 0, rect.right, rect.bottom, buffer, 0, 0, SRCCOPY);
    SelectObject(buffer, old);
    DeleteObject(bitmap);
    DeleteDC(buffer);
    DeleteObject(background);
    EndPaint(hwnd, &ps);
}

unsafe extern "system" fn procedure(hwnd: HWND, msg: u32, wp: usize, lp: isize) -> isize {
    if msg == WM_NCCREATE {
        let cs = &*(lp as *const CREATESTRUCTW);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, cs.lpCreateParams as isize);
    }
    let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Popup;
    if pointer.is_null() {
        return DefWindowProcW(hwnd, msg, wp, lp);
    }
    let state = &mut *pointer;
    match msg {
        WM_PAINT => {
            paint_surface(hwnd, state);
            0
        }
        WM_ERASEBKGND => 1,
        // Keep keyboard focus and mouse capture on the root while interacting
        // with its owned submenu (including the search field).
        WM_MOUSEACTIVATE => MA_NOACTIVATE as isize,
        WM_MOUSEMOVE => {
            let mut p: POINT = zeroed();
            GetCursorPos(&mut p);
            let hit = state.hit(p);
            if hit != state.hover {
                state.hover = hit;
                if let Some((false, i)) = hit {
                    if !state.rows[i].children.is_empty() {
                        state.open_child(i);
                    } else {
                        state.close_child();
                    }
                }
                InvalidateRect(state.root, null(), 0);
                if !state.child.is_null() {
                    InvalidateRect(state.child, null(), 0);
                }
            }
            0
        }
        WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_LBUTTONUP | WM_RBUTTONUP => {
            let mut p: POINT = zeroed();
            GetCursorPos(&mut p);
            if let Some((child, i)) = state.hit(p) {
                if msg == WM_LBUTTONUP || msg == WM_RBUTTONUP {
                    state.choose(child, i);
                }
            } else {
                let mut r: RECT = zeroed();
                if state.child.is_null() || {
                    GetWindowRect(state.child, &mut r);
                    PtInRect(&r, p) == 0
                } {
                    state.done = true;
                }
            }
            0
        }
        WM_MOUSEWHEEL => {
            let mut point: POINT = zeroed();
            GetCursorPos(&mut point);
            let mut rect: RECT = zeroed();
            let child = !state.child.is_null() && {
                GetWindowRect(state.child, &mut rect);
                PtInRect(&rect, point) != 0
            };
            if !child {
                GetWindowRect(state.root, &mut rect);
                if PtInRect(&rect, point) == 0 {
                    return 0;
                }
            }
            {
                let count = if child {
                    state.visible().len()
                } else {
                    state.rows.len()
                };
                let offset = if child {
                    &mut state.offset
                } else {
                    &mut state.root_offset
                };
                if (wp >> 16) as i16 > 0 {
                    *offset = offset.saturating_sub(3);
                } else {
                    *offset = (*offset + 3).min(count.saturating_sub(1));
                }
                state.hover = None;
                if !child {
                    state.close_child();
                }
                InvalidateRect(if child { state.child } else { state.root }, null(), 0);
            }
            0
        }
        WM_CHAR => {
            if state.branch.is_some() {
                if wp == 8 {
                    state.query.pop();
                } else if let Some(c) = char::from_u32(wp as u32) {
                    if !c.is_control() {
                        state.query.push(c);
                    }
                }
                state.offset = 0;
                state.hover = None;
                InvalidateRect(state.child, null(), 0);
            }
            0
        }
        WM_KEYDOWN => {
            match wp as u16 {
                27 => {
                    if state.branch.is_some() {
                        state.close_child();
                    } else {
                        state.done = true;
                    }
                }
                VK_LEFT => state.close_child(),
                VK_RIGHT => {
                    if let Some((false, i)) = state.hover {
                        if !state.rows[i].children.is_empty() {
                            state.open_child(i);
                        }
                    }
                }
                13 => {
                    if let Some((child, i)) = state.hover {
                        state.choose(child, i);
                    }
                }
                VK_DOWN | VK_UP => {
                    let child = state.branch.is_some();
                    let rows = if child {
                        &state.rows[state.branch.unwrap()].children
                    } else {
                        &state.rows
                    };
                    let indices: Vec<usize> = (if child {
                        state.visible()
                    } else {
                        (0..rows.len()).collect()
                    })
                    .into_iter()
                    .filter(|i| !rows[*i].separator && !rows[*i].disabled)
                    .collect();
                    if !indices.is_empty() {
                        let pos = state.hover.and_then(|(c, i)| {
                            if c == child {
                                indices.iter().position(|n| *n == i)
                            } else {
                                None
                            }
                        });
                        let next = match pos {
                            Some(p) if wp as u16 == VK_UP => {
                                (p + indices.len() - 1) % indices.len()
                            }
                            Some(p) => (p + 1) % indices.len(),
                            None => 0,
                        };
                        state.hover = Some((child, indices[next]));
                        if child {
                            state.reveal(true, indices[next]);
                            InvalidateRect(state.child, null(), 0);
                        } else {
                            state.reveal(false, indices[next]);
                            InvalidateRect(state.root, null(), 0);
                        }
                    }
                }
                _ => {}
            }
            0
        }
        WM_ACTIVATE
            if wp & 0xffff == WA_INACTIVE as usize
                && (lp == 0 || (lp as HWND != state.root && lp as HWND != state.child)) =>
        {
            state.done = true;
            0
        }
        WM_CAPTURECHANGED => {
            state.done = true;
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
    menu: HMENU,
    x: i32,
    y: i32,
    context: bool,
    receiver: Option<Receiver<PlaylistResult>>,
) -> (i32, Vec<crate::catalog::Entry>) {
    let app = &*(GetWindowLongPtrW(owner, GWLP_USERDATA) as *const App);
    let mut state = Box::new(Popup {
        root: null_mut(),
        child: null_mut(),
        rows: items(menu, context, false),
        branch: None,
        hover: None,
        query: String::new(),
        offset: 0,
        root_offset: 0,
        selected: 0,
        done: false,
        scale: GetDpiForWindow(owner) as f32 / 96.0,
        fonts: [app.fonts[0], app.fonts[4]],
        palette: app.palette(),
        status: if receiver.is_some() {
            app.ui.t("Loading playlists…").into()
        } else {
            String::new()
        },
        receiver,
        playlists: Vec::new(),
        labels: [
            app.ui.t("Search playlists…").into(),
            app.ui.t("No editable playlists").into(),
            app.ui.t("No matching playlists").into(),
        ],
    });
    let previous_focus = GetFocus();
    state.root = create(owner, &mut state, x, y, false);
    if state.root.is_null() {
        return (0, Vec::new());
    }
    // A custom popup needs the same input isolation as a native modal menu.
    // Capture alone does not stop wheel messages or focus on owner controls.
    let owner_enabled = IsWindowEnabled(owner) != 0;
    if owner_enabled {
        EnableWindow(owner, 0);
    }
    SetForegroundWindow(state.root);
    SetFocus(state.root);
    SetCapture(state.root);
    SetTimer(state.root, 1, 50, None);
    let mut message: MSG = zeroed();
    while !state.done {
        let result = GetMessageW(&mut message, null_mut(), 0, 0);
        if result <= 0 {
            if result == 0 {
                PostQuitMessage(message.wParam as i32);
            }
            break;
        }
        if message.message == WM_TIMER && message.hwnd == state.root {
            if let Some(result) = state.receiver.as_ref().and_then(|r| r.try_recv().ok()) {
                state.receiver = None;
                match result {
                    Ok(playlists) => {
                        state.playlists = playlists;
                        if let Some(branch) = state.rows.iter_mut().find(|r| !r.children.is_empty())
                        {
                            branch.children.retain(|r| r.id < 1000);
                            branch
                                .children
                                .extend(state.playlists.iter().enumerate().map(|(i, p)| Item {
                                    title: p.title.clone(),
                                    id: 1000 + i as i32,
                                    disabled: false,
                                    separator: false,
                                    icon: "",
                                    children: Vec::new(),
                                }));
                        }
                        state.status = if state.playlists.is_empty() {
                            state.labels[1].clone()
                        } else {
                            String::new()
                        };
                    }
                    Err(error) => state.status = error,
                }
                if let Some(index) = state.branch {
                    let query = state.query.clone();
                    state.close_child();
                    state.open_child(index);
                    state.query = query;
                    state.hover = None;
                    InvalidateRect(state.child, null(), 0);
                }
            }
        }
        if (WM_MOUSEFIRST..=WM_MOUSELAST).contains(&message.message) {
            // Windows can target wheel input at the window under the pointer,
            // even while the popup has capture. Route all mouse input here.
            message.hwnd = state.root;
        } else if message.hwnd != state.root
            && message.hwnd != state.child
            && ((WM_KEYFIRST..=WM_KEYLAST).contains(&message.message)
                || matches!(message.message, WM_MOUSEHOVER | WM_MOUSELEAVE))
        {
            continue;
        }
        TranslateMessage(&message);
        DispatchMessageW(&message);
    }
    KillTimer(state.root, 1);
    ReleaseCapture();
    state.close_child();
    DestroyWindow(state.root);
    if owner_enabled {
        EnableWindow(owner, 1);
    }
    if IsWindow(previous_focus) != 0 {
        SetFocus(previous_focus);
    }
    (state.selected, std::mem::take(&mut state.playlists))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: i32, title: &str) -> Item {
        Item {
            title: title.into(),
            id,
            disabled: false,
            separator: false,
            icon: "",
            children: Vec::new(),
        }
    }

    fn fixture() -> Popup {
        let mut branch = row(0, "Add to playlist");
        branch.children = vec![
            row(13, "New playlist"),
            row(1000, "GECE"),
            row(1001, "Road Trip"),
            row(1002, "Gece mix"),
        ];
        Popup {
            root: null_mut(),
            child: null_mut(),
            rows: vec![branch],
            branch: Some(0),
            hover: None,
            query: String::new(),
            offset: 0,
            root_offset: 0,
            selected: 0,
            done: false,
            scale: 1.0,
            fonts: [null_mut(); 2],
            palette: colors(false),
            receiver: None,
            playlists: Vec::new(),
            status: String::new(),
            labels: [
                "Search playlists…".into(),
                "No editable playlists".into(),
                "No matching playlists".into(),
            ],
        }
    }

    #[test]
    fn search_keeps_new_playlist_and_original_action_ids() {
        let mut popup = fixture();
        popup.query = "gece".into();
        assert_eq!(popup.visible(), vec![0, 1, 3]);
        unsafe {
            popup.choose(true, 3);
        }
        assert_eq!(popup.selected, 1002);
        assert!(popup.done);
    }

    #[test]
    fn empty_search_result_still_allows_new_playlist() {
        let mut popup = fixture();
        popup.query = "no matches".into();
        assert_eq!(popup.visible(), vec![0]);
    }

    #[test]
    fn disabled_provider_actions_cannot_be_selected() {
        let mut popup = fixture();
        popup.rows[0].children[1].disabled = true;
        unsafe {
            popup.choose(true, 1);
        }
        assert!(!popup.done);
        assert_eq!(popup.selected, 0);
    }

    #[test]
    fn closing_cascade_restores_the_root_keyboard_target() {
        let mut popup = fixture();
        popup.hover = Some((true, 2));
        unsafe {
            popup.close_child();
        }
        assert_eq!(popup.branch, None);
        assert_eq!(popup.hover, Some((false, 0)));
    }

    #[test]
    fn custom_windows_render_and_accept_keyboard_search_without_native_tracking() {
        unsafe {
            let mut popup = Box::new(fixture());
            popup.branch = None;
            popup.fonts = [GetStockObject(DEFAULT_GUI_FONT); 2];
            popup.root = create(null_mut(), &mut popup, 100, 100, false);
            assert!(!popup.root.is_null());
            popup.open_child(0);
            assert!(!popup.child.is_null());
            let mut class = [0u16; 80];
            let length = GetClassNameW(popup.child, class.as_mut_ptr(), class.len() as i32);
            assert_eq!(
                String::from_utf16_lossy(&class[..length as usize]),
                "DeezNats.CustomPopup"
            );
            UpdateWindow(popup.root);
            UpdateWindow(popup.child);
            for character in "road".chars() {
                SendMessageW(popup.root, WM_CHAR, character as usize, 0);
            }
            assert_eq!(popup.visible(), vec![0, 2]);
            SendMessageW(popup.root, WM_KEYDOWN, VK_DOWN as usize, 0);
            SendMessageW(popup.root, WM_KEYDOWN, VK_DOWN as usize, 0);
            SendMessageW(popup.root, WM_KEYDOWN, 13, 0);
            assert_eq!(popup.selected, 1001);
            assert!(popup.done);
            popup.close_child();
            DestroyWindow(popup.root);
        }
    }
}
