//! Rewind: start the chat again from one of the human's messages. Everything from that message
//! on is deleted and every agent's conversation goes back to before it (see `Back::plan`); then
//! the new text is sent in its place. Files the agents changed, and what they made (documents,
//! todos, polls), stay as they are.

use super::Hub;
use crate::i18n::tr;
use crate::workspace::{AgentKind, Back, MemberStatus, TurnMark};
use std::sync::Arc;

/// At most this many turns are remembered per agent; a rewind past them starts it afresh.
const MAX_TURNS: usize = 200;

impl Hub {
    /// Rewind to the human's message `target` and send `text` in its place. One rewind at a time
    /// per chat.
    pub async fn rewind(self: &Arc<Self>, panel: &str, target: &str, text: String, cwd: String, reply_to: Option<&str>, files: Vec<String>, refs: Vec<crate::workspace::OutputRef>) -> Result<(), String> {
        if self.rt(panel, |r| std::mem::replace(&mut r.rewinding, true)) {
            return Err(tr!("chat.err.rewinding"));
        }
        let done = self.rewind_to(panel, target, &cwd).await;
        self.rt(panel, |r| r.rewinding = false);
        done?;
        self.send(panel, text, cwd, None, reply_to, files, refs).await
    }

    async fn rewind_to(self: &Arc<Self>, panel: &str, target: &str, cwd: &str) -> Result<(), String> {
        let seq_of = || self.with_ws(panel, |ws| ws.messages.iter().find(|m| m.id == target && m.from == "you").map(|m| m.seq));
        seq_of().ok_or_else(|| tr!("chat.err.rewindGone"))?;
        // Stop whatever runs and drop what waits, then let the answers wind down: nothing they
        // still write may land after the cut.
        self.interrupt_all(panel).await;
        self.rt(panel, |r| {
            r.pending.clear();
            r.busy.clear();
            r.queue.clear();
            r.held.clear();
            r.turn_first.clear();
            r.send_gen += 1;
        });
        self.emit_round(panel);
        for _ in 0..80 {
            if self.with_ws(panel, |ws| ws.members.iter().all(|m| m.status == MemberStatus::Idle)) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        let seq = seq_of().ok_or_else(|| tr!("chat.err.rewindGone"))?;
        let gone: Vec<String> = self.with_ws(panel, |ws| ws.messages.iter().filter(|m| m.seq >= seq).map(|m| m.id.clone()).collect());
        self.remove_msgs(panel, &gone);
        let members = self.with_ws(panel, |ws| ws.members.clone());
        for m in members {
            let marks = self.with_ws(panel, |ws| ws.turns.get(&m.id).cloned().unwrap_or_default());
            let back = Back::plan(m.kind, &marks, seq);
            if back == Back::Keep {
                continue;
            }
            // It starts again from where it goes back to.
            self.kill(panel, &m.id).await;
            self.with_ws(panel, |ws| match &back {
                Back::To { point, turns, .. } => {
                    ws.turns.entry(m.id.clone()).or_default().truncate(*turns);
                    ws.rewinds.insert(m.id.clone(), point.clone());
                }
                _ => {
                    ws.turns.remove(&m.id);
                    ws.rewinds.remove(&m.id);
                }
            });
            let fresh = back == Back::Fresh;
            let seen = match back {
                Back::To { seen, .. } => seen,
                _ => 0,
            };
            self.update_member(panel, &m.id, |x| {
                x.seen = seen;
                if fresh {
                    x.cli_session_id = None;
                }
            });
        }
        self.rt(panel, |r| r.cwd = cwd.to_string());
        self.mark_dirty(panel);
        Ok(())
    }

    /// A turn was handed to an agent, having shown it messages up to `upto`. `point` is Codex's
    /// turn id; Claude's comes when the turn ends (`turn_point`).
    pub(crate) fn turn_started(&self, panel: &str, member: &str, kind: AgentKind, upto: u64, point: Option<String>) {
        self.with_ws(panel, |ws| {
            let marks = ws.turns.entry(member.to_string()).or_default();
            marks.push(TurnMark { upto, end: None, point });
            if marks.len() > MAX_TURNS {
                marks.remove(0);
            }
            // Claude took its rewind with this turn: from now on it resumes where it is.
            if kind == AgentKind::Claude {
                ws.rewinds.remove(member);
            }
        });
        self.mark_dirty(panel);
    }

    /// Claude's last message in the turn: where a rewind to after this turn resumes.
    pub(crate) fn turn_point(&self, panel: &str, member: &str, uuid: String) {
        self.with_ws(panel, |ws| {
            if let Some(t) = ws.turns.get_mut(member).and_then(|v| v.last_mut()) {
                t.point = Some(uuid);
            }
        });
    }

    /// The turn ended: it covers the chat up to here.
    pub(crate) fn turn_finished(&self, panel: &str, member: &str) {
        self.with_ws(panel, |ws| {
            let end = ws.next_seq;
            if let Some(t) = ws.turns.get_mut(member).and_then(|v| v.last_mut()).filter(|t| t.end.is_none()) {
                t.end = Some(end);
            }
        });
        self.mark_dirty(panel);
    }

    /// Where Claude resumes after a rewind, until it has taken it.
    pub(crate) fn rewind_point(&self, panel: &str, member: &str) -> Option<String> {
        self.with_ws(panel, |ws| ws.rewinds.get(member).cloned())
    }

    /// Codex took its rewind, or couldn't: either way it is no longer owed.
    pub(crate) fn rewind_taken(&self, panel: &str, member: &str, failed: bool) {
        self.with_ws(panel, |ws| {
            ws.rewinds.remove(member);
            if failed {
                ws.turns.remove(member);
            }
        });
        if failed {
            self.update_member(panel, member, |m| m.seen = 0);
        }
        self.mark_dirty(panel);
    }
}
