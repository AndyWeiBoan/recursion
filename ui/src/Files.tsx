// Project files: the 📎 picker in the chat box, the chips that show which files a message
// references, and the viewer a chip opens next to the chat.
import hljs from "highlight.js/lib/common";
import { useEffect, useMemo, useRef, useState } from "react";
import { pickComputerFiles, send, useProjectFile, useProjectFiles } from "./client";
import { t } from "./i18n";
import { Icon } from "./icons";

const base = (p: string) => p.slice(p.lastIndexOf("/") + 1);

/**
 * How well `q` matches `path`: its letters in order, scored up for a hit in the file name, for
 * letters right after each other, and for shorter paths. 0 = no match.
 */
function score(path: string, q: string) {
  if (!q) return 1;
  const p = path.toLowerCase();
  const name = base(p);
  let i = 0;
  let s = 0;
  let run = 0;
  for (let k = 0; k < p.length && i < q.length; k++) {
    if (p[k] === q[i]) {
      run++;
      s += 1 + run * 2 + (k >= p.length - name.length ? 3 : 0);
      i++;
    } else run = 0;
  }
  if (i < q.length) return 0;
  if (name.includes(q)) s += 30;
  return s - path.length * 0.05;
}

/** The 📎 picker: search the project's files, Enter or click adds one. */
export function FilePicker({ panel, onPick, onClose }: { panel: string; onPick: (path: string) => void; onClose: () => void }) {
  // The system dialog takes the focus; don't close on that blur.
  const choosing = useRef(false);
  const fromComputer = async () => {
    choosing.current = true;
    const picked = await pickComputerFiles(panel);
    choosing.current = false;
    picked.forEach(onPick);
    if (!picked.length) onClose();
  };
  const files = useProjectFiles(panel, true);
  const [q, setQ] = useState("");
  const [sel, setSel] = useState(0);
  const list = useRef<HTMLDivElement>(null);
  const shown = useMemo(() => {
    const query = q.trim().toLowerCase().replace(/\s+/g, "");
    return (files ?? [])
      .map((f) => [f, score(f, query)] as const)
      .filter(([, s]) => s > 0)
      .sort((a, b) => b[1] - a[1])
      .slice(0, 80)
      .map(([f]) => f);
  }, [files, q]);
  useEffect(() => setSel(0), [q]);
  useEffect(() => {
    list.current?.querySelector(".on")?.scrollIntoView({ block: "nearest" });
  }, [sel]);
  return (
    <div className="fp" onMouseDown={(e) => e.stopPropagation()}>
      <input
        autoFocus
        value={q}
        placeholder={t("file.search")}
        spellCheck={false}
        onChange={(e) => setQ(e.target.value)}
        onBlur={() => !choosing.current && onClose()}
        onKeyDown={(e) => {
          if (e.nativeEvent.isComposing) return;
          if (e.key === "ArrowDown") return e.preventDefault(), setSel((s) => Math.min(s + 1, shown.length - 1));
          if (e.key === "ArrowUp") return e.preventDefault(), setSel((s) => Math.max(s - 1, 0));
          if (e.key === "Enter" && shown[sel]) return e.preventDefault(), onPick(shown[sel]);
          if (e.key === "Escape") return e.preventDefault(), e.stopPropagation(), onClose();
        }}
      />
      <div className="fp-list" ref={list} role="listbox">
        {files === undefined && <div className="fp-empty">{t("file.loading")}</div>}
        {files !== undefined && shown.length === 0 && <div className="fp-empty">{t("file.none")}</div>}
        {shown.map((f, i) => (
          <button key={f} role="option" aria-selected={i === sel} className={i === sel ? "on" : ""} onMouseDown={(e) => (e.preventDefault(), onPick(f))} onMouseEnter={() => setSel(i)}>
            <b>{base(f)}</b>
            <span>{f.slice(0, f.length - base(f).length).replace(/\/$/, "")}</span>
          </button>
        ))}
      </div>
      <button className="fp-computer" onMouseDown={(e) => (e.preventDefault(), void fromComputer())} title={t("file.fromComputerHint")}>
        <Icon name="folder" size={14} />
        {t("file.fromComputer")}
      </button>
    </div>
  );
}

/** A referenced file: click opens it next to the chat; `onRemove` adds a ✕ (in the chat box). */
export function FileChip({ panel, path, onRemove }: { panel: string; path: string; onRemove?: () => void }) {
  return (
    <span className="file-chip">
      <button className={`file-chip-open${path.startsWith("/") ? " outside" : ""}`} onClick={() => send({ t: "file_open", panel, path })} title={t("file.open", { path }) + (path.startsWith("/") ? `\n${t("file.outside")}` : "")}>
        <Icon name="file" size={13} />
        {base(path)}
      </button>
      {onRemove && (
        <button className="file-chip-x" onClick={onRemove} aria-label={t("file.remove", { name: base(path) })}>
          ✕
        </button>
      )}
    </span>
  );
}

const MAX_HIGHLIGHT = 200_000;

/** A project file in a viewer panel: line numbers and colours, or the image. */
export function FileBody({ panel, path }: { panel: string; path: string }) {
  const [rev, setRev] = useState(0);
  const file = useProjectFile(panel, path, rev);
  const html = useMemo(() => {
    const text = file?.text;
    if (text === undefined) return "";
    const ext = path.split(".").pop() ?? "";
    if (text.length > MAX_HIGHLIGHT) return hljs.highlight(text, { language: "plaintext" }).value;
    return hljs.getLanguage(ext) ? hljs.highlight(text, { language: ext, ignoreIllegals: true }).value : hljs.highlightAuto(text).value;
  }, [file?.text, path]);
  const lines = file?.text ? file.text.split("\n").length - (file.text.endsWith("\n") ? 1 : 0) : 0;
  return (
    <div className="fv">
      <div className="fv-bar">
        <Icon name="file" size={13} />
        <span className="fv-path" title={path}>
          {path}
        </span>
        {file?.text !== undefined && <span className="fv-meta">{t("file.lines", { n: lines })}</span>}
        <button onClick={() => setRev((r) => r + 1)} title={t("browser.reload")} aria-label={t("browser.reload")}>
          ↻
        </button>
      </div>
      {file === undefined ? (
        <div className="br-empty">
          <p className="dim">{t("common.loading")}</p>
        </div>
      ) : file.error ? (
        <div className="br-empty">
          <p>{file.error}</p>
        </div>
      ) : file.image ? (
        <div className="fv-image">
          <img src={file.image} alt={path} />
        </div>
      ) : (
        <div className="fv-code">
          <pre className="fv-nums" aria-hidden="true">
            {Array.from({ length: Math.max(lines, 1) }, (_, i) => i + 1).join("\n")}
          </pre>
          <pre className="fv-text">
            <code className="hljs" dangerouslySetInnerHTML={{ __html: html }} />
          </pre>
        </div>
      )}
    </div>
  );
}
