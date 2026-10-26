//! Short-lived private WebView2 login for the Deezer ARL session.
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
use webview2_com::{
    CoTaskMemPWSTR, CreateCoreWebView2ControllerCompletedHandler,
    CreateCoreWebView2EnvironmentCompletedHandler, DownloadStartingEventHandler,
    GetCookiesCompletedHandler, Microsoft::Web::WebView2::Win32::*, NavigationStartingEventHandler,
    NewWindowRequestedEventHandler, PermissionRequestedEventHandler,
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
    // cookies retain browser origin scoping and capture still accepts only Deezer ARL.
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
        return Err("Deezer sign-in cancelled".into());
    }
    if started.elapsed() > Duration::from_secs(300) {
        return Err("Deezer sign-in timed out".into());
    }
    if IsWindow(hwnd) == 0 {
        return Err("Deezer sign-in window closed".into());
    }
    let mut message = MSG::default();
    while PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
        if message.message == WM_QUIT {
            return Err("Deezer sign-in window closed".into());
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
fn valid_deezer_cookie(name: &str, domain: &str, secure: bool, value: &str) -> bool {
    name == "arl"
        && matches!(
            domain.trim_start_matches('.'),
            "deezer.com" | "www.deezer.com" | "connect.deezer.com"
        )
        && secure
        && (20..=4096).contains(&value.len())
        && value.bytes().all(|b| b.is_ascii_alphanumeric())
}
pub(crate) fn run(cancel: &AtomicBool) -> Result<String, String> {
    unsafe {
        if CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32) < 0 {
            return Err("Could not initialize sign-in window".into());
        }
        CLOSED.set(false);
        let result = run_sta("https://www.deezer.com/login", cancel, false);
        CoUninitialize();
        result
    }
}
unsafe fn run_sta(url: &str, cancel: &AtomicBool, smoke: bool) -> Result<String, String> {
    if !url.starts_with("https://") || !allowed_navigation(url) {
        return Err("Invalid Deezer sign-in address".into());
    }
    let instance = GetModuleHandleW(std::ptr::null());
    let class_name = wide("DeezNatsDeezerAuth");
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
        wide("Deezer sign-in - DeezNats").as_ptr(),
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
        return Err("Could not create Deezer sign-in window".into());
    }
    let _window = Window(hwnd);
    if !smoke {
        ShowWindow(hwnd, SW_SHOWNORMAL);
    }
    let started = Instant::now();
    let folder = crate::storage::data_dir()?.join("DeezerAuthWebView");
    std::fs::create_dir_all(&folder)
        .map_err(|_| "Could not prepare the isolated Deezer browser profile")?;
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
        .map_err(|_| "Could not enable private Deezer sign-in")?;
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
                            .map_err(|_| "Deezer WebView2 controller could not start".into())
                            .and_then(|_| {
                                controller.ok_or_else(|| {
                                    "Deezer WebView2 controller is unavailable".into()
                                })
                            }),
                    );
                    Ok(())
                },
            )),
        )
        .map_err(|_| "Could not open private Deezer sign-in")?;
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
        .map_err(|_| "Could not size the Deezer window")?;
    controller
        .SetIsVisible(true)
        .map_err(|_| "Could not show Deezer sign-in")?;
    let webview = controller
        .CoreWebView2()
        .map_err(|_| "Deezer WebView2 is unavailable")?;
    let settings = webview
        .Settings()
        .map_err(|_| "Deezer WebView2 settings unavailable")?;
    settings
        .SetAreHostObjectsAllowed(false)
        .map_err(|_| "Could not isolate Deezer sign-in")?;
    settings
        .SetIsWebMessageEnabled(false)
        .map_err(|_| "Could not isolate Deezer messages")?;
    settings
        .SetAreDevToolsEnabled(false)
        .map_err(|_| "Could not configure Deezer sign-in")?;
    settings
        .SetAreDefaultContextMenusEnabled(false)
        .map_err(|_| "Could not configure Deezer context menus")?;
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
        .map_err(|_| "Could not isolate Deezer permissions")?;
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
    let mut nav_token = 0;
    webview
        .add_NavigationStarting(
            &NavigationStartingEventHandler::create(Box::new(|_, args| {
                if let Some(args) = args {
                    let uri = text(|out| args.Uri(out))?;
                    if !allowed_navigation(&uri) {
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
        .map_err(|_| "Deezer cookie manager unavailable")?;
    if smoke {
        let cookie = cookies
            .CreateCookie(
                PCWSTR(wide("arl").as_ptr()),
                PCWSTR(wide(&"a".repeat(192)).as_ptr()),
                PCWSTR(wide(".deezer.com").as_ptr()),
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
        .map_err(|_| "Could not open Deezer sign-in")?;
    let pending = Rc::new(Cell::new(false));
    let mut checked = Instant::now() - Duration::from_secs(1);
    let result = loop {
        if let Some(cookie) = captured.borrow_mut().take() {
            break Ok(cookie);
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
                    PCWSTR(wide("https://www.deezer.com/").as_ptr()),
                    &GetCookiesCompletedHandler::create(Box::new(move |result, list| {
                        busy.set(false);
                        if result.is_ok() {
                            if let Some(list) = list {
                                let mut count = 0;
                                list.Count(&mut count)?;
                                for i in 0..count.min(256) {
                                    let cookie = list.GetValueAtIndex(i)?;
                                    let name = text(|out| cookie.Name(out))?;
                                    if name != "arl" {
                                        continue;
                                    }
                                    let domain = text(|out| cookie.Domain(out))?;
                                    let value = text(|out| cookie.Value(out))?;
                                    let mut secure = windows::core::BOOL(0);
                                    cookie.IsSecure(&mut secure)?;
                                    if valid_deezer_cookie(&name, &domain, secure.as_bool(), &value)
                                    {
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
    let _ = webview.remove_NewWindowRequested(popup_token);
    result
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn captures_only_secure_deezer_arl() {
        assert!(valid_deezer_cookie(
            "arl",
            ".deezer.com",
            true,
            &"a".repeat(192)
        ));
        assert!(!valid_deezer_cookie(
            "arl",
            "example.com",
            true,
            &"a".repeat(192)
        ));
        assert!(!valid_deezer_cookie(
            "other",
            "deezer.com",
            true,
            &"a".repeat(192)
        ));
        assert!(!valid_deezer_cookie(
            "arl",
            "deezer.com",
            false,
            &"a".repeat(192)
        ));
    }
    #[test]
    #[ignore = "opens a private blank WebView2 and captures a synthetic Deezer cookie"]
    fn installed_runtime_captures_synthetic_cookie() {
        unsafe {
            assert!(CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32) >= 0);
            CLOSED.set(false);
            let result = run_sta(
                "https://www.deezer.com/login",
                &AtomicBool::new(false),
                true,
            );
            CoUninitialize();
            assert_eq!(result.unwrap(), "a".repeat(192));
        }
    }
}
