// A diagram the daemon drew (SVG): pan, zoom, light/dark, export, and click a box to highlight
// everything it connects to. The SVG carries its own styles, so exports look the same.
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { saveDownload } from "./clipboard";
import { t } from "./i18n";
import { Icon } from "./icons";

const MIN = 0.2;
const MAX = 4;
const THEME_KEY = "recursion.diagramTheme";

export function DiagramView({ svg, title }: { svg: string; title: string }) {
  const box = useRef<HTMLDivElement>(null);
  const inner = useRef<HTMLDivElement>(null);
  const [view, setView] = useState({ x: 0, y: 0, s: 1 });
  const [light, setLight] = useState(() => {
    try {
      return localStorage.getItem(THEME_KEY) === "light";
    } catch {
      return false;
    }
  });
  const svgEl = () => inner.current?.querySelector("svg") ?? null;
  const size = () => {
    const el = svgEl();
    return el ? { w: Number(el.getAttribute("width")), h: Number(el.getAttribute("height")) } : { w: 1, h: 1 };
  };

  const fit = () => {
    const b = box.current;
    if (!b) return;
    const { w, h } = size();
    const s = Math.min(1.5, Math.max(MIN, Math.min((b.clientWidth - 32) / w, (b.clientHeight - 32) / h)));
    setView({ s, x: (b.clientWidth - w * s) / 2, y: Math.max(16, (b.clientHeight - h * s) / 2) });
  };
  // A new drawing (or revision) fits itself to the panel.
  useLayoutEffect(fit, [svg]);
  useEffect(() => {
    svgEl()?.classList.toggle("light", light);
  }, [light, svg]);

  // Zoom around a point of the panel.
  const zoom = (factor: number, px: number, py: number) =>
    setView((v) => {
      const s = Math.min(MAX, Math.max(MIN, v.s * factor));
      return { s, x: px - ((px - v.x) * s) / v.s, y: py - ((py - v.y) * s) / v.s };
    });

  useEffect(() => {
    const b = box.current;
    if (!b) return;
    // Touchpad: two fingers pan, pinch (ctrl+wheel) zooms. Mouse wheel with Ctrl zooms too.
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      const r = b.getBoundingClientRect();
      if (e.ctrlKey || e.metaKey) zoom(Math.exp(-e.deltaY * 0.01), e.clientX - r.left, e.clientY - r.top);
      else setView((v) => ({ ...v, x: v.x - e.deltaX, y: v.y - e.deltaY }));
    };
    b.addEventListener("wheel", onWheel, { passive: false });
    return () => b.removeEventListener("wheel", onWheel);
  }, []);

  // Drag to pan; a press that doesn't move is a click (highlight or clear).
  const drag = useRef<{ x: number; y: number; vx: number; vy: number; moved: boolean } | null>(null);
  const onPointerDown = (e: React.PointerEvent) => {
    if (e.button !== 0) return;
    drag.current = { x: e.clientX, y: e.clientY, vx: view.x, vy: view.y, moved: false };
    e.currentTarget.setPointerCapture(e.pointerId);
  };
  const onPointerMove = (e: React.PointerEvent) => {
    const d = drag.current;
    if (!d) return;
    const dx = e.clientX - d.x;
    const dy = e.clientY - d.y;
    if (!d.moved && Math.hypot(dx, dy) < 4) return;
    d.moved = true;
    setView((v) => ({ ...v, x: d.vx + dx, y: d.vy + dy }));
  };
  const onPointerUp = (e: React.PointerEvent) => {
    const d = drag.current;
    drag.current = null;
    if (!d || d.moved) return;
    const hit = document.elementsFromPoint(e.clientX, e.clientY).find((el) => el.closest?.(".dg-node"))?.closest(".dg-node");
    highlight(hit?.getAttribute("data-id") ?? null);
  };

  const highlight = (id: string | null) => {
    const root = svgEl();
    if (!root) return;
    root.querySelectorAll(".hl").forEach((el) => el.classList.remove("hl"));
    root.classList.toggle("focus", !!id);
    if (!id) return;
    const ends = new Set([id]);
    root.querySelectorAll<SVGElement>("[data-from]").forEach((el) => {
      const [from, to] = [el.dataset.from, el.dataset.to];
      if (from === id || to === id) {
        el.classList.add("hl");
        if (from) ends.add(from);
        if (to) ends.add(to);
      }
    });
    root.querySelectorAll<SVGElement>(".dg-node").forEach((el) => ends.has(el.dataset.id ?? "") && el.classList.add("hl"));
  };

  const file = (ext: string) => `${title.replace(/[\\/:*?"<>|]+/g, " ").trim() || "diagram"}.${ext}`;
  // The SVG as it looks now (theme), without the highlight.
  const source = () => {
    const el = svgEl()!.cloneNode(true) as SVGSVGElement;
    el.classList.remove("focus");
    el.querySelectorAll(".hl").forEach((x) => x.classList.remove("hl"));
    return new XMLSerializer().serializeToString(el);
  };
  const exportSvg = () => void saveDownload(file("svg"), new TextEncoder().encode(source()).buffer as ArrayBuffer);
  const exportPng = () => {
    const { w, h } = size();
    const img = new Image();
    const url = URL.createObjectURL(new Blob([source()], { type: "image/svg+xml" }));
    img.onload = () => {
      const scale = 2;
      const c = document.createElement("canvas");
      c.width = w * scale;
      c.height = h * scale;
      const g = c.getContext("2d")!;
      g.scale(scale, scale);
      g.drawImage(img, 0, 0, w, h);
      URL.revokeObjectURL(url);
      c.toBlob((b) => b && b.arrayBuffer().then((buf) => saveDownload(file("png"), buf)), "image/png");
    };
    img.src = url;
  };

  const center = () => {
    const b = box.current!;
    return [b.clientWidth / 2, b.clientHeight / 2] as const;
  };
  return (
    <div className={`dgv${light ? " light" : ""}`}>
      <div className="dgv-bar">
        <button onClick={() => zoom(1 / 1.25, ...center())} aria-label="−">
          <Icon name="minus" />
        </button>
        <span className="dgv-pct">{Math.round(view.s * 100)}%</span>
        <button onClick={() => zoom(1.25, ...center())} aria-label="+">
          +
        </button>
        <button onClick={fit}>{t("dg.fit")}</button>
        <span className="dgv-hint">{t("dg.hint")}</span>
        <button
          onClick={() =>
            setLight((v) => {
              try {
                localStorage.setItem(THEME_KEY, v ? "dark" : "light");
              } catch {}
              return !v;
            })
          }
          title={t("dg.theme")}
          aria-label={t("dg.theme")}
        >
          <Icon name={light ? "dark" : "light"} size={14} />
        </button>
        <button onClick={exportPng}>{t("dg.png")}</button>
        <button onClick={exportSvg}>{t("dg.svg")}</button>
      </div>
      <div ref={box} className="dgv-view" onPointerDown={onPointerDown} onPointerMove={onPointerMove} onPointerUp={onPointerUp} onPointerCancel={() => (drag.current = null)}>
        <div ref={inner} className="dgv-inner" style={{ transform: `translate(${view.x}px, ${view.y}px) scale(${view.s})` }} dangerouslySetInnerHTML={{ __html: svg }} />
      </div>
    </div>
  );
}
