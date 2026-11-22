//! Model lists, asked from the CLIs themselves so the picker matches their own `/model` menu
//! and never needs updating here: Claude's `initialize` control request and Codex's `model/list`.

use crate::i18n::tr;
use super::AgentEnv;
use crate::workspace::AgentKind;
use serde::Serialize;
use serde_json::{json, Value};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

#[derive(Serialize, Clone, Debug)]
pub struct ModelOption {
    /// Passed back as the agent's `model`; empty = the CLI's own default.
    pub value: String,
    pub label: String,
    pub description: String,
}

/// Start `cmd`, write `input` lines, and return the first stdout JSON line `pick` accepts.
async fn ask(mut cmd: Command, input: &[Value], pick: impl Fn(&Value) -> Option<Value>) -> Result<Value, String> {
    cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).kill_on_drop(true);
    let mut child = cmd.spawn().map_err(|e| e.to_string())?;
    let mut stdin = child.stdin.take().unwrap();
    for line in input {
        stdin.write_all(format!("{line}\n").as_bytes()).await.map_err(|e| e.to_string())?;
    }
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    let found = tokio::time::timeout(Duration::from_secs(20), async {
        while let Ok(Some(l)) = lines.next_line().await {
            if let Some(v) = serde_json::from_str::<Value>(&l).ok().and_then(|v| pick(&v)) {
                return Some(v);
            }
        }
        None
    })
    .await;
    drop(stdin);
    let _ = child.kill().await;
    found.map_err(|_| tr!("err.timeout"))?.ok_or_else(|| tr!("models.err.none"))
}

pub async fn list(env: &AgentEnv, kind: AgentKind) -> Result<Vec<ModelOption>, String> {
    let home = dirs::home_dir().unwrap_or_default();
    match kind {
        AgentKind::Claude => {
            let bin = env.claude.clone().ok_or_else(|| tr!("err.noCli", cli = "claude"))?;
            let mut cmd = Command::new(bin);
            cmd.args(["-p", "--input-format", "stream-json", "--output-format", "stream-json", "--verbose"])
                .current_dir(&home)
                .env("PATH", &env.path)
                .env("CLAUDE_CODE_ENTRYPOINT", "recursion");
            let init = json!({ "type": "control_request", "request_id": "models", "request": { "subtype": "initialize" } });
            let r = ask(cmd, &[init], |v| {
                (v["type"] == "control_response" && v["response"]["request_id"] == "models").then(|| v["response"]["response"]["models"].clone())
            })
            .await?;
            Ok(r.as_array()
                .into_iter()
                .flatten()
                .filter_map(|m| {
                    let value = m["value"].as_str()?;
                    Some(ModelOption {
                        value: if value == "default" { String::new() } else { value.to_string() },
                        label: m["displayName"].as_str().unwrap_or(value).to_string(),
                        description: m["description"].as_str().unwrap_or("").to_string(),
                    })
                })
                .collect())
        }
        AgentKind::Codex => {
            let bin = env.codex.clone().ok_or_else(|| tr!("err.noCli", cli = "codex"))?;
            let mut cmd = Command::new(bin);
            cmd.arg("app-server").current_dir(&home).env("PATH", &env.path);
            let input = [
                json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "clientInfo": { "name": "recursion", "version": env!("CARGO_PKG_VERSION") } } }),
                json!({ "jsonrpc": "2.0", "method": "initialized" }),
                json!({ "jsonrpc": "2.0", "id": 2, "method": "model/list", "params": {} }),
            ];
            let r = ask(cmd, &input, |v| (v["id"] == 2).then(|| v.clone())).await?;
            if let Some(err) = r.get("error") {
                return Err(err["message"].as_str().unwrap_or("model/list failed").to_string());
            }
            let all: Vec<&Value> = r["result"]["data"].as_array().into_iter().flatten().filter(|m| m["hidden"] != true).collect();
            // Codex has no "default" alias: no model = whatever ~/.codex/config.toml says.
            let mut out = vec![ModelOption { value: String::new(), label: tr!("agent.defaultModel"), description: tr!("models.codexDefault") }];
            out.extend(all.iter().filter_map(|m| {
                let value = m["model"].as_str().or(m["id"].as_str())?;
                Some(ModelOption {
                    value: value.to_string(),
                    label: m["displayName"].as_str().unwrap_or(value).to_string(),
                    description: m["description"].as_str().unwrap_or("").to_string(),
                })
            }));
            Ok(out)
        }
    }
}
