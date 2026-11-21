// End-to-end test of agents driving a browser panel, with a REAL agent (uses a little quota)
// and the REAL desktop app (it owns the webviews), against a local test site.
// Needs `pnpm dev`'s vite on :1420 (the debug app loads its UI from there) and a built app:
//   bash scripts/sidecar.sh && cargo build -p recursion-app
// Usage: node scripts/browse-smoke.mjs   (KIND=codex for Codex)
import { spawn } from "node:child_process";
import { createServer } from "node:http";
import { mkdtempSync, readFileSync, existsSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const kind = process.env.KIND ?? "claude";
const root = new URL("..", import.meta.url).pathname;
const home = mkdtempSync(join(tmpdir(), "recursion-browse-"));
const proj = mkdtempSync(join(tmpdir(), "recursion-proj-"));
const env = { ...process.env, RECURSION_HOME: home, RUST_LOG: process.env.RUST_LOG ?? "warn" };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
let failures = 0;
const check = (ok, what) => { console.log(`${ok ? "✓" : "✗"} ${what}`); if (!ok) failures++; };

const PAGES = {
  "/": `<!doctype html><html><head><meta charset="utf-8"><title>Test Shop</title>
<style>body{font:16px sans-serif;margin:24px}.box{width:240px;height:140px;background:#1e7d32;color:#fff;display:grid;place-items:center}</style></head>
<body><h1>Welcome to the test page</h1><p>The secret word is <b>MARMALADE</b>.</p><div class="box">box</div>
<p><a href="/about">About us</a></p>
<form action="/hello" method="get"><label for="n">Your name</label> <input id="n" name="name" type="text">
<select name="color"><option>red</option><option>blue</option></select><button type="submit">Send</button></form>
<button style="display:none">Hidden button</button></body></html>`,
  "/about": `<!doctype html><html><head><meta charset="utf-8"><title>About</title></head><body><h1>About</h1><p>Founded in 1987 by otters.</p></body></html>`,
  "/hello": `<!doctype html><html><head><meta charset="utf-8"><title>Hello</title></head><body><h1 id="h">…</h1>
<script>const q=new URLSearchParams(location.search);document.getElementById('h').textContent='Hello, '+q.get('name')+' who likes '+q.get('color');</script></body></html>`,
};
const site = createServer((req, res) => {
  const page = PAGES[new URL(req.url, "http://x").pathname];
  res.writeHead(page ? 200 : 404, { "content-type": "text/html; charset=utf-8" });
  res.end(page ?? "not found");
});
await new Promise((r) => site.listen(0, "127.0.0.1", r));
const base = `http://127.0.0.1:${site.address().port}`;

const daemon = spawn(join(root, "target/debug/recursiond"), { env, stdio: "inherit" });
let d;
for (let i = 0; i < 50 && !d; i++) {
  await sleep(100);
  if (existsSync(join(home, "daemon.json"))) d = JSON.parse(readFileSync(join(home, "daemon.json")));
}
const app = spawn(join(root, "target/debug/recursion-app"), { env, stdio: "ignore" });

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
const until = async (pred, ms = 240000, from = 0) => {
  const end = Date.now() + ms;
  while (Date.now() < end) { const hit = msgs.slice(from).findLast(pred); if (hit) return hit; await sleep(200); }
  return null;
};

send({ t: "session_create", name: "browse", cwd: proj, kind: "workspace" });
const st = await until((m) => m.t === "state" && m.state.sessions.length === 1, 10000);
const panel = st.state.sessions[0].windows[0].focus;
send({ t: "ws_attach", panel });
send({ t: "ws_add_agent", panel, kind, alias: kind, model: kind === "claude" ? "haiku" : null });
const agent = (await until((m) => m.t === "ws_member" && m.member.alias === kind, 10000)).member.id;
await sleep(3000); // let the app connect and announce itself

send({
  t: "ws_send",
  panel,
  text: `Use your browser tools, step by step: (1) browse_open ${base}/ and find the secret word. (2) browse_screenshot and look at the colored box: what color is it? (3) With browse_fill, put "Andy" in the name field, choose "blue" in the select, then click Send. (4) browse_read the page you land on. Then answer in English in one line exactly like: WORD=<word> COLOR=<color> RESULT=<heading of the last page>`,
});
const reply = await until((m) => m.t === "ws_msg" && m.msg.from === agent && m.msg.kind === "text" && !m.msg.streaming && /WORD=/.test(m.msg.text), 300000);
const text = reply?.msg.text ?? "";
console.log("reply:", text.slice(0, 300));
const tools = msgs.filter((m) => m.t === "ws_msg" && m.msg.kind === "tool").map((m) => m.msg.name);
console.log("tools:", [...new Set(tools)].join(", "));
check(/MARMALADE/i.test(text), "read the page (secret word)");
check(/green/i.test(text), "saw the screenshot (green box)");
check(/Hello,?\s*Andy who likes blue/i.test(text), "filled the form, submitted, read the result");
const last = msgs.findLast((m) => m.t === "state");
const browser = Object.values(last.state.panels).find((p) => p.kind === "browser");
check(browser?.agent === agent, "the agent got its own browser panel");
check(browser?.url?.startsWith(`${base}/hello`), `panel remembers where it ended up (${browser?.url})`);
const pages = new Map(msgs.filter((m) => m.t === "ws_artifact" && m.artifact.kind === "page").map((m) => [m.artifact.id, m.artifact]));
const titles = [...pages.values()].map((a) => a.title);
console.log("產出 pages:", [...pages.values()].map((a) => `${a.title} ${a.target}`).join(" | "));
check(titles.includes("Test Shop") && titles.includes("Hello") && [...pages.values()].every((a) => a.by === agent), "pages it visited are in the chat's 產出");

ws.close();
app.kill();
daemon.kill();
site.close();
console.log(failures ? `\n${failures} check(s) failed` : "\nall checks passed");
process.exit(failures ? 1 : 0);
