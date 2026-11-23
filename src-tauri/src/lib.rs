//! Recursion desktop client. All state lives in `recursiond`; this app only makes sure the
//! daemon is running and hands its address to the web UI, which talks to it directly.

mod browser;

use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// Must match `recursiond::PROTOCOL`.
const PROTOCOL: u32 = 28;

#[derive(Serialize, Deserialize, Clone)]
struct DaemonInfo {
    port: u16,
    token: String,
    #[serde(default)]
    protocol: u32,
    #[serde(default)]
    pid: u32,
    /// When the daemon started (ms since epoch); 0 from daemons older than this field.
    #[serde(default)]
    started: u64,
}

/// The daemon speaks our protocol and runs the binary that is on disk now. A rebuilt binary
/// (dev, or an upgrade that kept the protocol) means the running one has old code.
fn current(info: &DaemonInfo) -> bool {
    let built = std::fs::metadata(daemon_binary())
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64);
    info.protocol == PROTOCOL && built.is_none_or(|b| b <= info.started)
}

fn home() -> PathBuf {
    if let Ok(dir) = std::env::var("RECURSION_HOME") {
        return PathBuf::from(dir);
    }
    dirs::home_dir().expect("no home dir").join(".recursion")
}

fn read_discovery() -> Option<DaemonInfo> {
    let raw = std::fs::read(home().join("daemon.json")).ok()?;
    serde_json::from_slice(&raw).ok()
}

fn healthy(info: &DaemonInfo) -> bool {
    let Ok(mut s) = TcpStream::connect_timeout(&([127, 0, 0, 1], info.port).into(), Duration::from_millis(400)) else {
        return false;
    };
    let _ = s.set_read_timeout(Some(Duration::from_millis(600)));
    let req = format!("GET /health?token={} HTTP/1.0\r\nHost: localhost\r\n\r\n", info.token);
    if s.write_all(req.as_bytes()).is_err() {
        return false;
    }
    let mut buf = [0u8; 128];
    let n = s.read(&mut buf).unwrap_or(0);
    String::from_utf8_lossy(&buf[..n]).contains(" 200 ")
}

fn daemon_binary() -> PathBuf {
    let exe_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf()));
    if let Some(dir) = exe_dir {
        let p = dir.join("recursiond");
        if p.exists() {
            if std::env::var_os("APPIMAGE").is_some() {
                return out_of_appimage(&p).unwrap_or(p);
            }
            return p;
        }
    }
    PathBuf::from("recursiond")
}

/// An AppImage's files go away when the app quits (it is mounted only while running), and the
/// daemon must outlive the app: run a copy in ~/.recursion/bin instead. The copy keeps the
/// original's time, so `current` still sees an upgrade.
fn out_of_appimage(inside: &std::path::Path) -> std::io::Result<PathBuf> {
    let meta = std::fs::metadata(inside)?;
    let copy = home().join("bin").join("recursiond");
    if let Ok(m) = std::fs::metadata(&copy) {
        if m.len() == meta.len() && m.modified().ok() == meta.modified().ok() {
            return Ok(copy);
        }
    }
    std::fs::create_dir_all(copy.parent().unwrap())?;
    // Beside it, then renamed over it: the running daemon's binary can't be written to.
    let tmp = copy.with_extension("new");
    std::fs::copy(inside, &tmp)?;
    std::fs::File::options().write(true).open(&tmp)?.set_modified(meta.modified()?)?;
    std::fs::rename(&tmp, &copy)?;
    Ok(copy)
}

fn spawn_daemon() -> std::io::Result<()> {
    std::fs::create_dir_all(home())?;
    let log = std::fs::OpenOptions::new().create(true).append(true).open(home().join("daemon.log"))?;
    let mut cmd = std::process::Command::new(daemon_binary());
    cmd.stdin(std::process::Stdio::null()).stdout(log.try_clone()?).stderr(log);
    if let Some(appdir) = std::env::var_os("APPDIR") {
        undo_appimage_env(&mut cmd, &appdir.to_string_lossy());
    }
    #[cfg(unix)]
    unsafe {
        use std::os::unix::process::CommandExt;
        // New session: the daemon must outlive this app (that is the whole point of detach).
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    cmd.spawn().map(|_| ())
}

/// An AppImage starts us with its own PATH, LD_LIBRARY_PATH, PYTHONHOME… pointing inside it,
/// and in a folder inside it. The daemon's terminals and agents must get the person's own
/// environment (and not keep the AppImage mounted): drop what points inside, start from home.
fn undo_appimage_env(cmd: &mut std::process::Command, appdir: &str) {
    for key in ["APPDIR", "APPIMAGE", "ARGV0", "OWD", "GTK_THEME", "QT_QUICK_FLICKABLE_WHEEL_DECELERATION", "PYTHONDONTWRITEBYTECODE"] {
        cmd.env_remove(key);
    }
    for (key, value) in std::env::vars_os() {
        let value = value.to_string_lossy();
        if !value.contains(appdir) {
            continue;
        }
        let kept: Vec<&str> = value.split(':').filter(|p| !p.is_empty() && !p.contains(appdir)).collect();
        if kept.is_empty() {
            cmd.env_remove(&key);
        } else {
            cmd.env(&key, kept.join(":"));
        }
    }
    if let Some(home) = dirs::home_dir() {
        cmd.current_dir(home);
    }
    // The AppImage runtime also hands us a handle on its mount and a pipe it watches: while
    // anyone holds them it stays mounted. The daemon needs nothing of ours but stdio.
    #[cfg(target_os = "linux")]
    for fd in std::fs::read_dir("/proc/self/fd").into_iter().flatten().flatten() {
        if let Ok(n @ 3..) = fd.file_name().to_string_lossy().parse::<i32>() {
            unsafe {
                libc::fcntl(n, libc::F_SETFD, libc::fcntl(n, libc::F_GETFD) | libc::FD_CLOEXEC);
            }
        }
    }
}

/// The daemon to talk to, started (or replaced when outdated) if need be. Runs on a worker
/// thread: it can wait several seconds, and the main thread must keep answering the window
/// system meanwhile (on macOS a blocked main thread bounces in the Dock and ignores Quit).
#[tauri::command]
async fn daemon_info() -> Result<DaemonInfo, String> {
    tauri::async_runtime::spawn_blocking(find_or_start_daemon).await.map_err(|e| e.to_string())?
}

fn find_or_start_daemon() -> Result<DaemonInfo, String> {
    if let Some(info) = read_discovery().filter(healthy) {
        if current(&info) {
            return Ok(info);
        }
        // Outdated daemon (app upgraded, or its binary rebuilt): stop it; layout is persisted, so nothing is lost but running shells.
        #[cfg(unix)]
        unsafe {
            libc::kill(info.pid as i32, libc::SIGTERM);
        }
        let start = Instant::now();
        while healthy(&info) && start.elapsed() < Duration::from_secs(3) {
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    spawn_daemon().map_err(|e| format!("Couldn't start recursiond: {e}"))?;
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(8) {
        std::thread::sleep(Duration::from_millis(150));
        if let Some(info) = read_discovery().filter(current).filter(healthy) {
            return Ok(info);
        }
    }
    Err(format!("recursiond did not start within 8 seconds; see {}", home().join("daemon.log").display()))
}

/// Dev builds only: `RECURSION_TOUR=<project folder>` makes the UI play the scripted walkthrough
/// (ui/src/devtour.ts) used to record the how-to video.
#[tauri::command]
fn dev_tour() -> Option<String> {
    if cfg!(debug_assertions) {
        std::env::var("RECURSION_TOUR").ok()
    } else {
        None
    }
}

/// Writes a file a doc page downloaded (an exported diagram, say) to the path the human picked.
/// The bytes are the raw request body; the path comes percent-encoded in the `path` header.
#[tauri::command]
async fn save_bytes(request: tauri::ipc::Request<'_>) -> Result<(), String> {
    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else {
        return Err("expected raw bytes".into());
    };
    let path = percent_decode(request.headers().get("path").and_then(|v| v.to_str().ok()).ok_or("missing path")?)?;
    // Off the main thread: a big export must not freeze the window.
    let bytes = bytes.clone();
    tauri::async_runtime::spawn_blocking(move || std::fs::write(path, bytes)).await.map_err(|e| e.to_string())?.map_err(|e| e.to_string())
}

/// Undoes `encodeURIComponent`.
fn percent_decode(s: &str) -> Result<String, String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let hex = std::str::from_utf8(&b[i + 1..i + 3]).map_err(|e| e.to_string())?;
            out.push(u8::from_str_radix(hex, 16).map_err(|e| e.to_string())?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).map_err(|e| e.to_string())
}

/// A newer release found by `update_check`, kept for `update_install`.
#[derive(Default)]
struct PendingUpdate(std::sync::Mutex<Option<tauri_plugin_updater::Update>>);

/// The version of a newer published release, if any. The background check ignores errors
/// (offline, no build for this platform); Settings shows them.
#[tauri::command]
async fn update_check(app: tauri::AppHandle, pending: tauri::State<'_, PendingUpdate>) -> Result<Option<String>, String> {
    use tauri_plugin_updater::UpdaterExt;
    let found = app.updater().map_err(|e| e.to_string())?.check().await.map_err(|e| e.to_string())?;
    let version = found.as_ref().map(|u| u.version.clone());
    *pending.0.lock().unwrap() = found;
    Ok(version)
}

/// Downloads and installs the release `update_check` found, then restarts into it. The daemon
/// keeps running; the new app replaces it because its binary on disk changed (see `current`).
#[tauri::command]
async fn update_install(app: tauri::AppHandle, pending: tauri::State<'_, PendingUpdate>) -> Result<(), String> {
    let update = pending.0.lock().unwrap().take().ok_or("no update")?;
    update.download_and_install(|_, _| {}, || {}).await.map_err(|e| e.to_string())?;
    app.restart();
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(PendingUpdate::default())
        .invoke_handler(tauri::generate_handler![
            daemon_info,
            dev_tour,
            save_bytes,
            update_check,
            update_install,
            browser::browser_show,
            browser::browser_bounds,
            browser::browser_hide,
            browser::browser_close,
            browser::browser_navigate,
            browser::browser_go,
            browser::browser_ensure,
            browser::browser_agent
        ])
        // macOS: closing the window keeps the app in the Dock, and clicking the Dock icon brings
        // the window back. Quitting the app on close looked like "it won't open" to people.
        .on_window_event(|window, event| {
            if cfg!(target_os = "macos") && window.label() == "main" {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("error while running Recursion")
        .run(|app, event| {
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen { .. } = event {
                use tauri::Manager;
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.show();
                    let _ = w.set_focus();
                }
            }
            let _ = (app, event);
        });
}
