// End-to-end smoke test for recursiond.
// Usage: node scripts/smoke.mjs   (builds nothing; expects target/debug/recursiond)
// Starts a daemon in a temp RECURSION_HOME, drives it over WebSocket, restarts it, and checks persistence.
import { spawn } from "node:child_process";
import { mkdtempSync, readFileSync, existsSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const bin = new URL("../target/debug/recursiond", import.meta.url).pathname;
const home = mkdtempSync(join(tmpdir(), "recursion-smoke-"));
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
  const waiters = [];
  ws.onmessage = (e) => {
    const m = JSON.parse(e.data);
    msgs.push(m);
    waiters.splice(0).forEach((w) => w());
  };
  const until = async (pred, ms = 4000) => {
    const end = Date.now() + ms;
    while (Date.now() < end) {
      const hit = msgs.findLast(pred);
      if (hit) return hit;
      await new Promise((r) => { waiters.push(r); setTimeout(r, 100); });
    }
    return null;
  };
  const send = (o) => ws.send(JSON.stringify(o));
  return new Promise((res) => (ws.onopen = () => res({ ws, msgs, until, send })));
}

const { p, d } = await startDaemon();
check(d.port > 0 && d.token.length === 32, "daemon.json written with port and token");

const bad = await fetch(`http://127.0.0.1:${d.port}/health?token=nope`);
check(bad.status === 401, "wrong token rejected");

const c = await connect(d);
const first = await c.until((m) => m.t === "state");
check(first && first.state.sessions.length === 0, "fresh state is empty");

c.send({ t: "session_create", name: "demo", cwd: home });
const s1 = await c.until((m) => m.t === "state" && m.state.sessions.length === 1);
check(!!s1, "session created");
const sess = s1.state.sessions[0];
const panel = sess.windows[0].focus;

c.send({ t: "panel_split", panel, dir: "row", kind: "browser" });
const s2 = await c.until((m) => m.t === "state" && m.state.sessions[0]?.windows[0]?.root.type === "split");
check(!!s2, "panel split into a row");

c.send({ t: "term_attach", panel, cols: 80, rows: 24 });
await c.until((m) => m.t === "replay" && m.panel === panel);
c.send({ t: "term_input", panel, data: "echo recursion-$((40+2))\r" });
const out = await c.until((m) => m.t === "out" && m.panel === panel && atob(m.data).includes("recursion-42"), 6000);
check(!!out, "terminal runs commands");

c.send({ t: "term_input", panel, data: "printf '\\344\\270\\255\\346\\226\\207\\n'\r" });
const zh = await c.until((m) => m.t === "out" && new TextDecoder().decode(Uint8Array.from(atob(m.data), (ch) => ch.charCodeAt(0))).includes("中文"), 6000);
check(!!zh, "UTF-8 output intact");

const fg = await c.until((m) => m.t === "state" && m.terms?.[panel]?.fg, 3000);
check(!!fg, `foreground process reported (${fg?.terms?.[panel]?.fg})`);

c.send({ t: "bogus" });
check(!!(await c.until((m) => m.t === "error")), "bad message gets an error reply");

// Detach: a second client attaching gets scrollback replay.
const c2 = await connect(d);
c2.send({ t: "term_attach", panel, cols: 80, rows: 24 });
const rep = await c2.until((m) => m.t === "replay" && m.panel === panel);
check(rep && atob(rep.data).includes("recursion-42"), "reattach replays scrollback");

c.ws.close(); c2.ws.close();
await sleep(500);
p.kill("SIGTERM");
await new Promise((r) => p.on("exit", r));
check(!existsSync(join(home, "daemon.json")), "daemon.json removed on shutdown");

// Restart: layout persisted.
const again = await startDaemon();
const c3 = await connect(again.d);
const s3 = await c3.until((m) => m.t === "state");
check(s3.state.sessions[0]?.windows[0]?.root?.type === "split", "layout survives daemon restart");
c3.ws.close();
again.p.kill("SIGTERM");
await new Promise((r) => again.p.on("exit", r));

console.log(failures ? `\n${failures} check(s) failed` : "\nall checks passed");
process.exit(failures ? 1 : 0);
