// Settings › Agent resources › Skills and rules: Recursion's own copies (the library), and what
// was found on this computer to copy in. An import is a copy, said so before it is made: the two
// part ways after it, and an original that changed since shows so, to be imported again.
import { useEffect, useState } from "react";
import { createPortal } from "react-dom";
import { overlay } from "./browsers";
import { send, useLibrary, useLibraryText, type LibraryFound, type LibraryKind } from "./client";
import { t } from "./i18n";
import { Icon } from "./icons";
import { PickList } from "./PickList";
import { useCardPlace } from "./TodoCard";
import { renderMarkdown } from "./WorkspaceView";

const home = (p: string) => p.replace(/^\/(Users|home)\/[^/]+/, "~");
const VENDOR: Record<LibraryFound["vendor"], string> = { claude: "Claude", codex: "Codex", cursor: "Cursor" };
const ASK = "recursion.importNoAsk";

/** One Skill's or rule's text, rendered, in a card moved and resized like a todo's. */
export function LibraryCard({ kind, name, path, onClose }: { kind: LibraryKind; name: string | null; path: string | null; onClose: () => void }) {
  const text = useLibraryText(kind, name, path);
  const { place, drag, resize } = useCardPlace("recursion.libraryCard");
  useEffect(() => overlay(), []);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !e.isComposing) (e.stopPropagation(), onClose());
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [onClose]);
  return createPortal(
    <div className="todo-card" style={{ left: place.x, top: place.y, width: place.w, height: place.h }} role="dialog" aria-label={name ?? path ?? ""}>
      <header className="tc-head" onPointerDown={drag}>
        <span className="tc-title guide-title">{name ?? home(path ?? "")}</span>
        <button type="button" className="tc-x" onClick={onClose} aria-label={t("common.close")} title={t("common.close") + "（Esc）"}>
          <Icon name="close" />
        </button>
      </header>
      <div className="tc-body md" dangerouslySetInnerHTML={{ __html: text === undefined ? "" : renderMarkdown(text, {}) }} />
      <div className="tc-grip" onPointerDown={resize} aria-hidden />
    </div>,
    document.body,
  );
}

/** The library of one kind, and what can be copied into it. */
export function LibraryPane({ kind }: { kind: LibraryKind }) {
  const [again, setAgain] = useState(0);
  const lib = useLibrary(kind, again);
  const [picked, setPicked] = useState<string[]>([]);
  const [asking, setAsking] = useState<string[] | null>(null);
  const [preview, setPreview] = useState<{ name: string | null; path: string | null } | null>(null);
  const [confirm, setConfirm] = useState<string | null>(null);
  const noun = kind === "skill" ? "Skills" : t("lib.rules");
  const items = lib?.items ?? [];
  const found = lib?.found ?? [];
  const importNow = (paths: string[]) => {
    send({ t: "library_import", kind, paths });
    setPicked([]);
  };
  const ask = (paths: string[]) => {
    let skip = false;
    try {
      skip = localStorage.getItem(ASK) === "1";
    } catch {}
    if (skip) importNow(paths);
    else setAsking(paths);
  };
  return (
    <div className="acct lib">
      <section className="acct-group">
        <h3>{t("lib.mine", { noun })}</h3>
        <div className="lib-box">
          {!lib ? (
            <p className="dim lib-none">{t("common.loading")}</p>
          ) : !items.length ? (
            <p className="dim lib-none">{t("lib.none", { noun })}</p>
          ) : (
            items.map((i) => (
              <div key={i.name} className="lib-row">
                <span className="lib-main">
                  <b>{i.name}</b>
                  <span className="dim">{i.description || "—"}</span>
                </span>
                <span className="lib-meta">
                  {i.changed && <span className="acct-tag changed">{t("mcp.changed")}</span>}
                  {i.missing && <span className="acct-tag">{t("lib.missing")}</span>}
                  {i.paths && i.paths.length > 0 && <span className="acct-tag" title={i.paths.join(", ")}>{t("lib.scoped")}</span>}
                  <span className="dim" title={i.from}>
                    {home(i.from)}
                  </span>
                  <button onClick={() => setPreview({ name: i.name, path: null })}>{t("lib.preview")}</button>
                  {i.changed && <button onClick={() => ask([i.from])}>{t("lib.reimport")}</button>}
                  {confirm === i.name ? (
                    <button className="danger" onClick={() => (setConfirm(null), send({ t: "library_remove", kind, name: i.name }))} onMouseLeave={() => setConfirm(null)}>
                      {t("common.confirmDelete")}
                    </button>
                  ) : (
                    <button onClick={() => setConfirm(i.name)}>{t("lib.remove")}</button>
                  )}
                </span>
              </div>
            ))
          )}
        </div>
      </section>
      <section className="acct-group">
        <h3>
          {t("lib.found")}
          <button className="lib-rescan" onClick={() => setAgain((n) => n + 1)}>
            {t("lib.rescan")}
          </button>
        </h3>
        <PickList
          items={found.map((f) => ({
            id: f.path,
            label: f.name,
            meta: (
              <>
                {f.state !== "new" && <span className={`acct-tag ${f.state === "same" ? "" : "changed"}`}>{t(f.state === "same" ? "mcp.imported" : "mcp.changed")}</span>}
                <span className="acct-tag">{VENDOR[f.vendor]}</span>
                <span title={f.path}>{home(f.path).replace(/\/[^/]+$/, "")}</span>
              </>
            ),
          }))}
          picked={picked}
          onChange={setPicked}
          onOpen={(path) => setPreview({ name: null, path })}
          search={t("lib.search", { noun })}
          empty={t("lib.foundNone", { noun })}
          action={
            <button type="button" className="picks-action primary-ish" disabled={!picked.length} onClick={() => ask(picked)}>
              {t("lib.import", { n: picked.length })}
            </button>
          }
        />
        <p className="acct-hint">{t(kind === "skill" ? "lib.skillHint" : "lib.ruleHint")}</p>
      </section>
      {asking && <ImportConfirm kind={kind} paths={asking} onCancel={() => setAsking(null)} onOk={() => (importNow(asking), setAsking(null))} />}
      {preview && <LibraryCard kind={kind} name={preview.name} path={preview.path} onClose={() => setPreview(null)} />}
    </div>
  );
}

/** Before a copy is made: what is copied, and that it parts ways with the original after. Its
 *  own layer over the settings (which leave Esc to it while it is up, see Modal). */
function ImportConfirm({ kind, paths, onCancel, onOk }: { kind: LibraryKind; paths: string[]; onCancel: () => void; onOk: () => void }) {
  const [quiet, setQuiet] = useState(false);
  const noun = kind === "skill" ? "Skills" : t("lib.rules");
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !e.isComposing) (e.stopPropagation(), onCancel());
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [onCancel]);
  return createPortal(
    <div className="modal-scrim lib-confirm-scrim" onMouseDown={(e) => e.target === e.currentTarget && onCancel()}>
      <div className="modal lib-confirm-box" role="dialog" aria-modal="true" aria-label={t("lib.importTitle", { n: paths.length, noun })}>
        <div className="poll-form lib-confirm">
          <h3>{t("lib.importTitle", { n: paths.length, noun })}</h3>
          <div className="lib-paths">
            {paths.map((p) => (
              <div key={p}>{home(p)}</div>
            ))}
          </div>
          <ul>
            <li>{t("lib.copy1")}</li>
            <li>{t("lib.copy2")}</li>
            <li>{t("lib.copy3")}</li>
          </ul>
          <label className="lib-quiet">
            <input type="checkbox" checked={quiet} onChange={(e) => setQuiet(e.target.checked)} /> {t("lib.noAsk")}
          </label>
          <div className="mcp-form-foot">
            <button type="button" onClick={onCancel}>
              {t("common.cancel")}
            </button>
            <button
              type="button"
              className="primary"
              onClick={() => {
                if (quiet)
                  try {
                    localStorage.setItem(ASK, "1");
                  } catch {}
                onOk();
              }}
            >
              {t("lib.copyOk")}
            </button>
          </div>
        </div>
      </div>
    </div>,
    document.body,
  );
}
