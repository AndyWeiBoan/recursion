// End-to-end test of threads with REAL agents (uses a little quota):
// ask in the main chat, open a thread on the reply, talk twice in the fork, bring the conclusion back.
// Usage: node scripts/thread-smoke.mjs            (Claude haiku)
//        KIND=codex node scripts/thread-smoke.mjs (Codex, default model)
import { spawn } from "node:child_process";
import { mkdtempSync, readFileSync, existsSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const kind = process.env.KIND ?? "claude";
const bin = new URL("../target/debug/recursiond", import.meta.url).pathname;
const home = mkdtempSync(join(tmpdir(), "recursion-thread-"));
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
ws.onmessage = (e) => {
  const m = JSON.parse(e.data);
  msgs.push(m);
  if (m.t === "ws_msg" && m.msg.kind === "permission" && m.msg.state === "pending")
    ws.send(JSON.stringify({ t: "ws_perm", request_id: m.msg.request_id, decision: "allow" }));
};
await new Promise((r) => (ws.onopen = r));
const send = (o) => ws.send(JSON.stringify(o));
const until = async (pred, ms = 180000, from = 0) => {
  const end = Date.now() + ms;
  while (Date.now() < end) { const hit = msgs.slice(from).findLast(pred); if (hit) return hit; await sleep(200); }
  return null;
};
const threadState = (id, from) => (m) => m.t === "ws_thread" && m.thread.id === id && m.thread.status !== "replying" && m.thread.messages.length >= from;

send({ t: "session_create", name: "thread", cwd: proj, kind: "workspace" });
const st = await until((m) => m.t === "state" && m.state.sessions.length === 1);
const panel = st.state.sessions[0].windows[0].focus;
send({ t: "ws_attach", panel });
send({ t: "ws_add_agent", panel, kind, alias: kind, model: kind === "claude" ? "haiku" : null });
const agent = (await until((m) => m.t === "ws_member" && m.member.alias === kind, 10000)).member.id;

send({ t: "ws_send", panel, text: "In one sentence: name three primary colors." });
const reply = await until((m) => m.t === "ws_msg" && m.msg.from === agent && m.msg.kind === "text" && !m.msg.streaming && m.msg.text.length > 0);
await until((m) => m.t === "ws_member" && m.member.id === agent && m.member.status === "idle" && m.member.cli_session_id, 60000, msgs.indexOf(reply));
check(!!reply, "main chat reply");

let mark = msgs.length;
send({ t: "thread_open", panel, msg: reply.msg.id, quote: "", text: "Remember the code word PELICAN. Which of those colors is warmest? One sentence." });
const opened = await until((m) => m.t === "thread_opened", 10000, mark);
check(!!opened, "thread opened");
const tid = opened?.thread;
const t1 = await until(threadState(tid, 2), 240000, mark);
check(t1?.thread.messages.length === 2 && t1.thread.fork && t1.thread.messages[1].from === agent, `thread reply #${t1?.thread.num}: ${t1?.thread.messages[1]?.text.slice(0, 80)}`);
check(!!t1?.thread.fork, "fork recorded");

mark = msgs.length;
send({ t: "thread_send", panel, thread: tid, text: "What was the code word I gave you? Answer with just the word." });
const t2 = await until(threadState(tid, 4), 240000, mark);
check(/pelican/i.test(t2?.thread.messages[3]?.text ?? ""), `fork remembers the thread (${t2?.thread.messages[3]?.text.slice(0, 40)})`);

const leaked = msgs.some((m) => m.t === "ws_msg" && m.msg.kind === "text" && /pelican/i.test(m.msg.text));
check(!leaked, "thread talk stays out of the main chat");

mark = msgs.length;
send({ t: "thread_bring", panel, thread: tid });
const back = await until((m) => m.t === "ws_msg" && m.msg.from_thread?.thread === tid, 240000, mark);
check(!!back, `conclusion brought back: ${back?.msg.text.slice(0, 100)}`);
const t3 = await until((m) => m.t === "ws_thread" && m.thread.id === tid && m.thread.status === "brought", 10000, mark);
check(!!t3, "thread marked as brought back");

ws.close();
p.kill();
console.log(failures ? `\n${failures} check(s) failed` : "\nall checks passed");
process.exit(failures ? 1 : 0);
