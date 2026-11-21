//! Threads: a side discussion about one message, with the agent that wrote it.
//!
//! Each reply runs in a fork of the agent's main conversation (Claude `--resume … --fork-session`,
//! Codex `thread/fork`), so it knows everything up to that point while the main chat and the other
//! agents never see the side talk. Thread turns are read-only (no edits, no shell): they are for
//! discussing, and nobody is watching the main chat's approval cards for them.

use super::{msg, AgentEnv, Hub};
use crate::workspace::{now_ms, AgentKind, Body, Member, Thread, ThreadMsg, ThreadRef, ThreadStatus};
use serde_json::{json, Value};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

const QUOTE_MAX: usize = 1200;
const TURN_TIMEOUT: Duration = Duration::from_secs(600);

const BRING_BACK: &str = "The human wants to take this thread's conclusion back to the main chat. \
Write it as a short message for everyone there: what was asked, what you concluded, and any decision or next step. \
A few sentences or a short list; no greeting, do not mention 'this thread'. \
Write it in the language the human used in this thread.";

impl Hub {
    fn emit_thread(&self, panel: &str, t: &Thread) {
        self.emit(json!({ "t": "ws_thread", "panel": panel, "thread": t }));
        self.mark_dirty(panel);
    }

    fn update_thread(&self, panel: &str, id: &str, f: impl FnOnce(&mut Thread)) -> Option<Thread> {
        let t = self.with_ws(panel, |ws| {
            let t = ws.threads.iter_mut().find(|t| t.id == id)?;
            f(t);
            t.updated = now_ms();
            Some(t.clone())
        })?;
        self.emit_thread(panel, &t);
        Some(t)
    }

    /// Start a thread about `msg` (a main-chat message by an agent) with the human's first message.
    pub async fn thread_open(self: &Arc<Self>, panel: &str, msg_id: &str, quote: &str, text: &str, cwd: String) -> Result<String, String> {
        let text = text.trim();
        if text.is_empty() {
            return Err("訊息是空的".into());
        }
        let (author, whole) = self
            .with_ws(panel, |ws| {
                let m = ws.messages.iter().find(|m| m.id == msg_id)?;
                let whole = if let Body::Text { text } = &m.body { text.clone() } else { String::new() };
                Some((m.from.clone(), whole))
            })
            .ok_or("找不到那則訊息")?;
        let member = self.with_ws(panel, |ws| ws.members.iter().find(|m| m.id == author).cloned()).ok_or("寫那則訊息的 agent 已經不在群聊裡了")?;
        if member.cli_session_id.is_none() {
            return Err(format!("@{} 還沒有對話可以分岔", member.alias));
        }
        let is_whole = quote.trim().is_empty();
        let quote = if is_whole { whole } else { quote.trim().to_string() };
        let quote: String = quote.chars().take(QUOTE_MAX).collect();
        let t = self.with_ws(panel, |ws| {
            ws.next_thread += 1;
            let t = Thread {
                id: crate::model::new_id(),
                num: ws.next_thread,
                msg: msg_id.to_string(),
                quote,
                whole: is_whole,
                agent: member.id.clone(),
                fork: None,
                messages: vec![],
                status: ThreadStatus::Waiting,
                updated: now_ms(),
            };
            ws.threads.push(t.clone());
            t
        });
        self.emit_thread(panel, &t);
        self.thread_send(panel, &t.id, text, cwd).await?;
        Ok(t.id)
    }

    /// The human says something in a thread; the agent answers in its fork.
    pub async fn thread_send(self: &Arc<Self>, panel: &str, thread: &str, text: &str, cwd: String) -> Result<(), String> {
        let text = text.trim().to_string();
        if text.is_empty() {
            return Ok(());
        }
        let t = self.with_ws(panel, |ws| ws.threads.iter().find(|t| t.id == thread).cloned()).ok_or("找不到這個討論串")?;
        if t.status == ThreadStatus::Replying {
            return Err("agent 還在回覆這個討論串".into());
        }
        self.update_thread(panel, thread, |t| {
            t.messages.push(ThreadMsg { from: "you".into(), text: text.clone(), ts: now_ms() });
            t.status = ThreadStatus::Replying;
        });
        // Everything the agent has not answered yet: a turn that died (daemon restart, error)
        // must not swallow what the human said before it.
        let mut all = t.messages.clone();
        all.push(ThreadMsg { from: "you".into(), text: text.clone(), ts: 0 });
        let text = unanswered(&all, &t.agent);
        // No fork yet means the agent has never seen this thread: introduce it.
        let prompt = if t.fork.is_none() {
            let quoted = t.quote.lines().map(|l| format!("> {l}")).collect::<Vec<_>>().join("\n");
            format!(
                "[Recursion thread #{}] The human opened a side thread about this part of your earlier reply in the group chat:\n\n{quoted}\n\n\
                 Only you and the human see this thread; the other agents and the main chat do not. You can read files here but not \
                 change anything or run commands. Answer the human directly, in their language.\n\nHuman: {text}",
                t.num
            )
        } else {
            text
        };
        let hub = self.clone();
        let (panel, thread) = (panel.to_string(), thread.to_string());
        tokio::spawn(async move { hub.thread_turn(&panel, &thread, prompt, cwd, false).await });
        Ok(())
    }

    /// Ask the thread's agent for a conclusion and post it to the main chat.
    pub async fn thread_bring(self: &Arc<Self>, panel: &str, thread: &str, cwd: String) -> Result<(), String> {
        let t = self.with_ws(panel, |ws| ws.threads.iter().find(|t| t.id == thread).cloned()).ok_or("找不到這個討論串")?;
        if t.status == ThreadStatus::Replying {
            return Err("agent 還在回覆這個討論串".into());
        }
        if t.fork.is_none() {
            return Err("這個討論串還沒有內容可以帶回".into());
        }
        self.update_thread(panel, thread, |t| t.status = ThreadStatus::Replying);
        let hub = self.clone();
        let (panel, thread) = (panel.to_string(), thread.to_string());
        tokio::spawn(async move { hub.thread_turn(&panel, &thread, BRING_BACK.to_string(), cwd, true).await });
        Ok(())
    }

    async fn thread_turn(self: &Arc<Self>, panel: &str, thread: &str, prompt: String, cwd: String, bring: bool) {
        let t = self.with_ws(panel, |ws| ws.threads.iter().find(|t| t.id == thread).cloned());
        let Some(t) = t else { return };
        let member = self.with_ws(panel, |ws| ws.members.iter().find(|m| m.id == t.agent).cloned());
        let result = match member {
            None => Err("這個 agent 已經離開群聊了".to_string()),
            Some(m) => {
                let env = self.env().await;
                let dir = if std::path::Path::new(&cwd).is_dir() { cwd.clone() } else { dirs::home_dir().unwrap_or_default().to_string_lossy().to_string() };
                let run = async {
                    match m.kind {
                        AgentKind::Claude => claude_turn(&env, &m, t.fork.as_deref(), &dir, &prompt).await,
                        AgentKind::Codex => codex_turn(&env, &m, t.fork.as_deref(), &dir, &prompt).await,
                    }
                };
                tokio::time::timeout(TURN_TIMEOUT, run).await.unwrap_or_else(|_| Err("等太久了，沒有回應".into())).map(|r| (r, m))
            }
        };
        match result {
            Ok(((text, fork), m)) => {
                if bring {
                    let mut out = msg(&m.id, Body::Text { text: text.clone() }, false);
                    out.turn = true;
                    out.from_thread = Some(ThreadRef { thread: t.id.clone(), num: t.num });
                    self.push_msg(panel, out);
                }
                self.update_thread(panel, thread, |t| {
                    t.fork = Some(fork);
                    if bring {
                        t.messages.push(ThreadMsg { from: "system".into(), text: "↩ 已帶回主對話".into(), ts: now_ms() });
                        t.status = ThreadStatus::Brought;
                    } else {
                        t.messages.push(ThreadMsg { from: m.id.clone(), text, ts: now_ms() });
                        t.status = ThreadStatus::Waiting;
                    }
                });
            }
            Err(e) => {
                self.update_thread(panel, thread, |t| {
                    t.messages.push(ThreadMsg { from: "system".into(), text: format!("出錯了：{e}"), ts: now_ms() });
                    t.status = ThreadStatus::Waiting;
                });
            }
        }
    }
}

/// The human's messages since the agent last answered, oldest first.
fn unanswered(messages: &[ThreadMsg], agent: &str) -> String {
    let since = messages.iter().rposition(|m| m.from == agent).map_or(0, |i| i + 1);
    messages[since..].iter().filter(|m| m.from == "you").map(|m| m.text.as_str()).collect::<Vec<_>>().join("\n\n")
}

/// One Claude reply in the fork. Returns (text, session id to resume next time).
async fn claude_turn(env: &AgentEnv, m: &Member, fork: Option<&str>, dir: &str, prompt: &str) -> Result<(String, String), String> {
    let bin = env.claude.clone().ok_or("找不到 claude 指令")?;
    let mut cmd = Command::new(bin);
    cmd.args(["-p", "--output-format", "json", "--permission-mode", "default", "--disallowedTools", "Bash,Edit,Write,MultiEdit,NotebookEdit"]);
    match fork {
        Some(sid) => cmd.args(["--resume", sid]),
        None => cmd.args(["--resume", m.cli_session_id.as_deref().ok_or("沒有可分岔的對話")?, "--fork-session"]),
    };
    if let Some(model) = &m.model {
        cmd.args(["--model", model]);
    }
    cmd.current_dir(dir).env("PATH", &env.path).env("CLAUDE_CODE_ENTRYPOINT", "recursion");
    cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    let mut child = cmd.spawn().map_err(|e| format!("無法啟動 claude：{e}"))?;
    let mut stdin = child.stdin.take().unwrap();
    stdin.write_all(prompt.as_bytes()).await.map_err(|e| e.to_string())?;
    drop(stdin);
    let out = child.wait_with_output().await.map_err(|e| e.to_string())?;
    let v: Value = serde_json::from_slice(&out.stdout).map_err(|_| {
        let err = String::from_utf8_lossy(&out.stderr);
        format!("claude 沒有回傳結果{}", if err.trim().is_empty() { String::new() } else { format!("：{}", err.trim().chars().take(400).collect::<String>()) })
    })?;
    let text = v["result"].as_str().unwrap_or("").to_string();
    if v["is_error"].as_bool().unwrap_or(false) {
        return Err(if text.is_empty() { "claude 回報錯誤".into() } else { text });
    }
    let sid = v["session_id"].as_str().ok_or("claude 沒有回傳 session id")?.to_string();
    Ok((text, sid))
}

/// One Codex reply in the fork, through a short-lived app-server. Returns (text, thread id).
async fn codex_turn(env: &AgentEnv, m: &Member, fork: Option<&str>, dir: &str, prompt: &str) -> Result<(String, String), String> {
    let bin = env.codex.clone().ok_or("找不到 codex 指令")?;
    let mut cmd = Command::new(bin);
    cmd.arg("app-server").current_dir(dir).env("PATH", &env.path);
    cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).kill_on_drop(true);
    let mut child = cmd.spawn().map_err(|e| format!("無法啟動 codex：{e}"))?;
    let mut stdin = child.stdin.take().unwrap();
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    let mut next_id = 0u64;
    let mut send = |method: &str, params: Value| {
        next_id += 1;
        (next_id, format!("{}\n", json!({ "jsonrpc": "2.0", "id": next_id, "method": method, "params": params })))
    };

    // Write a request, then read until its response arrives (declining anything Codex asks meanwhile).
    macro_rules! call {
        ($method:expr, $params:expr) => {{
            let (id, line) = send($method, $params);
            stdin.write_all(line.as_bytes()).await.map_err(|e| e.to_string())?;
            loop {
                let l = lines.next_line().await.map_err(|e| e.to_string())?.ok_or("codex 提前結束了")?;
                let Ok(v) = serde_json::from_str::<Value>(&l) else { continue };
                if v["id"] == id && v.get("method").is_none() {
                    if let Some(err) = v.get("error") {
                        break Err(format!("codex {} 失敗：{}", $method, err["message"].as_str().unwrap_or("")));
                    }
                    break Ok(v["result"].clone());
                }
            }
        }};
    }

    call!("initialize", json!({ "clientInfo": { "name": "recursion", "version": env!("CARGO_PKG_VERSION") } }))?;
    stdin.write_all(format!("{}\n", json!({ "jsonrpc": "2.0", "method": "initialized" })).as_bytes()).await.map_err(|e| e.to_string())?;
    let mut params = json!({ "cwd": dir, "approvalPolicy": "never", "sandbox": "read-only" });
    if let Some(model) = &m.model {
        params["model"] = json!(model);
    }
    let started = match fork {
        Some(tid) => {
            params["threadId"] = json!(tid);
            params["excludeTurns"] = json!(true);
            call!("thread/resume", params)?
        }
        None => {
            params["threadId"] = json!(m.cli_session_id.as_deref().ok_or("沒有可分岔的對話")?);
            params["excludeTurns"] = json!(true);
            call!("thread/fork", params)?
        }
    };
    let tid = started["thread"]["id"].as_str().ok_or("codex 沒有回傳 thread id")?.to_string();
    call!("turn/start", json!({ "threadId": tid, "input": [{ "type": "text", "text": prompt, "text_elements": [] }] }))?;

    let mut text = String::new();
    loop {
        let l = lines.next_line().await.map_err(|e| e.to_string())?.ok_or("codex 提前結束了")?;
        let Ok(v) = serde_json::from_str::<Value>(&l) else { continue };
        let method = v["method"].as_str().unwrap_or("");
        if !v["id"].is_null() && !method.is_empty() {
            // Approvals and the like: this is a read-only discussion.
            let reply = json!({ "jsonrpc": "2.0", "id": v["id"], "error": { "code": -32601, "message": "not available in a Recursion thread" } });
            stdin.write_all(format!("{reply}\n").as_bytes()).await.map_err(|e| e.to_string())?;
            continue;
        }
        match method {
            "item/completed" if v["params"]["item"]["type"] == "agentMessage" => {
                let t = v["params"]["item"]["text"].as_str().unwrap_or("");
                if !t.is_empty() {
                    if !text.is_empty() {
                        text.push_str("\n\n");
                    }
                    text.push_str(t);
                }
            }
            "turn/completed" => {
                let turn = &v["params"]["turn"];
                if turn["status"] == "failed" {
                    return Err(turn["error"]["message"].as_str().unwrap_or("codex 這一輪失敗了").to_string());
                }
                break;
            }
            "error" => {
                if v["params"]["willRetry"] != true {
                    return Err(v["params"]["error"]["message"].as_str().unwrap_or("codex 回報錯誤").to_string());
                }
            }
            _ => {}
        }
    }
    let _ = child.kill().await;
    Ok((text, tid))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(from: &str, text: &str) -> ThreadMsg {
        ThreadMsg { from: from.into(), text: text.into(), ts: 0 }
    }

    #[test]
    fn a_dead_turn_does_not_swallow_earlier_messages() {
        // First message's turn died with the daemon: no reply, no error line.
        let msgs = [m("you", "重點是經濟虧損"), m("you", "?")];
        assert_eq!(unanswered(&msgs, "a1"), "重點是經濟虧損\n\n?");
        let msgs = [m("you", "q1"), m("a1", "r1"), m("system", "出錯了"), m("you", "q2"), m("you", "q3")];
        assert_eq!(unanswered(&msgs, "a1"), "q2\n\nq3");
    }
}
