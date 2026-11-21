//! Polls: the human asks a question with fixed options and every agent picks one, with a reason,
//! through the `poll_vote` tool. Votes stay hidden from the agents until the poll closes (when
//! every agent has voted, or the human ends it), so nobody just follows whoever voted first.

use super::{msg, Hub};
use crate::workspace::{now_ms, option_letter, Body, Poll, PollStatus, POLL_OPTIONS_MAX};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;

impl Hub {
    fn emit_poll(&self, panel: &str, p: &Poll) {
        self.emit(json!({ "t": "ws_poll", "panel": panel, "poll": p }));
        self.mark_dirty(panel);
    }

    /// "@alias" for an agent, "the human" for "you".
    fn voter_name(&self, panel: &str, voter: &str) -> String {
        if voter == "you" {
            return "the human".into();
        }
        format!("@{}", self.alias_of(panel, voter).unwrap_or_else(|| "agent".into()))
    }

    /// Start a poll, optionally with the human's own vote, and ask every agent to vote.
    pub async fn poll_create(self: &Arc<Self>, panel: &str, question: &str, description: &str, options: Vec<String>, vote: Option<usize>, cwd: String) -> Result<(), String> {
        let question = question.trim().to_string();
        let options: Vec<String> = options.into_iter().map(|o| o.trim().to_string()).filter(|o| !o.is_empty()).collect();
        if question.is_empty() {
            return Err("投票要有問題".into());
        }
        if options.len() < 2 || options.len() > POLL_OPTIONS_MAX {
            return Err(format!("選項要 2 到 {POLL_OPTIONS_MAX} 個"));
        }
        let voters: Vec<String> = self.with_ws(panel, |ws| ws.members.iter().map(|m| m.id.clone()).collect());
        if voters.is_empty() {
            return Err("這個群聊還沒有 agent".into());
        }
        let mut p = self.with_ws(panel, |ws| {
            ws.next_poll += 1;
            Poll {
                id: crate::model::new_id(),
                num: ws.next_poll,
                question,
                description: description.trim().to_string(),
                options,
                voters,
                votes: vec![],
                status: PollStatus::Open,
                created: now_ms(),
                closed: None,
                msg: String::new(),
            }
        });
        if let Some(i) = vote {
            p.vote("you", i, "", now_ms())?;
        }
        let m = self.push_msg(panel, msg("you", Body::Poll { poll: p.id.clone() }, false));
        p.msg = m.id;
        self.with_ws(panel, |ws| ws.polls.push(p.clone()));
        self.emit_poll(panel, &p);
        self.queue_round(panel, None, cwd);
        Ok(())
    }

    /// A vote from the human (`voter` "you") or an agent. Closes the poll once every agent is in.
    pub fn poll_vote(&self, panel: &str, poll: &str, voter: &str, option: usize, reason: &str) -> Result<Poll, String> {
        let members: Vec<String> = self.with_ws(panel, |ws| ws.members.iter().map(|m| m.id.clone()).collect());
        let p = self.with_ws(panel, |ws| {
            let p = ws.polls.iter_mut().find(|p| p.id == poll).ok_or("No such poll.")?;
            p.vote(voter, option, reason, now_ms())?;
            Ok::<_, String>(p.clone())
        })?;
        if p.agents_done(&members) {
            return Ok(self.poll_close(panel, poll).unwrap_or(p));
        }
        self.emit_poll(panel, &p);
        Ok(p)
    }

    /// End a poll; agents read the result with their next turn.
    pub fn poll_close(&self, panel: &str, poll: &str) -> Option<Poll> {
        let p = self.with_ws(panel, |ws| {
            let p = ws.polls.iter_mut().find(|p| p.id == poll)?;
            p.close(now_ms()).then(|| p.clone())
        })?;
        self.emit_poll(panel, &p);
        self.push_msg(panel, msg("system", Body::PollClosed { poll: p.id.clone() }, false));
        Some(p)
    }

    /// The line an agent reads for a poll message in its next turn. Runs under the workspace lock,
    /// so names come from `names` (member id → alias) rather than a lookup.
    pub(super) fn poll_line(body: &Body, polls: &[Poll], names: &HashMap<String, String>) -> Option<String> {
        match body {
            Body::Poll { poll } => polls.iter().find(|p| &p.id == poll).map(|p| {
                format!(
                    "[human started {} — vote once with the poll_vote tool (poll id, option letter, a one-sentence reason). \
Votes stay hidden until the poll closes. After voting, reply with one short sentence at most.]",
                    p.ballot()
                )
            }),
            Body::PollClosed { poll } => polls.iter().find(|p| &p.id == poll).map(|p| format!("[{} closed. {}]", p.ballot(), p.result(&|v| if v == "you" { "the human".into() } else { format!("@{}", names.get(v).cloned().unwrap_or_else(|| "agent".into())) }))),
            _ => None,
        }
    }

    /// The `poll_*` MCP tools.
    pub fn poll_tool(&self, panel: &str, member: &str, name: &str, args: &Value) -> Result<String, String> {
        let polls: Vec<Poll> = self.with_ws(panel, |ws| ws.polls.clone());
        let find = |id: &str| polls.iter().find(|p| p.id == id || format!("#{}", p.num) == id || p.num.to_string() == id).cloned();
        // While open, an agent sees only how many voted and its own vote.
        let status = |p: &Poll| match p.status {
            PollStatus::Closed => p.result(&|v| self.voter_name(panel, v)),
            PollStatus::Open => {
                let mine = p.votes.iter().find(|v| v.voter == member).map(|v| format!("your vote: {}", option_letter(v.option))).unwrap_or_else(|| "you have not voted".into());
                format!("open — {} of {} agents voted, {mine}; who voted what stays hidden until it closes", p.votes.iter().filter(|v| v.voter != "you").count(), p.voters.len())
            }
        };
        match name {
            "poll_list" => {
                if polls.is_empty() {
                    return Ok("No polls in this chat yet.".into());
                }
                Ok(polls.iter().map(|p| format!("{}\n  {}", p.ballot(), status(p).replace('\n', "\n  "))).collect::<Vec<_>>().join("\n"))
            }
            "poll_read" => {
                let id = args["poll"].as_str().unwrap_or("");
                let p = find(id).ok_or_else(|| format!("No poll `{id}`. Use poll_list to see them."))?;
                Ok(format!("{}\n{}", p.ballot(), status(&p)))
            }
            "poll_vote" => {
                let id = args["poll"].as_str().unwrap_or("").trim();
                // No id: the newest poll still open, which is nearly always the one meant.
                let p = if id.is_empty() { polls.iter().rev().find(|p| p.status == PollStatus::Open).cloned() } else { find(id) };
                let p = p.ok_or_else(|| if id.is_empty() { "No poll is open right now.".to_string() } else { format!("No poll `{id}`. Use poll_list to see them.") })?;
                if !p.voters.iter().any(|v| v == member) {
                    return Err(format!("You joined after poll #{} started, so you are not part of it.", p.num));
                }
                let option = match &args["option"] {
                    Value::Number(n) => n.as_u64().and_then(|k| (k >= 1).then(|| k as usize - 1)).filter(|&k| k < p.options.len()),
                    v => v.as_str().and_then(|s| p.parse_option(s)),
                };
                let option = option.ok_or_else(|| format!("Pick one of the option letters: {}", (0..p.options.len()).map(option_letter).collect::<String>()))?;
                let p = self.poll_vote(panel, &p.id, member, option, args["reason"].as_str().unwrap_or(""))?;
                Ok(match p.status {
                    // This text shows in the chat's tool line, so it doesn't say which option.
                    PollStatus::Closed => format!("Vote recorded. It was the last one, so poll #{} is closed:\n{}", p.num, p.result(&|v| self.voter_name(panel, v))),
                    PollStatus::Open => format!("Vote recorded on poll #{}. The result is shown once everyone has voted.", p.num),
                })
            }
            _ => Err(format!("Unknown tool {name}")),
        }
    }
}

pub fn tools() -> Value {
    json!([
        {
            "name": "poll_vote",
            "description": "Vote in a poll the human started in this chat: pick one option and give a one-sentence reason. \
You can change your vote while the poll is open. Other agents' votes stay hidden until it closes.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "poll": { "type": "string", "description": "Poll id (or #number); omit for the poll that is open now" },
                    "option": { "type": "string", "description": "Option letter: A, B, C…" },
                    "reason": { "type": "string", "description": "Why, in one sentence" }
                },
                "required": ["option", "reason"]
            }
        },
        {
            "name": "poll_list",
            "description": "List this chat's polls: question, options, and the result of closed ones.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "poll_read",
            "description": "One poll in full: for a closed poll who voted what and why; for an open one only how many have voted.",
            "inputSchema": {
                "type": "object",
                "properties": { "poll": { "type": "string", "description": "Poll id (or #number)" } },
                "required": ["poll"]
            }
        }
    ])
}
