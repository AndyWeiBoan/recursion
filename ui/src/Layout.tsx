import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { send } from "./client";
import { requestKind } from "./picker";
import { WorkspaceView } from "./WorkspaceView";
import { browserGo, hideBrowser, moveBrowser, nativeBrowser, normalizeUrl, onBrowserEvent, overlay, showBrowser } from "./browsers";
import { focusTerminal, fitTerminal, mountTerminal, restartTerminal } from "./terminals";
import { KIND_LABEL, type Node, type Panel, type PanelKind, type TermInfo, type Window } from "./types";

interface Ctx {
  window: Window;
  panels: Record<string, Panel>;
  terms: Record<string, TermInfo>;
  cwd: string;
}

export function LayoutView({ ctx }: { ctx: Ctx }) {
  return (
    <div className="layout">
      <NodeView node={ctx.window.root} ctx={ctx} />
    </div>
  );
}

function NodeView({ node, ctx }: { node: Node; ctx: Ctx }) {
  if (node.type === "leaf") {
    const p = ctx.panels[node.panel];
    return p ? <PanelView panel={p} ctx={ctx} /> : null;
  }
  return <SplitView node={node} ctx={ctx} />;
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
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      document.body.classList.remove("dragging");
      send({ t: "layout_resize", window: ctx.window.id, split: node.id, sizes: latest });
    };
    document.body.classList.add("dragging", node.dir === "row" ? "dragging-row" : "dragging-col");
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up, { once: true });
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
  const info = ctx.terms[panel.id];
  const [menu, setMenu] = useState(false);
  useEffect(() => (menu ? overlay() : undefined), [menu]);
  const focus = () => {
    if (!focused) send({ t: "panel_focus", panel: panel.id });
  };
  const title =
    panel.kind === "terminal" ? info?.fg ?? "shell" : panel.kind === "browser" ? panel.url || "新分頁" : "";

  return (
    <section className={`panel${focused ? " focused" : ""}`} data-panel={panel.id} onMouseDown={focus}>
      <header className="panel-head">
        <button className="kind" onClick={() => setMenu((v) => !v)} title="切換 panel 種類">
          {KIND_LABEL[panel.kind]} ▾
        </button>
        <span className="title">{title}</span>
        <span className="tools">
          <button
            title="左右分割（⌘D）"
            onClick={(e) => requestKind("左右分割", e.currentTarget.getBoundingClientRect(), (kind) => send({ t: "panel_split", panel: panel.id, dir: "row", kind }))}
          >
            ◫
          </button>
          <button
            title="上下分割（⌘⇧D）"
            onClick={(e) => requestKind("上下分割", e.currentTarget.getBoundingClientRect(), (kind) => send({ t: "panel_split", panel: panel.id, dir: "column", kind }))}
          >
            ⊟
          </button>
          <button title="關閉（⌘W）" onClick={() => send({ t: "panel_close", panel: panel.id })}>✕</button>
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
        {panel.kind === "terminal" && <TerminalBody id={panel.id} focused={focused} />}
        {panel.kind === "workspace" && <WorkspaceView panel={panel.id} focused={focused} />}
        {panel.kind === "browser" && <BrowserBody panel={panel} />}
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
    return () => {
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
          <span>程序已結束</span>
          <button onClick={() => restartTerminal(id)}>重新啟動</button>
        </div>
      )}
    </div>
  );
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
        <button type="button" title="上一頁" disabled={!url} onClick={() => browserGo(panel.id, "back")}>
          ←
        </button>
        <button type="button" title="下一頁" disabled={!url} onClick={() => browserGo(panel.id, "forward")}>
          →
        </button>
        <button type="button" title="重新載入" disabled={!url} onClick={() => browserGo(panel.id, "reload")}>
          {loading ? "✕" : "↻"}
        </button>
        <input
          value={input}
          onChange={(e) => setInput(e.target.value)}
          onFocus={(e) => e.currentTarget.select()}
          placeholder="輸入網址或搜尋，例如 localhost:5173"
          spellCheck={false}
          title={title}
        />
      </form>
      <div ref={host} className="br-view">
        {!url ? (
          <div className="br-empty">
            <p>輸入網址開始瀏覽。</p>
          </div>
        ) : (
          !nativeBrowser && (
            <div className="br-empty">
              <p>瀏覽器 panel 只在桌面 app 裡能用。</p>
              <p className="dim">{url}</p>
            </div>
          )
        )}
      </div>
    </div>
  );
}
