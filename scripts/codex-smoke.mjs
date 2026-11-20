// Codex + Claude in one group chat, end to end. Uses a little quota on both accounts.
// Usage: node scripts/codex-smoke.mjs
import { spawn } from "node:child_process";
import { mkdtempSync, readFileSync, existsSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const bin = new URL("../target/debug/recursiond", import.meta.url).pathname;
const home = mkdtempSync(join(tmpdir(), "recursion-codex-"));
const proj = mkdtempSync(join(tmpdir(), "recursion-cproj-"));
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
send({ t: "session_create", name: "c", cwd: proj, kind: "workspace" });
const st = await until((m) => m.t === "state" && m.state.sessions.length === 1);
const panel = st.state.sessions[0].windows[0].focus;
send({ t: "ws_attach", panel });
send({ t: "ws_add_agent", panel, kind: "codex", alias: "cx", model: null });
await waitFor(() => members.size === 1, 20000);
send({ t: "ws_add_agent", panel, kind: "claude", alias: "claude", model: "haiku" });
await waitFor(() => members.size === 2, 20000);
const cx = [...members.values()].find((m) => m.alias === "cx");
const cl = [...members.values()].find((m) => m.alias === "claude");
check(cx?.kind === "codex" && cl?.kind === "claude", "codex + claude joined");

send({ t: "ws_send", panel, text: "@cx run the shell command `node -e \"console.log(6*7)\"` and tell me the printed number." });
const perm = await waitFor(() => [...transcript.values()].find((m) => m.kind === "permission" && m.from === cx.id && m.state === "pending") || (texts(cx.id).some((m) => /42/.test(m.text)) && "noperm"), 150000);
if (perm && perm !== "noperm") send({ t: "ws_perm", request_id: perm.request_id, decision: "allow" });
await waitFor(() => texts(cx.id).some((m) => /42/.test(m.text)), 150000);
await settle();
check(texts(cx.id).some((m) => /42/.test(m.text)), `codex ran a command and answered (${perm === "noperm" ? "sandboxed, no prompt" : "after approval"})`);
check([...transcript.values()].some((m) => m.kind === "tool" && m.from === cx.id && /node -e/.test(m.summary)), "codex command shown as a tool row");
check(cx && (members.get(cx.id).cli_session_id ?? "").length > 10, "codex thread id stored");

const clBefore = texts(cl.id).length;
send({ t: "ws_send", panel, text: "@cx ask @claude to name the largest planet in one word. Mention him as @claude." });
await waitFor(() => texts(cl.id).length > clBefore, 180000);
await settle();
check(texts(cl.id).slice(clBefore).some((m) => /jupiter|木星/i.test(m.text)), "codex @mentioned claude and claude answered");

send({ t: "ws_send", panel, text: "@cx what number did the node command print earlier? Reply with just the number." });
await waitFor(() => texts(cx.id).filter((m) => /^\s*`?42`?\s*$/.test(m.text)).length > 0, 150000);
await settle();
check(texts(cx.id).some((m) => /^\s*`?42`?\s*\.?$/.test(m.text.trim())), "codex keeps context across turns");

ws.close();
p.kill("SIGTERM");
await new Promise((r) => p.on("exit", r));
console.log(failures ? `\n${failures} check(s) failed` : "\nall checks passed");
process.exit(failures ? 1 : 0);
