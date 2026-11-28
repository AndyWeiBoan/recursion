// Project files: the 📎 picker in the chat box, the chips that show which files a message
// references, and the viewer a chip opens next to the chat.
import hljs from "highlight.js/lib/common";
import { useEffect, useMemo, useRef, useState } from "react";
import { pickComputerFiles, send, useProjectFile, useProjectFiles } from "./client";
import { t } from "./i18n";
import { Icon } from "./icons";
import { isNumber, isTablePath, parseCsv, separatorOf, sortRows } from "./csv";

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
            <span>
              <bdi dir="ltr">{f.slice(0, f.length - base(f).length).replace(/\/$/, "")}</bdi>
            </span>
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
/** A picture the agents can look at (what can be attached as an image). */
export const isImagePath = (path: string) => /\.(png|jpe?g|gif|webp)$/i.test(path);

/** A small preview of an attached picture. */
export function ImageThumb({ panel, path, size }: { panel: string; path: string; size: number }) {
  const file = useProjectFile(panel, path, 0);
  return file?.image ? <img className="img-thumb" src={file.image} alt="" style={{ maxHeight: size, maxWidth: size * 1.6 }} /> : <Icon name="file" size={13} />;
}

export function FileChip({ panel, path, onRemove }: { panel: string; path: string; onRemove?: () => void }) {
  const image = isImagePath(path);
  return (
    <span className={`file-chip${image ? " image" : ""}`}>
      <button className={`file-chip-open${path.startsWith("/") ? " outside" : ""}`} onClick={() => send({ t: "file_open", panel, path })} title={t("file.open", { path }) + (path.startsWith("/") && !image ? `\n${t("file.outside")}` : "")}>
        {image ? <ImageThumb panel={panel} path={path} size={44} /> : <Icon name="file" size={13} />}
        {!image && base(path)}
      </button>
      {onRemove && (
        <button className="file-chip-x" onClick={onRemove} aria-label={t("file.remove", { name: base(path) })}>
          <Icon name="close" />
        </button>
      )}
    </span>
  );
}

const MAX_HIGHLIGHT = 200_000;
/** A table shows this many rows at most (the rest is there in the raw text). */
const MAX_ROWS = 2000;

/** A CSV or TSV file as a table: the header stays on top, numbers line up on the right, and a
 *  click on a heading sorts by that column (again: the other way, a third time: as in the file). */
function CsvTable({ path, text }: { path: string; text: string }) {
  const { rows, more } = useMemo(() => parseCsv(text, separatorOf(path, text), MAX_ROWS + 1), [path, text]);
  const [sort, setSort] = useState<{ col: number; desc: boolean } | null>(null);
  const [head, ...body] = rows;
  const width = Math.max(0, ...rows.map((r) => r.length));
  // A column is a number column when its filled cells all are.
  const numeric = useMemo(() => Array.from({ length: width }, (_, c) => body.some((r) => (r[c] ?? "").trim()) && body.every((r) => !(r[c] ?? "").trim() || isNumber(r[c]))), [body, width]);
  const shown = sort ? sortRows(body, sort.col, sort.desc) : body;
  if (!head) return <div className="br-empty"><p className="dim">{t("file.csvEmpty")}</p></div>;
  const by = (c: number) => setSort((s) => (s?.col !== c ? { col: c, desc: false } : !s.desc ? { col: c, desc: true } : null));
  return (
    <div className="fv-table">
      <table>
        <thead>
          <tr>
            <th className="fv-rownum" />
            {Array.from({ length: width }, (_, c) => (
              <th key={c} className={numeric[c] ? "num" : ""} onClick={() => by(c)} aria-sort={sort?.col === c ? (sort.desc ? "descending" : "ascending") : "none"} title={t("file.sortBy")}>
                {head[c] ?? ""}
                <span className="fv-sort">{sort?.col === c ? (sort.desc ? "↓" : "↑") : ""}</span>
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {shown.map((r, i) => (
            <tr key={i}>
              <td className="fv-rownum">{i + 1}</td>
              {Array.from({ length: width }, (_, c) => (
                <td key={c} className={numeric[c] ? "num" : ""}>
                  {r[c] ?? ""}
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
      {more && <p className="fv-more">{t("file.csvMore", { n: MAX_ROWS })}</p>}
    </div>
  );
}

/** A project file in a viewer panel: line numbers and colours, or the image. */
export function FileBody({ panel, path }: { panel: string; path: string }) {
  const [rev, setRev] = useState(0);
  const file = useProjectFile(panel, path, rev);
  // A CSV or TSV file shows as a table; its raw text is a click away.
  const table = isTablePath(path);
  const [raw, setRaw] = useState(false);
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
      {/* The panel's own header names the file: this bar has what is about its contents. */}
      <div className="fv-bar" title={path}>
        {file?.text !== undefined && <span className="fv-meta">{t("file.lines", { n: lines })}</span>}
        <span className="fv-fill" />
        {table && file?.text !== undefined && (
          <span className="fv-mode" role="radiogroup" aria-label={t("file.view")}>
            <button className={raw ? "" : "on"} role="radio" aria-checked={!raw} onClick={() => setRaw(false)}>
              {t("file.asTable")}
            </button>
            <button className={raw ? "on" : ""} role="radio" aria-checked={raw} onClick={() => setRaw(true)}>
              {t("file.asText")}
            </button>
          </span>
        )}
        <button onClick={() => setRev((r) => r + 1)} title={t("browser.reload")} aria-label={t("browser.reload")}>
          <Icon name="reload" />
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
      ) : table && !raw && file.text !== undefined ? (
        <CsvTable path={path} text={file.text} />
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
