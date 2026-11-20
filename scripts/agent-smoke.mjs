// End-to-end test with a REAL Claude agent (uses a little of your quota; model: haiku).
// Usage: node scripts/agent-smoke.mjs
import { spawn } from "node:child_process";
import { mkdtempSync, readFileSync, existsSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const bin = new URL("../target/debug/recursiond", import.meta.url).pathname;
const home = mkdtempSync(join(tmpdir(), "recursion-agent-"));
const proj = mkdtempSync(join(tmpdir(), "recursion-proj-"));
const env = { ...process.env, RECURSION_HOME: home, RUST_LOG: "warn" };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
let failures = 0;
const check = (ok, what) => { console.log(`${ok ? "✓" : "✗"} ${what}`); if (!ok) failures++; };

async function startDaemon() {
  const p = spawn(bin, { env, stdio: "inherit" });
  for (let i = 0; i < 50; i++) {
    await sleep(100);
    if (existsSync(join(home, "daemon.json"))) return { p, d: JSON.parse(readFileSync(join(home, "daemon.json"))) };
  }
  throw new Error("daemon did not start");
}
function connect(d) {
  const ws = new WebSocket(`ws://127.0.0.1:${d.port}/ws?token=${d.token}`);
  const msgs = [];
  ws.onmessage = (e) => msgs.push(JSON.parse(e.data));
  const until = async (pred, ms = 60000) => {
    const end = Date.now() + ms;
    while (Date.now() < end) { const hit = msgs.findLast(pred); if (hit) return hit; await sleep(100); }
    return null;
  };
  return new Promise((res) => (ws.onopen = () => res({ ws, msgs, until, send: (o) => ws.send(JSON.stringify(o)) })));
}

const { p, d } = await startDaemon();
const c = await connect(d);
await c.until((m) => m.t === "state");
c.send({ t: "session_create", name: "agent", cwd: proj });
const st = await c.until((m) => m.t === "state" && m.state.sessions.length === 1);
const panel = st.state.sessions[0].windows[0].focus;
c.send({ t: "panel_kind", panel, kind: "workspace" });
c.send({ t: "ws_attach", panel });
check(!!(await c.until((m) => m.t === "ws_state" && m.panel === panel, 3000)), "workspace snapshot on attach");

c.send({ t: "ws_add_agent", panel, kind: "claude", alias: "claude", model: "haiku" });
const mem = await c.until((m) => m.t === "ws_member" && m.panel === panel, 5000);
check(!!mem, "agent added");
c.send({ t: "ws_add_agent", panel, kind: "claude", alias: "Claude", model: "haiku" });
check(!!(await c.until((m) => m.t === "error" && /已經有/.test(m.msg), 3000)), "duplicate alias rejected");

c.send({ t: "ws_send", panel, text: "Reply with exactly the word: pong" });
const reply = await c.until((m) => m.t === "ws_msg" && m.msg.from === mem.member.id && m.msg.kind === "text" && !m.msg.streaming && /pong/i.test(m.msg.text));
check(!!reply, "streamed text reply");
check(c.msgs.some((m) => m.t === "ws_delta"), "received streaming deltas");
const idle = await c.until((m) => m.t === "ws_member" && m.member.status === "idle" && m.member.cli_session_id, 30000);
check(!!idle, `turn finished, session id recorded (cost $${idle?.member.cost_usd?.toFixed(4)})`);

c.send({ t: "ws_send", panel, text: "Run exactly this shell command with the Bash tool: node -e \"console.log('recursion-' + 'ok')\" — then say done." });
const perm = await c.until((m) => m.t === "ws_msg" && m.msg.kind === "permission" && m.msg.state === "pending");
check(!!perm && /node -e/.test(perm.msg.summary), "Bash asks for permission");
if (perm) c.send({ t: "ws_perm", request_id: perm.msg.request_id, decision: "allow" });
const tool = await c.until((m) => m.t === "ws_msg" && m.msg.kind === "tool" && /recursion-ok/.test(m.msg.result ?? ""));
check(!!tool, "tool ran after approval, result captured");
await c.until((m) => m.t === "ws_member" && m.member.status === "idle" && c.msgs.indexOf(m) > c.msgs.indexOf(tool), 60000);

// Restart daemon: transcript persisted, conversation resumes with memory.
c.ws.close();
p.kill("SIGTERM");
await new Promise((r) => p.on("exit", r));
const again = await startDaemon();
const c2 = await connect(again.d);
await c2.until((m) => m.t === "state");
c2.send({ t: "ws_attach", panel });
const snap = await c2.until((m) => m.t === "ws_state", 3000);
check(snap && snap.ws.messages.length >= 5, `transcript persisted (${snap?.ws.messages.length} messages)`);
c2.send({ t: "ws_send", panel, text: "What exact word did you reply with the first time? One word." });
const recall = await c2.until((m) => m.t === "ws_msg" && m.msg.kind === "text" && !m.msg.streaming && m.msg.from !== "you" && /pong/i.test(m.msg.text));
check(!!recall, "resumed conversation remembers context");
c2.ws.close();
again.p.kill("SIGTERM");
await new Promise((r) => again.p.on("exit", r));
console.log(failures ? `\n${failures} check(s) failed` : "\nall checks passed");
process.exit(failures ? 1 : 0);
