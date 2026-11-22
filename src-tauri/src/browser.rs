//! Browser panels: one native child webview per panel (WKWebView on macOS, WebKitGTK on Linux),
//! laid over the panel body. The web UI owns the layout and tells us where each one goes.
//!
//! These webviews load arbitrary sites, so they get no Tauri IPC: the capability file grants
//! permissions to the `main` webview only. They talk back through Rust-side callbacks.

use serde::{Deserialize, Serialize};
use tauri::webview::{PageLoadEvent, WebviewBuilder};
use tauri::{AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, Rect, Runtime, Url, Webview, WebviewUrl};

#[derive(Deserialize, Clone, Copy)]
pub struct Bounds {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

impl Bounds {
    fn rect(self) -> Rect {
        Rect {
            position: LogicalPosition::new(self.x, self.y).into(),
            size: LogicalSize::new(self.w.max(1.0), self.h.max(1.0)).into(),
        }
    }
}

/// Sent to the main webview whenever a browser panel's page or title changes.
#[derive(Serialize, Clone)]
struct BrowserEvent {
    panel: String,
    url: Option<String>,
    title: Option<String>,
    loading: Option<bool>,
}

fn label(panel: &str) -> String {
    format!("browser-{panel}")
}

fn get<R: Runtime>(app: &AppHandle<R>, panel: &str) -> Option<Webview<R>> {
    app.get_webview(&label(panel))
}

fn parse(url: &str) -> Result<Url, String> {
    let u = Url::parse(url).map_err(|e| format!("Invalid address: {e}"))?;
    match u.scheme() {
        "http" | "https" | "file" | "about" => Ok(u),
        s => Err(format!("Unsupported address type: {s}")),
    }
}

/// What an agent is told when `browse_open` could not load the page.
fn load_failed(url: &str, why: Option<&str>) -> String {
    match why {
        Some(w) => format!("Could not load {url}: {w}. The page was not opened."),
        None => format!("Could not load {url}: the address may not exist or the server did not answer. The page was not opened."),
    }
}

/// Move/resize a browser webview. Linux goes through our own GTK overlay (see `gtk_layer`).
fn place<R: Runtime>(wv: &Webview<R>, b: Bounds) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    return wv
        .with_webview(move |p| gtk_layer::place(&p.inner(), b))
        .map_err(|e| e.to_string());
    #[cfg(not(target_os = "linux"))]
    wv.set_bounds(b.rect()).map_err(|e| e.to_string())
}

fn emit<R: Runtime>(app: &AppHandle<R>, ev: BrowserEvent) {
    let _ = app.emit_to("main", "browser", ev);
}

/// Show the panel's webview at `bounds`, creating it (and loading `url`) the first time.
/// Async on purpose: creating a webview from a sync command deadlocks the main thread.
#[tauri::command]
pub async fn browser_show<R: Runtime>(app: AppHandle<R>, panel: String, url: String, bounds: Bounds) -> Result<(), String> {
    if let Some(wv) = get(&app, &panel) {
        // Show first: a size set while hidden (an agent's webview starts hidden at AGENT_VIEW) is not applied.
        wv.show().map_err(|e| e.to_string())?;
        return place(&wv, bounds);
    }
    create(&app, &panel, &url, bounds).map(|_| ())
}

/// Viewport for a page an agent opened while its panel is off screen.
const AGENT_VIEW: Bounds = Bounds { x: 0.0, y: 0.0, w: 1280.0, h: 800.0 };

/// Make sure the panel has a webview, creating it hidden (an agent is about to use it while the
/// panel is off screen). True when it was just created and is already loading `url`.
#[tauri::command]
pub async fn browser_ensure<R: Runtime>(app: AppHandle<R>, panel: String, url: String) -> Result<bool, String> {
    if get(&app, &panel).is_some() {
        return Ok(false);
    }
    let wv = create(&app, &panel, &url, AGENT_VIEW)?;
    wv.hide().map_err(|e| e.to_string())?;
    Ok(true)
}

fn create<R: Runtime>(app: &AppHandle<R>, panel: &str, url: &str, bounds: Bounds) -> Result<Webview<R>, String> {
    let panel = panel.to_string();
    let target = parse(url)?;
    let window = app.get_window("main").ok_or("main window not found")?;
    let (a, b, c) = (app.clone(), app.clone(), panel.clone());
    let p1 = panel.clone();
    let builder = WebviewBuilder::new(label(&panel), WebviewUrl::External(target))
        .on_page_load(move |_, payload| {
            let done = payload.event() == PageLoadEvent::Finished;
            emit(
                &a,
                BrowserEvent {
                    panel: p1.clone(),
                    url: Some(payload.url().to_string()),
                    title: None,
                    loading: Some(!done),
                },
            );
        })
        .on_document_title_changed(move |_, title| {
            emit(&b, BrowserEvent { panel: c.clone(), url: None, title: Some(title), loading: None });
        });
    let r = bounds.rect();
    let wv = window.add_child(builder, r.position, r.size).map_err(|e| e.to_string())?;
    #[cfg(target_os = "linux")]
    {
        let main = app.get_webview("main").ok_or("main webview not found")?;
        main.with_webview(|p| gtk_layer::remember_main(p.inner())).map_err(|e| e.to_string())?;
        wv.with_webview(move |p| gtk_layer::adopt(&p.inner(), bounds)).map_err(|e| e.to_string())?;
    }
    Ok(wv)
}

#[tauri::command]
pub fn browser_bounds<R: Runtime>(app: AppHandle<R>, panel: String, bounds: Bounds) -> Result<(), String> {
    match get(&app, &panel) {
        Some(wv) => place(&wv, bounds),
        None => Ok(()),
    }
}

#[tauri::command]
pub fn browser_hide<R: Runtime>(app: AppHandle<R>, panel: String) -> Result<(), String> {
    match get(&app, &panel) {
        Some(wv) => wv.hide().map_err(|e| e.to_string()),
        None => Ok(()),
    }
}

#[tauri::command]
pub fn browser_close<R: Runtime>(app: AppHandle<R>, panel: String) -> Result<(), String> {
    match get(&app, &panel) {
        Some(wv) => wv.close().map_err(|e| e.to_string()),
        None => Ok(()),
    }
}

#[tauri::command]
pub fn browser_navigate<R: Runtime>(app: AppHandle<R>, panel: String, url: String) -> Result<(), String> {
    let wv = get(&app, &panel).ok_or("The browser is not open yet")?;
    wv.navigate(parse(&url)?).map_err(|e| e.to_string())
}

/// back / forward / reload / focus.
#[tauri::command]
pub fn browser_go<R: Runtime>(app: AppHandle<R>, panel: String, action: String) -> Result<(), String> {
    let wv = get(&app, &panel).ok_or("The browser is not open yet")?;
    let r = match action.as_str() {
        "back" => wv.eval("history.back()"),
        "forward" => wv.eval("history.forward()"),
        "reload" => wv.reload(),
        "focus" => wv.set_focus(),
        _ => return Err(format!("Unknown action: {action}")),
    };
    r.map_err(|e| e.to_string())
}

/// One step of an agent driving a browser panel: `open` (navigate, wait, read), `read`, `click`,
/// `fill`, `screenshot`. Runs against the OS webview directly, since Tauri's `eval` returns nothing
/// and these pages must never get Tauri IPC.
#[tauri::command]
pub async fn browser_agent<R: Runtime>(app: AppHandle<R>, panel: String, op: String, args: serde_json::Value) -> Result<serde_json::Value, String> {
    let wv = get(&app, &panel).ok_or("The browser is not open yet")?;
    #[cfg(target_os = "linux")]
    {
        let (tx, rx) = tokio::sync::oneshot::channel();
        wv.with_webview(move |p| {
            let v = p.inner();
            webkit2gtk::glib::MainContext::default().spawn_local(async move {
                let _ = tx.send(agent::run(&v, &op, &args).await);
            });
        })
        .map_err(|e| e.to_string())?;
        match tokio::time::timeout(std::time::Duration::from_secs(60), rx).await {
            Ok(Ok(r)) => r,
            Ok(Err(_)) => Err("The browser went away".into()),
            Err(_) => Err("The browser did not answer within 60 seconds".into()),
        }
    }
    #[cfg(target_os = "macos")]
    {
        match tokio::time::timeout(std::time::Duration::from_secs(60), mac_agent::run(&wv, &op, &args)).await {
            Ok(r) => r,
            Err(_) => Err("The browser did not answer within 60 seconds".into()),
        }
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (wv, op, args);
        Err("Agents can't drive the browser on this system yet".into())
    }
}

/// The same steps as `agent` (Linux) on macOS's WKWebView. Each WebKit call happens on the main
/// thread (`with_webview`) and hands its answer back through a channel; the waiting happens here.
#[cfg(target_os = "macos")]
mod mac_agent {
    use base64::Engine;
    use block2::RcBlock;
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSImage};
    use objc2_foundation::{NSDictionary, NSError, NSNumber, NSString};
    use objc2_web_kit::{WKContentWorld, WKSnapshotConfiguration, WKWebView};
    use serde_json::{json, Value};
    use std::sync::Mutex;
    use std::time::{Duration, Instant};
    use tauri::{Runtime, Webview};
    use tokio::sync::oneshot;

    const PAGE: &str = include_str!("agent_page.js");
    /// Screenshots are at most this many pixels wide.
    const SHOT_WIDTH: f64 = 1280.0;

    /// Run `f` with the WKWebView on the main thread; `f` answers through the sender it gets.
    async fn on_main<R: Runtime, T: Send + 'static>(wv: &Webview<R>, f: impl FnOnce(&WKWebView, oneshot::Sender<T>) + Send + 'static) -> Result<T, String> {
        let (tx, rx) = oneshot::channel();
        wv.with_webview(move |p| {
            // SAFETY: on macOS `inner()` is the WKWebView, and this closure runs on the main thread.
            let v: &WKWebView = unsafe { &*(p.inner() as *const WKWebView) };
            f(v, tx);
        })
        .map_err(|e| e.to_string())?;
        rx.await.map_err(|_| "The browser went away".to_string())
    }

    fn error_text(e: *mut NSError) -> String {
        // SAFETY: WebKit passes a valid NSError or null.
        unsafe { e.as_ref() }.map(|e| e.localizedDescription().to_string()).unwrap_or_else(|| "unknown error".into())
    }

    /// The page script, in WebKit's isolated client world (the page's own scripts can't see it).
    async fn page<R: Runtime>(wv: &Webview<R>, op: &str, args: &Value) -> Result<Value, String> {
        let body = format!("const op = {};\nconst args = {};\n{PAGE}", json!(op), args);
        let r: Result<String, String> = on_main(wv, move |v, tx| {
            let tx = Mutex::new(Some(tx));
            let done = RcBlock::new(move |res: *mut AnyObject, err: *mut NSError| {
                let out = if !err.is_null() {
                    Err(format!("Script error in the page: {}", error_text(err)))
                } else {
                    // SAFETY: WebKit passes the function's result (our JSON string) or null.
                    match unsafe { res.as_ref() }.and_then(|o| o.downcast_ref::<NSString>()) {
                        Some(s) => Ok(s.to_string()),
                        None => Err("The page returned nothing".into()),
                    }
                };
                if let Some(tx) = tx.lock().unwrap().take() {
                    let _ = tx.send(out);
                }
            });
            let mtm = MainThreadMarker::new().expect("with_webview runs on the main thread");
            let args: Retained<NSDictionary<NSString, AnyObject>> = NSDictionary::new();
            // SAFETY: plain WebKit call on the main thread with valid arguments.
            unsafe {
                v.callAsyncJavaScript_arguments_inFrame_inContentWorld_completionHandler(
                    &NSString::from_str(&body),
                    Some(&args),
                    None,
                    &WKContentWorld::defaultClientWorld(mtm),
                    Some(&done),
                )
            };
        })
        .await?;
        let out: Value = serde_json::from_str(&r?).map_err(|e| format!("Couldn't read what the page returned: {e}"))?;
        match out.get("error").and_then(|e| e.as_str()) {
            Some(e) => Err(e.to_string()),
            None => Ok(out),
        }
    }

    /// (loading, current url)
    async fn state<R: Runtime>(wv: &Webview<R>) -> (bool, Option<String>) {
        on_main(wv, |v, tx| {
            // SAFETY: plain property reads on the main thread.
            let r = unsafe { (v.isLoading(), v.URL().and_then(|u| u.absoluteString()).map(|s| s.to_string())) };
            let _ = tx.send(r);
        })
        .await
        .unwrap_or((false, None))
    }

    /// Until the page has loaded (or 20 s): WebKit's own flag, then the document's.
    async fn settle<R: Runtime>(wv: &Webview<R>) {
        let end = Instant::now() + Duration::from_secs(20);
        tokio::time::sleep(Duration::from_millis(250)).await;
        while Instant::now() < end {
            if !state(wv).await.0 {
                if let Ok(i) = page(wv, "info", &json!({})).await {
                    if i["ready"] == "complete" {
                        return;
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
    }

    async fn screenshot<R: Runtime>(wv: &Webview<R>) -> Result<Value, String> {
        let png: Result<(Vec<u8>, usize, usize), String> = on_main(wv, |v, tx| {
            let mtm = MainThreadMarker::new().expect("with_webview runs on the main thread");
            let width = v.bounds().size.width;
            if width < 2.0 || v.bounds().size.height < 2.0 {
                let _ = tx.send(Err("The view is empty (the panel may not have been shown yet)".into()));
                return;
            }
            let scale = v.window().map(|w| w.backingScaleFactor()).unwrap_or(1.0);
            // SAFETY: WebKit objects created and used on the main thread.
            let conf = unsafe { WKSnapshotConfiguration::new(mtm) };
            unsafe { conf.setSnapshotWidth(Some(&NSNumber::new_f64(width.min(SHOT_WIDTH / scale)))) };
            let tx = Mutex::new(Some(tx));
            let done = RcBlock::new(move |img: *mut NSImage, err: *mut NSError| {
                // SAFETY: WebKit passes a valid NSImage or null.
                let out = match unsafe { img.as_ref() } {
                    None => Err(format!("Screenshot failed: {}", error_text(err))),
                    Some(img) => img
                        .TIFFRepresentation()
                        .and_then(|tiff| NSBitmapImageRep::imageRepWithData(&tiff))
                        .and_then(|rep| {
                            // SAFETY: standard PNG encoding with no options.
                            let data = unsafe { rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new()) }?;
                            Some((data.to_vec(), rep.pixelsWide() as usize, rep.pixelsHigh() as usize))
                        })
                        .ok_or_else(|| "Couldn't encode the screenshot".to_string()),
                };
                if let Some(tx) = tx.lock().unwrap().take() {
                    let _ = tx.send(out);
                }
            });
            // SAFETY: plain WebKit call on the main thread.
            unsafe { v.takeSnapshotWithConfiguration_completionHandler(Some(&conf), &done) };
        })
        .await?;
        let (png, w, h) = png?;
        Ok(json!({ "png": base64::engine::general_purpose::STANDARD.encode(png), "width": w, "height": h }))
    }

    pub async fn run<R: Runtime>(wv: &Webview<R>, op: &str, args: &Value) -> Result<Value, String> {
        match op {
            "open" => {
                let url = args["url"].as_str().ok_or("Missing url")?;
                let before = state(wv).await.1;
                // Already there, or already loading it (a new webview, or the panel itself navigated).
                if before.as_deref() != Some(url) {
                    wv.navigate(super::parse(url)?).map_err(|e| e.to_string())?;
                }
                settle(wv).await;
                // WebKit keeps the old page when a load fails (no such host), or shows nothing
                // (connection refused); either way the address did not move to anything new.
                let now = state(wv).await.1;
                let blank = now.as_deref().is_none_or(|u| u.is_empty() || u == "about:blank");
                if before.as_deref() != Some(url) && ((now == before && before.is_some()) || (blank && url != "about:blank")) {
                    return Err(super::load_failed(url, None));
                }
                page(wv, "read", &json!({})).await
            }
            "read" => page(wv, "read", args).await,
            "click" | "fill" => {
                let before = state(wv).await.1;
                let mut quiet = args.clone();
                quiet["defer"] = json!(false);
                let mut r = page(wv, op, &quiet).await?;
                let what = if op == "click" { Some("click") } else if args["submit"].as_bool().unwrap_or(false) { Some("submit") } else { None };
                if let Some(what) = what {
                    // Its own call, since it may navigate away mid-call and never answer.
                    tokio::time::sleep(Duration::from_millis(150)).await;
                    let _ = tokio::time::timeout(Duration::from_secs(2), page(wv, "act", &json!({ "ref": args["ref"], "what": what }))).await;
                }
                // A click may start a navigation, and a single-page site (GitHub's Turbo, say) only
                // changes the address after fetching: watch for up to 2 s before reporting.
                let mut moved = false;
                for _ in 0..13 {
                    tokio::time::sleep(Duration::from_millis(150)).await;
                    let (loading, now) = state(wv).await;
                    if loading || now != before {
                        moved = true;
                        break;
                    }
                }
                if moved {
                    settle(wv).await;
                    if let Ok(i) = page(wv, "info", &json!({})).await {
                        r["url"] = i["url"].clone();
                        r["title"] = i["title"].clone();
                        r["navigated"] = json!(true);
                    }
                }
                Ok(r)
            }
            "screenshot" => {
                let mut r = screenshot(wv).await?;
                if let Ok(i) = page(wv, "info", &json!({})).await {
                    r["url"] = i["url"].clone();
                    r["title"] = i["title"].clone();
                }
                Ok(r)
            }
            other => Err(format!("Unknown browser action: {other}")),
        }
    }
}

#[cfg(target_os = "linux")]
mod agent {
    use base64::Engine;
    use javascriptcore::ValueExt;
    use serde_json::{json, Value};
    use std::time::{Duration, Instant};
    use webkit2gtk::{glib, SnapshotOptions, SnapshotRegion, WebView, WebViewExt};

    const PAGE: &str = include_str!("agent_page.js");
    /// Screenshots are scaled down to at most this many pixels wide.
    const SHOT_WIDTH: f64 = 1280.0;

    async fn page(v: &WebView, op: &str, args: &Value) -> Result<Value, String> {
        let body = format!("const op = {};\nconst args = {};\n{PAGE}", json!(op), args);
        let r = v
            .call_async_javascript_function_future(&body, None, Some("recursion-agent"), None)
            .await
            .map_err(|e| format!("Script error in the page: {e}"))?;
        let out: Value = serde_json::from_str(&r.to_str()).map_err(|e| format!("Couldn't read what the page returned: {e}"))?;
        match out.get("error").and_then(|e| e.as_str()) {
            Some(e) => Err(e.to_string()),
            None => Ok(out),
        }
    }

    /// Until the page has loaded (or 20 s): WebKit's own flag, then the document's.
    async fn settle(v: &WebView) {
        let end = Instant::now() + Duration::from_secs(20);
        glib::timeout_future(Duration::from_millis(250)).await;
        while Instant::now() < end {
            if !v.is_loading() {
                if let Ok(i) = page(v, "info", &json!({})).await {
                    if i["ready"] == "complete" {
                        return;
                    }
                }
            }
            glib::timeout_future(Duration::from_millis(150)).await;
        }
    }

    async fn screenshot(v: &WebView) -> Result<Value, String> {
        let shot = v.snapshot_future(SnapshotRegion::Visible, SnapshotOptions::NONE).await.map_err(|e| format!("Screenshot failed: {e}"))?;
        let src = gtk::cairo::ImageSurface::try_from(shot).map_err(|_| "Unexpected screenshot format")?;
        let (w, h) = (src.width() as f64, src.height() as f64);
        if w < 2.0 || h < 2.0 {
            return Err("The view is empty (the panel may not have been shown yet)".into());
        }
        let k = (SHOT_WIDTH / w).min(1.0);
        let out = gtk::cairo::ImageSurface::create(gtk::cairo::Format::Rgb24, (w * k) as i32, (h * k) as i32).map_err(|e| e.to_string())?;
        {
            let cr = gtk::cairo::Context::new(&out).map_err(|e| e.to_string())?;
            cr.scale(k, k);
            cr.set_source_surface(&src, 0.0, 0.0).map_err(|e| e.to_string())?;
            cr.paint().map_err(|e| e.to_string())?;
        }
        let mut png = Vec::new();
        out.write_to_png(&mut png).map_err(|e| e.to_string())?;
        Ok(json!({ "png": base64::engine::general_purpose::STANDARD.encode(png), "width": out.width(), "height": out.height() }))
    }

    pub async fn run(v: &WebView, op: &str, args: &Value) -> Result<Value, String> {
        match op {
            "open" => {
                let url = args["url"].as_str().ok_or("Missing url")?;
                let failed = std::rc::Rc::new(std::cell::RefCell::new(None::<String>));
                let f = failed.clone();
                let handler = v.connect_load_failed(move |_, _, _, err| {
                    // A load that another navigation replaced (a redirect, say) is not a failure.
                    if !err.matches(webkit2gtk::NetworkError::Cancelled) {
                        *f.borrow_mut() = Some(err.to_string());
                    }
                    false
                });
                // Already there, or already loading it (a new webview, or the panel itself navigated).
                if v.uri().as_deref() != Some(url) {
                    v.load_uri(url);
                }
                settle(v).await;
                glib::prelude::ObjectExt::disconnect(v, handler);
                if let Some(e) = failed.borrow_mut().take() {
                    return Err(super::load_failed(url, Some(&e)));
                }
                page(v, "read", &json!({})).await
            }
            "read" => page(v, "read", args).await,
            "click" | "fill" => {
                let before = v.uri().map(|u| u.to_string());
                let mut r = page(v, op, args).await?;
                // A click may start a navigation, and a single-page site (GitHub's Turbo, say) only
                // changes the address after fetching: watch for up to 2 s before reporting.
                let mut moved = false;
                for _ in 0..13 {
                    glib::timeout_future(Duration::from_millis(150)).await;
                    if v.is_loading() || v.uri().map(|u| u.to_string()) != before {
                        moved = true;
                        break;
                    }
                }
                if moved {
                    settle(v).await;
                    if let Ok(i) = page(v, "info", &json!({})).await {
                        r["url"] = i["url"].clone();
                        r["title"] = i["title"].clone();
                        r["navigated"] = json!(true);
                    }
                }
                Ok(r)
            }
            "screenshot" => {
                let mut r = screenshot(v).await?;
                if let Ok(i) = page(v, "info", &json!({})).await {
                    r["url"] = i["url"].clone();
                    r["title"] = i["title"].clone();
                }
                Ok(r)
            }
            other => Err(format!("Unknown browser action: {other}")),
        }
    }
}

/// Tauri on Linux packs every webview of a window into one vertical GtkBox, so a child webview
/// lands *below* the main UI and its bounds are ignored. Instead we wrap the main webview in a
/// GtkOverlay (once) and float each browser webview on top of it, positioned by margins.
/// Everything here runs on the GTK main thread (`with_webview` callbacks).
#[cfg(target_os = "linux")]
mod gtk_layer {
    use super::Bounds;
    use gtk::prelude::*;
    use std::cell::RefCell;
    use webkit2gtk::WebView;

    thread_local! {
        static MAIN: RefCell<Option<WebView>> = const { RefCell::new(None) };
    }

    pub fn remember_main(main: WebView) {
        MAIN.with(|m| *m.borrow_mut() = Some(main));
    }

    fn overlay() -> Option<gtk::Overlay> {
        let main = MAIN.with(|m| m.borrow().clone())?;
        let parent = main.parent()?;
        if let Ok(ov) = parent.clone().downcast::<gtk::Overlay>() {
            return Some(ov);
        }
        let vbox = parent.downcast::<gtk::Box>().ok()?;
        let ov = gtk::Overlay::new();
        vbox.remove(&main);
        ov.add(&main);
        vbox.pack_start(&ov, true, true, 0);
        ov.show();
        Some(ov)
    }

    pub fn adopt(child: &WebView, b: Bounds) {
        let Some(ov) = overlay() else {
            eprintln!("[browser] no GTK overlay; browser panel will be misplaced");
            return;
        };
        if let Some(parent) = child.parent().and_then(|p| p.downcast::<gtk::Container>().ok()) {
            parent.remove(child);
        }
        child.set_halign(gtk::Align::Start);
        child.set_valign(gtk::Align::Start);
        place(child, b);
        ov.add_overlay(child);
        child.show();
    }

    pub fn place(child: &WebView, b: Bounds) {
        child.set_margin_start(b.x.round() as i32);
        child.set_margin_top(b.y.round() as i32);
        child.set_size_request(b.w.max(1.0).round() as i32, b.h.max(1.0).round() as i32);
        child.queue_resize();
    }
}
