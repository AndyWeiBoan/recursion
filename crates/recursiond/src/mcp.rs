//! MCP servers the human set up for agents. Recursion keeps its own list in `~/.recursion/mcp.json`
//! (readable only by the human: it can hold API keys, as the CLIs' own files do), copied from
//! Claude's and Codex's settings or added by hand. Each session turns some of them on, and its agents
//! get exactly those (plus Recursion's own server), whatever the CLIs have set up themselves.

use crate::i18n::tr;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Server {
    /// Started as a local process (stdio)…
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    /// …or reached over HTTP.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
    /// The older HTTP transport (server-sent events); Codex can't use these.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub sse: bool,
    /// Where it was copied from: "claude", "claude:<project>" or "codex".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
}

impl Server {
    fn valid(&self) -> bool {
        self.command.as_deref().is_some_and(|c| !c.trim().is_empty()) || self.url.as_deref().is_some_and(|u| u.starts_with("http://") || u.starts_with("https://"))
    }

    /// For the UI: env and header values hidden.
    fn masked(&self) -> Value {
        let hide = |m: &BTreeMap<String, String>| m.keys().map(|k| (k.clone(), Value::Null)).collect::<Map<_, _>>();
        json!({ "command": self.command, "args": self.args, "env": hide(&self.env), "url": self.url, "headers": hide(&self.headers), "sse": self.sse, "from": self.from })
    }

    /// The same server, ignoring where it came from.
    fn same(&self, other: &Server) -> bool {
        Server { from: None, ..self.clone() } == Server { from: None, ..other.clone() }
    }
}

/// A server as the UI sends it back: `null` env/header values keep what was there.
#[derive(Debug, Deserialize)]
pub struct Edit {
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, Option<String>>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub headers: BTreeMap<String, Option<String>>,
    #[serde(default)]
    pub sse: bool,
}

#[derive(Default, Serialize, Deserialize)]
struct File {
    #[serde(default)]
    servers: BTreeMap<String, Server>,
    /// What the last session saved turned on: new sessions start from it.
    #[serde(default)]
    last: Vec<String>,
}

/// The last connection test of a server.
#[derive(Debug, Clone, Serialize)]
pub struct Status {
    /// "testing", "ok" or "failed".
    pub state: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub at: u64,
}

pub struct Mcps {
    file: Mutex<File>,
    status: Mutex<BTreeMap<String, Status>>,
}

fn path() -> PathBuf {
    crate::home().join("mcp.json")
}

fn home_file(p: &str) -> PathBuf {
    dirs::home_dir().unwrap_or_default().join(p)
}

/// Names are what agents see in tool names (`mcp__<name>__tool`): keep them plain.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty() && name != "recursion" && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

impl Mcps {
    pub fn load() -> Self {
        let file = std::fs::read(path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        Mcps { file: Mutex::new(file), status: Default::default() }
    }

    fn save(&self) {
        let json = serde_json::to_vec_pretty(&*self.file.lock().unwrap()).unwrap();
        let p = path();
        let tmp = p.with_extension("json.tmp");
        let _ = std::fs::create_dir_all(crate::home());
        let mut o = std::fs::OpenOptions::new();
        o.write(true).create(true).truncate(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
        if o.open(&tmp).and_then(|mut f| std::io::Write::write_all(&mut f, &json)).is_ok() {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
            }
            let _ = std::fs::rename(&tmp, &p);
        }
    }

    pub fn names(&self) -> Vec<String> {
        self.file.lock().unwrap().servers.keys().cloned().collect()
    }

    /// What a new session turns on: what the last one saved did.
    pub fn last(&self) -> Vec<String> {
        let f = self.file.lock().unwrap();
        f.last.iter().filter(|n| f.servers.contains_key(*n)).cloned().collect()
    }

    pub fn set_last(&self, names: &[String]) {
        self.file.lock().unwrap().last = names.to_vec();
        self.save();
    }

    /// Everything the settings page shows: our servers (keys hidden), their last test, and what
    /// Claude and Codex have that could be copied in.
    pub fn view(&self) -> Value {
        let f = self.file.lock().unwrap();
        let servers: Map<String, Value> = f.servers.iter().map(|(k, s)| (k.clone(), s.masked())).collect();
        let found: Vec<Value> = found()
            .into_iter()
            .map(|(name, s)| {
                let state = match f.servers.get(&name) {
                    None => "new",
                    Some(ours) if ours.same(&s) => "same",
                    Some(_) => "changed",
                };
                json!({ "name": name, "from": s.from, "server": s.masked(), "state": state })
            })
            .collect();
        json!({ "t": "mcp_state", "servers": servers, "status": *self.status.lock().unwrap(), "found": found })
    }

    pub fn save_server(&self, name: &str, old: Option<&str>, e: Edit) -> Result<(), String> {
        let name = name.trim();
        if !valid_name(name) {
            return Err(tr!("mcp.err.name"));
        }
        let mut f = self.file.lock().unwrap();
        if old != Some(name) && f.servers.contains_key(name) {
            return Err(tr!("mcp.err.duplicate", name = name));
        }
        let prev = old.and_then(|o| f.servers.get(o)).cloned().unwrap_or_default();
        let keep = |new: BTreeMap<String, Option<String>>, was: &BTreeMap<String, String>| -> BTreeMap<String, String> {
            new.into_iter().filter(|(k, _)| !k.trim().is_empty()).filter_map(|(k, v)| v.or_else(|| was.get(&k).cloned()).map(|v| (k.trim().to_string(), v))).collect()
        };
        let s = Server {
            command: e.command.map(|c| c.trim().to_string()).filter(|c| !c.is_empty()),
            args: e.args,
            env: keep(e.env, &prev.env),
            url: e.url.map(|u| u.trim().to_string()).filter(|u| !u.is_empty()),
            headers: keep(e.headers, &prev.headers),
            sse: e.sse,
            from: prev.from,
        };
        if !s.valid() {
            return Err(tr!("mcp.err.invalid"));
        }
        if let Some(o) = old.filter(|o| *o != name) {
            f.servers.remove(o);
            f.last.iter_mut().filter(|n| *n == o).for_each(|n| *n = name.to_string());
            self.status.lock().unwrap().remove(o);
        }
        f.servers.insert(name.to_string(), s);
        drop(f);
        self.save();
        Ok(())
    }

    pub fn remove(&self, name: &str) {
        let mut f = self.file.lock().unwrap();
        f.servers.remove(name);
        f.last.retain(|n| n != name);
        drop(f);
        self.status.lock().unwrap().remove(name);
        self.save();
    }

    /// Copy servers Claude or Codex has (by name and where from), replacing ours of the same name.
    pub fn import(&self, picks: &[(String, String)]) -> Vec<String> {
        let all = found();
        let mut f = self.file.lock().unwrap();
        let mut done = vec![];
        for (name, from) in picks {
            if let Some((_, s)) = all.iter().find(|(n, s)| n == name && s.from.as_deref() == Some(from)) {
                f.servers.insert(name.clone(), s.clone());
                done.push(name.clone());
            }
        }
        drop(f);
        self.save();
        done
    }

    pub fn get(&self, name: &str) -> Option<Server> {
        self.file.lock().unwrap().servers.get(name).cloned()
    }

    /// Servers not tested since the daemon started.
    pub fn untested(&self) -> Vec<String> {
        let st = self.status.lock().unwrap();
        self.names().into_iter().filter(|n| !st.contains_key(n)).collect()
    }

    pub fn set_status(&self, name: &str, s: Status) {
        self.status.lock().unwrap().insert(name.to_string(), s);
    }

    /// Claude's `--mcp-config` entries for these servers.
    pub fn claude_config(&self, names: &[String]) -> Map<String, Value> {
        let f = self.file.lock().unwrap();
        names
            .iter()
            .filter_map(|n| f.servers.get(n).map(|s| (n, s)))
            .map(|(n, s)| {
                let v = match &s.command {
                    Some(c) => json!({ "type": "stdio", "command": c, "args": s.args, "env": s.env }),
                    None => json!({ "type": if s.sse { "sse" } else { "http" }, "url": s.url, "headers": s.headers }),
                };
                (n.clone(), v)
            })
            .collect()
    }

    /// Codex's `mcp_servers` config for these servers. The ones Codex has set up itself (in the
    /// account's `config.toml`) and the session didn't turn on are switched off.
    pub fn codex_config(&self, names: &[String], codex_home: Option<PathBuf>) -> Map<String, Value> {
        let f = self.file.lock().unwrap();
        let mut out: Map<String, Value> = codex_servers(&codex_home.unwrap_or_else(|| home_file(".codex")).join("config.toml"))
            .into_iter()
            .filter(|(n, _)| !names.contains(n))
            .map(|(n, _)| (n, json!({ "enabled": false })))
            .collect();
        for n in names {
            let Some(s) = f.servers.get(n).filter(|s| !s.sse) else { continue };
            let mut v = match &s.command {
                Some(c) => json!({ "command": c, "args": s.args, "env": s.env }),
                None => json!({ "url": s.url, "http_headers": s.headers }),
            };
            // The human chose these for the session: their tools need no asking.
            v["default_tools_approval_mode"] = json!("approve");
            v["enabled"] = json!(true);
            out.insert(n.clone(), v);
        }
        out
    }
}

/// Writes an agent's MCP config where only the human can read it, returning its path.
pub fn write_run_config(key: &str, config: &Value) -> Result<String, String> {
    let dir = crate::home().join("run");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
    }
    let p = dir.join(format!("mcp-{key}.json"));
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
    let mut f = o.open(&p).map_err(|e| e.to_string())?;
    std::io::Write::write_all(&mut f, config.to_string().as_bytes()).map_err(|e| e.to_string())?;
    Ok(p.to_string_lossy().to_string())
}

/// Servers Claude (user and project scope, in `~/.claude.json`) and Codex (`~/.codex/config.toml`)
/// have set up, each with where it came from.
fn found() -> Vec<(String, Server)> {
    let mut out = vec![];
    if let Some(d) = std::fs::read(home_file(".claude.json")).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()) {
        let take = |servers: &Value, from: String, out: &mut Vec<(String, Server)>| {
            for (name, v) in servers.as_object().into_iter().flatten() {
                if let Some(s) = claude_server(v, from.clone()).filter(|_| valid_name(name)) {
                    out.push((name.clone(), s));
                }
            }
        };
        take(&d["mcpServers"], "claude".into(), &mut out);
        for (project, p) in d["projects"].as_object().into_iter().flatten() {
            take(&p["mcpServers"], format!("claude:{project}"), &mut out);
        }
    }
    out.extend(codex_servers(&home_file(".codex/config.toml")).into_iter().filter(|(n, _)| valid_name(n)));
    out
}

fn str_map(v: &Value) -> BTreeMap<String, String> {
    v.as_object().into_iter().flatten().filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string()))).collect()
}

fn claude_server(v: &Value, from: String) -> Option<Server> {
    let s = Server {
        command: v["command"].as_str().map(String::from),
        args: v["args"].as_array().into_iter().flatten().filter_map(|a| a.as_str().map(String::from)).collect(),
        env: str_map(&v["env"]),
        url: v["url"].as_str().map(String::from),
        headers: str_map(&v["headers"]),
        sse: v["type"] == "sse",
        from: Some(from),
    };
    s.valid().then_some(s)
}

fn codex_servers(path: &std::path::Path) -> Vec<(String, Server)> {
    let Some(doc) = std::fs::read_to_string(path).ok().and_then(|s| s.parse::<toml::Table>().ok()) else { return vec![] };
    let Some(servers) = doc.get("mcp_servers").and_then(|v| v.as_table()) else { return vec![] };
    let strs = |v: Option<&toml::Value>| -> BTreeMap<String, String> {
        v.and_then(|v| v.as_table()).into_iter().flatten().filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string()))).collect()
    };
    servers
        .iter()
        .filter_map(|(name, v)| {
            let mut headers = strs(v.get("http_headers"));
            // A token Codex reads from the environment: copy its value now, as the CLIs' own files hold keys.
            if let Some(var) = v.get("bearer_token_env_var").and_then(|x| x.as_str()) {
                if let Ok(token) = std::env::var(var) {
                    headers.insert("Authorization".into(), format!("Bearer {token}"));
                }
            }
            let s = Server {
                command: v.get("command").and_then(|x| x.as_str()).map(String::from),
                args: v.get("args").and_then(|x| x.as_array()).into_iter().flatten().filter_map(|a| a.as_str().map(String::from)).collect(),
                env: strs(v.get("env")),
                url: v.get("url").and_then(|x| x.as_str()).map(String::from),
                headers,
                sse: false,
                from: Some("codex".into()),
            };
            s.valid().then(|| (name.clone(), s))
        })
        .collect()
}

const PROTOCOL: &str = "2025-06-18";

fn initialize() -> Value {
    json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "protocolVersion": PROTOCOL, "capabilities": {}, "clientInfo": { "name": "recursion", "version": env!("CARGO_PKG_VERSION") } } })
}

fn tools_list() -> Value {
    json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {} })
}

fn initialized() -> Value {
    json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })
}

/// The tool count from a `tools/list` reply, or the server's error.
fn tool_count(reply: &Value) -> Result<usize, String> {
    if let Some(e) = reply.get("error") {
        return Err(e["message"].as_str().unwrap_or("error").to_string());
    }
    Ok(reply["result"]["tools"].as_array().map(|a| a.len()).unwrap_or(0))
}

/// Connect to a server the way an agent would and list its tools. `path` is the PATH agents get.
pub async fn test(s: &Server, path: &str) -> Result<usize, String> {
    let run = async {
        match &s.command {
            Some(c) => test_stdio(c, s, path).await,
            None if s.sse => test_sse(s).await,
            None => test_http(s).await,
        }
    };
    tokio::time::timeout(Duration::from_secs(30), run).await.unwrap_or_else(|_| Err(tr!("mcp.err.timeout")))
}

/// The reply with this id; servers may log or notify in between.
async fn line_reply<R: tokio::io::AsyncBufRead + Unpin>(lines: &mut tokio::io::Lines<R>, id: u64) -> Result<Value, String> {
    while let Some(l) = lines.next_line().await.map_err(|e| e.to_string())? {
        if let Ok(v) = serde_json::from_str::<Value>(&l) {
            if v["id"] == id {
                return Ok(v);
            }
        }
    }
    Err(String::new())
}

async fn test_stdio(command: &str, s: &Server, path: &str) -> Result<usize, String> {
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
    let mut child = tokio::process::Command::new(command)
        .args(&s.args)
        .envs(&s.env)
        .env("PATH", path)
        .current_dir(dirs::home_dir().unwrap_or_default())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| tr!("mcp.err.spawn", command = command, error = e))?;
    let mut stdin = child.stdin.take().unwrap();
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    let mut stderr = child.stderr.take().unwrap();
    let err_tail = tokio::spawn(async move {
        let mut buf = String::new();
        let _ = stderr.read_to_string(&mut buf).await;
        buf
    });
    let result = async {
        stdin.write_all(format!("{}\n", initialize()).as_bytes()).await.map_err(|e| e.to_string())?;
        let init = line_reply(&mut lines, 1).await?;
        if let Some(e) = init.get("error") {
            return Err(e["message"].as_str().unwrap_or("error").to_string());
        }
        stdin.write_all(format!("{}\n{}\n", initialized(), tools_list()).as_bytes()).await.map_err(|e| e.to_string())?;
        tool_count(&line_reply(&mut lines, 2).await?)
    }
    .await;
    match result {
        Ok(n) => Ok(n),
        Err(e) => {
            let _ = child.start_kill();
            let tail = tokio::time::timeout(Duration::from_secs(2), err_tail).await.ok().and_then(|r| r.ok()).unwrap_or_default();
            let tail = tail.trim().lines().rev().take(6).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n");
            Err([e, tail].into_iter().filter(|x| !x.is_empty()).collect::<Vec<_>>().join("\n").chars().take(800).collect::<String>().trim().to_string()).map_err(|e: String| if e.is_empty() { tr!("mcp.err.exited") } else { e })
        }
    }
}

fn client() -> reqwest::Client {
    reqwest::Client::builder().connect_timeout(Duration::from_secs(10)).build().unwrap_or_default()
}

fn http_error(status: reqwest::StatusCode, body: &str) -> String {
    let hint = if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN { format!(" {}", tr!("mcp.err.auth")) } else { String::new() };
    format!("HTTP {status}{hint}\n{}", body.chars().take(300).collect::<String>()).trim().to_string()
}

/// A JSON-RPC reply from a streamable-HTTP response: plain JSON, or the matching SSE `data:` line.
fn parse_reply(body: &str, id: u64) -> Option<Value> {
    if let Ok(v) = serde_json::from_str::<Value>(body) {
        return Some(v);
    }
    body.lines().filter_map(|l| l.strip_prefix("data:")).filter_map(|d| serde_json::from_str::<Value>(d.trim()).ok()).find(|v| v["id"] == id)
}

async fn test_http(s: &Server) -> Result<usize, String> {
    let url = s.url.as_deref().unwrap_or_default();
    let c = client();
    let mut session: Option<String> = None;
    let mut post = async |body: Value, id: Option<u64>| -> Result<Option<Value>, String> {
        let mut r = c.post(url).header("Accept", "application/json, text/event-stream").json(&body).timeout(Duration::from_secs(20));
        for (k, v) in &s.headers {
            r = r.header(k, v);
        }
        if let Some(sid) = &session {
            r = r.header("Mcp-Session-Id", sid);
        }
        let resp = r.send().await.map_err(|e| e.to_string())?;
        if let Some(sid) = resp.headers().get("mcp-session-id").and_then(|v| v.to_str().ok()) {
            session = Some(sid.to_string());
        }
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(http_error(status, &text));
        }
        Ok(id.and_then(|id| parse_reply(&text, id)))
    };
    let init = post(initialize(), Some(1)).await?.ok_or_else(|| tr!("mcp.err.noReply"))?;
    if let Some(e) = init.get("error") {
        return Err(e["message"].as_str().unwrap_or("error").to_string());
    }
    post(initialized(), None).await?;
    tool_count(&post(tools_list(), Some(2)).await?.ok_or_else(|| tr!("mcp.err.noReply"))?)
}

/// The older transport: a GET stream that first names where to POST, then carries the replies.
async fn test_sse(s: &Server) -> Result<usize, String> {
    use futures_util::StreamExt;
    let url = reqwest::Url::parse(s.url.as_deref().unwrap_or_default()).map_err(|e| e.to_string())?;
    let c = client();
    let mut r = c.get(url.clone()).header("Accept", "text/event-stream");
    for (k, v) in &s.headers {
        r = r.header(k, v);
    }
    let resp = r.send().await.map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        let status = resp.status();
        return Err(http_error(status, &resp.text().await.unwrap_or_default()));
    }
    let mut stream = resp.bytes_stream();
    let mut buf = String::new();
    // Events, one at a time: (event name, data).
    let mut next = async || -> Result<(String, String), String> {
        loop {
            if let Some(end) = buf.find("\n\n") {
                let ev: String = buf.drain(..end + 2).collect();
                let name = ev.lines().find_map(|l| l.strip_prefix("event:")).map(|x| x.trim().to_string()).unwrap_or_else(|| "message".into());
                let data = ev.lines().filter_map(|l| l.strip_prefix("data:")).map(|x| x.trim()).collect::<Vec<_>>().join("\n");
                return Ok((name, data));
            }
            match stream.next().await {
                Some(Ok(b)) => buf.push_str(&String::from_utf8_lossy(&b).replace("\r\n", "\n")),
                Some(Err(e)) => return Err(e.to_string()),
                None => return Err(tr!("mcp.err.noReply")),
            }
        }
    };
    let endpoint = loop {
        let (name, data) = next().await?;
        if name == "endpoint" {
            break url.join(&data).map_err(|e| e.to_string())?;
        }
    };
    let post = async |body: Value| -> Result<(), String> {
        let mut r = c.post(endpoint.clone()).json(&body);
        for (k, v) in &s.headers {
            r = r.header(k, v);
        }
        let resp = r.send().await.map_err(|e| e.to_string())?;
        let status = resp.status();
        if !status.is_success() {
            return Err(http_error(status, &resp.text().await.unwrap_or_default()));
        }
        Ok(())
    };
    let mut reply = async |id: u64| -> Result<Value, String> {
        loop {
            let (_, data) = next().await?;
            if let Ok(v) = serde_json::from_str::<Value>(&data) {
                if v["id"] == id {
                    return Ok(v);
                }
            }
        }
    };
    post(initialize()).await?;
    let init = reply(1).await?;
    if let Some(e) = init.get("error") {
        return Err(e["message"].as_str().unwrap_or("error").to_string());
    }
    post(initialized()).await?;
    post(tools_list()).await?;
    tool_count(&reply(2).await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_claude_and_codex_entries() {
        let c = claude_server(&json!({ "type": "http", "url": "https://x.test/mcp", "headers": { "Authorization": "Bearer k" } }), "claude".into()).unwrap();
        assert_eq!(c.url.as_deref(), Some("https://x.test/mcp"));
        assert_eq!(c.headers["Authorization"], "Bearer k");
        assert!(claude_server(&json!({ "type": "stdio" }), "claude".into()).is_none());
        let dir = std::env::temp_dir().join(format!("rec-mcp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("config.toml");
        std::fs::write(&p, "model = \"x\"\n[mcp_servers.fs]\ncommand = \"npx\"\nargs = [\"-y\", \"fs\"]\n[mcp_servers.fs.env]\nKEY = \"v\"\n[mcp_servers.web]\nurl = \"https://w.test\"\n").unwrap();
        let mut got = codex_servers(&p);
        got.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(got[0].0, "fs");
        assert_eq!(got[0].1.args, vec!["-y", "fs"]);
        assert_eq!(got[0].1.env["KEY"], "v");
        assert_eq!(got[1].1.url.as_deref(), Some("https://w.test"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn sse_replies_are_found_by_id() {
        let body = "event: message\ndata: {\"jsonrpc\":\"2.0\",\"method\":\"note\"}\n\nevent: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{\"tools\":[{},{}]}}\n\n";
        assert_eq!(tool_count(&parse_reply(body, 2).unwrap()), Ok(2));
    }

    #[test]
    fn masked_hides_values() {
        let s = Server { command: Some("x".into()), env: [("K".to_string(), "secret".to_string())].into(), ..Default::default() };
        assert!(!s.masked().to_string().contains("secret"));
    }
}
