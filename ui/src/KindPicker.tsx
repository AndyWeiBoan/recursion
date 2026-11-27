// "What do you want to open?" menu shown before creating any panel (new window or split).
import { t } from "./i18n";
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import type { PanelKind } from "./types";
import { Icon, type IconName } from "./icons";

export interface PickRequest {
  title: string;
  /** Anchor rect (from a clicked button); null → centered (keyboard). */
  anchor: DOMRect | null;
  onPick: (kind: PanelKind) => void;
}

const ITEMS: { kind: PanelKind; label: string; hint: string; key: string; icon: IconName }[] = [
  { kind: "terminal", get label() { return t("kind.terminal"); }, get hint() { return t("kind.terminal.hint"); }, key: "t", icon: "terminal" },
  { kind: "workspace", get label() { return t("kind.workspace"); }, get hint() { return t("kind.workspace.hint"); }, key: "g", icon: "chat" },
  { kind: "browser", get label() { return t("kind.browser"); }, get hint() { return t("kind.browser.hint"); }, key: "b", icon: "browser" },
];

const LAST_KEY = "recursion.lastKind";

function lastKind(): PanelKind {
  try {
    const v = localStorage.getItem(LAST_KEY);
    if (v === "terminal" || v === "workspace" || v === "browser") return v;
  } catch {}
  return "terminal";
}

export function KindPicker({ req, close }: { req: PickRequest; close: () => void }) {
  const [sel, setSelState] = useState(() => ITEMS.findIndex((i) => i.kind === lastKind()));
  const selRef = useRef(sel);
  const setSel = (f: number | ((s: number) => number)) => {
    selRef.current = typeof f === "function" ? f(selRef.current) : f;
    setSelState(selRef.current);
  };
  const box = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState<{ left: number; top: number } | null>(null);

  const choose = (kind: PanelKind) => {
    try {
      localStorage.setItem(LAST_KEY, kind);
    } catch {}
    close();
    req.onPick(kind);
  };

  useLayoutEffect(() => {
    const el = box.current;
    if (!el) return;
    const w = el.offsetWidth;
    const h = el.offsetHeight;
    if (req.anchor) {
      const left = Math.min(Math.max(8, req.anchor.left), window.innerWidth - w - 8);
      const below = req.anchor.bottom + 6;
      const top = below + h > window.innerHeight - 8 ? Math.max(8, req.anchor.top - h - 6) : below;
      setPos({ left, top });
    } else {
      setPos({ left: (window.innerWidth - w) / 2, top: Math.max(60, window.innerHeight * 0.22) });
    }
  }, [req.anchor]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const k = e.key.toLowerCase();
      const handled = () => {
        e.preventDefault();
        e.stopPropagation();
      };
      if (k === "escape") return handled(), close();
      if (k === "enter") return handled(), choose(ITEMS[selRef.current].kind);
      if (k === "arrowdown" || (k === "j" && !e.metaKey && !e.ctrlKey)) return handled(), setSel((s) => (s + 1) % ITEMS.length);
      if (k === "arrowup" || (k === "k" && !e.metaKey && !e.ctrlKey)) return handled(), setSel((s) => (s + ITEMS.length - 1) % ITEMS.length);
      const hit = ITEMS.find((i) => i.key === k);
      if (hit && !e.metaKey && !e.ctrlKey && !e.altKey) return handled(), choose(hit.kind);
      handled();
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, []);

  return (
    <div className="picker-scrim" onMouseDown={close}>
      <div
        ref={box}
        className="picker"
        role="menu"
        aria-label={req.title}
        style={pos ? { left: pos.left, top: pos.top } : { visibility: "hidden" }}
        onMouseDown={(e) => e.stopPropagation()}
      >
        <div className="picker-title">{req.title}</div>
        {ITEMS.map((it, i) => (
          <button
            key={it.kind}
            role="menuitem"
            className={`picker-item${i === sel ? " on" : ""}`}
            onMouseEnter={() => setSel(i)}
            onClick={() => choose(it.kind)}
          >
            <span className={`picker-ic ic-${it.kind}`}><Icon name={it.icon} size={20} /></span>
            <span className="picker-text">
              <span className="picker-label">{it.label}</span>
              <span className="picker-hint">{it.hint}</span>
            </span>
            <kbd>{it.key.toUpperCase()}</kbd>
          </button>
        ))}
        <div className="picker-foot">{t("picker.foot")}</div>
      </div>
    </div>
  );
}
