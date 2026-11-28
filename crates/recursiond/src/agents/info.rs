//! What one agent works with, for its info card: what Recursion tells it (Recursion's rules, the human's rules for it alone, the
//! project's prompt and instructions files, the chat's rules, what each message carries) and what
//! its CLI said it has when it started — Claude reports its tools, MCP servers, skills,
//! subagents and plugins; Codex its model and sandbox. The CLIs' own built-in prompts are not
//! reported, so they can't be shown.

use super::{group_prompt, Hub};
use crate::workspace::AgentKind;
use serde_json::{json, Value};

impl Hub {
    /// What Claude's `system/init` event says it has (kept until it starts again).
    pub(super) fn claude_started(&self, panel: &str, member: &str, init: &Value) {
        let names = |k: &str| -> Value { init[k].as_array().map(|a| a.iter().filter_map(|x| x.as_str().or(x["name"].as_str())).map(String::from).collect::<Vec<_>>()).map(Value::from).unwrap_or(Value::Null) };
        let info = json!({
            "version": init["claude_code_version"],
            "model": init["model"],
            "permission_mode": init["permissionMode"],
            "output_style": init["output_style"],
            "tools": names("tools"),
            "mcp_servers": init["mcp_servers"],
            "skills": names("skills"),
            "agents": names("agents"),
            "plugins": names("plugins"),
        });
        self.rt(panel, |r| r.cli_info.insert(member.to_string(), info));
    }

    /// What Codex's thread start answered (its model, approval, sandbox…), and the MCP servers
    /// Recursion gave it.
    pub(super) fn codex_started(&self, panel: &str, member: &str, result: &Value, mcp: Vec<String>) {
        let mut info = result.clone();
        if let Some(o) = info.as_object_mut() {
            o.remove("thread");
        }
        info["mcp_servers"] = json!(mcp.iter().map(|n| json!({ "name": n })).collect::<Vec<_>>());
        self.rt(panel, |r| r.cli_info.insert(member.to_string(), info));
    }

    /// Everything the info card shows, as of now.
    pub fn agent_info(&self, panel: &str, member: &str) -> Option<Value> {
        let m = self.with_ws(panel, |ws| ws.members.iter().find(|m| m.id == member).cloned())?;
        let (rules, roster) = self.with_ws(panel, |ws| {
            let roster = ws.members.iter().map(|x| format!("@{}{}", x.alias, x.model.as_ref().map(|mm| format!(" ({mm})")).unwrap_or_default())).collect::<Vec<_>>().join(", ");
            (ws.settings.rules.clone(), roster)
        });
        let account = self.accounts.resolve(m.kind, m.account.as_deref());
        let project = self.member_brief(panel, member);
        let todos = self.todo_text(panel);
        let cli = self.rt(panel, |r| r.cli_info.get(member).cloned());
        Some(json!({
            "t": "agent_info",
            "panel": panel,
            "member": member,
            "kind": m.kind,
            "account": self.accounts.get(&account).map(|a| a.name).unwrap_or(account),
            "running": cli.is_some(),
            "cli": cli,
            "mcp": self.member_mcp(panel, member),
            // In the order it reads them; empty parts are left for the card to say so.
            "parts": [
                { "id": "system", "text": group_prompt(&m.alias), "how": if m.kind == AgentKind::Claude { "append-system-prompt" } else { "developerInstructions" } },
                { "id": "project", "text": project },
                { "id": "rules", "text": rules },
                { "id": "agent", "text": m.prompt },
                { "id": "members", "text": format!("[Recursion] Members of this chat: {roster}. You are @{}.", m.alias) },
                { "id": "todos", "text": todos },
            ],
        }))
    }
}
