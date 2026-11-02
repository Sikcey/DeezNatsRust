//! Short-lived private WebView2 login for the Spotify sp_dc session.
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
use webview2_com::{
    AddScriptToExecuteOnDocumentCreatedCompletedHandler, CoTaskMemPWSTR,
    CreateCoreWebView2ControllerCompletedHandler, CreateCoreWebView2EnvironmentCompletedHandler,
    DownloadStartingEventHandler, ExecuteScriptCompletedHandler, GetCookiesCompletedHandler,
    Microsoft::Web::WebView2::Win32::*, NavigationCompletedEventHandler,
    NavigationStartingEventHandler, NewWindowRequestedEventHandler,
    PermissionRequestedEventHandler,
};
use windows::core::{Interface, PCWSTR, PWSTR};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::{Com::*, LibraryLoader::GetModuleHandleW},
    UI::WindowsAndMessaging::*,
};
thread_local! { static CONTROLLER:RefCell<Option<ICoreWebView2Controller>>=const {RefCell::new(None)}; static CLOSED:Cell<bool>=const {Cell::new(false)}; }
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}
fn allowed_navigation(uri: &str) -> bool {
    // window.open() may create a blank document before navigating to sign-in.
    if uri == "about:blank" {
        return true;
    }
    let Some((_, rest)) = uri
        .split_once("://")
        .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("https"))
    else {
        return false;
    };
    let host = rest
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    // Identity providers may redirect through other HTTPS origins (for example,
    // an identity-provider domain). This is browser navigation, not credential export:
    // cookies retain browser origin scoping and capture still accepts only Spotify sp_dc.
    let host = host.strip_suffix(":443").unwrap_or(&host);
    !host.is_empty()
        && host.len() <= 253
        && host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".-".contains(&b))
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
        WM_SIZE => {
            CONTROLLER.with(|slot| {
                if let Some(controller) = slot.borrow().as_ref() {
                    let mut r = RECT::default();
                    GetClientRect(hwnd, &mut r);
                    let _ = controller.SetBounds(windows::Win32::Foundation::RECT {
                        left: 0,
                        top: 0,
                        right: r.right,
                        bottom: r.bottom,
                    });
                }
            });
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
        return Err("Spotify sign-in cancelled".into());
    }
    if started.elapsed() > Duration::from_secs(300) {
        return Err("Spotify sign-in timed out".into());
    }
    if IsWindow(hwnd) == 0 {
        return Err("Spotify sign-in window closed".into());
    }
    let mut message = MSG::default();
    while PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
        if message.message == WM_QUIT {
            return Err("Spotify sign-in window closed".into());
        }
        TranslateMessage(&message);
        DispatchMessageW(&message);
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
        if started.elapsed() > Duration::from_secs(30) {
            return Err("Spotify WebView2 initialization timed out. Check the WebView2 Runtime and try again.".into());
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
                DestroyWindow(self.0);
            }
        }
    }
}
pub(crate) fn valid_spotify_cookie(name: &str, domain: &str, secure: bool, value: &str) -> bool {
    name == "sp_dc"
        && matches!(
            domain.trim_start_matches('.'),
            "spotify.com" | "www.spotify.com" | "accounts.spotify.com" | "open.spotify.com"
        )
        && secure
        && (20..=4096).contains(&value.len())
        && value.bytes().all(|b| b.is_ascii_graphic() && b != b';')
}
pub(crate) fn run(url: &str, redirect: &str, cancel: &AtomicBool) -> Result<String, String> {
    unsafe {
        if CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32) < 0 {
            return Err("Could not initialize sign-in window".into());
        }
        CLOSED.set(false);
        let result = run_sta(url, redirect, cancel, false, None);
        CoUninitialize();
        result
    }
}
/// Cookie-only login; closing the private window or setting cancel aborts capture.
pub(crate) fn login_web(cancel: &AtomicBool) -> Result<String, String> {
    run(
        "https://accounts.spotify.com/en/login?continue=https%3A%2F%2Fopen.spotify.com%2F",
        "",
        cancel,
    )
}
pub(crate) fn transport(transport: crate::spotify::web::Transport) -> Result<(), String> {
    unsafe {
        if CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32) < 0 {
            return Err("Could not initialize Spotify web transport".into());
        }
        CLOSED.set(false);
        let cancel = transport.cancel.clone();
        let result = run_sta(
            "https://open.spotify.com/",
            "",
            &cancel,
            false,
            Some(transport),
        );
        CoUninitialize();
        result.map(|_| ())
    }
}
unsafe fn run_sta(
    url: &str,
    redirect: &str,
    cancel: &AtomicBool,
    smoke: bool,
    transport: Option<crate::spotify::web::Transport>,
) -> Result<String, String> {
    if !url.starts_with("https://") || !allowed_navigation(url) {
        return Err("Invalid Spotify sign-in address".into());
    }
    let instance = GetModuleHandleW(std::ptr::null());
    let class_name = wide("DeezNatsSpotifyAuth");
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
        wide("Spotify sign-in - DeezNats").as_ptr(),
        WS_OVERLAPPEDWINDOW,
        CW_USEDEFAULT,
        CW_USEDEFAULT,
        620,
        780,
        std::ptr::null_mut(),
        std::ptr::null_mut(),
        instance,
        std::ptr::null(),
    );
    if hwnd.is_null() {
        return Err("Could not create Spotify sign-in window".into());
    }
    let _window = Window(hwnd);
    if !smoke {
        if transport.is_none() {
            ShowWindow(hwnd, SW_SHOWMAXIMIZED);
        }
    }
    let started = Instant::now();
    let folder = crate::storage::data_dir()?.join("SpotifyAuthWebView");
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
                    args.SetState(COREWEBVIEW2_PERMISSION_STATE_DENY)?;
                }
                Ok(())
            })),
            &mut permission_token,
        )
        .map_err(|_| "Could not isolate Spotify permissions")?;
    let mut download_token = 0;
    let downloads: ICoreWebView2_4 = webview
        .cast()
        .map_err(|_| "Update WebView2 Runtime for isolated sign-in")?;
    downloads
        .add_DownloadStarting(
            &DownloadStartingEventHandler::create(Box::new(|_, args| {
                if let Some(args) = args {
                    args.SetCancel(true)?;
                }
                Ok(())
            })),
            &mut download_token,
        )
        .map_err(|_| "Could not disable sign-in downloads")?;

    let captured = Rc::new(RefCell::new(None::<String>));
    let callback = Rc::new(RefCell::new(None::<String>));
    let navigation_error = Rc::new(RefCell::new(None::<String>));
    let error_output = navigation_error.clone();
    let mut completed_token = 0;
    webview.add_NavigationCompleted(
        &NavigationCompletedEventHandler::create(Box::new(move |_, args| {
            if let Some(args) = args {
                let mut success = windows::core::BOOL(0);
                args.IsSuccess(&mut success)?;
                if !success.as_bool() {
                    let mut status = COREWEBVIEW2_WEB_ERROR_STATUS::default();
                    args.WebErrorStatus(&mut status)?;
                    if status != COREWEBVIEW2_WEB_ERROR_STATUS_OPERATION_CANCELED {
                        *error_output.borrow_mut() = Some(format!("Spotify page failed to load (WebView2 error {}). Check your connection and try again.", status.0));
                    }
                }
            }
            Ok(())
        })),
        &mut completed_token,
    ).map_err(|_| "Could not monitor Spotify navigation")?;
    let cookie_only = redirect.is_empty();
    let callback_output = callback.clone();
    let redirect = redirect.to_owned();
    let mut nav_token = 0;
    webview
        .add_NavigationStarting(
            &NavigationStartingEventHandler::create(Box::new(move |_, args| {
                if let Some(args) = args {
                    let uri = text(|out| args.Uri(out))?;
                    if crate::spotify::is_callback(&uri, &redirect) {
                        *callback_output.borrow_mut() = Some(uri);
                        args.SetCancel(true)?;
                    } else if !allowed_navigation(&uri) {
                        args.SetCancel(true)?;
                    }
                }
                Ok(())
            })),
            &mut nav_token,
        )
        .map_err(|_| "Could not configure sign-in navigation")?;
    let mut popup_token = 0;
    webview
        .add_NewWindowRequested(
            &NewWindowRequestedEventHandler::create(Box::new(|_, args| {
                if let Some(args) = args {
                    let uri = text(|out| args.Uri(out))?;
                    args.SetHandled(!allowed_navigation(&uri))?;
                }
                Ok(())
            })),
            &mut popup_token,
        )
        .map_err(|_| "Could not configure sign-in popups")?;
    let modern: ICoreWebView2_2 = webview
        .cast()
        .map_err(|_| "Update WebView2 Runtime for sign-in")?;
    let cookies = modern
        .CookieManager()
        .map_err(|_| "Spotify cookie manager unavailable")?;
    if let Some(session) = &transport {
        controller
            .SetIsVisible(false)
            .map_err(|_| "Could not hide Spotify web session")?;
        let cookie = cookies
            .CreateCookie(
                PCWSTR(wide("sp_dc").as_ptr()),
                PCWSTR(wide(&session.cookie).as_ptr()),
                PCWSTR(wide(".spotify.com").as_ptr()),
                PCWSTR(wide("/").as_ptr()),
            )
            .map_err(|_| "Could not restore Spotify web session")?;
        cookie
            .SetIsSecure(true)
            .map_err(|_| "Could not secure Spotify web session")?;
        cookie
            .SetIsHttpOnly(true)
            .map_err(|_| "Could not isolate Spotify cookie")?;
        cookies
            .AddOrUpdateCookie(&cookie)
            .map_err(|_| "Could not restore Spotify cookie")?;
        let installed = Rc::new(RefCell::new(None));
        let output = installed.clone();
        webview
            .AddScriptToExecuteOnDocumentCreated(
                PCWSTR(wide(crate::spotify::web::BOOTSTRAP).as_ptr()),
                &AddScriptToExecuteOnDocumentCreatedCompletedHandler::create(Box::new(
                    move |result, _| {
                        *output.borrow_mut() = Some(
                            result
                                .map(|_| ())
                                .map_err(|_| "Could not install Spotify session observer".into()),
                        );
                        Ok(())
                    },
                )),
            )
            .map_err(|_| "Could not install Spotify session observer")?;
        wait(&installed, hwnd, cancel, started)?;
    }
    if smoke {
        let cookie = cookies
            .CreateCookie(
                PCWSTR(wide("sp_dc").as_ptr()),
                PCWSTR(wide(&"a".repeat(192)).as_ptr()),
                PCWSTR(wide(".spotify.com").as_ptr()),
                PCWSTR(wide("/").as_ptr()),
            )
            .map_err(|_| "Could not create synthetic cookie")?;
        cookie
            .SetIsSecure(true)
            .map_err(|_| "Could not secure synthetic cookie")?;
        cookies
            .AddOrUpdateCookie(&cookie)
            .map_err(|_| "Could not set synthetic cookie")?;
        let profile = webview
            .cast::<ICoreWebView2_13>()
            .map_err(|_| "Private profile unavailable")?
            .Profile()
            .map_err(|_| "Private profile unavailable")?;
        let mut private = windows::core::BOOL(0);
        profile
            .IsInPrivateModeEnabled(&mut private)
            .map_err(|_| "Could not verify private profile")?;
        if !private.as_bool() {
            return Err("Sign-in profile is not private".into());
        }
    }
    webview
        .Navigate(PCWSTR(
            wide(if smoke { "about:blank" } else { url }).as_ptr(),
        ))
        .map_err(|_| "Could not open Spotify sign-in")?;
    if let Some(session) = transport {
        return transport_loop(&webview, hwnd, session).map(|_| String::new());
    }
    let pending = Rc::new(Cell::new(false));
    let mut checked = Instant::now() - Duration::from_secs(1);
    let result = loop {
        if let Some(error) = navigation_error.borrow_mut().take() {
            break Err(error);
        }
        if let Some(uri) = callback.borrow().as_ref() {
            if url::Url::parse(uri)
                .is_ok_and(|url| url.query_pairs().any(|(key, _)| key == "error"))
            {
                break Err("Spotify authorization was declined".into());
            }
        }
        // Keep the same private window alive until BOTH grants are present.
        // OAuth may return before the next asynchronous cookie poll completes.
        if (cookie_only || callback.borrow().is_some()) && captured.borrow().is_some() {
            break Ok(serde_json::json!({
                "callback": callback.borrow_mut().take().unwrap_or_default(),
                "sp_dc": captured.borrow_mut().take().unwrap(),
            })
            .to_string());
        }
        if let Err(error) = pump(hwnd, cancel, started) {
            break Err(error);
        }
        if checked.elapsed() >= Duration::from_millis(750) && !pending.get() {
            checked = Instant::now();
            pending.set(true);
            let output = captured.clone();
            let busy = pending.clone();
            if cookies
                .GetCookies(
                    PCWSTR(wide("https://open.spotify.com/").as_ptr()),
                    &GetCookiesCompletedHandler::create(Box::new(move |result, list| {
                        busy.set(false);
                        if result.is_ok() {
                            if let Some(list) = list {
                                let mut count = 0;
                                list.Count(&mut count)?;
                                for i in 0..count.min(256) {
                                    let cookie = list.GetValueAtIndex(i)?;
                                    let name = text(|out| cookie.Name(out))?;
                                    if name != "sp_dc" {
                                        continue;
                                    }
                                    let domain = text(|out| cookie.Domain(out))?;
                                    let value = text(|out| cookie.Value(out))?;
                                    let mut secure = windows::core::BOOL(0);
                                    cookie.IsSecure(&mut secure)?;
                                    if valid_spotify_cookie(
                                        &name,
                                        &domain,
                                        secure.as_bool(),
                                        &value,
                                    ) {
                                        *output.borrow_mut() = Some(value);
                                        break;
                                    }
                                }
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
    };
    let _ = webview.remove_PermissionRequested(permission_token);
    let _ = downloads.remove_DownloadStarting(download_token);
    let _ = webview.remove_NavigationStarting(nav_token);
    let _ = webview.remove_NavigationCompleted(completed_token);
    let _ = webview.remove_NewWindowRequested(popup_token);
    result
}

unsafe fn evaluate(
    webview: &ICoreWebView2,
    hwnd: HWND,
    cancel: &AtomicBool,
    script: &str,
) -> Result<serde_json::Value, String> {
    let slot = Rc::new(RefCell::new(None));
    let output = slot.clone();
    webview
        .ExecuteScript(
            PCWSTR(wide(script).as_ptr()),
            &ExecuteScriptCompletedHandler::create(Box::new(move |result, value| {
                let parsed = if result.is_err() {
                    Err("Spotify WebView2 script failed".into())
                } else {
                    let raw = value.to_string();
                    if raw.len() > 6 * 1024 * 1024 {
                        Err("Spotify web response is too large".into())
                    } else {
                        serde_json::from_str(&raw)
                            .map_err(|_| "Invalid Spotify web response".into())
                    }
                };
                *output.borrow_mut() = Some(parsed);
                Ok(())
            })),
        )
        .map_err(|_| "Spotify WebView2 script unavailable")?;
    let started = Instant::now();
    loop {
        if let Some(value) = slot.borrow_mut().take() {
            return value;
        }
        if started.elapsed() > Duration::from_secs(30) {
            return Err("Spotify WebView2 is not responding. Try Refresh session again.".into());
        }
        pump(hwnd, cancel, started)?;
    }
}
unsafe fn transport_ready(
    webview: &ICoreWebView2,
    hwnd: HWND,
    cancel: &AtomicBool,
    deadline: Instant,
) -> Result<(), String> {
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(30) && Instant::now() < deadline {
        if evaluate(
            webview,
            hwnd,
            cancel,
            "Boolean(window.__deeznatsSpotifyWeb?.ready())",
        )?
        .as_bool()
            == Some(true)
        {
            return Ok(());
        }
        for _ in 0..15 {
            pump(hwnd, cancel, started)?;
        }
    }
    Err("Spotify web authorization expired or could not initialize; sign in again (WebView2 Runtime required)".into())
}
pub(crate) unsafe fn capture_session(
    webview: &ICoreWebView2,
    hwnd: HWND,
    cancel: &AtomicBool,
) -> Result<(), String> {
    crate::spotify::web::save_session(transport_request(
        webview,
        hwnd,
        cancel,
        &serde_json::json!({"kind":"export"}),
    )?)
}
unsafe fn transport_request(
    webview: &ICoreWebView2,
    hwnd: HWND,
    cancel: &AtomicBool,
    command: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let deadline = Instant::now() + Duration::from_secs(90);
    if (command["kind"] == "browse" || command["kind"] == "export")
        && evaluate(
            webview,
            hwnd,
            cancel,
            "location.pathname.split('/').includes('search')",
        )?
        .as_bool()
            != Some(true)
    {
        webview
            .Navigate(PCWSTR(wide("https://open.spotify.com/search").as_ptr()))
            .map_err(|_| "Could not open Spotify browse categories")?;
        let navigating = Instant::now();
        while evaluate(
            webview,
            hwnd,
            cancel,
            "location.pathname.split('/').includes('search')",
        )?
        .as_bool()
            != Some(true)
        {
            if navigating.elapsed() > Duration::from_secs(15) {
                return Err("Spotify browse navigation timed out".into());
            }
            pump(hwnd, cancel, navigating)?;
        }
    }
    for attempt in 0..2 {
        transport_ready(webview, hwnd, cancel, deadline)?;
        evaluate(
            webview,
            hwnd,
            cancel,
            &format!("window.__deeznatsSpotifyWeb.run({command});null"),
        )?;
        loop {
            if Instant::now() >= deadline {
                return Err("Spotify web request timed out".into());
            }
            let value = evaluate(
                webview,
                hwnd,
                cancel,
                "window.__deeznatsSpotifyWeb?.result ?? null",
            )?;
            if value["refresh"] == true {
                if attempt != 0 {
                    return Err(
                        "Spotify web authorization expired or revoked; sign in again".into(),
                    );
                }
                webview
                    .Reload()
                    .map_err(|_| "Could not refresh Spotify Web Player")?;
                let started = Instant::now();
                while started.elapsed() < Duration::from_millis(750) {
                    pump(hwnd, cancel, started)?;
                }
                break;
            }
            if value["ok"] == true {
                return Ok(value["value"].clone());
            }
            if value["ok"] == false {
                return Err(value["error"]
                    .as_str()
                    .unwrap_or("Spotify web request failed")
                    .to_owned());
            }
            for _ in 0..10 {
                pump(hwnd, cancel, Instant::now())?;
            }
        }
    }
    Err("Spotify web authorization expired; sign in again".into())
}
unsafe fn transport_loop(
    webview: &ICoreWebView2,
    hwnd: HWND,
    session: crate::spotify::web::Transport,
) -> Result<(), String> {
    loop {
        if session.cancel.load(Ordering::Relaxed) {
            return Ok(());
        }
        match session.requests.try_recv() {
            Ok(request) => {
                let result = transport_request(webview, hwnd, &session.cancel, &request.command);
                let _ = request.reply.send(result);
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => return Ok(()),
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
        }
        pump(hwnd, &session.cancel, Instant::now())?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn captures_only_secure_spotify_session_cookie() {
        let value = "a".repeat(64);
        assert!(valid_spotify_cookie("sp_dc", ".spotify.com", true, &value));
        assert!(!valid_spotify_cookie(
            "sp_dc",
            ".spotify.com.evil",
            true,
            &value
        ));
        assert!(!valid_spotify_cookie(
            "sp_dc",
            ".evilspotify.com",
            true,
            &value
        ));
        assert!(!valid_spotify_cookie(
            "sp_dc",
            ".spotify.com",
            false,
            &value
        ));
        assert!(!valid_spotify_cookie("other", ".spotify.com", true, &value));
        assert!(!valid_spotify_cookie(
            "sp_dc",
            ".spotify.com",
            true,
            &";".repeat(64)
        ));
    }
}
