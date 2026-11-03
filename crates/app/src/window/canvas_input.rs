//! Canvas text fields. No EDIT window (visible or hidden) participates in editing.
use super::*;
use unicode_segmentation::UnicodeSegmentation;
use windows_sys::Win32::{
    System::{DataExchange::*, Memory::*},
    UI::Input::{Ime::*, KeyboardAndMouse::GetKeyState},
};

#[derive(Clone, Debug, Default, PartialEq)]
struct Snapshot {
    text: String,
    caret: usize,
    anchor: usize,
}

#[derive(Default)]
struct Editor {
    value: Snapshot,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    multiline: bool,
}
impl Editor {
    fn range(&self) -> std::ops::Range<usize> {
        self.value.anchor.min(self.value.caret)..self.value.anchor.max(self.value.caret)
    }
    fn boundary(&self, pos: usize) -> usize {
        self.value
            .text
            .grapheme_indices(true)
            .map(|(i, _)| i)
            .chain(std::iter::once(self.value.text.len()))
            .take_while(|&i| i <= pos)
            .last()
            .unwrap_or(0)
    }
    fn previous(&self, pos: usize) -> usize {
        self.value
            .text
            .grapheme_indices(true)
            .map(|(i, _)| i)
            .take_while(|&i| i < pos)
            .last()
            .unwrap_or(0)
    }
    fn next(&self, pos: usize) -> usize {
        self.value
            .text
            .grapheme_indices(true)
            .map(|(i, _)| i)
            .find(|&i| i > pos)
            .unwrap_or(self.value.text.len())
    }
    fn word(&self, pos: usize, forward: bool) -> usize {
        let words: Vec<_> = self.value.text.unicode_word_indices().collect();
        if forward {
            words
                .iter()
                .map(|(i, _)| *i)
                .find(|&i| i > pos)
                .unwrap_or(self.value.text.len())
        } else {
            words
                .iter()
                .map(|(i, _)| *i)
                .take_while(|&i| i < pos)
                .last()
                .unwrap_or(0)
        }
    }
    fn select(&mut self, pos: usize, extend: bool) {
        self.value.caret = self.boundary(pos.min(self.value.text.len()));
        if !extend {
            self.value.anchor = self.value.caret;
        }
    }
    fn replace(&mut self, text: &str) {
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        let text: String = text
            .chars()
            .filter(|&c| c != '\0' && (self.multiline || c != '\n'))
            .collect();
        let range = self.range();
        if self.value.text[range.clone()] == text {
            return;
        }
        self.undo.push(self.value.clone());
        if self.undo.len() > 256 {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.value.text.replace_range(range.clone(), &text);
        self.value.caret = range.start + text.len();
        // Inserting a combining mark may merge grapheme clusters.
        if self.boundary(self.value.caret) != self.value.caret {
            self.value.caret = self.next(self.value.caret);
        }
        self.value.anchor = self.value.caret;
    }
    fn history(&mut self, redo: bool) {
        let (from, to) = if redo {
            (&mut self.redo, &mut self.undo)
        } else {
            (&mut self.undo, &mut self.redo)
        };
        if let Some(value) = from.pop() {
            to.push(std::mem::replace(&mut self.value, value));
        }
    }
    fn delete(&mut self, backwards: bool, word: bool) {
        if self.range().is_empty() {
            let pos = self.value.caret;
            self.value.anchor = if word {
                self.word(pos, !backwards)
            } else if backwards {
                self.previous(pos)
            } else {
                self.next(pos)
            };
        }
        self.replace("");
    }
}

#[derive(Clone)]
struct Line {
    start: usize,
    end: usize,
    stops: Vec<(usize, i32)>,
}
struct Input {
    caret_visible: bool,
    editor: Editor,
    font: HFONT,
    hint: String,
    scroll_x: i32,
    scroll_y: i32,
    preferred_x: Option<i32>,
    dragging: bool,
    high_surrogate: Option<u16>,
    composing: bool,
    preedit: String,
    preedit_cursor: usize,
}
impl Input {
    fn new(multiline: bool) -> Self {
        Self {
            caret_visible: true,
            editor: Editor {
                multiline,
                ..Default::default()
            },
            font: null_mut(),
            hint: String::new(),
            scroll_x: 0,
            scroll_y: 0,
            preferred_x: None,
            dragging: false,
            high_surrogate: None,
            composing: false,
            preedit: String::new(),
            preedit_cursor: 0,
        }
    }
}

unsafe fn state(hwnd: HWND) -> *mut Input {
    GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Input
}
pub(super) unsafe fn composing(hwnd: HWND) -> bool {
    let mut class = [0u16; 64];
    GetClassNameW(hwnd, class.as_mut_ptr(), class.len() as i32);
    if String::from_utf16_lossy(&class).trim_end_matches('\0') != "DeezNatsCanvasInput" {
        return false;
    }
    let p = state(hwnd);
    !p.is_null() && (*p).composing
}
pub(super) unsafe fn create(parent: HWND, id: i32, font: HFONT) -> HWND {
    let class = wide("DeezNatsCanvasInput");
    let wc = WNDCLASSW {
        style: CS_DBLCLKS,
        lpfnWndProc: Some(proc),
        hInstance: GetModuleHandleW(null()),
        hCursor: LoadCursorW(null_mut(), IDC_IBEAM),
        lpszClassName: class.as_ptr(),
        ..zeroed()
    };
    RegisterClassW(&wc);
    let hwnd = CreateWindowExW(
        0,
        class.as_ptr(),
        wide("").as_ptr(),
        WS_CHILD | WS_TABSTOP,
        0,
        0,
        1,
        1,
        parent,
        id as HMENU,
        wc.hInstance,
        null(),
    );
    if !hwnd.is_null() {
        SendMessageW(hwnd, WM_SETFONT, font as usize, 0);
    }
    hwnd
}
unsafe fn text_at(ptr: *const u16) -> String {
    if ptr.is_null() {
        return String::new();
    }
    let mut n = 0;
    while *ptr.add(n) != 0 {
        n += 1;
    }
    String::from_utf16_lossy(std::slice::from_raw_parts(ptr, n))
}
unsafe fn copy(hwnd: HWND, s: &Input) -> bool {
    let range = s.editor.range();
    if range.is_empty() || OpenClipboard(hwnd) == 0 {
        return false;
    }
    let text = wide(&s.editor.value.text[range].replace('\n', "\r\n"));
    let handle = GlobalAlloc(GMEM_MOVEABLE, text.len() * 2);
    let mut ok = false;
    if !handle.is_null() {
        let p = GlobalLock(handle) as *mut u16;
        if !p.is_null() {
            std::ptr::copy_nonoverlapping(text.as_ptr(), p, text.len());
            GlobalUnlock(handle);
            if EmptyClipboard() != 0 && !SetClipboardData(13, handle).is_null() {
                ok = true;
            }
        }
        if !ok {
            GlobalFree(handle);
        }
    }
    CloseClipboard();
    ok
}
unsafe fn paste(hwnd: HWND, s: &mut Input) {
    if OpenClipboard(hwnd) == 0 {
        return;
    }
    let handle = GetClipboardData(13);
    if !handle.is_null() {
        let p = GlobalLock(handle) as *const u16;
        if !p.is_null() {
            let capacity = GlobalSize(handle) / 2;
            let slice = std::slice::from_raw_parts(p, capacity);
            let length = slice.iter().position(|&c| c == 0).unwrap_or(capacity);
            let value = String::from_utf16_lossy(&slice[..length]);
            GlobalUnlock(handle);
            s.editor.replace(&value);
        }
    }
    CloseClipboard();
}

unsafe fn width(dc: HDC, text: &str) -> i32 {
    let text: Vec<u16> = text.encode_utf16().collect();
    let mut size = zeroed();
    GetTextExtentPoint32W(dc, text.as_ptr(), text.len() as i32, &mut size);
    size.cx
}
unsafe fn layout(hwnd: HWND, s: &Input, dc: HDC) -> (Vec<Line>, i32, RECT, i32) {
    SelectObject(dc, s.font);
    let mut tm: TEXTMETRICW = zeroed();
    GetTextMetricsW(dc, &mut tm);
    let height = (tm.tmHeight + tm.tmExternalLeading + 2).max(1);
    let mut rect = zeroed();
    GetClientRect(hwnd, &mut rect);
    let pad = (GetDpiForWindow(hwnd).max(96) as i32 * 4 / 96).max(2);
    let available = (rect.right - pad * 2).max(1);
    let mut lines = Vec::new();
    let mut start = 0;
    let mut stops = vec![(0, 0)];
    let text = &s.editor.value.text;
    if !s.editor.multiline && !text.is_empty() {
        // One GDI measurement keeps long pasted searches linear rather than
        // measuring every increasingly long prefix separately.
        let utf16: Vec<u16> = text.encode_utf16().collect();
        let mut advances = vec![0; utf16.len()];
        let mut size = zeroed();
        GetTextExtentExPointW(
            dc,
            utf16.as_ptr(),
            utf16.len() as i32,
            i32::MAX,
            null_mut(),
            advances.as_mut_ptr(),
            &mut size,
        );
        let mut units = 0;
        for (i, g) in text.grapheme_indices(true) {
            units += g.encode_utf16().count();
            stops.push((i + g.len(), advances[units - 1]));
        }
        return (
            vec![Line {
                start: 0,
                end: text.len(),
                stops,
            }],
            height,
            rect,
            pad,
        );
    }
    for (i, g) in text.grapheme_indices(true) {
        let x = width(dc, &text[start..i + g.len()]);
        if g == "\n" || (s.editor.multiline && x > available && i > start) {
            lines.push(Line {
                start,
                end: i,
                stops,
            });
            start = if g == "\n" { i + 1 } else { i };
            stops = vec![(start, 0)];
        }
        if g != "\n" {
            stops.push((i + g.len(), width(dc, &text[start..i + g.len()])));
        }
    }
    lines.push(Line {
        start,
        end: text.len(),
        stops,
    });
    (lines, height, rect, pad)
}
fn position(lines: &[Line], caret: usize) -> (usize, i32) {
    let row = lines.iter().rposition(|l| l.start <= caret).unwrap_or(0);
    let x = lines[row]
        .stops
        .iter()
        .find(|(i, _)| *i == caret)
        .map_or(0, |(_, x)| *x);
    (row, x)
}
fn nearest(line: &Line, x: i32) -> usize {
    line.stops
        .iter()
        .min_by_key(|(_, px)| (px - x).abs())
        .map_or(line.start, |(i, _)| *i)
}
unsafe fn hit(hwnd: HWND, s: &Input, x: i32, y: i32) -> usize {
    let dc = GetDC(hwnd);
    let old = SelectObject(dc, s.font);
    let (lines, height, rect, pad) = layout(hwnd, s, dc);
    let top = if s.editor.multiline {
        pad
    } else {
        (rect.bottom - height).max(0) / 2
    };
    let row = ((y - top + s.scroll_y).max(0) / height) as usize;
    let pos = nearest(&lines[row.min(lines.len() - 1)], x - pad + s.scroll_x);
    SelectObject(dc, old);
    ReleaseDC(hwnd, dc);
    pos
}
unsafe fn refresh(hwnd: HWND, s: &mut Input) {
    s.caret_visible = true;
    let dc = GetDC(hwnd);
    let old = SelectObject(dc, s.font);
    let (lines, height, rect, pad) = layout(hwnd, s, dc);
    let (row, mut x) = position(
        &lines,
        if s.composing {
            s.editor.range().start
        } else {
            s.editor.value.caret
        },
    );
    if s.composing {
        let units: Vec<u16> = s.preedit.encode_utf16().take(s.preedit_cursor).collect();
        x += width(dc, &String::from_utf16_lossy(&units));
    }
    let visible_w = (rect.right - pad * 2 - 2).max(1);
    if s.editor.multiline {
        s.scroll_x = 0;
    } else if x < s.scroll_x {
        s.scroll_x = x;
    } else if x > s.scroll_x + visible_w {
        s.scroll_x = x - visible_w;
    }
    let visible_h = (rect.bottom - pad * 2).max(height);
    let y = row as i32 * height;
    if y < s.scroll_y {
        s.scroll_y = y;
    }
    if y + height > s.scroll_y + visible_h {
        s.scroll_y = y + height - visible_h;
    }
    s.scroll_y = s
        .scroll_y
        .min((lines.len() as i32 * height - visible_h).max(0));
    let top = if s.editor.multiline {
        pad
    } else {
        (rect.bottom - height).max(0) / 2
    };
    if GetFocus() == hwnd {
        let caret_x = pad + x - s.scroll_x;
        let caret_y = top + y - s.scroll_y;
        SetCaretPos(caret_x, caret_y);
        let imc = ImmGetContext(hwnd);
        if !imc.is_null() {
            let form = COMPOSITIONFORM {
                dwStyle: CFS_POINT,
                ptCurrentPos: POINT {
                    x: caret_x,
                    y: caret_y,
                },
                rcArea: rect,
            };
            ImmSetCompositionWindow(imc, &form);
            let candidate = CANDIDATEFORM {
                dwIndex: 0,
                dwStyle: CFS_CANDIDATEPOS,
                ptCurrentPos: POINT {
                    x: caret_x,
                    y: caret_y + height,
                },
                rcArea: rect,
            };
            ImmSetCandidateWindow(imc, &candidate);
            ImmReleaseContext(hwnd, imc);
        }
    }
    SelectObject(dc, old);
    ReleaseDC(hwnd, dc);
    InvalidateRect(hwnd, null(), 0);
    invalidate_canvas(hwnd);
}
unsafe fn invalidate_canvas(hwnd: HWND) {
    let parent = GetParent(hwnd);
    let mut rect = zeroed();
    GetWindowRect(hwnd, &mut rect);
    MapWindowPoints(null_mut(), parent, (&mut rect as *mut RECT).cast(), 2);
    InvalidateRect(parent, &rect, 0);
}

/// Draw inside the application's composition surface, never in an overlaid GDI child.
pub(super) unsafe fn render(app: &App, dc: HDC) {
    for (id, rect) in [
        (SEARCH, app.scene.search),
        (NAME, app.scene.name),
        (DESCRIPTION, app.scene.description),
    ] {
        let (Some(rect), Some(&hwnd)) = (rect, app.edits.get(&id)) else {
            continue;
        };
        if GetWindowLongW(hwnd, GWL_STYLE) as u32 & WS_VISIBLE == 0 {
            continue;
        }
        let p = state(hwnd);
        if p.is_null() {
            continue;
        }
        let s = &*p;
        let measure = GetDC(hwnd);
        let old = SelectObject(measure, s.font);
        let (lines, height, client, pad) = layout(hwnd, s, measure);
        let bounds = app.rect(rect);
        let origin = bounds.left + pad - s.scroll_x;
        let top = bounds.top
            + if s.editor.multiline {
                pad
            } else {
                (client.bottom - height).max(0) / 2
            };
        let palette = app.palette();
        let saved = SaveDC(dc);
        IntersectClipRect(
            dc,
            bounds.left + pad,
            bounds.top,
            bounds.right - pad,
            bounds.bottom,
        );
        let line_text = |line: &Line, y: i32, color: u32| {
            let text = &s.editor.value.text[line.start..line.end];
            let r = RECT {
                left: origin,
                top: y,
                right: origin + width(measure, text).max(client.right) + 4,
                bottom: y + height,
            };
            draw_text(
                dc,
                s.font,
                text,
                r,
                color,
                DT_LEFT | DT_NOPREFIX | crate::drawing::LYRICS_UNTRIMMED,
            );
        };
        let selection = s.editor.range();
        for (row, line) in lines.iter().enumerate() {
            let y = top + row as i32 * height - s.scroll_y;
            if y + height <= bounds.top || y >= bounds.bottom {
                continue;
            }
            line_text(
                line,
                y,
                if IsWindowEnabled(hwnd) != 0 {
                    palette[4]
                } else {
                    palette[5]
                },
            );
            if !selection.is_empty() && selection.start <= line.end && selection.end > line.start {
                let a = selection.start.max(line.start);
                let b = selection.end.min(line.end);
                let x1 = width(measure, &s.editor.value.text[line.start..a]);
                let x2 = width(measure, &s.editor.value.text[line.start..b])
                    + if selection.end > line.end { 3 } else { 0 };
                let r = RECT {
                    left: origin + x1,
                    top: y,
                    right: origin + x2,
                    bottom: y + height,
                };
                crate::drawing::fill(
                    dc,
                    r,
                    crate::drawing::argb(
                        if GetFocus() == hwnd {
                            rgb(48, 104, 190)
                        } else {
                            rgb(85, 85, 85)
                        },
                        255,
                    ),
                    0.,
                );
                let selected = SaveDC(dc);
                IntersectClipRect(dc, r.left, r.top, r.right, r.bottom);
                line_text(line, y, rgb(255, 255, 255));
                RestoreDC(dc, selected);
            }
        }
        if s.editor.value.text.is_empty() && s.preedit.is_empty() {
            draw_text(
                dc,
                s.font,
                &s.hint,
                RECT {
                    left: bounds.left + pad,
                    top,
                    right: bounds.right - pad,
                    bottom: top + height,
                },
                palette[5],
                DT_LEFT | DT_NOPREFIX,
            );
        }
        let (row, x) = position(
            &lines,
            if s.composing {
                selection.start
            } else {
                s.editor.value.caret
            },
        );
        let mut caret_x = origin + x;
        let y = top + row as i32 * height - s.scroll_y;
        if !s.preedit.is_empty() {
            let w = width(measure, &s.preedit);
            draw_text(
                dc,
                s.font,
                &s.preedit,
                RECT {
                    left: caret_x,
                    top: y,
                    right: caret_x + w + 4,
                    bottom: y + height,
                },
                palette[4],
                DT_LEFT | DT_NOPREFIX | crate::drawing::LYRICS_UNTRIMMED,
            );
            crate::drawing::fill(
                dc,
                RECT {
                    left: caret_x,
                    top: y + height - 1,
                    right: caret_x + w,
                    bottom: y + height,
                },
                crate::drawing::argb(palette[4], 255),
                0.,
            );
            let units: Vec<_> = s.preedit.encode_utf16().take(s.preedit_cursor).collect();
            caret_x += width(measure, &String::from_utf16_lossy(&units));
        }
        if GetFocus() == hwnd && s.caret_visible {
            crate::drawing::fill(
                dc,
                RECT {
                    left: caret_x,
                    top: y,
                    right: caret_x + app.px(1).max(1),
                    bottom: y + height - 2,
                },
                crate::drawing::argb(palette[4], 255),
                0.,
            );
        }
        RestoreDC(dc, saved);
        SelectObject(measure, old);
        ReleaseDC(hwnd, measure);
    }
}
unsafe fn paint(hwnd: HWND, s: &Input, dc: HDC) {
    let mut rect = zeroed();
    GetClientRect(hwnd, &mut rect);
    let parent = GetWindowLongPtrW(GetParent(hwnd), GWLP_USERDATA) as *mut App;
    let palette = if parent.is_null() {
        [
            0,
            rgb(30, 30, 30),
            0,
            rgb(70, 110, 200),
            rgb(240, 240, 240),
            rgb(150, 150, 150),
        ]
    } else {
        (*parent).palette()
    };
    let background = CreateSolidBrush(palette[1]);
    FillRect(dc, &rect, background);
    DeleteObject(background);
    if GetDlgCtrlID(hwnd) == SEARCH && !parent.is_null() {
        if let Some(back) = (*parent)
            .search_backdrop
            .borrow()
            .as_ref()
            .filter(|b| b.width == rect.right && b.height == rect.bottom)
        {
            let info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: back.width,
                    biHeight: -back.height,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB,
                    ..zeroed()
                },
                ..zeroed()
            };
            SetDIBitsToDevice(
                dc,
                0,
                0,
                back.width as u32,
                back.height as u32,
                0,
                0,
                0,
                back.height as u32,
                back.pixels.as_ptr().cast(),
                &info,
                DIB_RGB_COLORS,
            );
        }
    }
    let old = SelectObject(dc, s.font);
    let (lines, height, rect, pad) = layout(hwnd, s, dc);
    let top = if s.editor.multiline {
        pad
    } else {
        (rect.bottom - height).max(0) / 2
    };
    IntersectClipRect(dc, pad, 0, (rect.right - pad).max(pad), rect.bottom);
    SetBkMode(dc, TRANSPARENT as i32);
    let selection = s.editor.range();
    let focused = GetFocus() == hwnd;
    for (row, line) in lines.iter().enumerate() {
        let y = top + row as i32 * height - s.scroll_y;
        if y + height < 0 || y >= rect.bottom {
            continue;
        }
        let origin = pad - s.scroll_x;
        let text = &s.editor.value.text[line.start..line.end];
        SetTextColor(
            dc,
            if IsWindowEnabled(hwnd) != 0 {
                palette[4]
            } else {
                palette[5]
            },
        );
        let text = wide(text);
        TextOutW(dc, origin, y, text.as_ptr(), text.len() as i32 - 1);
        if !selection.is_empty() && selection.start <= line.end && selection.end > line.start {
            let a = selection.start.max(line.start);
            let b = selection.end.min(line.end);
            let x1 = width(dc, &s.editor.value.text[line.start..a]);
            let x2 = width(dc, &s.editor.value.text[line.start..b])
                + if selection.end > line.end { 3 } else { 0 };
            let selected = RECT {
                left: origin + x1,
                top: y,
                right: origin + x2,
                bottom: y + height,
            };
            let brush = CreateSolidBrush(if focused {
                rgb(48, 104, 190)
            } else {
                rgb(85, 85, 85)
            });
            FillRect(dc, &selected, brush);
            DeleteObject(brush);
            SaveDC(dc);
            IntersectClipRect(
                dc,
                selected.left,
                selected.top,
                selected.right,
                selected.bottom,
            );
            SetTextColor(dc, rgb(255, 255, 255));
            TextOutW(dc, origin, y, text.as_ptr(), text.len() as i32 - 1);
            RestoreDC(dc, -1);
        }
    }
    if s.editor.value.text.is_empty() && s.preedit.is_empty() {
        SetTextColor(dc, palette[5]);
        let hint = wide(&s.hint);
        let mut r = RECT {
            left: pad,
            top,
            right: rect.right - pad,
            bottom: rect.bottom,
        };
        DrawTextW(
            dc,
            hint.as_ptr(),
            -1,
            &mut r,
            DT_LEFT | DT_NOPREFIX | DT_SINGLELINE | DT_END_ELLIPSIS,
        );
    }
    if !s.preedit.is_empty() {
        let (row, x) = position(&lines, s.editor.range().start);
        let x = pad + x - s.scroll_x;
        let y = top + row as i32 * height - s.scroll_y;
        let w = width(dc, &s.preedit);
        let r = RECT {
            left: x,
            top: y,
            right: x + w,
            bottom: y + height,
        };
        let brush = CreateSolidBrush(palette[1]);
        FillRect(dc, &r, brush);
        DeleteObject(brush);
        SetTextColor(dc, palette[4]);
        let text = wide(&s.preedit);
        TextOutW(dc, x, y, text.as_ptr(), text.len() as i32 - 1);
        let pen = CreatePen(PS_SOLID, 1, palette[4]);
        let prev = SelectObject(dc, pen);
        MoveToEx(dc, x, y + height - 1, null_mut());
        LineTo(dc, x + w, y + height - 1);
        SelectObject(dc, prev);
        DeleteObject(pen);
    }
    SelectObject(dc, old);
}

unsafe fn ime_string(imc: HIMC, kind: u32) -> String {
    let bytes = ImmGetCompositionStringW(imc, kind, null_mut(), 0);
    if bytes <= 0 {
        return String::new();
    }
    let mut text = vec![0u16; bytes as usize / 2];
    ImmGetCompositionStringW(imc, kind, text.as_mut_ptr().cast(), bytes as u32);
    String::from_utf16_lossy(&text)
}
unsafe fn notify(hwnd: HWND) {
    // Post, rather than reenter the root while a mutable input state is borrowed.
    PostMessageW(
        GetParent(hwnd),
        WM_COMMAND,
        GetDlgCtrlID(hwnd) as usize | ((EN_CHANGE as usize) << 16),
        hwnd as isize,
    );
}
unsafe fn context_menu(hwnd: HWND, lp: isize) {
    let p = state(hwnd);
    let selected = !(*p).editor.range().is_empty();
    let enabled = IsWindowEnabled(hwnd) != 0;
    let undo = !(*p).editor.undo.is_empty();
    let redo = !(*p).editor.redo.is_empty();
    let menu = CreatePopupMenu();
    let parent = GetWindowLongPtrW(GetParent(hwnd), GWLP_USERDATA) as *mut App;
    for (id, label, active) in [
        (1, "Undo", enabled && undo),
        (2, "Redo", enabled && redo),
        (3, "Cut", enabled && selected),
        (4, "Copy", selected),
        (5, "Paste", enabled && IsClipboardFormatAvailable(13) != 0),
        (6, "Select all", true),
    ] {
        AppendMenuW(
            menu,
            MF_STRING | if active { MF_ENABLED } else { MF_GRAYED },
            id,
            wide(if parent.is_null() {
                label
            } else {
                (*parent).ui.t(label)
            })
            .as_ptr(),
        );
    }
    let mut point = POINT {
        x: lp as i16 as i32,
        y: (lp >> 16) as i16 as i32,
    };
    if lp == -1 {
        GetCaretPos(&mut point);
        ClientToScreen(hwnd, &mut point);
    }
    let choice = TrackPopupMenu(
        menu,
        TPM_RETURNCMD | TPM_RIGHTBUTTON,
        point.x,
        point.y,
        0,
        hwnd,
        null(),
    );
    DestroyMenu(menu);
    match choice {
        1 => {
            SendMessageW(hwnd, WM_UNDO, 0, 0);
        }
        2 => {
            (*p).editor.history(true);
            refresh(hwnd, &mut *p);
            notify(hwnd);
        }
        3 => {
            SendMessageW(hwnd, WM_CUT, 0, 0);
        }
        4 => {
            SendMessageW(hwnd, WM_COPY, 0, 0);
        }
        5 => {
            SendMessageW(hwnd, WM_PASTE, 0, 0);
        }
        6 => {
            let s = &mut *p;
            s.editor.value.anchor = 0;
            s.editor.value.caret = s.editor.value.text.len();
            refresh(hwnd, s);
        }
        _ => {}
    }
}
unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wp: usize, lp: isize) -> isize {
    if msg == WM_NCCREATE {
        let create = &*(lp as *const CREATESTRUCTW);
        SetWindowLongPtrW(
            hwnd,
            GWLP_USERDATA,
            Box::into_raw(Box::new(Input::new(create.hMenu as i32 == DESCRIPTION))) as isize,
        );
        return 1;
    }
    let p = state(hwnd);
    if p.is_null() {
        return DefWindowProcW(hwnd, msg, wp, lp);
    }
    if msg == WM_NCDESTROY {
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
        drop(Box::from_raw(p));
        return DefWindowProcW(hwnd, msg, wp, lp);
    }
    if msg == WM_CONTEXTMENU {
        SetFocus(hwnd);
        context_menu(hwnd, lp);
        return 0;
    }
    // These APIs can synchronously send messages back into this window.
    // Complete them before taking the mutable state reference below.
    if matches!(msg, WM_LBUTTONDOWN | WM_LBUTTONDBLCLK) {
        SetFocus(hwnd);
        SetCapture(hwnd);
    }
    if msg == WM_KILLFOCUS {
        let imc = ImmGetContext(hwnd);
        if !imc.is_null() {
            ImmNotifyIME(imc, NI_COMPOSITIONSTR, CPS_COMPLETE, 0);
            ImmReleaseContext(hwnd, imc);
        }
    }
    if msg == WM_LBUTTONUP && GetCapture() == hwnd {
        ReleaseCapture();
    }
    let s = &mut *p;
    let before = s.editor.value.text.clone();
    // AltGr often appears as Ctrl+Alt and must still produce WM_CHAR text.
    let ctrl = GetKeyState(0x11) < 0 && GetKeyState(0x12) >= 0;
    let shift = GetKeyState(0x10) < 0;
    match msg {
        WM_GETTEXTLENGTH => return s.editor.value.text.encode_utf16().count() as isize,
        WM_GETTEXT => {
            if wp == 0 || lp == 0 {
                return 0;
            }
            let text: Vec<u16> = s.editor.value.text.encode_utf16().collect();
            let n = text.len().min(wp - 1);
            std::ptr::copy_nonoverlapping(text.as_ptr(), lp as *mut u16, n);
            *(lp as *mut u16).add(n) = 0;
            return n as isize;
        }
        WM_SETTEXT => {
            s.editor.value.anchor = 0;
            s.editor.value.caret = s.editor.value.text.len();
            s.editor.replace(&text_at(lp as *const u16));
            s.editor.undo.clear();
            s.editor.redo.clear();
            s.editor.select(0, false);
            s.scroll_x = 0;
            s.scroll_y = 0;
        }
        WM_GETFONT => return s.font as isize,
        WM_SETFONT => {
            s.font = wp as HFONT;
        }
        EM_SETCUEBANNER => {
            s.hint = text_at(lp as *const u16);
        }
        EM_GETCUEBANNER => {
            if wp != 0 && lp > 0 {
                let text = wide(&s.hint);
                let n = (lp as usize - 1).min(text.len() - 1);
                std::ptr::copy_nonoverlapping(text.as_ptr(), wp as *mut u16, n);
                *(wp as *mut u16).add(n) = 0;
            }
            return 1;
        }
        EM_SETSEL => {
            let byte_at = |units: usize| {
                let mut count = 0;
                s.editor
                    .value
                    .text
                    .char_indices()
                    .find_map(|(i, c)| {
                        if count >= units {
                            Some(i)
                        } else {
                            count += c.len_utf16();
                            None
                        }
                    })
                    .unwrap_or(s.editor.value.text.len())
            };
            if wp == usize::MAX {
                s.editor.value.anchor = s.editor.value.caret;
            } else {
                let a = byte_at(wp);
                let b = byte_at(lp as usize);
                s.editor.select(a, false);
                s.editor.select(b, true);
            }
        }
        EM_GETSEL => {
            let range = s.editor.range();
            let a = s.editor.value.text[..range.start].encode_utf16().count() as u32;
            let b = s.editor.value.text[..range.end].encode_utf16().count() as u32;
            if wp != 0 {
                *(wp as *mut u32) = a;
            }
            if lp != 0 {
                *(lp as *mut u32) = b;
            }
            return if a > 65535 || b > 65535 {
                -1
            } else {
                (a | b << 16) as isize
            };
        }
        EM_REPLACESEL => {
            if IsWindowEnabled(hwnd) != 0 {
                s.editor.replace(&text_at(lp as *const u16));
            }
        }
        EM_CANUNDO => return (!s.editor.undo.is_empty()) as isize,
        EM_EMPTYUNDOBUFFER => {
            s.editor.undo.clear();
            s.editor.redo.clear();
        }
        WM_GETDLGCODE => {
            let mut flags = DLGC_WANTCHARS | DLGC_WANTARROWS;
            if lp != 0 {
                let m = &*(lp as *const MSG);
                if m.wParam == 13 || (s.composing && m.wParam != 9) {
                    flags |= DLGC_WANTALLKEYS;
                }
            }
            return flags as isize;
        }
        WM_SETFOCUS => {
            let dc = GetDC(hwnd);
            let old = SelectObject(dc, s.font);
            let mut tm = zeroed();
            GetTextMetricsW(dc, &mut tm);
            SelectObject(dc, old);
            ReleaseDC(hwnd, dc);
            CreateCaret(hwnd, null_mut(), 2, tm.tmHeight.max(1));
            SetTimer(hwnd, 1, GetCaretBlinkTime().max(100), None);
        }
        WM_KILLFOCUS => {
            KillTimer(hwnd, 1);
            s.composing = false;
            s.preedit.clear();
            s.high_surrogate = None;
            DestroyCaret();
        }
        WM_ERASEBKGND => return 1,
        WM_PAINT | WM_PRINTCLIENT => {
            let parent = GetWindowLongPtrW(GetParent(hwnd), GWLP_USERDATA) as *mut App;
            if msg == WM_PAINT && !parent.is_null() {
                // The parent canvas owns all visible pixels. A child GDI surface
                // can be hidden by DirectComposition even while WS_VISIBLE is set.
                let mut ps = zeroed();
                BeginPaint(hwnd, &mut ps);
                EndPaint(hwnd, &ps);
                return 0;
            }
            let mut ps = zeroed();
            let dc = if msg == WM_PAINT {
                BeginPaint(hwnd, &mut ps)
            } else {
                wp as HDC
            };
            let mut rect = zeroed();
            GetClientRect(hwnd, &mut rect);
            let mem = CreateCompatibleDC(dc);
            let bitmap = CreateCompatibleBitmap(dc, rect.right.max(1), rect.bottom.max(1));
            let old = SelectObject(mem, bitmap);
            paint(hwnd, s, mem);
            BitBlt(dc, 0, 0, rect.right, rect.bottom, mem, 0, 0, SRCCOPY);
            SelectObject(mem, old);
            DeleteObject(bitmap);
            DeleteDC(mem);
            if msg == WM_PAINT {
                EndPaint(hwnd, &ps);
            }
            return 0;
        }
        WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => {
            let pos = hit(hwnd, s, lp as i16 as i32, (lp >> 16) as i16 as i32);
            s.editor.select(pos, shift);
            s.dragging = true;
            s.preferred_x = None;
            if msg == WM_LBUTTONDBLCLK {
                let target = if pos == s.editor.value.text.len() {
                    s.editor.previous(pos)
                } else {
                    pos
                };
                if let Some((start, part)) = s
                    .editor
                    .value
                    .text
                    .split_word_bound_indices()
                    .find(|(i, part)| *i <= target && target < *i + part.len())
                {
                    s.editor.value.anchor = start;
                    s.editor.value.caret = start + part.len();
                }
            }
        }
        WM_MOUSEMOVE if s.dragging => {
            let pos = hit(hwnd, s, lp as i16 as i32, (lp >> 16) as i16 as i32);
            s.editor.select(pos, true);
        }
        WM_LBUTTONUP => {
            s.dragging = false;
        }
        WM_CAPTURECHANGED => {
            s.dragging = false;
        }
        WM_COPY => {
            copy(hwnd, s);
            return 0;
        }
        WM_CUT => {
            if IsWindowEnabled(hwnd) != 0 && copy(hwnd, s) {
                s.editor.replace("");
            }
        }
        WM_PASTE => {
            if IsWindowEnabled(hwnd) != 0 {
                paste(hwnd, s);
            }
        }
        WM_CLEAR => {
            if IsWindowEnabled(hwnd) != 0 {
                s.editor.replace("");
            }
        }
        WM_UNDO => {
            if IsWindowEnabled(hwnd) != 0 {
                s.editor.history(false);
            }
        }
        WM_MOUSEWHEEL if s.editor.multiline => {
            let dc = GetDC(hwnd);
            let old = SelectObject(dc, s.font);
            let (lines, height, rect, pad) = layout(hwnd, s, dc);
            let delta = (wp >> 16) as u16 as i16 as i32;
            let max = (lines.len() as i32 * height - (rect.bottom - pad * 2)).max(0);
            s.scroll_y = (s.scroll_y - delta * height * 3 / 120).clamp(0, max);
            if GetFocus() == hwnd {
                let (row, x) = position(&lines, s.editor.value.caret);
                SetCaretPos(pad + x, pad + row as i32 * height - s.scroll_y);
            }
            SelectObject(dc, old);
            ReleaseDC(hwnd, dc);
            InvalidateRect(hwnd, null(), 0);
            invalidate_canvas(hwnd);
            return 0;
        }
        WM_TIMER if wp == 1 => {
            s.caret_visible = !s.caret_visible;
            invalidate_canvas(hwnd);
            return 0;
        }
        WM_KEYDOWN => {
            if IsWindowEnabled(hwnd) == 0 {
                return 0;
            }
            if s.composing {
                return DefWindowProcW(hwnd, msg, wp, lp);
            }
            let pos = s.editor.value.caret;
            match wp as u32 {
                0x41 if ctrl => {
                    s.editor.value.anchor = 0;
                    s.editor.value.caret = s.editor.value.text.len();
                }
                0x43 if ctrl => {
                    copy(hwnd, s);
                }
                0x58 if ctrl => {
                    if copy(hwnd, s) {
                        s.editor.replace("");
                    }
                }
                0x56 if ctrl => paste(hwnd, s),
                0x5a if ctrl => s.editor.history(shift),
                0x59 if ctrl => s.editor.history(true),
                0x2d if ctrl => {
                    copy(hwnd, s);
                }
                0x2d if shift => paste(hwnd, s),
                0x2e if shift => {
                    if copy(hwnd, s) {
                        s.editor.replace("");
                    }
                }
                8 => s.editor.delete(true, ctrl),
                0x2e => s.editor.delete(false, ctrl),
                0x25 | 0x27 => {
                    let right = wp == 0x27;
                    let next = if !shift && !s.editor.range().is_empty() {
                        if right {
                            s.editor.range().end
                        } else {
                            s.editor.range().start
                        }
                    } else if ctrl {
                        s.editor.word(pos, right)
                    } else if right {
                        s.editor.next(pos)
                    } else {
                        s.editor.previous(pos)
                    };
                    s.editor.select(next, shift);
                    s.preferred_x = None;
                }
                0x24 | 0x23 | 0x26 | 0x28 | 0x21 | 0x22 => {
                    let dc = GetDC(hwnd);
                    let old = SelectObject(dc, s.font);
                    let (lines, height, rect, _) = layout(hwnd, s, dc);
                    let (row, x) = position(&lines, pos);
                    let next = match wp {
                        0x24 => {
                            if ctrl {
                                0
                            } else {
                                lines[row].start
                            }
                        }
                        0x23 => {
                            if ctrl {
                                s.editor.value.text.len()
                            } else {
                                lines[row].end
                            }
                        }
                        _ => {
                            let x = *s.preferred_x.get_or_insert(x);
                            let step = if wp == 0x21 || wp == 0x22 {
                                (rect.bottom / height).max(1)
                            } else {
                                1
                            };
                            let delta = if wp == 0x26 || wp == 0x21 {
                                -step
                            } else {
                                step
                            };
                            let row =
                                (row as i32 + delta).clamp(0, lines.len() as i32 - 1) as usize;
                            nearest(&lines[row], x)
                        }
                    };
                    s.editor.select(next, shift);
                    SelectObject(dc, old);
                    ReleaseDC(hwnd, dc);
                }
                _ => return DefWindowProcW(hwnd, msg, wp, lp),
            }
        }
        WM_CHAR => {
            if IsWindowEnabled(hwnd) == 0 || ctrl || s.composing {
                return 0;
            }
            let c = wp as u16;
            if c == 13 {
                if s.editor.multiline {
                    s.editor.replace("\n");
                }
            } else if c >= 32 && c != 127 {
                if (0xd800..=0xdbff).contains(&c) {
                    s.high_surrogate = Some(c);
                    return 0;
                }
                let value = if let Some(high) = s.high_surrogate.take() {
                    String::from_utf16_lossy(&[high, c])
                } else {
                    String::from_utf16_lossy(&[c])
                };
                s.editor.replace(&value);
            }
            s.preferred_x = None;
        }
        WM_UNICHAR => {
            if wp == 0xffff {
                return 1;
            }
            if IsWindowEnabled(hwnd) != 0 {
                if let Some(c) = char::from_u32(wp as u32) {
                    s.editor.replace(&c.to_string());
                }
            }
        }
        WM_IME_STARTCOMPOSITION => {
            s.composing = true;
            s.preedit.clear();
        }
        WM_IME_SETCONTEXT => {
            // We draw the composition string; Windows still owns candidate UI.
            return DefWindowProcW(hwnd, msg, wp, lp & !(ISC_SHOWUICOMPOSITIONWINDOW as isize));
        }
        WM_IME_COMPOSITION => {
            let imc = ImmGetContext(hwnd);
            if !imc.is_null() {
                if lp as u32 & GCS_RESULTSTR != 0 {
                    let text = ime_string(imc, GCS_RESULTSTR);
                    s.editor.replace(&text);
                    s.preedit.clear();
                }
                if lp as u32 & GCS_COMPSTR != 0 {
                    s.preedit = ime_string(imc, GCS_COMPSTR);
                }
                if lp as u32 & GCS_CURSORPOS != 0 {
                    s.preedit_cursor =
                        ImmGetCompositionStringW(imc, GCS_CURSORPOS, null_mut(), 0).max(0) as usize;
                }
                ImmReleaseContext(hwnd, imc);
            }
        }
        WM_IME_ENDCOMPOSITION => {
            s.composing = false;
            s.preedit.clear();
        }
        WM_IME_CHAR => return 0, // committed text was consumed through GCS_RESULTSTR
        WM_SIZE | WM_ENABLE => {}
        _ => return DefWindowProcW(hwnd, msg, wp, lp),
    }
    let changed = before != s.editor.value.text;
    refresh(hwnd, s);
    if changed {
        notify(hwnd);
    }
    if msg == WM_SETTEXT || msg == EM_SETCUEBANNER {
        1
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn graphemes_selection_and_history_round_trip() {
        let mut e = Editor::default();
        e.replace("A👩‍💻e\u{301}東京");
        let end = e.value.caret;
        e.delete(true, false);
        assert_eq!(e.value.text, "A👩‍💻e\u{301}東");
        e.history(false);
        assert_eq!(e.value.caret, end);
        e.select(1, false);
        e.select(e.next(1), true);
        e.replace("🌻");
        assert_eq!(e.value.text, "A🌻e\u{301}東京");
        e.history(false);
        e.history(true);
        assert_eq!(e.value.text, "A🌻e\u{301}東京");
        e.select("A🌻e\u{301}".len(), false);
        e.delete(true, false);
        assert_eq!(e.value.text, "A🌻東京");
    }
    #[test]
    fn multiline_paste_normalization_and_branching_undo() {
        let mut e = Editor {
            multiline: true,
            ..Default::default()
        };
        e.replace("one\r\ntwo\rthree");
        assert_eq!(e.value.text, "one\ntwo\nthree");
        e.select(0, false);
        e.select(3, true);
        e.replace("first");
        e.history(false);
        assert_eq!(e.range(), 0..3);
        e.replace("new");
        e.history(true);
        assert_eq!(e.value.text, "new\ntwo\nthree");
        let mut single = Editor::default();
        single.replace("a\r\nb\0c");
        assert_eq!(single.value.text, "abc");
    }
    #[test]
    fn custom_windows_edit_and_paint_without_native_edit_children() {
        unsafe {
            let parent = CreateWindowExW(
                0,
                wide("STATIC").as_ptr(),
                wide("").as_ptr(),
                WS_OVERLAPPED,
                0,
                0,
                640,
                480,
                null_mut(),
                null_mut(),
                GetModuleHandleW(null()),
                null(),
            );
            assert!(!parent.is_null());
            let font = GetStockObject(DEFAULT_GUI_FONT) as HFONT;
            let search = create(parent, SEARCH, font);
            let name = create(parent, NAME, font);
            let description = create(parent, DESCRIPTION, font);
            for control in [search, name, description] {
                assert!(!control.is_null());
                SetWindowPos(control, null_mut(), 0, 0, 240, 80, SWP_NOZORDER);
                let mut class = [0u16; 64];
                GetClassNameW(control, class.as_mut_ptr(), 64);
                assert_eq!(text_at(class.as_ptr()), "DeezNatsCanvasInput");
                assert!(GetWindow(control, GW_CHILD).is_null());
                assert_eq!(
                    SendMessageW(control, WM_GETDLGCODE, 0, 0) as u32 & DLGC_WANTALLKEYS,
                    0
                );
            }
            for unit in "A🌻e\u{301}".encode_utf16() {
                SendMessageW(search, WM_CHAR, unit as usize, 0);
            }
            assert_eq!(read(search), "A🌻e\u{301}");
            SendMessageW(search, WM_KEYDOWN, 8, 0);
            assert_eq!(read(search), "A🌻");
            SendMessageW(search, WM_UNDO, 0, 0);
            assert_eq!(read(search), "A🌻e\u{301}");
            SendMessageW(search, EM_SETSEL, 1, 3);
            SendMessageW(search, EM_REPLACESEL, 1, wide("東京").as_ptr() as isize);
            assert_eq!(read(search), "A東京e\u{301}");
            SetWindowTextW(name, wide("Draft title").as_ptr());
            SetWindowTextW(description, wide("First line").as_ptr());
            SendMessageW(description, EM_SETSEL, usize::MAX - 1, -1);
            SendMessageW(description, WM_CHAR, 13, 0);
            for unit in "Second line".encode_utf16() {
                SendMessageW(description, WM_CHAR, unit as usize, 0);
            }
            assert_eq!(read(description), "First line\nSecond line");
            SendMessageW(description, WM_KEYDOWN, 0x24, 0); // Home on the second visual line
            assert_eq!(
                (*state(description)).editor.value.caret,
                "First line\n".len()
            );
            SendMessageW(description, WM_KEYDOWN, 0x26, 0); // Up preserves horizontal position
            assert_eq!((*state(description)).editor.value.caret, 0);
            SendMessageW(description, WM_KEYDOWN, 0x23, 0);
            assert_eq!((*state(description)).editor.value.caret, "First line".len());
            SendMessageW(description, WM_IME_STARTCOMPOSITION, 0, 0);
            assert!(composing(description));
            SendMessageW(description, WM_IME_CHAR, 'X' as usize, 0);
            SendMessageW(description, WM_IME_ENDCOMPOSITION, 0, 0);
            assert!(!composing(description));
            assert_eq!(read(description), "First line\nSecond line");
            assert_eq!(SendMessageW(name, WM_UNICHAR, 0xffff, 0), 1);
            // Hiding/showing for cover navigation does not reconstruct or lose drafts.
            ShowWindow(name, SW_HIDE);
            ShowWindow(description, SW_HIDE);
            assert_eq!(read(name), "Draft title");
            assert_eq!(read(description), "First line\nSecond line");
            let dc = GetDC(description);
            let mem = CreateCompatibleDC(dc);
            let bitmap = CreateCompatibleBitmap(dc, 240, 80);
            let old = SelectObject(mem, bitmap);
            SendMessageW(description, WM_PRINTCLIENT, mem as usize, 0);
            let background = GetPixel(mem, 239, 79);
            let mut ink = false;
            for y in 0..60 {
                for x in 4..100 {
                    if GetPixel(mem, x, y) != background {
                        ink = true;
                    }
                }
            }
            assert!(ink, "canvas must paint visible glyphs");
            SelectObject(mem, old);
            DeleteObject(bitmap);
            DeleteDC(mem);
            ReleaseDC(description, dc);
            DestroyWindow(parent);
        }
    }
}
