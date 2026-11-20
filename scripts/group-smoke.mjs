// Turn-based group chat end-to-end with TWO real Claude agents (haiku). Uses a little quota.
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
const raw = [];
ws.onmessage = (e) => raw.push(JSON.parse(e.data));
await new Promise((r) => (ws.onopen = r));
const send = (o) => ws.send(JSON.stringify(o));

const transcript = new Map();
const members = new Map();
let round = { active: false, queued: 0 };
let roundsStarted = 0;
let state = null;
const pump = () => {
  for (const m of raw.splice(0)) {
    if (m.t === "state") state = m.state;
    if (m.t === "ws_msg") transcript.set(m.msg.id, m.msg);
    if (m.t === "ws_msg_removed") transcript.delete(m.id);
    if (m.t === "ws_member") members.set(m.member.id, m.member);
    if (m.t === "ws_round") {
      if (m.active && !round.active) roundsStarted++;
      round = { active: m.active, queued: m.queued };
    }
  }
};
const waitFor = async (pred, ms = 120000) => { const end = Date.now() + ms; while (Date.now() < end) { pump(); const r = pred(); if (r) return r; await sleep(150); } pump(); return null; };
const texts = (id) => [...transcript.values()].filter((m) => m.kind === "text" && m.from === id && !m.streaming);
const idle = () => !round.active && [...members.values()].every((m) => m.status === "idle");
const settle = async () => { await sleep(2200); return waitFor(idle, 180000); };

await waitFor(() => state);
send({ t: "session_create", name: "g", cwd: proj, kind: "workspace" });
await waitFor(() => state?.sessions.length === 1);
const panel = state.sessions[0].windows[0].focus;
send({ t: "ws_attach", panel });
send({ t: "ws_add_agent", panel, kind: "claude", alias: "ann", model: "haiku" });
send({ t: "ws_add_agent", panel, kind: "claude", alias: "bob", model: "haiku" });
await waitFor(() => members.size === 2);
const ann = [...members.values()].find((m) => m.alias === "ann");
const bob = [...members.values()].find((m) => m.alias === "bob");

// 1) one round, both reply once, in parallel, and then silence even if they @ each other
send({ t: "ws_send", panel, text: "Everyone: reply with your alias, then @-mention the other agent and ask them a question." });
await waitFor(() => round.active, 5000);
const bothBusy = await waitFor(() => members.get(ann.id).status !== "idle" && members.get(bob.id).status !== "idle", 20000);
check(!!bothBusy, "both agents answer at the same time");
await settle();
check(texts(ann.id).length === 1 && texts(bob.id).length === 1, "each agent spoke exactly once in the round");
const before = transcript.size;
await sleep(6000);
pump();
check(transcript.size === before && idle(), "@mentions between agents did not start another round");

// 2) rapid messages merge into one round
const r0 = roundsStarted;
send({ t: "ws_send", panel, text: "Remember the number 7." });
await sleep(400);
send({ t: "ws_send", panel, text: "And the number 5. Everyone: reply with only the sum of the two numbers." });
await waitFor(() => round.active, 5000);
await settle();
check(roundsStarted - r0 === 1, "two quick messages became one round");
check(texts(ann.id).at(-1)?.text.includes("12") && texts(bob.id).at(-1)?.text.includes("12"), "agents saw both merged messages");

// 3) a message during a round is queued for the next one
const r1 = roundsStarted;
send({ t: "ws_send", panel, text: "Everyone: write a 3 sentence story about a cat." });
await waitFor(() => round.active, 5000);
send({ t: "ws_send", panel, text: "@bob now reply with only the word: queued-ok" });
const q = await waitFor(() => round.queued === 1, 5000);
check(!!q, "message sent mid-round is queued (queued = 1)");
await waitFor(() => texts(bob.id).some((m) => /queued-ok/i.test(m.text)), 150000);
await settle();
check(roundsStarted - r1 === 2, "queued message ran as the next round");
check(!texts(ann.id).some((m) => /queued-ok/i.test(m.text)), "next round only reached the @mentioned agent");

// 4) send now interrupts the running round
send({ t: "ws_send", panel, text: "Everyone: write a 300 word essay about rivers." });
await waitFor(() => round.active && [...members.values()].some((m) => m.status === "thinking"), 20000);
send({ t: "ws_send", panel, text: "@ann stop that, reply with only: now-ok" });
await waitFor(() => round.queued === 1, 5000);
send({ t: "ws_send_now", panel });
await waitFor(() => texts(ann.id).some((m) => /now-ok/i.test(m.text)), 120000);
await settle();
check(texts(ann.id).some((m) => /now-ok/i.test(m.text)), "send-now started the queued round right away");

const cost = [...members.values()].reduce((s, m) => s + m.cost_usd, 0);
console.log(`cost ≈ $${cost.toFixed(4)}`);
ws.close();
p.kill("SIGTERM");
await new Promise((r) => p.on("exit", r));
console.log(failures ? `\n${failures} check(s) failed` : "\nall checks passed");
process.exit(failures ? 1 : 0);
