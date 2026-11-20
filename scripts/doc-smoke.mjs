// End-to-end test of session documents with REAL agents (uses a little quota):
// Claude (haiku) writes a doc through the built-in MCP server, then Codex reads it back.
// Usage: node scripts/doc-smoke.mjs   (set SKIP_CODEX=1 to test Claude only)
import { spawn } from "node:child_process";
import { mkdtempSync, readFileSync, existsSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const bin = new URL("../target/debug/recursiond", import.meta.url).pathname;
const home = mkdtempSync(join(tmpdir(), "recursion-doc-"));
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
  // Approve anything an agent asks for (MCP tool calls may need it, depending on the CLI).
  if (m.t === "ws_msg" && m.msg.kind === "permission" && m.msg.state === "pending")
    ws.send(JSON.stringify({ t: "ws_perm", request_id: m.msg.request_id, decision: "allow" }));
};
await new Promise((r) => (ws.onopen = r));
const send = (o) => ws.send(JSON.stringify(o));
// `from`: only look at messages received after that index.
const until = async (pred, ms = 120000, from = 0) => {
  const end = Date.now() + ms;
  while (Date.now() < end) { const hit = msgs.slice(from).findLast(pred); if (hit) return hit; await sleep(150); }
  return null;
};

send({ t: "session_create", name: "doc", cwd: proj });
const st = await until((m) => m.t === "state" && m.state.sessions.length === 1);
const session = st.state.sessions[0].id;
const panel = st.state.sessions[0].windows[0].focus;
send({ t: "panel_kind", panel, kind: "workspace" });
send({ t: "ws_attach", panel });
send({ t: "ws_add_agent", panel, kind: "claude", alias: "claude", model: "haiku" });
const claude = (await until((m) => m.t === "ws_member" && m.member.alias === "claude", 5000)).member.id;

send({
  t: "ws_send",
  panel,
  text: "@claude Use the doc_write tool to create a document titled \"Zebra Report\" containing an HTML table of three animals. Then reply with one short sentence.",
});
const card = await until((m) => m.t === "ws_msg" && m.msg.kind === "doc" && m.msg.from === claude);
check(!!card, `doc card posted (${card?.msg.title})`);
const docId = card?.msg.doc;
const withDoc = await until((m) => m.t === "state" && m.state.sessions[0]?.docs?.some((x) => x.id === docId), 5000);
check(!!withDoc, "doc listed in session state");
const opened = withDoc && Object.values(withDoc.state.panels).find((x) => x.url === `doc:${docId}`);
check(!!opened, "doc opened in a browser panel");
check(withDoc?.state.sessions[0].windows[0].focus === panel, "chat panel kept the focus");
send({ t: "doc_get", session, doc: docId });
const got = await until((m) => m.t === "doc" && m.doc === docId, 5000);
check(/<table/i.test(got?.html ?? ""), "doc HTML saved and served");
const cardAt = msgs.indexOf(card);
await until((m) => m.t === "ws_member" && m.member.id === claude && m.member.status === "idle", 60000, cardAt);
send({ t: "ws_roster" });
const roster = await until((m) => m.t === "ws_roster", 5000);
const rc = roster?.panels[panel]?.find((x) => x.id === claude);
check(rc?.context_used > 0 && rc?.context_window > 0, `claude context in roster (${rc?.context_used}/${rc?.context_window})`);
const usage = await until((m) => m.t === "ws_msg" && m.msg.from === claude && m.msg.turn && m.msg.usage, 5000);
check(usage?.msg.usage.output > 0 && usage.msg.usage.cost_usd > 0, `claude turn usage recorded (${JSON.stringify(usage?.msg.usage)})`);

if (!process.env.SKIP_CODEX) {
  send({ t: "ws_add_agent", panel, kind: "codex", alias: "codex", model: null });
  const codex = (await until((m) => m.t === "ws_member" && m.member.alias === "codex", 10000))?.member.id;
  const mark = msgs.length;
  send({ t: "ws_send", panel, text: "@codex Use the doc_read tool on the document claude shared and tell me which animals are in its table." });
  await until((m) => m.t === "ws_member" && m.member.id === codex && m.member.status === "thinking", 60000, mark);
  const thinking = msgs.findLastIndex((m) => m.t === "ws_member" && m.member.id === codex && m.member.status === "thinking");
  await until((m) => m.t === "ws_member" && m.member.id === codex && m.member.status === "idle", 240000, thinking);
  const answer = msgs.findLast((m) => m.t === "ws_msg" && m.msg.from === codex && m.msg.kind === "text" && !m.msg.streaming);
  if (process.env.DEBUG)
    for (const m of msgs.slice(mark)) if (m.t === "error" || m.t === "ws_round" || (m.t === "ws_member" && m.member.id === codex)) console.log("  ~", m.t, m.msg ?? m.member?.status ?? JSON.stringify(m.active));
  if (process.env.DEBUG)
    for (const m of msgs) if (m.t === "ws_msg" && m.msg.from === codex) console.log("  ·", m.msg.kind, m.msg.name ?? "", (m.msg.summary ?? m.msg.text ?? "").slice(0, 160));
  const cu = msgs.findLast((m) => m.t === "ws_msg" && m.msg.from === codex && m.msg.turn && m.msg.usage);
  check(cu?.msg.usage.output > 0, `codex turn usage recorded (${JSON.stringify(cu?.msg.usage)})`);
  const cm = msgs.findLast((m) => m.t === "ws_member" && m.member.id === codex)?.member;
  check(cm?.context_used > 0 && cm?.context_window > 0, `codex context tracked (${cm?.context_used}/${cm?.context_window})`);
  const usedTool = msgs.some((m) => m.t === "ws_msg" && m.msg.from === codex && m.msg.kind === "tool" && /doc_read/.test(m.msg.name + m.msg.summary));
  check(usedTool, "codex called doc_read");
  console.log("  codex said:", answer?.msg.text.slice(0, 200));
}

ws.close();
p.kill();
console.log(failures ? `\n${failures} check(s) failed` : "\nall checks passed");
process.exit(failures ? 1 : 0);
