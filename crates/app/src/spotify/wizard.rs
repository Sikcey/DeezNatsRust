//! Isolated, user-driven Spotify Soloist setup with a native secret field.
use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, VecDeque},
    path::PathBuf,
    rc::Rc,
    sync::atomic::{AtomicBool, Ordering},
    sync::{mpsc, Arc},
    time::{Duration, Instant},
};
use webview2_com::{
    AddScriptToExecuteOnDocumentCreatedCompletedHandler, CoTaskMemPWSTR,
    CreateCoreWebView2ControllerCompletedHandler, CreateCoreWebView2EnvironmentCompletedHandler,
    DownloadStartingEventHandler, ExecuteScriptCompletedHandler, GetCookiesCompletedHandler,
    Microsoft::Web::WebView2::Win32::*, NavigationStartingEventHandler,
    NewWindowRequestedEventHandler, PermissionRequestedEventHandler, StateChangedEventHandler,
};
use windows::core::{Interface, PCWSTR, PWSTR};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::{Dwm::DwmSetWindowAttribute, Gdi::*},
    System::{Com::*, LibraryLoader::GetModuleHandleW},
    UI::{
        Controls::{Dialogs::*, *},
        Input::KeyboardAndMouse::{EnableWindow, SetActiveWindow, SetFocus},
        WindowsAndMessaging::*,
    },
};
struct WizardTheme {
    colors: [u32; 9],
    background: HBRUSH,
    field: HBRUSH,
}
impl Drop for WizardTheme {
    fn drop(&mut self) {
        unsafe {
            DeleteObject(self.background as _);
            DeleteObject(self.field as _);
        }
    }
}
thread_local! { static THEME: RefCell<Option<WizardTheme>> = const { RefCell::new(None) }; }
thread_local! { static CONTROLLER:RefCell<Option<ICoreWebView2Controller>>=const {RefCell::new(None)}; static CLOSED:Cell<bool>=const {Cell::new(false)}; static ACTION:Cell<u32>=const {Cell::new(0)}; }
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}
unsafe extern "system" fn confirm_proc(hwnd: HWND, msg: u32, wp: usize, lp: isize) -> isize {
    match msg {
        WM_COMMAND if (wp >> 16) == BN_CLICKED as usize => {
            ACTION.set((wp & 0xffff) as u32);
            0
        }
        WM_CLOSE => {
            CLOSED.set(true);
            0
        }
        WM_ERASEBKGND | WM_CTLCOLORSTATIC | WM_DRAWITEM => window_proc(hwnd, msg, wp, lp),
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

pub(crate) unsafe fn confirm(
    owner: HWND,
    palette: [u32; 9],
    title: &str,
    message: &str,
    ok: &str,
    cancel: &str,
) -> bool {
    CLOSED.set(false);
    ACTION.set(0);
    THEME.with(|slot| {
        *slot.borrow_mut() = Some(WizardTheme {
            colors: palette,
            background: CreateSolidBrush(palette[0]),
            field: CreateSolidBrush(palette[2]),
        })
    });
    let instance = GetModuleHandleW(std::ptr::null());
    let class_name = wide("DeezNatsSpotifyConfirm");
    RegisterClassW(&WNDCLASSW {
        lpfnWndProc: Some(confirm_proc),
        hInstance: instance,
        lpszClassName: class_name.as_ptr(),
        hCursor: LoadCursorW(std::ptr::null_mut(), IDC_ARROW),
        ..Default::default()
    });
    let scale = windows_sys::Win32::UI::HiDpi::GetDpiForWindow(owner).max(96) as f64 / 96.0;
    let px = |n: i32| (n as f64 * scale).round() as i32;
    let dc = GetDC(owner);
    let old_font = SelectObject(dc, GetStockObject(DEFAULT_GUI_FONT));
    let mut text_rect = RECT {
        left: 0,
        top: 0,
        right: px(500),
        bottom: 0,
    };
    let mut label = wide(message);
    DrawTextW(
        dc,
        label.as_mut_ptr(),
        -1,
        &mut text_rect,
        DT_CALCRECT | DT_WORDBREAK | DT_NOPREFIX,
    );
    SelectObject(dc, old_font);
    ReleaseDC(owner, dc);
    let width = px(550);
    let height = (text_rect.bottom + px(130)).max(px(260));
    let mut parent_rect = RECT::default();
    GetWindowRect(owner, &mut parent_rect);
    let hwnd = CreateWindowExW(
        WS_EX_DLGMODALFRAME,
        class_name.as_ptr(),
        wide(title).as_ptr(),
        WS_POPUP | WS_CAPTION | WS_SYSMENU,
        parent_rect.left + (parent_rect.right - parent_rect.left - width) / 2,
        parent_rect.top + (parent_rect.bottom - parent_rect.top - height) / 2,
        width,
        height,
        owner,
        std::ptr::null_mut(),
        instance,
        std::ptr::null(),
    );
    if hwnd.is_null() {
        THEME.with(|slot| {
            slot.borrow_mut().take();
        });
        return false;
    }
    let dark = ((palette[0] & 255) * 299
        + ((palette[0] >> 8) & 255) * 587
        + ((palette[0] >> 16) & 255) * 114)
        < 128000;
    let dark_mode = dark as i32;
    DwmSetWindowAttribute(hwnd, 20, &dark_mode as *const _ as _, 4);
    DwmSetWindowAttribute(hwnd, 35, &palette[0] as *const _ as _, 4);
    DwmSetWindowAttribute(hwnd, 36, &palette[4] as *const _ as _, 4);
    let mut client = RECT::default();
    GetClientRect(hwnd, &mut client);
    for (id, class, text, x, y, w, h, style) in [
        (
            200,
            "STATIC",
            message,
            px(20),
            px(20),
            client.right - px(40),
            text_rect.bottom + px(4),
            0,
        ),
        (
            201,
            "BUTTON",
            ok,
            client.right - px(230),
            client.bottom - px(50),
            px(100),
            px(32),
            WS_TABSTOP | BS_OWNERDRAW as u32,
        ),
        (
            202,
            "BUTTON",
            cancel,
            client.right - px(120),
            client.bottom - px(50),
            px(100),
            px(32),
            WS_TABSTOP | BS_OWNERDRAW as u32,
        ),
    ] {
        let child = CreateWindowExW(
            0,
            wide(class).as_ptr(),
            wide(text).as_ptr(),
            WS_CHILD | WS_VISIBLE | style,
            x,
            y,
            w,
            h,
            hwnd,
            id as usize as HMENU,
            instance,
            std::ptr::null(),
        );
        if child.is_null() {
            DestroyWindow(hwnd);
            THEME.with(|slot| {
                slot.borrow_mut().take();
            });
            return false;
        }
        SendMessageW(
            child,
            WM_SETFONT,
            GetStockObject(DEFAULT_GUI_FONT) as usize,
            1,
        );
    }
    EnableWindow(owner, 0);
    ShowWindow(hwnd, SW_SHOWNORMAL);
    SetFocus(GetDlgItem(hwnd, 202));
    let mut msg = MSG::default();
    while !CLOSED.get() && ACTION.get() == 0 {
        let result = GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0);
        if result <= 0 {
            if result == 0 {
                PostQuitMessage(msg.wParam as i32);
            }
            break;
        }
        if msg.message == WM_KEYDOWN && msg.wParam == 27 {
            break;
        }
        if IsDialogMessageW(hwnd, &msg) == 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    let accepted = ACTION.get() == 201;
    EnableWindow(owner, 1);
    DestroyWindow(hwnd);
    SetActiveWindow(owner);
    THEME.with(|slot| {
        slot.borrow_mut().take();
    });
    accepted
}
unsafe fn text(
    get: impl FnOnce(*mut PWSTR) -> windows::core::Result<()>,
) -> windows::core::Result<String> {
    let mut value = PWSTR::null();
    get(&mut value)?;
    Ok(CoTaskMemPWSTR::from(value).to_string())
}
unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wp: usize, lp: isize) -> isize {
    match msg {
        WM_ERASEBKGND => THEME.with(|slot| {
            if let Some(theme) = slot.borrow().as_ref() {
                let mut rect = RECT::default();
                GetClientRect(hwnd, &mut rect);
                FillRect(wp as HDC, &rect, theme.background);
                1
            } else {
                DefWindowProcW(hwnd, msg, wp, lp)
            }
        }),
        WM_CTLCOLORSTATIC | WM_CTLCOLOREDIT => THEME.with(|slot| {
            if let Some(theme) = slot.borrow().as_ref() {
                let dc = wp as HDC;
                let field = msg == WM_CTLCOLOREDIT;
                SetTextColor(dc, theme.colors[4]);
                SetBkColor(dc, theme.colors[if field { 2 } else { 0 }]);
                if field {
                    theme.field as isize
                } else {
                    theme.background as isize
                }
            } else {
                DefWindowProcW(hwnd, msg, wp, lp)
            }
        }),
        WM_DRAWITEM => THEME.with(|slot| {
            let state = slot.borrow();
            let Some(theme) = state.as_ref() else {
                return 0;
            };
            let draw = &*(lp as *const DRAWITEMSTRUCT);
            let selected = draw.itemState & ODS_SELECTED != 0;
            let disabled = draw.itemState & ODS_DISABLED != 0;
            let brush = CreateSolidBrush(theme.colors[if selected { 6 } else { 2 }]);
            FillRect(draw.hDC, &draw.rcItem, brush);
            DeleteObject(brush as _);
            let border = CreateSolidBrush(theme.colors[3]);
            FrameRect(draw.hDC, &draw.rcItem, border);
            DeleteObject(border as _);
            let mut label = [0u16; 256];
            let len = GetWindowTextW(draw.hwndItem, label.as_mut_ptr(), label.len() as i32);
            let old_font = SelectObject(draw.hDC, GetStockObject(DEFAULT_GUI_FONT));
            SetBkMode(draw.hDC, TRANSPARENT as i32);
            SetTextColor(draw.hDC, theme.colors[if disabled { 5 } else { 4 }]);
            let mut rect = draw.rcItem;
            DrawTextW(
                draw.hDC,
                label.as_mut_ptr(),
                len,
                &mut rect,
                DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS,
            );
            if draw.itemState & ODS_FOCUS != 0 {
                InflateRect(&mut rect, -4, -4);
                DrawFocusRect(draw.hDC, &rect);
            }
            SelectObject(draw.hDC, old_font);
            1
        }),
        WM_SIZE => {
            layout(hwnd);
            0
        }
        WM_COMMAND => {
            let id = (wp & 0xffff) as u32;
            if ((wp >> 16) & 0xffff) == BN_CLICKED as usize
                && matches!(id, KEY_PAGE | DOWNLOAD_PAGE | SIGN_IN | FINISH | LOCAL_FILE)
            {
                ACTION.set(id);
            }
            0
        }
        WM_GETMINMAXINFO => {
            let limits = &mut *(lp as *mut MINMAXINFO);
            let scale = windows_sys::Win32::UI::HiDpi::GetDpiForWindow(hwnd).max(96) as f64 / 96.0;
            limits.ptMinTrackSize.x = (760.0 * scale).round() as i32;
            limits.ptMinTrackSize.y = (620.0 * scale).round() as i32;
            0
        }
        WM_CLOSE => {
            CLOSED.set(true);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}
unsafe fn pump(hwnd: HWND, cancel: &AtomicBool, started: Instant) -> Result<(), String> {
    if cancel.load(Ordering::Relaxed) || CLOSED.get() {
        return Err("Spotify setup cancelled".into());
    }
    if started.elapsed() > Duration::from_secs(1800) {
        return Err("Spotify setup timed out".into());
    }
    if IsWindow(hwnd) == 0 {
        return Err("Spotify setup window closed".into());
    }
    let mut message = MSG::default();
    while PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
        if message.message == WM_QUIT {
            return Err("Spotify setup window closed".into());
        }
        TranslateMessage(&message);
        DispatchMessageW(&message);
    }
    if cancel.load(Ordering::Relaxed) || CLOSED.get() {
        return Err("Spotify setup cancelled".into());
    }
    std::thread::sleep(Duration::from_millis(10));
    Ok(())
}
unsafe fn wait<T>(
    slot: &Rc<RefCell<Option<Result<T, String>>>>,
    hwnd: HWND,
    cancel: &AtomicBool,
    started: Instant,
) -> Result<T, String> {
    loop {
        if let Some(value) = slot.borrow_mut().take() {
            return value;
        }
        pump(hwnd, cancel, started)?;
    }
}
struct Window(HWND);
impl Drop for Window {
    fn drop(&mut self) {
        unsafe {
            CONTROLLER.with(|slot| {
                if let Some(controller) = slot.borrow_mut().take() {
                    let _ = controller.Close();
                }
            });
            if IsWindow(self.0) != 0 {
                SetWindowTextW(GetDlgItem(self.0, KEY as i32), wide("").as_ptr());
                DestroyWindow(self.0);
            }
            THEME.with(|slot| {
                slot.borrow_mut().take();
            });
        }
    }
}

pub(crate) const NOTICE: &str = "Spotify playback uses Spotify Soloist, installed separately.\n\nYou need Spotify Premium for initial setup and your own personal Soloist API key. Spotify may ask you to accept its terms.\n\nThe setup window opens Spotify's official pages. Download Windows x64 Soloist there and paste your personal key into the setup window. The download will be checked and installed on this PC. Your Spotify web session will also be captured and saved securely.\n\nSoloist builds expire after 90 days. After setup, Soloist starts and opens Spotify for pairing. Select DeezNats once; saved pairing is restored later.\n\nContinue to Spotify setup?";
const KEY_PAGE: u32 = 101;
const DOWNLOAD_PAGE: u32 = 102;
const SIGN_IN: u32 = 103;
const KEY: u32 = 104;
const FINISH: u32 = 105;
const STATUS: u32 = 106;
const HEADING: u32 = 107;
const KEY_LABEL: u32 = 108;
const LOCAL_FILE: u32 = 109;
const WINDOWS_DOWNLOAD: &str =
    "https://soloist-builds.spotifycdn.com/soloist_release_windows_x86_64.zip";
type Translations = Arc<HashMap<String, String>>;

unsafe fn controls(hwnd: HWND, tr: &impl Fn(&str) -> String) -> Result<(), String> {
    let instance = GetModuleHandleW(std::ptr::null());
    for (id, class, label, extra) in [
        (HEADING, "STATIC", "Sign in to Spotify to finish setup.", 0),
        (KEY_PAGE, "BUTTON", "1. Get personal key", WS_TABSTOP),
        (DOWNLOAD_PAGE, "BUTTON", "2. Download Soloist", WS_TABSTOP),
        (SIGN_IN, "BUTTON", "Spotify sign-in", WS_TABSTOP),
        (LOCAL_FILE, "BUTTON", "Use local ZIP / EXE", WS_TABSTOP),
        (KEY_LABEL, "STATIC", "Personal key", 0),
        (
            KEY,
            "EDIT",
            "",
            WS_TABSTOP | ES_PASSWORD as u32 | ES_AUTOHSCROLL as u32 | WS_BORDER,
        ),
        (FINISH, "BUTTON", "Finish setup", WS_TABSTOP),
        (
            STATUS,
            "STATIC",
            "Sign in to Spotify. Setup will continue automatically.",
            0,
        ),
    ] {
        let child = CreateWindowExW(
            0,
            wide(class).as_ptr(),
            wide(&tr(label)).as_ptr(),
            WS_CHILD
                | WS_VISIBLE
                | extra
                | if class == "BUTTON" {
                    BS_OWNERDRAW as u32
                } else {
                    0
                },
            0,
            0,
            1,
            1,
            hwnd,
            id as usize as HMENU,
            instance,
            std::ptr::null(),
        );
        if child.is_null() {
            return Err("Could not create Spotify setup controls".into());
        }
        SendMessageW(
            child,
            WM_SETFONT,
            GetStockObject(DEFAULT_GUI_FONT) as usize,
            1,
        );
    }
    SendMessageW(GetDlgItem(hwnd, KEY as i32), EM_SETLIMITTEXT, 4096, 0);
    let initial_key = std::env::var("DEEZNATS_SOLOIST_API_KEY")
        .ok()
        .or_else(|| super::soloist::Config::saved().ok().map(|c| c.api_key));
    if let Some(key) = initial_key {
        SetWindowTextW(GetDlgItem(hwnd, KEY as i32), wide(&key).as_ptr());
    }
    layout(hwnd);
    Ok(())
}
unsafe fn layout(hwnd: HWND) {
    let mut rect = RECT::default();
    GetClientRect(hwnd, &mut rect);
    let scale = windows_sys::Win32::UI::HiDpi::GetDpiForWindow(hwnd).max(96) as f64 / 96.0;
    let px = |v: i32| (v as f64 * scale).round() as i32;
    let width = rect.right;
    let height = rect.bottom;
    for (id, x, y, w, h) in [
        (HEADING, px(16), px(10), width - px(32), px(24)),
        (KEY_PAGE, px(16), px(42), px(180), px(32)),
        (DOWNLOAD_PAGE, px(208), px(42), px(180), px(32)),
        (SIGN_IN, px(400), px(42), px(160), px(32)),
        (LOCAL_FILE, px(572), px(42), px(164), px(32)),
        (KEY_LABEL, px(16), height - px(108), px(116), px(24)),
        (
            KEY,
            px(136),
            height - px(112),
            (width - px(336)).max(px(90)),
            px(30),
        ),
        (FINISH, width - px(180), height - px(112), px(164), px(32)),
        (STATUS, px(16), height - px(68), width - px(32), px(56)),
    ] {
        let control = GetDlgItem(hwnd, id as i32);
        if !control.is_null() {
            MoveWindow(control, x, y, w.max(1), h.max(1), 1);
        }
    }
    CONTROLLER.with(|slot| {
        if let Some(controller) = slot.borrow().as_ref() {
            let _ = controller.SetBounds(windows::Win32::Foundation::RECT {
                left: 0,
                top: px(88),
                right: width,
                bottom: (height - px(132)).max(px(100)),
            });
        }
    });
}
unsafe fn key_text(hwnd: HWND) -> String {
    let control = GetDlgItem(hwnd, KEY as i32);
    let count = GetWindowTextLengthW(control).clamp(0, 4096) as usize;
    let mut text = vec![0u16; count + 1];
    let len = GetWindowTextW(control, text.as_mut_ptr(), text.len() as i32).max(0) as usize;
    let value = String::from_utf16_lossy(&text[..len]).trim().to_owned();
    text.fill(0);
    value
}
fn allowed_page(uri: &str) -> bool {
    let Ok(url) = url::Url::parse(uri) else {
        return false;
    };
    url.scheme() == "https"
        && url.port_or_known_default() == Some(443)
        && url.username().is_empty()
        && url.password().is_none()
        && url.host_str().is_some_and(|host| {
            host == "spotify.com"
                || host.ends_with(".spotify.com")
                || matches!(
                    host,
                    "soloist-builds.spotifycdn.com"
                        | "accounts.google.com"
                        | "appleid.apple.com"
                        | "account.apple.com"
                )
        })
}
fn allowed_download(uri: &str) -> bool {
    let Ok(url) = url::Url::parse(uri) else {
        return false;
    };
    allowed_page(uri)
        && url.host_str() == Some("soloist-builds.spotifycdn.com")
        && url.path().ends_with("/soloist_release_windows_x86_64.zip")
}
enum Event {
    Download(PathBuf),
    Status(&'static str),
}
unsafe fn local_file(hwnd: HWND) -> Option<PathBuf> {
    let mut path = vec![0u16; 32768];
    let filter =
        wide("Soloist ZIP or executable (*.zip;*.exe)\0*.zip;*.exe\0All files (*.*)\0*.*\0");
    let mut dialog: OPENFILENAMEW = std::mem::zeroed();
    dialog.lStructSize = std::mem::size_of::<OPENFILENAMEW>() as u32;
    dialog.hwndOwner = hwnd;
    dialog.lpstrFilter = filter.as_ptr();
    dialog.lpstrFile = path.as_mut_ptr();
    dialog.nMaxFile = path.len() as u32;
    dialog.Flags = OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR;
    if GetOpenFileNameW(&mut dialog) == 0 {
        return None;
    }
    let end = path.iter().position(|&v| v == 0)?;
    Some(PathBuf::from(String::from_utf16_lossy(&path[..end])))
}
struct Downloads(Vec<(ICoreWebView2DownloadOperation, i64)>);
impl Drop for Downloads {
    fn drop(&mut self) {
        unsafe {
            for (operation, token) in &self.0 {
                let _ = operation.remove_StateChanged(*token);
                let mut state = COREWEBVIEW2_DOWNLOAD_STATE_IN_PROGRESS;
                if operation.State(&mut state).is_ok()
                    && state == COREWEBVIEW2_DOWNLOAD_STATE_IN_PROGRESS
                {
                    let _ = operation.Cancel();
                }
            }
        }
    }
}
pub(crate) fn run(
    cancel: &Arc<AtomicBool>,
    translations: Translations,
    palette: [u32; 9],
) -> Result<(), String> {
    unsafe {
        if CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32) < 0 {
            return Err("Could not initialize Spotify setup".into());
        }
        CLOSED.set(false);
        ACTION.set(0);
        let result = run_sta(cancel, translations, palette);
        CoUninitialize();
        result
    }
}
unsafe fn run_sta(
    cancel: &Arc<AtomicBool>,
    translations: Translations,
    palette: [u32; 9],
) -> Result<(), String> {
    THEME.with(|slot| {
        *slot.borrow_mut() = Some(WizardTheme {
            colors: palette,
            background: CreateSolidBrush(palette[0]),
            field: CreateSolidBrush(palette[2]),
        })
    });
    let tr = |text: &str| {
        translations
            .get(text)
            .cloned()
            .unwrap_or_else(|| text.into())
    };
    let instance = GetModuleHandleW(std::ptr::null());
    let class_name = wide("DeezNatsSpotifySetup");
    let class = WNDCLASSW {
        lpfnWndProc: Some(window_proc),
        hInstance: instance,
        lpszClassName: class_name.as_ptr(),
        hCursor: LoadCursorW(std::ptr::null_mut(), IDC_ARROW),
        hbrBackground: GetStockObject(BLACK_BRUSH) as HBRUSH,
        ..Default::default()
    };
    RegisterClassW(&class);
    let hwnd = CreateWindowExW(
        0,
        class_name.as_ptr(),
        wide(&tr("Spotify playback setup")).as_ptr(),
        WS_OVERLAPPEDWINDOW,
        CW_USEDEFAULT,
        CW_USEDEFAULT,
        900,
        850,
        std::ptr::null_mut(),
        std::ptr::null_mut(),
        instance,
        std::ptr::null(),
    );
    if hwnd.is_null() {
        THEME.with(|slot| {
            slot.borrow_mut().take();
        });
        return Err("Could not create Spotify sign-in window".into());
    }
    let _window = Window(hwnd);
    let dark = ((palette[0] & 255) * 299
        + ((palette[0] >> 8) & 255) * 587
        + ((palette[0] >> 16) & 255) * 114)
        < 128000;
    let dark_mode = dark as i32;
    DwmSetWindowAttribute(hwnd, 20, &dark_mode as *const _ as _, 4);
    DwmSetWindowAttribute(hwnd, 35, &palette[0] as *const _ as _, 4);
    DwmSetWindowAttribute(hwnd, 36, &palette[4] as *const _ as _, 4);
    controls(hwnd, &tr)?;
    ShowWindow(hwnd, SW_SHOWNORMAL);
    let started = Instant::now();
    let folder = crate::storage::data_dir()?.join("SpotifySetupWebView");
    std::fs::create_dir_all(&folder)
        .map_err(|_| "Could not prepare the isolated Spotify browser profile")?;
    let folder = wide(&folder.to_string_lossy());
    let environment = Rc::new(RefCell::new(None));
    let output = environment.clone();
    CreateCoreWebView2EnvironmentWithOptions(
        PCWSTR::null(),
        PCWSTR(folder.as_ptr()),
        None,
        &CreateCoreWebView2EnvironmentCompletedHandler::create(Box::new(
            move |result, environment| {
                *output.borrow_mut() = Some(
                    result
                        .map_err(|_| {
                            "WebView2 Runtime could not start. Use normal browser sign-in instead."
                                .into()
                        })
                        .and_then(|_| {
                            environment.ok_or_else(|| "WebView2 environment is unavailable".into())
                        }),
                );
                Ok(())
            },
        )),
    )
    .map_err(|_| "WebView2 Runtime is unavailable. Use normal browser sign-in instead.")?;
    let environment: ICoreWebView2Environment = wait(&environment, hwnd, cancel, started)?;
    let modern: ICoreWebView2Environment10 = environment
        .cast()
        .map_err(|_| "Update WebView2 Runtime to use private sign-in")?;
    let options = modern
        .CreateCoreWebView2ControllerOptions()
        .map_err(|_| "Private login options are unavailable")?;
    options
        .SetIsInPrivateModeEnabled(true)
        .map_err(|_| "Could not enable private Spotify sign-in")?;
    let controller = Rc::new(RefCell::new(None));
    let output = controller.clone();
    modern
        .CreateCoreWebView2ControllerWithOptions(
            windows::Win32::Foundation::HWND(hwnd),
            &options,
            &CreateCoreWebView2ControllerCompletedHandler::create(Box::new(
                move |result, controller| {
                    *output.borrow_mut() = Some(
                        result
                            .map_err(|_| "Spotify WebView2 controller could not start".into())
                            .and_then(|_| {
                                controller.ok_or_else(|| {
                                    "Spotify WebView2 controller is unavailable".into()
                                })
                            }),
                    );
                    Ok(())
                },
            )),
        )
        .map_err(|_| "Could not open private Spotify sign-in")?;
    let controller: ICoreWebView2Controller = wait(&controller, hwnd, cancel, started)?;
    CONTROLLER.with(|slot| *slot.borrow_mut() = Some(controller.clone()));
    layout(hwnd);
    let mut bounds = RECT::default();
    GetClientRect(hwnd, &mut bounds);
    controller
        .SetBounds(windows::Win32::Foundation::RECT {
            left: 0,
            top: 0,
            right: bounds.right,
            bottom: bounds.bottom,
        })
        .map_err(|_| "Could not size the Spotify window")?;
    controller
        .SetIsVisible(true)
        .map_err(|_| "Could not show Spotify sign-in")?;
    let webview = controller
        .CoreWebView2()
        .map_err(|_| "Spotify WebView2 is unavailable")?;
    let installed = Rc::new(RefCell::new(None));
    let output = installed.clone();
    webview
        .AddScriptToExecuteOnDocumentCreated(
            PCWSTR(wide(super::web::BOOTSTRAP).as_ptr()),
            &AddScriptToExecuteOnDocumentCreatedCompletedHandler::create(Box::new(
                move |result, _| {
                    *output.borrow_mut() = Some(
                        result
                            .map(|_| ())
                            .map_err(|_| "Could not capture Spotify API session".into()),
                    );
                    Ok(())
                },
            )),
        )
        .map_err(|_| "Could not prepare Spotify session capture")?;
    wait(&installed, hwnd, cancel, started)?;
    let settings = webview
        .Settings()
        .map_err(|_| "Spotify WebView2 settings unavailable")?;
    settings
        .SetAreHostObjectsAllowed(false)
        .map_err(|_| "Could not isolate Spotify sign-in")?;
    settings
        .SetIsWebMessageEnabled(false)
        .map_err(|_| "Could not isolate Spotify messages")?;
    settings
        .SetAreDevToolsEnabled(false)
        .map_err(|_| "Could not configure Spotify sign-in")?;
    settings
        .SetAreDefaultContextMenusEnabled(false)
        .map_err(|_| "Could not configure Spotify context menus")?;
    let mut permission_token = 0;
    webview
        .add_PermissionRequested(
            &PermissionRequestedEventHandler::create(Box::new(|_, args| {
                if let Some(args) = args {
                    let mut kind = COREWEBVIEW2_PERMISSION_KIND_UNKNOWN_PERMISSION;
                    if args.PermissionKind(&mut kind).is_ok() {
                        match kind {
                            COREWEBVIEW2_PERMISSION_KIND_CAMERA
                            | COREWEBVIEW2_PERMISSION_KIND_MICROPHONE
                            | COREWEBVIEW2_PERMISSION_KIND_GEOLOCATION
                            | COREWEBVIEW2_PERMISSION_KIND_NOTIFICATIONS
                            | COREWEBVIEW2_PERMISSION_KIND_OTHER_SENSORS => {
                                let _ = args.SetState(COREWEBVIEW2_PERMISSION_STATE_DENY);
                            }
                            _ => {
                                let _ = args.SetState(COREWEBVIEW2_PERMISSION_STATE_ALLOW);
                            }
                        }
                    } else {
                        let _ = args.SetState(COREWEBVIEW2_PERMISSION_STATE_DENY);
                    }
                }
                Ok(())
            })),
            &mut permission_token,
        )
        .map_err(|_| "Could not isolate Spotify permissions")?;
    let events = Rc::new(RefCell::new(VecDeque::<Event>::new()));
    let downloading = Rc::new(Cell::new(false));
    let operations = Rc::new(RefCell::new(Downloads(Vec::new())));
    let modern: ICoreWebView2_4 = webview
        .cast()
        .map_err(|_| "Update WebView2 Runtime to download Soloist")?;
    let mut download_token = 0;
    let download_events = events.clone();
    let download_busy = downloading.clone();
    let download_operations = operations.clone();
    modern
        .add_DownloadStarting(
            &DownloadStartingEventHandler::create(Box::new(move |_, args| {
                if let Some(args) = args {
                    let operation = args.DownloadOperation()?;
                    let uri = text(|out| operation.Uri(out))?;
                    if !allowed_download(&uri) || download_busy.get() {
                        let _ = args.SetCancel(true);
                        download_events.borrow_mut().push_back(Event::Status(
                            "Only the official Windows x64 Soloist download is supported here.",
                        ));
                        return Ok(());
                    }
                    let mut size = 0;
                    if operation.TotalBytesToReceive(&mut size).is_ok() && size > 512 * 1024 * 1024
                    {
                        let _ = args.SetCancel(true);
                        download_events
                            .borrow_mut()
                            .push_back(Event::Status("The Soloist download is too large."));
                        return Ok(());
                    }
                    let directory = match crate::storage::data_dir().and_then(|path| {
                        let unique = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_nanos();
                        let folder = path
                            .join("spotify-soloist-install")
                            .join("downloads")
                            .join(format!("{}-{unique}", std::process::id()));
                        std::fs::create_dir_all(&folder)
                            .map_err(|_| "Could not prepare the Soloist download".to_owned())?;
                        Ok(folder)
                    }) {
                        Ok(path) => path,
                        Err(_) => {
                            let _ = args.SetCancel(true);
                            download_events.borrow_mut().push_back(Event::Status(
                                "Could not prepare the Soloist download.",
                            ));
                            return Ok(());
                        }
                    };
                    let path = directory.join("soloist_release_windows_x86_64.zip");
                    let clean_path = super::setup::strip_verbatim(&path).to_path_buf();
                    args.SetResultFilePath(PCWSTR(wide(&clean_path.to_string_lossy()).as_ptr()))?;
                    args.SetHandled(true)?;
                    let output = download_events.clone();
                    let busy = download_busy.clone();
                    let saved_path = clean_path.clone();
                    let terminal = Cell::new(false);
                    let mut token = 0;
                    operation.add_StateChanged(
                        &StateChangedEventHandler::create(Box::new(move |sender, _| {
                            if let Some(sender) = sender {
                                let mut state = COREWEBVIEW2_DOWNLOAD_STATE_IN_PROGRESS;
                                sender.State(&mut state)?;
                                if state != COREWEBVIEW2_DOWNLOAD_STATE_IN_PROGRESS
                                    && terminal.replace(true)
                                {
                                    return Ok(());
                                }
                                if state == COREWEBVIEW2_DOWNLOAD_STATE_COMPLETED {
                                    output
                                        .borrow_mut()
                                        .push_back(Event::Download(saved_path.clone()));
                                } else if state == COREWEBVIEW2_DOWNLOAD_STATE_INTERRUPTED {
                                    busy.set(false);
                                    output.borrow_mut().push_back(Event::Status(
                                        "Soloist download was interrupted. Try the download again.",
                                    ));
                                }
                            }
                            Ok(())
                        })),
                        &mut token,
                    )?;
                    download_operations.borrow_mut().0.push((operation, token));
                    download_busy.set(true);
                    download_events
                        .borrow_mut()
                        .push_back(Event::Status("Downloading Soloist..."));
                }
                Ok(())
            })),
            &mut download_token,
        )
        .map_err(|_| "Could not configure the Soloist download")?;
    let mut nav_token = 0;
    let nav_events = events.clone();
    webview
        .add_NavigationStarting(
            &NavigationStartingEventHandler::create(Box::new(move |_, args| {
                if let Some(args) = args {
                    let uri = text(|out| args.Uri(out))?;
                    if uri != "about:blank" && !allowed_page(&uri) {
                        args.SetCancel(true)?;
                        nav_events.borrow_mut().push_back(Event::Status(
                            "This setup window only opens Spotify and supported sign-in pages.",
                        ));
                    }
                }
                Ok(())
            })),
            &mut nav_token,
        )
        .map_err(|_| "Could not configure Spotify setup navigation")?;
    let mut popup_token = 0;
    let browser = webview.clone();
    webview
        .add_NewWindowRequested(
            &NewWindowRequestedEventHandler::create(Box::new(move |_, args| {
                if let Some(args) = args {
                    args.SetHandled(true)?;
                    let uri = text(|out| args.Uri(out))?;
                    if allowed_page(&uri) {
                        browser.Navigate(PCWSTR(wide(&uri).as_ptr()))?;
                    }
                }
                Ok(())
            })),
            &mut popup_token,
        )
        .map_err(|_| "Could not configure Spotify setup popups")?;
    let cookies = webview
        .cast::<ICoreWebView2_2>()
        .map_err(|_| "Update WebView2 Runtime for Spotify setup")?
        .CookieManager()
        .map_err(|_| "Spotify cookie manager unavailable")?;
    let captured = Rc::new(RefCell::new(None::<String>));
    let pending = Rc::new(Cell::new(false));
    let mut checked = Instant::now() - Duration::from_secs(1);
    let inspect = std::env::args().any(|arg| arg == "--inspect-soloist-dashboard");
    let mut inspected = Instant::now() - Duration::from_secs(3);
    let mut installing: Option<mpsc::Receiver<Result<PathBuf, String>>> = None;
    let mut selected_executable = None;
    layout(hwnd);
    let mut key_checked = Instant::now() - Duration::from_secs(2);
    let key_busy = Rc::new(Cell::new(false));
    let captured_key = Rc::new(RefCell::new(None::<String>));
    let mut auto_finish_attempted = false;
    let mut initial_download = !inspect && super::setup::preferred_executable().is_err();
    let mut download_receiver = if initial_download {
        let (send, receive) = mpsc::sync_channel::<Result<PathBuf, String>>(1);
        downloading.set(true);
        std::thread::Builder::new()
            .name("soloist-download".into())
            .spawn(move || {
                let result = (|| {
                    let folder = crate::storage::data_dir()?
                        .join("spotify-soloist-install")
                        .join("downloads");
                    std::fs::create_dir_all(&folder)
                        .map_err(|_| "Could not prepare Soloist download".to_owned())?;
                    let path = folder.join(format!(
                        "setup-{}-{}.zip",
                        std::process::id(),
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_nanos()
                    ));
                    let result = (|| {
                        let mut file = std::fs::File::create(&path)
                            .map_err(|_| "Could not prepare Soloist download".to_owned())?;
                        crate::updater::http::get(
                            WINDOWS_DOWNLOAD,
                            &mut file,
                            512 * 1024 * 1024,
                            |_| {},
                        )?;
                        Ok(path.clone())
                    })();
                    if result.is_err() {
                        let _ = std::fs::remove_file(path);
                    }
                    result
                })();
                if let Err(mpsc::SendError(Ok(path))) = send.send(result) {
                    let _ = std::fs::remove_file(path);
                }
            })
            .map_err(|_| "Could not start Soloist download")?;
        Some(receive)
    } else {
        None
    };
    webview
        .Navigate(PCWSTR(wide(super::setup::KEY_PAGE).as_ptr()))
        .map_err(|_| "Could not open Spotify setup")?;
    let result = loop {
        if let Err(error) = pump(hwnd, cancel, started) {
            break Err(error);
        }
        if !inspect
            && key_checked.elapsed() >= Duration::from_secs(2)
            && !key_busy.get()
            && captured_key.borrow().is_none()
        {
            key_checked = Instant::now();
            key_busy.set(true);
            let busy = key_busy.clone();
            let result_key = captured_key.clone();
            let script = r#"(() => {
              if (location.origin !== 'https://developer.spotify.com' || !location.pathname.startsWith('/dashboard/soloist')) return {dashboard:false};
              if (typeof window.__deeznatsSoloistKey === 'string') {
                const key = window.__deeznatsSoloistKey; delete window.__deeznatsSoloistKey; return key;
              }
              const heading = [...document.querySelectorAll('h1,h2,h3')].find(e => /soloist/i.test(e.textContent) && /api key/i.test(e.textContent));
              if (!heading) return {dashboard:true, heading:false};
              let root = heading; for (let i=0; i<4 && root.parentElement; i++) root = root.parentElement;
              const buttons = [...root.querySelectorAll('button')];
              const label = e => [e.innerText, e.getAttribute('aria-label'), e.getAttribute('title')].filter(Boolean).join(' ').toLowerCase();
              const copy = buttons.find(e => /copy/.test(label(e)));
              if (copy && !window.__deeznatsCopyStarted && navigator.clipboard) {
                window.__deeznatsCopyStarted = true;
                const clipboard = navigator.clipboard;
                try {
                  Object.defineProperty(clipboard, 'writeText', {configurable: true, value: function(value) {
                    if (typeof value === 'string') window.__deeznatsSoloistKey = value;
                    delete clipboard.writeText;
                    return Promise.resolve();
                  }});
                  copy.click();
                  setTimeout(() => { delete clipboard.writeText; }, 10000);
                } catch (_) { window.__deeznatsCopyStarted = false; }
              } else if (!copy && !window.__deeznatsGenerateStarted) {
                const generate = buttons.find(e => /generate/.test(label(e)) && /api key/.test(label(e)) && !/regenerate/.test(label(e)));
                if (generate) { window.__deeznatsGenerateStarted = true; generate.click(); }
              }
              return {dashboard:true, heading:true, copy:!!copy, clipboard:!!navigator.clipboard, started:!!window.__deeznatsCopyStarted};
            })()"#;
            if webview
                .ExecuteScript(
                    PCWSTR(wide(script).as_ptr()),
                    &ExecuteScriptCompletedHandler::create(Box::new(move |result, value| {
                        busy.set(false);
                        if result.is_ok() {
                            if let Ok(serde_json::Value::String(key)) =
                                serde_json::from_str::<serde_json::Value>(&value.to_string())
                            {
                                let key = key.trim();
                                if !key.is_empty()
                                    && key.len() <= 4096
                                    && !key.chars().any(char::is_control)
                                {
                                    *result_key.borrow_mut() = Some(key.to_owned());
                                }
                            }
                        }
                        Ok(())
                    })),
                )
                .is_err()
            {
                key_busy.set(false);
            }
        }
        if let Some(result) =
            download_receiver
                .as_ref()
                .and_then(|receiver| match receiver.try_recv() {
                    Ok(result) => Some(result),
                    Err(mpsc::TryRecvError::Disconnected) => {
                        Some(Err("Soloist download stopped unexpectedly.".into()))
                    }
                    Err(mpsc::TryRecvError::Empty) => None,
                })
        {
            download_receiver = None;
            match result {
                Ok(path) => events.borrow_mut().push_back(Event::Download(path)),
                Err(error) => {
                    downloading.set(false);
                    initial_download = false;
                    SetWindowTextW(GetDlgItem(hwnd, STATUS as i32), wide(&error).as_ptr());
                }
            }
        }
        if inspect && inspected.elapsed() >= Duration::from_secs(3) {
            inspected = Instant::now();
            // Only report categorical structure: never read values, cookies, URLs,
            // arbitrary attributes, or page text into the diagnostic file.
            let script = r#"(() => {
              if (location.origin !== 'https://developer.spotify.com' || !location.pathname.startsWith('/dashboard/soloist')) return null;
              const categories = t => {
                t = (t || '').toLowerCase();
                return ['soloist','api key','generate','create','copy','show','reveal','regenerate','terms','accept'].filter(x => t.includes(x));
              };
              const heading = [...document.querySelectorAll('h1,h2,h3')].find(e => /soloist/i.test(e.innerText) && /api key/i.test(e.innerText));
              if (!heading) return null;
              let root = heading; for (let i=0; i<4 && root.parentElement; i++) root = root.parentElement;
              const elements = [root, ...root.querySelectorAll('*')].slice(0, 200);
              return elements.map(e => ({
                tag: e.tagName.toLowerCase(),
                type: ['password','text','button','submit','checkbox'].includes(e.type) ? e.type : '',
                role: e.getAttribute('role') === 'button' ? 'button' : '',
                labels: categories(e.innerText || e.getAttribute('aria-label') || e.getAttribute('placeholder')),
                parent: e.parentElement ? e.parentElement.tagName.toLowerCase() : '',
                sibling: e.parentElement ? [...e.parentElement.children].indexOf(e) : 0,
                key_like: !e.children.length && /^[A-Za-z0-9_-]{20,4096}$/.test((e.value || e.textContent || '').trim())
              }));
            })()"#;
            let _ = webview.ExecuteScript(
                PCWSTR(wide(script).as_ptr()),
                &ExecuteScriptCompletedHandler::create(Box::new(|result, value| {
                    if result.is_ok() {
                        let raw = value.to_string();
                        if let Ok(serde_json::Value::Array(items)) = serde_json::from_str::<serde_json::Value>(&raw) {
                            // Revalidate every value against fixed vocabulary before writing.
                            let tags = ["button", "input", "textarea", "label", "h1", "h2", "h3", "div", "span", "form", "section", "main", "p", "li"];
                            let labels = ["soloist", "api key", "generate", "create", "copy", "show", "reveal", "regenerate", "terms", "accept"];
                            let clean: Vec<_> = items.iter().take(200).filter_map(|item| {
                                let tag = item.get("tag")?.as_str()?;
                                let parent = item.get("parent")?.as_str()?;
                                if !tags.contains(&tag) || (!parent.is_empty() && !tags.contains(&parent)) { return None; }
                                let typ = item.get("type")?.as_str()?;
                                if !["", "password", "text", "button", "submit", "checkbox"].contains(&typ) { return None; }
                                let words: Vec<_> = item.get("labels")?.as_array()?.iter().filter_map(|v| v.as_str()).filter(|v| labels.contains(v)).collect();
                                Some(serde_json::json!({"tag": tag, "type": typ, "parent": parent, "sibling": item.get("sibling")?.as_u64()?.min(200), "key_like": item.get("key_like")?.as_bool()?, "labels": words}))
                            }).collect();
                            if let Ok(bytes) = serde_json::to_vec_pretty(&clean) {
                                let _ = std::fs::write(".privateinfo/soloist-dashboard-structure.json", bytes);
                            }
                        }
                    }
                    Ok(())
                })),
            );
        }
        if checked.elapsed() >= Duration::from_millis(750) && !pending.get() {
            checked = Instant::now();
            pending.set(true);
            let busy = pending.clone();
            let output = captured.clone();
            if cookies
                .GetCookies(
                    PCWSTR(wide("").as_ptr()),
                    &GetCookiesCompletedHandler::create(Box::new(move |result, list| {
                        busy.set(false);
                        if result.is_ok() {
                            if let Some(list) = list {
                                let mut count = 0;
                                list.Count(&mut count)?;
                                let mut found = None;
                                for i in 0..count.min(1024) {
                                    let cookie = list.GetValueAtIndex(i)?;
                                    let name = text(|out| cookie.Name(out))?;
                                    if name != "sp_dc" {
                                        continue;
                                    }
                                    let domain = text(|out| cookie.Domain(out))?;
                                    let value = text(|out| cookie.Value(out))?;
                                    let mut secure = windows::core::BOOL(0);
                                    cookie.IsSecure(&mut secure)?;
                                    if crate::spotify_webview::valid_spotify_cookie(
                                        &name,
                                        &domain,
                                        secure.as_bool(),
                                        &value,
                                    ) {
                                        found = Some(value);
                                        break;
                                    }
                                }
                                *output.borrow_mut() = found;
                            }
                        }
                        Ok(())
                    })),
                )
                .is_err()
            {
                pending.set(false);
            }
        }
        let queued: Vec<_> = events.borrow_mut().drain(..).collect();
        for event in queued {
            match event {
                Event::Status(message) => {
                    SetWindowTextW(GetDlgItem(hwnd, STATUS as i32), wide(&tr(message)).as_ptr());
                }
                Event::Download(path) => {
                    let remove_download = initial_download;
                    initial_download = false;
                    let flag = cancel.clone();
                    let (send, receive) = mpsc::sync_channel(1);
                    match std::thread::Builder::new()
                        .name("soloist-install".into())
                        .spawn(move || {
                            let result = (|| {
                                if flag.load(Ordering::Relaxed) {
                                    return Err("Spotify setup cancelled".into());
                                }
                                if path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("exe")) {
                                    let build = super::setup::check(&path)?;
                                    if !build.usable() {
                                        return Err("Selected Soloist build is expired or has an invalid future build date".into());
                                    }
                                    return Ok(build.executable);
                                }
                                let installation = super::setup::import_archive(&path)?;
                                if flag.load(Ordering::Relaxed) {
                                    return Err("Spotify setup cancelled".into());
                                }
                                super::setup::activate(&installation.id)?;
                                Ok(installation.executable)
                            })();
                            if remove_download { let _ = std::fs::remove_file(&path); }
                            let _ = send.send(result);
                        }) {
                        Ok(_) => {
                            installing = Some(receive);
                            EnableWindow(GetDlgItem(hwnd, FINISH as i32), 0);
                            SetWindowTextW(
                                GetDlgItem(hwnd, STATUS as i32),
                                wide(&tr("Checking and installing Soloist...")).as_ptr(),
                            );
                        }
                        Err(_) => {
                            downloading.set(false);
                            SetWindowTextW(
                                GetDlgItem(hwnd, STATUS as i32),
                                wide(&tr("Could not start the Soloist installation.")).as_ptr(),
                            );
                        }
                    }
                }
            }
        }
        let installed = installing
            .as_ref()
            .and_then(|receive| match receive.try_recv() {
                Ok(result) => Some(result),
                Err(mpsc::TryRecvError::Disconnected) => {
                    Some(Err("Soloist installation stopped unexpectedly.".into()))
                }
                Err(_) => None,
            });
        if let Some(result) = installed {
            installing = None;
            downloading.set(false);
            EnableWindow(GetDlgItem(hwnd, FINISH as i32), 1);
            let message = match result {
                Ok(path) => {
                    selected_executable = Some(path);
                    tr("Soloist installed. Sign in to finish setup.")
                }
                Err(error) => tr(&error),
            };
            SetWindowTextW(GetDlgItem(hwnd, STATUS as i32), wide(&message).as_ptr());
        }
        if !inspect
            && !auto_finish_attempted
            && !initial_download
            && installing.is_none()
            && !downloading.get()
            && captured.borrow().is_some()
            && captured_key.borrow().is_some()
        {
            if let Some(key) = captured_key.borrow().as_deref() {
                SetWindowTextW(GetDlgItem(hwnd, KEY as i32), wide(key).as_ptr());
                auto_finish_attempted = true;
                ACTION.set(FINISH);
            }
        }
        match ACTION.replace(0) {
            KEY_PAGE => {
                let _ = webview.Navigate(PCWSTR(wide(super::setup::KEY_PAGE).as_ptr()));
            }
            DOWNLOAD_PAGE => {
                if !downloading.get() && installing.is_none() {
                    if webview
                        .Navigate(PCWSTR(wide(WINDOWS_DOWNLOAD).as_ptr()))
                        .is_err()
                    {
                        SetWindowTextW(GetDlgItem(hwnd, STATUS as i32), wide(&tr("Could not start the Soloist download. Use a local ZIP or EXE instead.")).as_ptr());
                    }
                }
            }
            LOCAL_FILE if installing.is_none() && !downloading.get() => {
                if let Some(path) = local_file(hwnd) {
                    downloading.set(true);
                    events.borrow_mut().push_back(Event::Download(path));
                }
            }
            SIGN_IN => {
                let _ = webview.Navigate(PCWSTR(wide("https://open.spotify.com/").as_ptr()));
            }
            FINISH if installing.is_none() && !downloading.get() => {
                let key = key_text(hwnd);
                let cookie = captured.borrow().clone();
                let save = (|| {
                    if key.is_empty() {
                        return Err(
                            "Paste your personal Soloist key into the field below.".to_owned()
                        );
                    }
                    let executable = selected_executable
                        .clone()
                        .map(Ok)
                        .unwrap_or_else(super::setup::preferred_executable)
                        .map_err(|_| {
                            "Download and install Soloist in this window before finishing."
                        })?;
                    if let Some(cookie) = cookie.as_deref() {
                        super::save_web_cookie(cookie)?;
                        crate::spotify_webview::capture_session(&webview, hwnd, cancel)?;
                    }
                    super::soloist::Config {
                        executable,
                        api_key: key,
                    }
                    .save()
                })();
                match save {
                    Ok(()) => break Ok(()),
                    Err(error) => {
                        SetWindowTextW(GetDlgItem(hwnd, STATUS as i32), wide(&tr(&error)).as_ptr());
                    }
                }
            }
            _ => {}
        }
    };
    // Cancellation also prevents an installation worker from activating after this window closes.
    cancel.store(true, Ordering::Relaxed);
    let _ = webview.remove_NavigationStarting(nav_token);
    let _ = webview.remove_NewWindowRequested(popup_token);
    let _ = webview.remove_PermissionRequested(permission_token);
    let _ = modern.remove_DownloadStarting(download_token);
    let completed_downloads =
        std::mem::replace(&mut *operations.borrow_mut(), Downloads(Vec::new()));
    drop(completed_downloads);
    result
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn download_requires_the_official_windows_archive_origin() {
        assert!(allowed_page(WINDOWS_DOWNLOAD));
        assert!(allowed_download(WINDOWS_DOWNLOAD));
        assert!(allowed_download(
            "https://soloist-builds.spotifycdn.com/releases/soloist_release_windows_x86_64.zip"
        ));
        for uri in [
            "https://evil.example/soloist_release_windows_x86_64.zip",
            "https://soloist-builds.spotifycdn.com.evil/soloist_release_windows_x86_64.zip",
            "http://soloist-builds.spotifycdn.com/soloist_release_windows_x86_64.zip",
            "https://soloist-builds.spotifycdn.com/soloist_release_arm64.tar.gz",
            "https://user@soloist-builds.spotifycdn.com/soloist_release_windows_x86_64.zip",
        ] {
            assert!(!allowed_download(uri));
        }
    }
    #[test]
    fn browser_navigation_is_limited_to_setup_and_identity_sites() {
        assert!(allowed_page(super::super::setup::KEY_PAGE));
        assert!(allowed_page("https://accounts.spotify.com/login"));
        assert!(!allowed_page("https://spotify.com.evil/login"));
        assert!(!allowed_page("file:///C:/secrets"));
        assert!(!allowed_page("javascript:alert(1)"));
    }
}
