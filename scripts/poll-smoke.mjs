// End-to-end test of polls with REAL agents (uses a little quota): two agents vote, votes stay
// hidden until the last one is in, then everyone (and poll_read) sees the result.
// Usage: node scripts/poll-smoke.mjs   (Claude haiku ×2; KIND=mixed for Claude + Codex)
import { spawn } from "node:child_process";
import { mkdtempSync, readFileSync, existsSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const bin = new URL("../target/debug/recursiond", import.meta.url).pathname;
const home = mkdtempSync(join(tmpdir(), "recursion-poll-"));
const proj = mkdtempSync(join(tmpdir(), "recursion-proj-"));
const env = { ...process.env, RECURSION_HOME: home, RUST_LOG: process.env.RUST_LOG ?? "warn" };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
let failures = 0;
const check = (ok, what) => { console.log(`${ok ? "✓" : "✗"} ${what}`); if (!ok) failures++; };

const p = spawn(bin, { env, stdio: "inherit" });
let d;
for (let i = 0; i < 50 && !d; i++) {
  await sleep(100);
  if (existsSync(join(home, "daemon.json"))) d = JSON.parse(readFileSync(join(home, "daemon.json")));
}
const ws = new WebSocket(`ws://127.0.0.1:${d.port}/ws?token=${d.token}`);
const msgs = [];
ws.onmessage = (e) => msgs.push(JSON.parse(e.data));
await new Promise((r) => (ws.onopen = r));
const send = (o) => ws.send(JSON.stringify(o));
const until = async (pred, ms = 180000, from = 0) => {
  const end = Date.now() + ms;
  while (Date.now() < end) { const hit = msgs.slice(from).findLast(pred); if (hit) return hit; await sleep(200); }
  return null;
};

send({ t: "session_create", name: "poll", cwd: proj, kind: "workspace" });
const st = await until((m) => m.t === "state" && m.state.sessions.length === 1, 10000);
const panel = st.state.sessions[0].windows[0].focus;
send({ t: "ws_attach", panel });
send({ t: "ws_add_agent", panel, kind: "claude", alias: "a", model: "haiku" });
const a = (await until((m) => m.t === "ws_member" && m.member.alias === "a", 10000)).member.id;
const mixed = process.env.KIND === "mixed";
send({ t: "ws_add_agent", panel, kind: mixed ? "codex" : "claude", alias: "b", model: mixed ? null : "haiku" });
const b = (await until((m) => m.t === "ws_member" && m.member.alias === "b", 10000)).member.id;

send({ t: "poll_create", panel, question: "Which is a fruit?", options: ["Carrot", "Apple", "Potato"], vote: 1 });
const opened = await until((m) => m.t === "ws_poll", 5000);
check(opened?.poll.status === "open" && opened.poll.votes.length === 1 && opened.poll.voters.length === 2, "poll open with my vote, both agents asked");
const card = await until((m) => m.t === "ws_msg" && m.msg.kind === "poll", 5000);
check(card?.msg.poll === opened?.poll.id, "poll card posted in the chat");

const closed = await until((m) => m.t === "ws_poll" && m.poll.status === "closed", 240000);
const pl = closed?.poll;
console.log("votes:", JSON.stringify(pl?.votes.map((v) => [v.voter === a ? "a" : v.voter === b ? "b" : v.voter, v.option, v.reason])));
check(pl?.votes.length === 3, "closed by itself once both agents voted");
check([a, b].every((id) => pl?.votes.some((v) => v.voter === id && v.option === 1 && v.reason)), "both agents picked Apple, with a reason");
check(!!(await until((m) => m.t === "ws_msg" && m.msg.kind === "poll_closed", 5000)), "closing line posted");
// While it is open, a vote's tool line must not reveal the pick (the last vote's reply shows the
// closed result, which is fine).
const votes = msgs.filter((m) => m.t === "ws_msg" && m.msg.kind === "tool" && m.msg.name === "📊 投票");
const firstVote = new Map(votes.map((m) => [m.msg.id, m])); // last update of each line
const lines = [...firstVote.values()];
for (const m of lines) console.log("  tool:", m.msg.summary, "|", m.msg.detail.slice(0, 60), "|", (m.msg.result ?? "").slice(0, 70).replace(/\n/g, " "));
check(lines.length >= 2 && lines.every((m) => !m.msg.detail && !/Apple|\bB\b/.test(m.msg.summary)) && lines.some((m) => /Vote recorded on poll/.test(m.msg.result ?? "")), "vote tool lines don't show the pick while open");

// Wait for both to finish, then ask one to read the result back.
await until((m) => m.t === "ws_round" && !m.active, 120000, msgs.indexOf(closed));
const mark = msgs.length;
send({ t: "ws_send", panel, text: "@a Use poll_read on poll #1 and tell me in one line how many votes Apple got and who voted." });
await until((m) => m.t === "ws_msg" && m.msg.kind === "tool" && /看投票/.test(m.msg.name), 120000, mark);
const answer = await until((m) => m.t === "ws_msg" && m.msg.from === a && m.msg.kind === "text" && !m.msg.streaming && m.msg.text.trim(), 120000, mark);
console.log("a said:", answer?.msg.text.slice(0, 200));
check(/3/.test(answer?.msg.text ?? ""), "poll_read gives the result after closing");

ws.close();
p.kill();
console.log(failures ? `\n${failures} check(s) failed` : "\nall checks passed");
process.exit(failures ? 1 : 0);
