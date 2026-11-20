// Group chat end-to-end with TWO real Claude agents (haiku). Uses a little quota.
// Usage: node scripts/group-smoke.mjs
import { spawn } from "node:child_process";
import { mkdtempSync, readFileSync, existsSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const bin = new URL("../target/debug/recursiond", import.meta.url).pathname;
const home = mkdtempSync(join(tmpdir(), "recursion-group-"));
const proj = mkdtempSync(join(tmpdir(), "recursion-gproj-"));
const env = { ...process.env, RECURSION_HOME: home, RUST_LOG: "warn" };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
let failures = 0;
const check = (ok, what) => { console.log(`${ok ? "✓" : "✗"} ${what}`); if (!ok) failures++; };

const p = spawn(bin, { env, stdio: "inherit" });
let d;
for (let i = 0; i < 50 && !d; i++) { await sleep(100); if (existsSync(join(home, "daemon.json"))) d = JSON.parse(readFileSync(join(home, "daemon.json"))); }
const ws = new WebSocket(`ws://127.0.0.1:${d.port}/ws?token=${d.token}`);
const msgs = [];
ws.onmessage = (e) => msgs.push(JSON.parse(e.data));
await new Promise((r) => (ws.onopen = r));
const send = (o) => ws.send(JSON.stringify(o));
const until = async (pred, ms = 90000) => { const end = Date.now() + ms; while (Date.now() < end) { const h = msgs.findLast(pred); if (h) return h; await sleep(150); } return null; };

// transcript helpers
const transcript = new Map();
const members = new Map();
const pump = () => {
  for (const m of msgs.splice(0)) {
    seen.push(m);
    if (m.t === "ws_msg") transcript.set(m.msg.id, m.msg);
    if (m.t === "ws_msg_removed") transcript.delete(m.id);
    if (m.t === "ws_member") members.set(m.member.id, m.member);
  }
};
const seen = [];
const waitFor = async (pred, ms = 120000) => { const end = Date.now() + ms; while (Date.now() < end) { pump(); const r = pred(); if (r) return r; await sleep(200); } return null; };
const allIdle = () => [...members.values()].every((m) => m.status === "idle");
const texts = (fromId) => [...transcript.values()].filter((m) => m.kind === "text" && m.from === fromId && !m.streaming);
const settle = async () => { await sleep(1500); return waitFor(() => allIdle() && (sleep(0), true), 180000); };

await until((m) => m.t === "state");
send({ t: "session_create", name: "g", cwd: proj, kind: "workspace" });
const st = await until((m) => m.t === "state" && m.state.sessions.length === 1);
const panel = st.state.sessions[0].windows[0].focus;
send({ t: "ws_attach", panel });
send({ t: "ws_add_agent", panel, kind: "claude", alias: "ann", model: "haiku" });
send({ t: "ws_add_agent", panel, kind: "claude", alias: "bob", model: "haiku" });
await waitFor(() => members.size === 2);
const [ann, bob] = [...members.values()].sort((a, b) => a.alias.localeCompare(b.alias));
check(ann.alias === "ann" && bob.alias === "bob", "two agents joined");

// 1) broadcast
send({ t: "ws_send", panel, text: "Everyone: reply with only your own alias, nothing else." });
await waitFor(() => texts(ann.id).length >= 1 && texts(bob.id).length >= 1);
await settle();
check(texts(ann.id).some((m) => /ann/i.test(m.text)) && texts(bob.id).some((m) => /bob/i.test(m.text)), "no @ → both agents answered");

// 2) targeted + agent-to-agent
const bobBefore = texts(bob.id).length;
send({ t: "ws_send", panel, text: "@ann ask @bob what 2+3 is. Mention him with @bob in your reply. Do not answer it yourself." });
await waitFor(() => texts(bob.id).length > bobBefore, 150000);
await settle();
const annAsk = texts(ann.id).at(-1);
check(/@bob/i.test(annAsk?.text ?? ""), "only @ann was addressed and she @mentioned bob");
check(texts(bob.id).slice(bobBefore).some((m) => /5|five|五/i.test(m.text)), "bob was triggered by ann's mention and answered");

// 3) [PASS] is hidden
const annCount = texts(ann.id).length;
send({ t: "ws_send", panel, text: "Question for bob only: what is the capital of France? ann, you have nothing to add, so reply exactly [PASS]." });
await waitFor(() => texts(bob.id).some((m) => /paris|巴黎/i.test(m.text)), 120000);
await settle();
check(!texts(ann.id).slice(annCount).some((m) => /\[PASS\]/.test(m.text)), "[PASS] reply not kept in the transcript");

// 4) loop guard
send({ t: "ws_send", panel, text: "@ann @bob Let's test a loop. Play ping-pong forever: every reply must be exactly the word ping followed by @-mentioning the other agent (ann mentions @bob, bob mentions @ann). Never stop, never say anything else." });
const guard = await waitFor(() => [...transcript.values()].find((m) => m.kind === "guard" && !m.resolved), 240000);
check(!!guard, `loop guard tripped: ${guard?.text}`);
await settle();
const countAfterGuard = transcript.size;
await sleep(4000);
pump();
check(transcript.size === countAfterGuard, "agents stay quiet while paused");
send({ t: "ws_continue", panel });
await waitFor(() => transcript.size > countAfterGuard, 120000);
check(transcript.size > countAfterGuard, "continue releases the held agents");
send({ t: "ws_stop", panel });
await settle();

const cost = [...members.values()].reduce((s, m) => s + m.cost_usd, 0);
console.log(`cost ≈ $${cost.toFixed(4)}`);
ws.close();
p.kill("SIGTERM");
await new Promise((r) => p.on("exit", r));
console.log(failures ? `\n${failures} check(s) failed` : "\nall checks passed");
process.exit(failures ? 1 : 0);
