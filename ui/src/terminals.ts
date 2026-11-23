// xterm.js instances live outside React so switching windows never loses a terminal.
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";
import { isMac, onReconnect, onTerm, send } from "./client";
import { copyText, pasteText } from "./clipboard";

interface Entry {
  term: Terminal;
  fit: FitAddon;
  el: HTMLDivElement;
  attached: boolean;
  exited: boolean;
  onExitChange?: (exited: boolean) => void;
}

const entries = new Map<string, Entry>();

const theme = {
  background: "#1d1e22",
  foreground: "#e6e6e8",
  cursor: "#e8714a",
  cursorAccent: "#1d1e22",
  selectionBackground: "#4a4b5599",
  black: "#26272c", red: "#ff6b6b", green: "#4ade80", yellow: "#f6c177",
  blue: "#6aa6ff", magenta: "#c792ea", cyan: "#3fd6c3", white: "#d7d8dc",
  brightBlack: "#6b6d76", brightRed: "#ff8787", brightGreen: "#86efac", brightYellow: "#ffd28a",
  brightBlue: "#93c0ff", brightMagenta: "#dab3f5", brightCyan: "#7fe8da", brightWhite: "#ffffff",
};

function attach(id: string, e: Entry) {
  e.attached = true;
  send({ t: "term_attach", panel: id, cols: e.term.cols, rows: e.term.rows });
}

onReconnect(() => {
  entries.forEach((e, id) => e.attached && attach(id, e));
});

export function getTerminal(id: string): Entry {
  let e = entries.get(id);
  if (e) return e;
  const el = document.createElement("div");
  el.className = "xterm-host";
  const term = new Terminal({
    fontFamily: '"JetBrains Mono", "SF Mono", Menlo, "Symbols Nerd Font Mono", "JetBrainsMono Nerd Font Mono", "Hack Nerd Font Mono", "MesloLGS Nerd Font Mono", "FiraCode Nerd Font Mono", "Noto Sans Mono CJK TC", monospace',
    fontSize: 13,
    lineHeight: 1.15,
    cursorBlink: true,
    allowProposedApi: true,
    scrollback: 10000,
    macOptionIsMeta: true,
    theme,
  });
  const fit = new FitAddon();
  term.loadAddon(fit);
  term.open(el);
  const entry: Entry = { term, fit, el, attached: false, exited: false };
  e = entry;
  // Ctrl+Shift+C / Ctrl+Shift+V copy and paste, as in other Linux terminals (Ctrl+C stays ^C).
  // On macOS ⌘C / ⌘V already work through the Edit menu.
  term.attachCustomKeyEventHandler((ev) => {
    if (isMac || !ev.ctrlKey || !ev.shiftKey || ev.altKey || ev.metaKey) return true;
    const k = ev.key.toLowerCase();
    if (k !== "c" && k !== "v") return true;
    ev.preventDefault();
    if (ev.type !== "keydown") return false;
    if (k === "c") {
      if (term.hasSelection()) void copyText(term.getSelection());
    } else {
      void pasteText().then((t) => t && term.paste(t));
    }
    return false;
  });
  term.onData((data) => send({ t: "term_input", panel: id, data }));
  term.onResize(({ cols, rows }) => entry.attached && send({ t: "term_resize", panel: id, cols, rows }));
  onTerm(id, {
    onReplay(data) {
      term.reset();
      if (data.length) term.write(data);
      setExited(entry, false);
    },
    onOutput(data) {
      term.write(data);
    },
    onExit() {
      setExited(entry, true);
    },
  });
  entries.set(id, entry);
  return entry;
}

function setExited(e: Entry, v: boolean) {
  e.exited = v;
  e.onExitChange?.(v);
}

/** Mount into a host element, fit, and attach to the daemon on first show. */
export function mountTerminal(id: string, host: HTMLElement) {
  const e = getTerminal(id);
  host.appendChild(e.el);
  requestAnimationFrame(() => {
    try {
      e.fit.fit();
    } catch {}
    if (!e.attached) attach(id, e);
  });
  return e;
}

const fitTimers = new Map<string, number>();

/** Fit once the size has settled: during the sidebar's slide (or a drag) every frame is a new
 * size, and each fit makes the program inside (nvim…) redraw its whole screen. */
export function fitTerminal(id: string) {
  clearTimeout(fitTimers.get(id));
  fitTimers.set(
    id,
    window.setTimeout(() => {
      fitTimers.delete(id);
      const e = entries.get(id);
      if (!e || !e.el.isConnected) return;
      try {
        e.fit.fit();
      } catch {}
    }, 100),
  );
}

/** Dropped files: their paths pasted in, quoted for the shell where needed, like other terminals
 * do. Pasted, not typed: nvim in normal mode would run typed text as commands. */
export function typePaths(id: string, paths: string[]) {
  const e = entries.get(id);
  if (!e) return;
  const quote = (p: string) => (/^[\w@%+=:,./~-]+$/.test(p) ? p : `'${p.replace(/'/g, `'\\''`)}'`);
  e.term.paste(paths.map(quote).join(" ") + " ");
  e.term.focus();
}

export function focusTerminal(id: string) {
  entries.get(id)?.term.focus();
}

export function restartTerminal(id: string) {
  const e = entries.get(id);
  if (e) send({ t: "term_restart", panel: id, cols: e.term.cols, rows: e.term.rows });
}

/** Drop one terminal (a closed setup terminal). */
export function disposeTerminal(id: string) {
  const e = entries.get(id);
  if (!e) return;
  e.term.dispose();
  e.el.remove();
  entries.delete(id);
}

/** Drop terminals whose panels no longer exist. Setup terminals belong to no panel. */
export function pruneTerminals(alive: Set<string>) {
  entries.forEach((e, id) => {
    if (!alive.has(id) && !id.startsWith("setup-")) {
      e.term.dispose();
      e.el.remove();
      entries.delete(id);
    }
  });
}
