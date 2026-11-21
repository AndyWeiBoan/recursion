//! Recursion desktop client. All state lives in `recursiond`; this app only makes sure the
//! daemon is running and hands its address to the web UI, which talks to it directly.

mod browser;

use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// Must match `recursiond::PROTOCOL`.
const PROTOCOL: u32 = 23;

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
            return p;
        }
    }
    PathBuf::from("recursiond")
}

fn spawn_daemon() -> std::io::Result<()> {
    std::fs::create_dir_all(home())?;
    let log = std::fs::OpenOptions::new().create(true).append(true).open(home().join("daemon.log"))?;
    let mut cmd = std::process::Command::new(daemon_binary());
    cmd.stdin(std::process::Stdio::null()).stdout(log.try_clone()?).stderr(log);
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

#[tauri::command]
fn daemon_info() -> Result<DaemonInfo, String> {
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
    spawn_daemon().map_err(|e| format!("無法啟動 recursiond：{e}"))?;
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(8) {
        std::thread::sleep(Duration::from_millis(150));
        if let Some(info) = read_discovery().filter(current).filter(healthy) {
            return Ok(info);
        }
    }
    Err(format!("recursiond 沒有在 8 秒內啟動，請看 {}", home().join("daemon.log").display()))
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
fn save_bytes(request: tauri::ipc::Request<'_>) -> Result<(), String> {
    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else {
        return Err("expected raw bytes".into());
    };
    let path = request.headers().get("path").and_then(|v| v.to_str().ok()).ok_or("missing path")?;
    std::fs::write(percent_decode(path)?, bytes).map_err(|e| e.to_string())
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .invoke_handler(tauri::generate_handler![
            daemon_info,
            dev_tour,
            save_bytes,
            browser::browser_show,
            browser::browser_bounds,
            browser::browser_hide,
            browser::browser_close,
            browser::browser_navigate,
            browser::browser_go,
            browser::browser_ensure,
            browser::browser_agent
        ])
        .run(tauri::generate_context!())
        .expect("error while running Recursion");
}
