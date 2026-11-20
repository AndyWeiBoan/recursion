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
    let u = Url::parse(url).map_err(|e| format!("網址無效：{e}"))?;
    match u.scheme() {
        "http" | "https" | "file" | "about" => Ok(u),
        s => Err(format!("不支援的網址類型：{s}")),
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
        place(&wv, bounds)?;
        return wv.show().map_err(|e| e.to_string());
    }
    let target = parse(&url)?;
    let window = app.get_window("main").ok_or("找不到主視窗")?;
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
        let main = app.get_webview("main").ok_or("找不到主畫面")?;
        main.with_webview(|p| gtk_layer::remember_main(p.inner())).map_err(|e| e.to_string())?;
        wv.with_webview(move |p| gtk_layer::adopt(&p.inner(), bounds)).map_err(|e| e.to_string())?;
    }
    #[cfg(not(target_os = "linux"))]
    let _ = wv;
    Ok(())
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
    let wv = get(&app, &panel).ok_or("瀏覽器還沒開")?;
    wv.navigate(parse(&url)?).map_err(|e| e.to_string())
}

/// back / forward / reload / focus.
#[tauri::command]
pub fn browser_go<R: Runtime>(app: AppHandle<R>, panel: String, action: String) -> Result<(), String> {
    let wv = get(&app, &panel).ok_or("瀏覽器還沒開")?;
    let r = match action.as_str() {
        "back" => wv.eval("history.back()"),
        "forward" => wv.eval("history.forward()"),
        "reload" => wv.reload(),
        "focus" => wv.set_focus(),
        _ => return Err(format!("未知動作：{action}")),
    };
    r.map_err(|e| e.to_string())
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
    }
}
