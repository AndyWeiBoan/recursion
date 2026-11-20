//! PTY processes owned by the daemon, so terminals survive the UI being closed.

use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use std::collections::{HashMap, VecDeque};
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;

const SCROLLBACK_BYTES: usize = 1 << 20;

#[derive(Debug, Clone)]
pub enum TermEvent {
    Output { panel: String, data: Vec<u8> },
    Exit { panel: String },
}

struct Term {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
    scrollback: Arc<Mutex<VecDeque<u8>>>,
    alive: Arc<AtomicBool>,
}

pub struct Terms {
    map: Mutex<HashMap<String, Term>>,
    pub tx: broadcast::Sender<TermEvent>,
}

#[derive(Debug, Clone, serde::Serialize, PartialEq)]
pub struct TermInfo {
    pub alive: bool,
    pub fg: Option<String>,
}

fn default_shell() -> String {
    std::env::var("SHELL").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "/bin/zsh".into())
}

impl Terms {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(4096);
        Self { map: Mutex::new(HashMap::new()), tx }
    }

    /// Make sure a shell is running for `panel`. Returns the current scrollback (for replay).
    pub fn ensure(&self, panel: &str, cwd: &str, cols: u16, rows: u16) -> anyhow::Result<Vec<u8>> {
        let mut map = self.map.lock().unwrap();
        if let Some(t) = map.get(panel) {
            if t.alive.load(Ordering::Relaxed) {
                let _ = t.master.resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 });
                return Ok(t.scrollback.lock().unwrap().iter().copied().collect());
            }
            map.remove(panel);
        }

        let pair = native_pty_system().openpty(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })?;
        let mut cmd = CommandBuilder::new(default_shell());
        cmd.arg("-l");
        let dir = if std::path::Path::new(cwd).is_dir() { cwd.to_string() } else { dirs::home_dir().unwrap_or_default().to_string_lossy().to_string() };
        cmd.cwd(dir);
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        cmd.env("TERM_PROGRAM", "Recursion");
        cmd.env("RECURSION_PANEL", panel);
        if std::env::var("LANG").map(|l| l.is_empty()).unwrap_or(true) {
            cmd.env("LANG", "en_US.UTF-8");
        }
        let child = pair.slave.spawn_command(cmd)?;
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;

        let scrollback = Arc::new(Mutex::new(VecDeque::with_capacity(64 * 1024)));
        let alive = Arc::new(AtomicBool::new(true));
        {
            let (sb, alive, tx, panel) = (scrollback.clone(), alive.clone(), self.tx.clone(), panel.to_string());
            std::thread::Builder::new().name(format!("pty-{panel}")).spawn(move || {
                let mut buf = [0u8; 16 * 1024];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            {
                                let mut sb = sb.lock().unwrap();
                                sb.extend(&buf[..n]);
                                let over = sb.len().saturating_sub(SCROLLBACK_BYTES);
                                sb.drain(..over);
                            }
                            let _ = tx.send(TermEvent::Output { panel: panel.clone(), data: buf[..n].to_vec() });
                        }
                    }
                }
                alive.store(false, Ordering::Relaxed);
                let _ = tx.send(TermEvent::Exit { panel });
            })?;
        }
        map.insert(panel.to_string(), Term { master: pair.master, writer, child, scrollback, alive });
        Ok(Vec::new())
    }

    pub fn write(&self, panel: &str, data: &[u8]) {
        if let Some(t) = self.map.lock().unwrap().get_mut(panel) {
            let _ = t.writer.write_all(data);
            let _ = t.writer.flush();
        }
    }

    pub fn resize(&self, panel: &str, cols: u16, rows: u16) {
        if let Some(t) = self.map.lock().unwrap().get(panel) {
            let _ = t.master.resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 });
        }
    }

    pub fn kill(&self, panel: &str) {
        if let Some(mut t) = self.map.lock().unwrap().remove(panel) {
            let _ = t.child.kill();
        }
    }

    pub fn kill_all(&self) {
        let mut map = self.map.lock().unwrap();
        for (_, mut t) in map.drain() {
            let _ = t.child.kill();
        }
    }

    /// Alive flag and foreground process name for every running terminal.
    pub fn infos(&self) -> HashMap<String, TermInfo> {
        let map = self.map.lock().unwrap();
        map.iter()
            .map(|(id, t)| {
                let alive = t.alive.load(Ordering::Relaxed);
                let fg = if alive { t.master.process_group_leader().and_then(|pid| process_name(pid as i32)) } else { None };
                (id.clone(), TermInfo { alive, fg })
            })
            .collect()
    }
}

#[cfg(target_os = "macos")]
fn process_name(pid: i32) -> Option<String> {
    let mut buf = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
    let n = unsafe { libc::proc_pidpath(pid, buf.as_mut_ptr() as *mut _, buf.len() as u32) };
    if n <= 0 {
        return None;
    }
    let path = String::from_utf8_lossy(&buf[..n as usize]).to_string();
    path.rsplit('/').next().map(|s| s.to_string())
}

#[cfg(target_os = "linux")]
fn process_name(pid: i32) -> Option<String> {
    std::fs::read_to_string(format!("/proc/{pid}/comm")).ok().map(|s| s.trim().to_string())
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn process_name(_pid: i32) -> Option<String> {
    None
}
