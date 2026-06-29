use crate::util::{
    check_file_or_append, get_app_name, get_download_message_with_lang, show_toast, MessageType,
};
use std::fs::File;
use std::io::Write;
use std::str::FromStr;
use std::sync::atomic::{AtomicI64, Ordering};
use tauri::http::Method;
use tauri::{command, AppHandle, Manager, Url, WebviewWindow};
use tauri_plugin_http::reqwest::{ClientBuilder, Request};

use tauri::Theme;

static BADGE_COUNT: AtomicI64 = AtomicI64::new(0);
const MAX_BADGE_COUNT: i64 = 99_999;
const MAX_BADGE_LABEL_CHARS: usize = 16;

fn normalize_badge_count(count: Option<i64>) -> Option<i64> {
    count.filter(|n| (1..=MAX_BADGE_COUNT).contains(n))
}

fn normalize_badge_label(label: Option<&str>) -> Result<Option<String>, String> {
    let Some(label) = label.map(str::trim).filter(|label| !label.is_empty()) else {
        return Ok(None);
    };

    if label.chars().count() > MAX_BADGE_LABEL_CHARS {
        return Err(format!(
            "Badge label must be {MAX_BADGE_LABEL_CHARS} characters or fewer"
        ));
    }

    Ok(Some(label.to_string()))
}

fn apply_badge(app: &AppHandle, count: Option<i64>) -> Result<(), String> {
    let label = normalize_badge_count(count).map(|n| n.to_string());
    apply_badge_label(app, label.as_deref())
}

#[cfg(target_os = "macos")]
fn apply_badge_label(app: &AppHandle, label: Option<&str>) -> Result<(), String> {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSApplication;
    use objc2_foundation::NSString;

    let label = label.map(str::to_owned);
    app.run_on_main_thread(move || {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let dock_tile = NSApplication::sharedApplication(mtm).dockTile();
        let ns_label = label.as_deref().map(NSString::from_str);
        dock_tile.setBadgeLabel(ns_label.as_deref());
    })
    .map_err(|e| format!("Failed to dispatch dock badge update: {e}"))
}

#[cfg(not(target_os = "macos"))]
fn apply_badge_label(app: &AppHandle, label: Option<&str>) -> Result<(), String> {
    let window = app
        .get_webview_window("pake")
        .ok_or("Main window not found")?;
    let count = label.and_then(|s| s.parse::<i64>().ok());
    window
        .set_badge_count(count)
        .map_err(|e| format!("Failed to set badge count: {e}"))
}

#[derive(serde::Deserialize)]
pub struct DownloadFileParams {
    url: String,
    filename: String,
    language: Option<String>,
}

#[derive(serde::Deserialize)]
pub struct NotificationParams {
    title: String,
    body: String,
    icon: String,
}

#[command]
pub async fn download_file(app: AppHandle, params: DownloadFileParams) -> Result<(), String> {
    let window: WebviewWindow = app.get_webview_window("pake").ok_or("Window not found")?;

    show_toast(
        &window,
        &get_download_message_with_lang(MessageType::Start, params.language.clone()),
    );

    let download_dir = app
        .path()
        .download_dir()
        .map_err(|e| format!("Failed to get download dir: {}", e))?;

    let output_path = download_dir.join(&params.filename);

    let path_str = output_path.to_str().ok_or("Invalid output path")?;

    let file_path = check_file_or_append(path_str);

    let client = ClientBuilder::new()
        .build()
        .map_err(|e| format!("Failed to build client: {}", e))?;

    let url = Url::from_str(&params.url).map_err(|e| format!("Invalid URL: {}", e))?;

    let request = Request::new(Method::GET, url);

    let response = client.execute(request).await;

    match response {
        Ok(mut res) => {
            let mut file =
                File::create(file_path).map_err(|e| format!("Failed to create file: {}", e))?;

            while let Some(chunk) = res
                .chunk()
                .await
                .map_err(|e| format!("Failed to get chunk: {}", e))?
            {
                file.write_all(&chunk)
                    .map_err(|e| format!("Failed to write chunk: {}", e))?;
            }

            show_toast(
                &window,
                &get_download_message_with_lang(MessageType::Success, params.language.clone()),
            );
            Ok(())
        }
        Err(e) => {
            show_toast(
                &window,
                &get_download_message_with_lang(MessageType::Failure, params.language),
            );
            Err(e.to_string())
        }
    }
}

// Windows taskbar flash via Win32 API (raw FFI, no extra crate needed)
#[cfg(target_os = "windows")]
mod win_flash {
    use std::os::raw::c_int;

    #[repr(C)]
    pub struct FLASHWINFO {
        pub cbSize: u32,
        pub hwnd: *mut std::ffi::c_void,
        pub dwFlags: u32,
        pub uCount: u32,
        pub dwTimeout: u32,
    }

    // FLASHW_TRAY = 0x00000002 (flash taskbar button)
    // FLASHW_TIMERNOFG = 0x0000000C (flash until window gets focus)
    pub const FLASHW_TRAY: u32 = 0x00000002;
    pub const FLASHW_TIMERNOFG: u32 = 0x0000000C;

    #[link(name = "user32")]
    extern "system" {
        pub fn FlashWindowEx(pFWI: *const FLASHWINFO) -> c_int;
        pub fn GetForegroundWindow() -> *mut std::ffi::c_void;
    }

    pub unsafe fn flash_taskbar(hwnd: *mut std::ffi::c_void) {
        // Only flash if the window is NOT the foreground window
        let foreground = GetForegroundWindow();
        if hwnd == foreground {
            return;
        }
        let info = FLASHWINFO {
            cbSize: std::mem::size_of::<FLASHWINFO>() as u32,
            hwnd,
            dwFlags: FLASHW_TRAY | FLASHW_TIMERNOFG,
            uCount: 5,
            dwTimeout: 0, // Use default cursor blink rate
        };
        FlashWindowEx(&info);
    }
}

#[command]
pub fn send_notification(app: AppHandle, params: NotificationParams) -> Result<(), String> {
    use tauri_plugin_notification::NotificationExt;

    // Use the locale-aware app name as the notification title so it shows
    // "允知智构" on Chinese systems and "Dedalix" elsewhere, regardless of
    // what title the web page originally supplied.
    let app_name = get_app_name();

    app.notification()
        .builder()
        .title(app_name)
        .body(&params.body)
        .icon(&params.icon)
        .show()
        .map_err(|e| format!("Failed to show notification: {}", e))?;

    // Flash the Windows taskbar button when a notification arrives
    // and the window is not currently in the foreground
    #[cfg(target_os = "windows")]
    {
        if let Some(window) = app.get_webview_window("pake") {
            if let Ok(hwnd) = window.hwnd() {
                unsafe {
                    win_flash::flash_taskbar(hwnd.0 as *mut std::ffi::c_void);
                }
            }
        }
    }

    Ok(())
}

#[command]
pub fn set_dock_badge(app: AppHandle, count: Option<i64>) -> Result<(), String> {
    let normalized = normalize_badge_count(count);
    BADGE_COUNT.store(normalized.unwrap_or(0), Ordering::SeqCst);
    apply_badge(&app, normalized)
}

#[command]
pub fn increment_dock_badge(app: AppHandle) -> Result<(), String> {
    let current = BADGE_COUNT.load(Ordering::SeqCst);
    let next = current.saturating_add(1).clamp(1, MAX_BADGE_COUNT);
    BADGE_COUNT.store(next, Ordering::SeqCst);
    apply_badge(&app, Some(next))
}

#[command]
pub fn clear_dock_badge(app: AppHandle) -> Result<(), String> {
    BADGE_COUNT.store(0, Ordering::SeqCst);
    apply_badge(&app, None)
}

#[command]
pub fn set_dock_badge_label(app: AppHandle, label: Option<String>) -> Result<(), String> {
    BADGE_COUNT.store(0, Ordering::SeqCst);
    let label = normalize_badge_label(label.as_deref())?;
    apply_badge_label(&app, label.as_deref())
}

#[command]
pub async fn update_theme_mode(app: AppHandle, mode: String) {
    if let Some(window) = app.get_webview_window("pake") {
        let theme = if mode == "dark" {
            Theme::Dark
        } else {
            Theme::Light
        };
        let _ = window.set_theme(Some(theme));
    }
}

// Apply native WebView zoom (WKWebView pageZoom / WebView2 ZoomFactor / WebKitGTK
// zoom level) instead of CSS hacks. CSS `transform: scale` and `html.style.zoom`
// break complex SPAs like ChatGPT (fixed positioning shifts, unrepainted layers);
// native zoom recalculates layout the same way a browser does for Cmd/Ctrl +/-.
#[command]
pub fn set_zoom(window: WebviewWindow, percent: f64) -> Result<(), String> {
    let factor = (percent / 100.0).clamp(0.3, 2.0);
    window
        .set_zoom(factor)
        .map_err(|e| format!("Failed to set zoom: {}", e))
}
