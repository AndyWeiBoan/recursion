// Scripted walkthrough for the how-to video. Dev builds only: the app starts it when run with
// RECURSION_TOUR=<project folder> (see main.tsx). It drives the real UI with a drawn cursor and
// real agents. Two things are stand-ins: the folder dialog (native, so the session is created
// directly), and the click inside a document page (sandboxed, so it is sent the way the page would).
// Waits for agents are marked (`__mark` pings to the demo site) so the video can fast-forward them.
import { getSnapshot, send } from "./client";
import { requestKind } from "./picker";

const SITE = "http://127.0.0.1:8765/";
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
const q = <T extends HTMLElement = HTMLElement>(sel: string, root: ParentNode = document) => root.querySelector<T>(sel);
const qa = <T extends HTMLElement = HTMLElement>(sel: string, root: ParentNode = document) => [...root.querySelectorAll<T>(sel)];

let cursor: HTMLDivElement;
let banner: HTMLDivElement;
let badge: HTMLDivElement;
let pos = { x: 700, y: 420 };
const t0 = performance.now();

function mark(e: string) {
  new Image().src = `${SITE}__mark?e=${e}&t=${Math.round(performance.now() - t0)}&now=${Date.now()}`;
}

function overlay() {
  const css = document.createElement("style");
  css.textContent = `
  .tour-cursor{position:fixed;left:0;top:0;width:26px;height:26px;z-index:2147483647;pointer-events:none;transform:translate(-3px,-2px);filter:drop-shadow(0 2px 3px #0009)}
  .tour-ripple{position:fixed;z-index:2147483646;pointer-events:none;width:44px;height:44px;margin:-22px 0 0 -22px;border-radius:50%;border:3px solid #ff8a4c;animation:tour-rip .5s ease-out forwards}
  @keyframes tour-rip{from{transform:scale(.3);opacity:1}to{transform:scale(1.4);opacity:0}}
  .tour-banner{position:fixed;top:14px;left:50%;transform:translateX(-50%);z-index:2147483645;pointer-events:none;display:flex;align-items:center;gap:16px;max-width:calc(100vw - 80px);
    padding:12px 28px 12px 12px;border-radius:20px;background:#121316f7;border:2px solid #e87746;box-shadow:0 16px 40px -12px #000,0 0 0 6px #e8774622;font-family:"PingFang TC",sans-serif;color:#fff;transition:opacity .25s}
  .tour-banner .n{min-width:52px;height:52px;padding:0 12px;border-radius:14px;background:#e87746;color:#fff;display:grid;place-items:center;font-weight:700;font-size:28px;flex:none}
  .tour-banner .tx{display:grid;gap:3px}.tour-banner b{font-size:27px;font-weight:600;line-height:1.2}.tour-banner span.d{font-size:18px;color:#d6d1ca;line-height:1.3}
  .tour-badge{position:fixed;right:20px;bottom:40px;z-index:2147483645;pointer-events:none;padding:8px 16px;border-radius:999px;background:#17181cf0;border:1px solid #5a5d68;
    color:#f2eee9;font:17px "PingFang TC",sans-serif;opacity:0;transition:opacity .2s}`;
  document.head.append(css);
  cursor = document.createElement("div");
  cursor.className = "tour-cursor";
  cursor.innerHTML = `<svg viewBox="0 0 26 26" width="26" height="26"><path d="M3 2 L3 21 L8.2 16.4 L11.6 24 L15 22.5 L11.7 15.2 L18.6 15 Z" fill="#fff" stroke="#111" stroke-width="1.6" stroke-linejoin="round"/></svg>`;
  banner = document.createElement("div");
  banner.className = "tour-banner";
  banner.style.opacity = "0";
  badge = document.createElement("div");
  badge.className = "tour-badge";
  badge.textContent = "⏩ 等 agent 回覆（已快轉）";
  document.body.append(cursor, banner, badge);
  place();
}

function place() {
  cursor.style.left = `${pos.x}px`;
  cursor.style.top = `${pos.y}px`;
}

function caption(n: number | string, title: string, detail = "") {
  banner.innerHTML = `<span class="n">${n}</span><span class="tx"><b>${title}</b>${detail ? `<span class="d">${detail}</span>` : ""}</span>`;
  banner.style.opacity = "1";
  mark(`step-${encodeURIComponent(String(n))}`);
}

const ease = (t: number) => (t < 0.5 ? 4 * t * t * t : 1 - Math.pow(-2 * t + 2, 3) / 2);

async function moveTo(x: number, y: number, ms?: number) {
  const from = { ...pos };
  const dist = Math.hypot(x - from.x, y - from.y);
  const dur = ms ?? Math.min(1100, 350 + dist * 0.9);
  const start = performance.now();
  await new Promise<void>((done) => {
    const step = (now: number) => {
      const k = ease(Math.min(1, (now - start) / dur));
      pos = { x: from.x + (x - from.x) * k, y: from.y + (y - from.y) * k };
      place();
      if (k < 1) requestAnimationFrame(step);
      else done();
    };
    requestAnimationFrame(step);
  });
}

function center(el: Element) {
  const r = el.getBoundingClientRect();
  return { x: r.left + r.width / 2, y: r.top + r.height / 2 };
}

async function moveToEl(el: Element) {
  const c = center(el);
  await moveTo(c.x, c.y);
}

function ripple() {
  const r = document.createElement("div");
  r.className = "tour-ripple";
  r.style.left = `${pos.x}px`;
  r.style.top = `${pos.y}px`;
  document.body.append(r);
  setTimeout(() => r.remove(), 600);
}

/** A real press: some controls act on mousedown (the @ menu), most on click. */
function press(el: HTMLElement) {
  const at = { bubbles: true, cancelable: true, clientX: pos.x, clientY: pos.y, button: 0 };
  el.dispatchEvent(new MouseEvent("mousedown", at));
  el.dispatchEvent(new MouseEvent("mouseup", at));
  el.click();
}

/** Move to `el`, show the click, then run `act` (default: a real press on it). */
async function click(el: HTMLElement, act: () => void = () => press(el)) {
  await moveToEl(el);
  await sleep(180);
  ripple();
  act();
  await sleep(450);
}

async function waitFor<T>(get: () => T | null | undefined | false, ms = 20000): Promise<T> {
  const end = Date.now() + ms;
  for (;;) {
    const v = get();
    if (v) return v;
    if (Date.now() > end) throw new Error("tour: timed out");
    await sleep(120);
  }
}

/** A wait for agents: shown with a badge and marked so the video fast-forwards it. */
async function agentWait<T>(get: () => T | null | undefined | false, ms = 300000): Promise<T> {
  badge.style.opacity = "1";
  mark("ff-start");
  try {
    return await waitFor(get, ms);
  } finally {
    mark("ff-end");
    badge.style.opacity = "0";
  }
}

function setValue(el: HTMLInputElement | HTMLTextAreaElement | HTMLSelectElement, v: string) {
  const proto = el instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : el instanceof HTMLSelectElement ? HTMLSelectElement.prototype : HTMLInputElement.prototype;
  Object.getOwnPropertyDescriptor(proto, "value")!.set!.call(el, v);
  el.dispatchEvent(new Event(el instanceof HTMLSelectElement ? "change" : "input", { bubbles: true }));
}

async function type(el: HTMLInputElement | HTMLTextAreaElement, text: string) {
  el.focus();
  for (const ch of text) {
    setValue(el, el.value + ch);
    el.setSelectionRange(el.value.length, el.value.length);
    await sleep(/[，。、？！\s]/.test(ch) ? 110 : 45 + Math.random() * 35);
  }
}

function key(el: Element, k: string) {
  el.dispatchEvent(new KeyboardEvent("keydown", { key: k, code: k, bubbles: true, cancelable: true }));
}

const composer = () => q<HTMLTextAreaElement>(".ws-input textarea")!;
const agentBubbles = () => qa(".ws-log .bubble.agent[data-threadable]");
const idle = () => !q(".ws-busy") && !q(".ws-log .caret");
const byText = (sel: string, text: string) => qa(sel).find((e) => e.textContent?.includes(text)) ?? null;
const panelEl = (id: string) => q(`section.panel[data-panel="${id}"]`)!;
const panels = () => Object.values(getSnapshot().state?.panels ?? {});

async function ask(text: string, replies = 1) {
  const before = agentBubbles().length;
  const box = composer();
  await click(box, () => box.focus());
  await type(box, text);
  await sleep(400);
  key(box, "Enter");
  await sleep(600);
  await agentWait(() => agentBubbles().length >= before + replies && idle());
  await sleep(1200);
}

/** Where a button inside a sandboxed doc frame sits: lay the same page out in a same-origin copy. */
async function docButton(frame: HTMLIFrameElement, label: string) {
  const r = frame.getBoundingClientRect();
  const probe = document.createElement("iframe");
  probe.setAttribute("sandbox", "allow-scripts allow-same-origin");
  probe.style.cssText = `position:fixed;left:-${r.width + 100}px;top:0;width:${r.width}px;height:${r.height}px;border:0;visibility:hidden`;
  probe.srcdoc = frame.getAttribute("srcdoc") ?? "";
  document.body.append(probe);
  await new Promise((ok) => (probe.onload = ok));
  await sleep(600);
  const b = [...(probe.contentDocument?.querySelectorAll("button") ?? [])].find((x) => x.textContent?.includes(label));
  const br = b?.getBoundingClientRect();
  probe.remove();
  return br ? { x: r.left + br.left + br.width / 2, y: r.top + br.top + br.height / 2 } : { x: r.left + r.width / 2, y: r.top + 140 };
}

export async function start(project: string) {
  await waitFor(() => getSnapshot().state, 15000);
  overlay();
  await sleep(1500);
  mark("start");

  // 1. Session
  caption(1, "建立 session", "按左上角的 ＋，選專案資料夾，再選「群聊」");
  await sleep(1800);
  const name = project.split("/").filter(Boolean).pop() ?? "project";
  await click(q('button[aria-label="新 session"]')!, () =>
    requestKind(`在「${name}」開啟`, null, (kind) => send({ t: "session_create", name, cwd: project, kind })),
  );
  const chatItem = await waitFor(() => byText(".picker-item", "群聊"));
  await sleep(700);
  await click(chatItem);
  await waitFor(() => q(".add-card.big"));
  await sleep(900);

  // 2. Agents
  caption(2, "加入 agent", "選 Claude 或 Codex，取個別名、挑模型");
  await sleep(1500);
  await click(byText(".add-card.big .kind-card", "Claude")!);
  const model = await waitFor(() => {
    const s = q<HTMLSelectElement>(".add-card.big select[id^='model-']");
    return s && [...s.options].some((o) => /sonnet/i.test(o.text + o.value)) ? s : null;
  });
  await click(model, () => setValue(model, [...model.options].find((o) => /sonnet/i.test(o.text + o.value))!.value));
  await sleep(500);
  await click(q(".add-card.big .add-submit")!);
  const add = await waitFor(() => q(".ws-members .chip.add"));
  await sleep(900);
  caption(2, "再加一個 agent", "按「＋ agent」，這次選 Codex");
  await click(add);
  const form = await waitFor(() => q(".add-card:not(.big)"));
  await click(byText(".add-card:not(.big) .kind-card", "Codex")!);
  await sleep(700);
  await click(q(".add-card:not(.big) .add-submit", form)!);
  await waitFor(() => qa(".tree-agent").length >= 2);
  await sleep(1200);

  // 3. Ask everyone
  caption(3, "問所有 agent", "沒有 @ 就是問全部，大家同時回答");
  await ask("我想幫這個咖啡網站加會員登入，你們各用三點講建議，簡短就好。", 2);

  // 4. @ one agent → a document
  caption(4, "用 @ 指定一個 agent", "輸入 @ 會跳出成員選單");
  const box = composer();
  await click(box, () => box.focus());
  await type(box, "@");
  const opt = await waitFor(() => byText(".mention-menu button", "@claude"));
  await sleep(700);
  await click(opt);
  await type(box, "把帳號密碼、Magic link、Google 登入做成一份比較文件，有比較表和長條圖，最上面放三個按鈕讓我選。聊天裡只要一句話。");
  await sleep(400);
  const before = agentBubbles().length;
  key(box, "Enter");
  await sleep(600);
  const frame = await agentWait(() => agentBubbles().length > before && idle() && q<HTMLIFrameElement>(".doc-frame"));
  caption(5, "agent 寫的文件", "長內容、表格、圖表會變成一份 HTML 文件，自動開在旁邊");
  await sleep(3500);

  // 5. The document talks back
  caption(5, "文件可以回話", "按文件裡的按鈕，你的選擇直接送給 agent");
  const target = await docButton(frame, "Magic");
  const st = getSnapshot().state!;
  const session = st.sessions[0];
  const doc = session.docs?.[0];
  const n = agentBubbles().length;
  await moveTo(target.x, target.y);
  await sleep(250);
  ripple();
  if (doc) send({ t: "doc_action", session: session.id, doc: doc.id, action: "reply", data: JSON.stringify({ 方案: "Magic link" }) });
  await sleep(600);
  await moveTo(target.x - 520, target.y + 120);
  await agentWait(() => agentBubbles().length > n && idle());
  await sleep(2500);

  // 6. Split + terminal
  caption(6, "分割 panel", "每個 panel 都能左右或上下分割：群聊、終端、瀏覽器");
  await sleep(1200);
  const docPanel = panels().find((p) => p.kind === "browser" && p.url?.startsWith("doc:"))!.id;
  await click(panelEl(docPanel).querySelector<HTMLElement>('button[title^="上下分割"]')!);
  await click(await waitFor(() => byText(".picker-item", "終端")));
  const term = await waitFor(() => panels().find((p) => p.kind === "terminal"));
  await sleep(1500);
  await click(panelEl(term.id), () => {});
  for (const ch of "cat README.md\r") {
    send({ t: "term_input", panel: term.id, data: ch });
    await sleep(70);
  }
  await sleep(2500);

  // 7. Thread on a selected phrase
  caption(7, "反白一段字，開討論串", "在 agent 的回覆裡反白，按「💬 討論這段」");
  await sleep(1200);
  const reply = agentBubbles().at(-1)!;
  // Bring the reply to the middle first, or the drag happens off screen.
  reply.scrollIntoView({ block: "center" });
  await sleep(700);
  const walker = document.createTreeWalker(reply.querySelector(".md")!, NodeFilter.SHOW_TEXT);
  let node: Text | null = null;
  while (walker.nextNode()) if ((walker.currentNode as Text).data.trim().length > 20) { node = walker.currentNode as Text; break; }
  if (node) {
    const len = Math.min(node.data.length, 30);
    const at = (i: number) => {
      const r = document.createRange();
      r.setStart(node!, i);
      r.setEnd(node!, i);
      const b = r.getBoundingClientRect();
      return { x: b.left, y: b.top + b.height / 2 };
    };
    const a = at(0);
    await moveTo(a.x, a.y);
    await sleep(200);
    const sel = window.getSelection()!;
    for (let i = 1; i <= len; i++) {
      const p = at(i);
      pos = p;
      place();
      const r = document.createRange();
      r.setStart(node, 0);
      r.setEnd(node, i);
      sel.removeAllRanges();
      sel.addRange(r);
      await sleep(75);
    }
    await sleep(500);
    q(".ws-log")!.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, clientX: pos.x, clientY: pos.y }));
  }
  const pop = await waitFor(() => q(".sel-thread"));
  await sleep(1500);
  await click(pop);
  const tbox = await waitFor(() => q<HTMLTextAreaElement>(".td-compose textarea"));
  await sleep(600);
  await click(tbox, () => tbox.focus());
  await type(tbox, "這點可以多解釋一下嗎？我們只有一個工程師。");
  key(tbox, "Enter");
  await sleep(800);
  await agentWait(() => q(".td-msg.agent") && !q(".td-body .spin"));
  caption(7, "討論串", "只有你和這個 agent 看得到，主對話不會被洗版");
  await sleep(3000);

  // 8. Bring back
  caption(8, "把結論帶回主對話", "agent 寫一段結論貼回群聊，其他 agent 下一輪就看得到");
  await sleep(1000);
  await click(q(".td-bring")!);
  await agentWait(() => q(".ws-log .from-thread") && idle());
  await sleep(1500);
  await click(q(".thread-drawer .td-x")!);
  await sleep(1500);

  // 9. Double-click = whole message
  caption(9, "雙擊整則訊息", "針對整則回覆開討論串；再雙擊一次會回到同一個");
  const first = agentBubbles()[0];
  first.scrollIntoView({ block: "center" });
  await sleep(600);
  await moveToEl(first);
  await sleep(200);
  ripple();
  await sleep(120);
  ripple();
  first.dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));
  await sleep(2800);
  await click(byText(".thread-drawer .td-back", "返回")!);
  await sleep(600);
  await click(q(".thread-drawer .td-x")!);
  await sleep(1000);

  // 10. The agent's browser
  caption(10, "關掉不用的 panel", "按 panel 右上角的 ✕");
  for (const id of [term.id, docPanel]) {
    await click(panelEl(id).querySelector<HTMLElement>('button[title^="關閉"]')!);
    await sleep(500);
  }
  caption(10, "agent 有自己的瀏覽器", "它會開網頁、填表單、按按鈕，你在旁邊看得到");
  await ask("@claude 用你的瀏覽器打開 127.0.0.1:8765 ，幫我點一杯燕麥奶拿鐵、半糖，名字填 Andy，送出後截圖給我確認。");
  await sleep(2500);

  // 11. Sidebar
  caption(11, "側欄", "session → window → panel → agent，一目了然");
  await sleep(1500);
  const fold = q("button.tree-fold");
  if (fold) {
    await click(fold);
    await sleep(1000);
    await click(fold);
    await sleep(800);
  }
  caption(11, "改名", "雙擊 window 或 agent 的名字就能改");
  const row = q(".tree-win .tree-row")!;
  await moveToEl(row);
  ripple();
  row.dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));
  const edit = await waitFor(() => q<HTMLInputElement>(".tree-edit"));
  setValue(edit, "");
  await type(edit, "會員功能");
  await sleep(400);
  key(edit, "Enter");
  await sleep(1200);
  caption(11, "context 用量", "agent 下面的進度條：用掉多少 context，快滿就該換新對話");
  await moveToEl(q(".tree-agent .ctx")!);
  await sleep(3000);

  caption("✓", "就這些！", "Recursion · github.com/AndyWeiBoan/recursion");
  await moveTo(pos.x + 300, pos.y + 200);
  await sleep(3500);
  mark("end");
}
