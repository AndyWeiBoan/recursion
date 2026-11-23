import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { openThreads } from "./threadsUI";
import { FileBody } from "./Files";
import { DiagramView } from "./Diagram";
import { t } from "./i18n";
import { copyText, saveDownload } from "./clipboard";
import { send, useSnapshot } from "./client";
import { requestKind } from "./picker";
import { ChatHead, WorkspaceView } from "./WorkspaceView";
import { browserGo, hideBrowser, moveBrowser, nativeBrowser, normalizeUrl, onBrowserEvent, overlay, showBrowser } from "./browsers";
import { useDocHtml } from "./client";
import { focusTerminal, fitTerminal, mountTerminal, restartTerminal, typePaths } from "./terminals";
import { takeDrops, useDropOver } from "./drop";
import { useZoomed } from "./threadsUI";
import { KIND_LABEL, leaves, type Doc, type Node, type Panel, type PanelKind, type TermInfo, type Window } from "./types";

interface Ctx {
  window: Window;
  panels: Record<string, Panel>;
  terms: Record<string, TermInfo>;
  cwd: string;
  session: string;
  docs: Doc[];
}

export function LayoutView({ ctx }: { ctx: Ctx }) {
  // While a chat's thread drawer is open, that chat fills the window; the other panels are only
  // hidden (terminals keep running) and come back exactly as they were when the drawer closes.
  const zoomed = useZoomed();
  const zoom = zoomed && leaves(ctx.window.root).includes(zoomed) ? ctx.panels[zoomed] : undefined;
  return <div className="layout">{zoom ? <PanelView panel={zoom} ctx={ctx} /> : <NodeView node={ctx.window.root} ctx={ctx} />}</div>;
}

function NodeView({ node, ctx }: { node: Node; ctx: Ctx }) {
  if (node.type === "leaf") {
    const p = ctx.panels[node.panel];
    return p ? <PanelView panel={p} ctx={ctx} /> : null;
  }
  return <SplitView node={node} ctx={ctx} />;
}

/** A see-through cover over the whole window for the length of a drag, so documents (iframes) and
 *  terminals underneath can't take the pointer. Their own styles stay untouched: WebKitGTK stops
 *  touchpad scrolling in an iframe whose pointer-events were switched off and back on. */
export function shield(cursor: string) {
  const d = document.createElement("div");
  d.className = "drag-shield";
  d.style.cursor = cursor;
  document.body.appendChild(d);
  return () => d.remove();
}

/**
 * A size the user drags by one edge of the element: `right` / `left` drag a width (the
 * sidebar grows to the right, a drawer to the left), `top` a height that grows upwards (the
 * chat box). Double-click the handle to reset; remembered across launches.
 */
export function useDragWidth({ key, initial, min, max, edge, from }: { key: string; initial: number; min: number; max: number; edge: "left" | "right" | "top"; from?: () => number }) {
  const clamp = (w: number) => Math.round(Math.min(max, Math.max(min, w)));
  const [width, setWidth] = useState(() => {
    try {
      return clamp(Number(localStorage.getItem(key)) || initial);
    } catch {
      return initial;
    }
  });
  const [dragging, setDragging] = useState(false);
  const save = (w: number) => {
    try {
      localStorage.setItem(key, String(w));
    } catch {}
  };
  const onPointerDown = (e: React.PointerEvent<HTMLElement>) => {
    e.preventDefault();
    const el = e.currentTarget;
    el.setPointerCapture(e.pointerId);
    const sign = edge === "right" ? 1 : -1;
    const vertical = edge === "top";
    const start = vertical ? e.clientY : e.clientX;
    // Start from what is on screen (a box that grew with its text), not the remembered size.
    const base = from ? clamp(from()) : width;
    let last = base;
    setWidth(base);
    setDragging(true);
    const unshield = shield(vertical ? "row-resize" : "col-resize");
    const move = (ev: PointerEvent) => {
      last = clamp(base + sign * ((vertical ? ev.clientY : ev.clientX) - start));
      setWidth(last);
    };
    let done = false;
    const up = () => {
      if (done) return;
      done = true;
      el.removeEventListener("pointermove", move);
      for (const t of ["pointerup", "pointercancel", "lostpointercapture"]) el.removeEventListener(t, up);
      window.removeEventListener("blur", up);
      setDragging(false);
      unshield();
      save(last);
    };
    el.addEventListener("pointermove", move);
    for (const t of ["pointerup", "pointercancel", "lostpointercapture"]) el.addEventListener(t, up);
    window.addEventListener("blur", up);
  };
  const reset = () => (setWidth(initial), save(initial));
  return { width, dragging, onPointerDown, reset };
}

function SplitView({ node, ctx }: { node: Extract<Node, { type: "split" }>; ctx: Ctx }) {
  const ref = useRef<HTMLDivElement>(null);
  const [local, setLocal] = useState<number[] | null>(null);
  const sizes = local ?? node.sizes;
  useEffect(() => setLocal(null), [node.sizes.join(",")]);

  const startDrag = (i: number) => (ev: React.PointerEvent) => {
    ev.preventDefault();
    const box = ref.current!.getBoundingClientRect();
    const total = node.dir === "row" ? box.width : box.height;
    const start = node.dir === "row" ? ev.clientX : ev.clientY;
    const base = [...sizes];
    let latest = base;
    const move = (e: PointerEvent) => {
      const delta = ((node.dir === "row" ? e.clientX : e.clientY) - start) / total;
      const min = 0.06;
      const pair = base[i] + base[i + 1];
      const a = Math.min(Math.max(base[i] + delta, min), pair - min);
      latest = base.map((s, k) => (k === i ? a : k === i + 1 ? pair - a : s));
      setLocal(latest);
    };
    // The divider keeps the pointer until it is let go, wherever that happens; any way the drag can
    // end must end it, or the panels stay unclickable (they ignore the mouse while dragging).
    const el = ev.currentTarget as HTMLElement;
    el.setPointerCapture(ev.pointerId);
    let done = false;
    const up = () => {
      if (done) return;
      done = true;
      el.removeEventListener("pointermove", move);
      for (const t of ["pointerup", "pointercancel", "lostpointercapture"]) el.removeEventListener(t, up);
      window.removeEventListener("blur", up);
      document.body.classList.remove("dragging", "dragging-row", "dragging-col");
      unshield();
      send({ t: "layout_resize", window: ctx.window.id, split: node.id, sizes: latest });
    };
    document.body.classList.add("dragging", node.dir === "row" ? "dragging-row" : "dragging-col");
    const unshield = shield(node.dir === "row" ? "col-resize" : "row-resize");
    el.addEventListener("pointermove", move);
    for (const t of ["pointerup", "pointercancel", "lostpointercapture"]) el.addEventListener(t, up);
    window.addEventListener("blur", up);
  };

  return (
    <div ref={ref} className={`split split-${node.dir}`}>
      {node.children.map((child, i) => (
        <div key={child.type === "leaf" ? child.panel : child.id} className="split-cell-wrap" style={{ flexGrow: sizes[i], flexBasis: 0 }}>
          <div className="split-cell">
            <NodeView node={child} ctx={ctx} />
          </div>
          {i < node.children.length - 1 && (
            <div className={`divider divider-${node.dir}`} onPointerDown={startDrag(i)} role="separator" aria-orientation={node.dir === "row" ? "vertical" : "horizontal"} />
          )}
        </div>
      ))}
    </div>
  );
}

function PanelView({ panel, ctx }: { panel: Panel; ctx: Ctx }) {
  const focused = ctx.window.focus === panel.id;
  const dropOver = useDropOver();
  const info = ctx.terms[panel.id];
  const [menu, setMenu] = useState(false);
  useEffect(() => (menu ? overlay() : undefined), [menu]);
  const focus = () => {
    if (!focused) send({ t: "panel_focus", panel: panel.id });
  };
  const docId = panel.kind === "browser" && panel.url?.startsWith("doc:") ? panel.url.slice(4) : null;
  const doc = docId ? ctx.docs.find((d) => d.id === docId) : undefined;
  // A document, diagram or file shown here came from a chat: a way back to that chat's 產出.
  const { state } = useSnapshot();
  const viewer = panel.kind === "browser" && (docId || panel.url?.startsWith("file:"));
  const sourceChat = useMemo(() => {
    if (!viewer || !state) return null;
    if (doc?.chat && state.panels[doc.chat]) return doc.chat;
    const win = state.sessions.flatMap((s) => s.windows).find((w) => leaves(w.root).includes(panel.id));
    return win ? leaves(win.root).find((p) => state.panels[p]?.kind === "workspace") ?? null : null;
  }, [viewer, state, doc?.chat, panel.id]);
  const title =
    panel.kind === "terminal"
      ? info?.fg ?? "shell"
      : docId
        ? doc
          ? `${doc.title} · @${doc.author}`
          : t("doc.gone")
        : panel.kind === "browser" && panel.url?.startsWith("file:")
          ? panel.url.slice(5)
          : panel.kind === "browser"
          ? panel.url || t("browser.newTab")
          : "";

  return (
    <section className={`panel${focused ? " focused" : ""}`} data-panel={panel.id} onMouseDown={focus}>
      <header className="panel-head">
        <button className="kind" onClick={() => setMenu((v) => !v)} title={t("panel.switchKind")}>
          {(panel.kind === "workspace" && panel.title) || KIND_LABEL[panel.kind]} ▾
        </button>
        {panel.kind === "workspace" ? <ChatHead panel={panel.id} /> : <span className="title">{title}</span>}
        {sourceChat && (
          <button
            className="chat-outputs"
            onClick={() => {
              send({ t: "panel_focus", panel: sourceChat });
              openThreads(sourceChat, { view: { kind: "list" }, msg: null });
            }}
            title={t("outputs.backHint")}
          >
            📎 {t("outputs.title")}
          </button>
        )}
        <span className="tools">
          <button
            title={t("panel.splitRight") + "（⌘D）"}
            onClick={(e) => requestKind(t("panel.splitRight"), e.currentTarget.getBoundingClientRect(), (kind) => send({ t: "panel_split", panel: panel.id, dir: "row", kind }))}
          >
            ◫
          </button>
          <button
            title={t("panel.splitDown") + "（⌘⇧D）"}
            onClick={(e) => requestKind(t("panel.splitDown"), e.currentTarget.getBoundingClientRect(), (kind) => send({ t: "panel_split", panel: panel.id, dir: "column", kind }))}
          >
            ⊟
          </button>
          <button title={t("common.close") + "（⌘W）"} onClick={() => send({ t: "panel_close", panel: panel.id })}>✕</button>
        </span>
        {menu && (
          <div className="menu" onMouseLeave={() => setMenu(false)}>
            {(["workspace", "terminal", "browser"] as PanelKind[]).map((k) => (
              <button
                key={k}
                className={k === panel.kind ? "on" : ""}
                onClick={() => {
                  setMenu(false);
                  if (k !== panel.kind) send({ t: "panel_kind", panel: panel.id, kind: k });
                }}
              >
                {KIND_LABEL[k]}
              </button>
            ))}
          </div>
        )}
      </header>
      <div className="panel-body">
        {dropOver === panel.id && <div className="drop-over">{t(panel.kind === "terminal" ? "drop.paste" : "drop.attach")}</div>}
        {panel.kind === "terminal" && <TerminalBody id={panel.id} focused={focused} />}
        {panel.kind === "workspace" && <WorkspaceView panel={panel.id} focused={focused} />}
        {panel.kind === "browser" && panel.url?.startsWith("file:") ? (
          <FileBody panel={panel.id} path={panel.url.slice(5)} />
        ) : docId ? (
          <DocBody session={ctx.session} doc={docId} rev={doc?.rev ?? 0} diagram={doc?.kind === "diagram"} title={doc?.title ?? ""} />
        ) : (
          panel.kind === "browser" && <BrowserBody panel={panel} />
        )}
      </div>
    </section>
  );
}

function TerminalBody({ id, focused }: { id: string; focused: boolean }) {
  const host = useRef<HTMLDivElement>(null);
  const [exited, setExited] = useState(false);
  useLayoutEffect(() => {
    const e = mountTerminal(id, host.current!);
    setExited(e.exited);
    e.onExitChange = setExited;
    const ro = new ResizeObserver(() => fitTerminal(id));
    ro.observe(host.current!);
    const undrop = takeDrops(id, (paths) => typePaths(id, paths));
    return () => {
      undrop();
      ro.disconnect();
      e.onExitChange = undefined;
      e.el.remove();
    };
  }, [id]);
  useEffect(() => {
    if (focused) focusTerminal(id);
  }, [focused, id]);
  return (
    <div className="term-wrap">
      <div ref={host} className="term" />
      {exited && (
        <div className="term-exited">
          <span>{t("term.exited")}</span>
          <button onClick={() => restartTerminal(id)}>{t("term.restart")}</button>
        </div>
      )}
    </div>
  );
}

/** Gives a doc page `recursion.reply(data)` and `recursion.sendPrompt(text)`; both reach us via postMessage. */
const DOC_BRIDGE = `<script>(function () {
  function post(action, data) {
    if (typeof data !== "string") { try { data = JSON.stringify(data, null, 2); } catch (e) { data = String(data); } }
    parent.postMessage({ recursion: 1, action: action, data: data }, "*");
  }
  window.recursion = { reply: function (d) { post("reply", d); }, sendPrompt: function (t) { post("prompt", t); } };
  // The sandbox can't save files or reach the clipboard: hand both to the app.
  function save(a) {
    fetch(a.href).then(function (r) { return r.arrayBuffer(); }).then(function (buf) {
      parent.postMessage({ recursion: 1, action: "download", name: a.download || "download", data: buf }, "*", [buf]);
    });
  }
  var click = HTMLAnchorElement.prototype.click;
  HTMLAnchorElement.prototype.click = function () { if (this.hasAttribute("download")) return save(this); return click.call(this); };
  document.addEventListener("click", function (e) {
    var a = e.target && e.target.closest && e.target.closest("a[download]");
    if (a) { e.preventDefault(); save(a); }
  }, true);
  var copy = function (t) { parent.postMessage({ recursion: 1, action: "copy", data: String(t) }, "*"); return Promise.resolve(); };
  try {
    if (navigator.clipboard) navigator.clipboard.writeText = copy;
    else Object.defineProperty(navigator, "clipboard", { value: { writeText: copy } });
  } catch (e) {}
})();</script>`;

/** Put the bridge first thing in <head>, keeping a leading doctype (standards mode) intact. */
function withBridge(html: string) {
  const head = /<head[^>]*>/i.exec(html);
  if (head) return html.slice(0, head.index + head[0].length) + DOC_BRIDGE + html.slice(head.index + head[0].length);
  const doctype = /^\s*<!doctype[^>]*>/i.exec(html);
  if (doctype) return doctype[0] + DOC_BRIDGE + html.slice(doctype[0].length);
  return DOC_BRIDGE + html;
}

/** A session doc: agent-written HTML in a sandboxed iframe. No `allow-same-origin`, so its scripts
 *  run in an opaque origin and can't reach this page, the daemon token or Tauri; all they can do
 *  is post a message, which goes to the chat in the human's name, labelled with the doc. */
function DocBody({ session, doc, rev, diagram, title }: { session: string; doc: string; rev: number; diagram: boolean; title: string }) {
  const html = useDocHtml(session, doc, rev);
  const frame = useRef<HTMLIFrameElement>(null);
  const src = useMemo(() => (html ? withBridge(html) : html), [html]);
  useEffect(() => {
    const onMessage = (e: MessageEvent) => {
      if (e.source !== frame.current?.contentWindow || e.data?.recursion !== 1) return;
      const { action, data } = e.data;
      if ((action === "reply" || action === "prompt") && typeof data === "string") send({ t: "doc_action", session, doc, action, data });
      else if (action === "copy" && typeof data === "string") void copyText(data);
      else if (action === "download" && data instanceof ArrayBuffer) void saveDownload(String(e.data.name), data);
    };
    window.addEventListener("message", onMessage);
    return () => window.removeEventListener("message", onMessage);
  }, [session, doc]);
  if (src === null) return <div className="br-empty"><p>{t("doc.goneLong")}</p></div>;
  if (src === undefined) return <div className="br-empty"><p className="dim">{t("common.loading")}</p></div>;
  // A diagram's file is our own SVG: shown directly, not in a sandboxed page.
  if (diagram && html) return <DiagramView svg={html} title={title} />;
  return <iframe ref={frame} className="doc-frame" title={doc} srcDoc={src} sandbox="allow-scripts allow-forms allow-modals" />;
}

function BrowserBody({ panel }: { panel: Panel }) {
  const url = panel.url ?? "";
  const [input, setInput] = useState(url);
  const [title, setTitle] = useState("");
  const [loading, setLoading] = useState(false);
  const host = useRef<HTMLDivElement>(null);
  useEffect(() => setInput(url), [url]);

  useEffect(
    () =>
      onBrowserEvent(panel.id, (e) => {
        if (e.title !== null) setTitle(e.title);
        if (e.loading !== null) setLoading(e.loading);
        if (e.url) setInput(e.url);
        // Remember where the user ended up (link clicks, redirects) so reopening lands there.
        if (e.loading === false && e.url && e.url !== panel.url) send({ t: "panel_url", panel: panel.id, url: e.url });
      }),
    [panel.id, panel.url],
  );

  // Keep the native webview glued to our placeholder box; it can move without resizing
  // (sidebar toggle, sibling split dragged), so poll the rect each frame instead of observing size.
  const showRef = useRef<() => void>(() => {});
  const urlRef = useRef(url);
  urlRef.current = url;
  useLayoutEffect(() => {
    if (!nativeBrowser || !url) return;
    const el = host.current!;
    let last = "";
    let raf = 0;
    const rect = () => {
      const r = el.getBoundingClientRect();
      return { x: Math.round(r.left), y: Math.round(r.top), w: Math.round(r.width), h: Math.round(r.height) };
    };
    const show = () => {
      const b = rect();
      last = JSON.stringify(b);
      showBrowser(panel.id, urlRef.current, b, show);
    };
    const tick = () => {
      const b = rect();
      const key = JSON.stringify(b);
      if (key !== last) {
        last = key;
        moveBrowser(panel.id, b);
      }
      raf = requestAnimationFrame(tick);
    };
    showRef.current = show;
    show();
    raf = requestAnimationFrame(tick);
    return () => {
      cancelAnimationFrame(raf);
      showRef.current = () => {};
      hideBrowser(panel.id);
    };
  }, [panel.id, !!url]);
  // A new URL (typed, or recorded after a link click) reuses the same webview.
  const shown = useRef(url);
  useEffect(() => {
    if (shown.current === url) return;
    shown.current = url;
    showRef.current();
  }, [url]);

  const go = (e: React.FormEvent) => {
    e.preventDefault();
    if (!input.trim()) return;
    const next = normalizeUrl(input);
    setInput(next);
    if (next === url) browserGo(panel.id, "reload");
    else send({ t: "panel_url", panel: panel.id, url: next });
  };

  return (
    <div className="br">
      <form className="br-bar" onSubmit={go}>
        <button type="button" title={t("browser.back")} disabled={!url} onClick={() => browserGo(panel.id, "back")}>
          ←
        </button>
        <button type="button" title={t("browser.forward")} disabled={!url} onClick={() => browserGo(panel.id, "forward")}>
          →
        </button>
        <button type="button" title={t("browser.reload")} disabled={!url} onClick={() => browserGo(panel.id, "reload")}>
          {loading ? "✕" : "↻"}
        </button>
        <input
          value={input}
          onChange={(e) => setInput(e.target.value)}
          onFocus={(e) => e.currentTarget.select()}
          placeholder={t("browser.addressPlaceholder")}
          spellCheck={false}
          title={title}
        />
      </form>
      <div ref={host} className="br-view">
        {!url ? (
          <div className="br-empty">
            <p>{t("browser.empty")}</p>
          </div>
        ) : (
          !nativeBrowser && (
            <div className="br-empty">
              <p>{t("browser.desktopOnly")}</p>
              <p className="dim">{url}</p>
            </div>
          )
        )}
      </div>
    </div>
  );
}
