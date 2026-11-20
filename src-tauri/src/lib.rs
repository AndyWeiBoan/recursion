//! Recursion desktop client. All state lives in `recursiond`; this app only makes sure the
//! daemon is running and hands its address to the web UI, which talks to it directly.

use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// Must match `recursiond::PROTOCOL`.
const PROTOCOL: u32 = 3;

#[derive(Serialize, Deserialize, Clone)]
struct DaemonInfo {
    port: u16,
    token: String,
    #[serde(default)]
    protocol: u32,
    #[serde(default)]
    pid: u32,
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
        if info.protocol == PROTOCOL {
            return Ok(info);
        }
        // Outdated daemon (app was upgraded): stop it; layout is persisted, so nothing is lost but running shells.
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
        if let Some(info) = read_discovery().filter(|i| i.protocol == PROTOCOL).filter(healthy) {
            return Ok(info);
        }
    }
    Err(format!("recursiond 沒有在 8 秒內啟動，請看 {}", home().join("daemon.log").display()))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![daemon_info])
        .run(tauri::generate_context!())
        .expect("error while running Recursion");
}
