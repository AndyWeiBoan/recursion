// The sidebar's Agents list: every agent of every chat, grouped by what it is doing — waiting for
// the human first, then working, then idle — so one that is stuck in a folded project is still in
// sight. Working ones' marks move (BrandMark), idle ones with a reply the human hasn't seen say so.
// The three groups always show, empty or not, so a change of state moves an agent and nothing
// else. It folds like Usage below it, remembered.
import { useEffect, useState } from "react";
import { send } from "./client";
import { t } from "./i18n";
import { Icon } from "./icons";
import { markSeen, seenBaseline, useSeen } from "./seen";
import type { Chat, Member, State } from "./types";
import { Avatar, isRunning } from "./WorkspaceView";

type Entry = { m: Member; panel: string; where: string };

const KEY = "recursion.agentsOpen";
const loadOpen = () => {
  try {
    return localStorage.getItem(KEY) !== "0";
  } catch {
    return true;
  }
};

export function AgentsPanel({ state, chats, roster }: { state: State; chats: Record<string, Chat[]>; roster: Record<string, Member[]> }) {
  const seen = useSeen();
  const [open, setOpen] = useState(loadOpen);
  const toggle = () =>
    setOpen((v) => {
      try {
        localStorage.setItem(KEY, v ? "0" : "1");
      } catch {}
      return !v;
    });
  const all: Entry[] = state.sessions.flatMap((s) =>
    (chats[s.id] ?? []).flatMap((c) => (roster[c.panel] ?? []).map((m) => ({ m, panel: c.panel, where: `${s.name} · ${c.label}` }))),
  );
  // Replies from before this list first saw a chat aren't news.
  useEffect(() => {
    for (const e of all) seenBaseline(e.panel, e.m.replied ?? 0);
  }, [all.length]);
  if (!all.length) return null;

  const waiting = all.filter((e) => e.m.status === "waiting_permission");
  const running = all.filter((e) => isRunning(e.m));
  const idle = all.filter((e) => e.m.status === "idle");
  const go = (e: Entry) => {
    send({ t: "panel_focus", panel: e.panel });
    markSeen(e.panel, e.m.replied ?? 0);
  };
  const row = (e: Entry, kind: "wait" | "run" | "idle") => {
    const fresh = kind === "idle" && (e.m.replied ?? 0) > (seen[e.panel] ?? Infinity);
    return (
      <button key={`${e.panel}/${e.m.id}`} className={`agents-row${kind === "idle" && !fresh ? " seen" : ""}`} onClick={() => go(e)} title={`@${e.m.alias} · ${e.where}`}>
        <Avatar m={e.m} size={16} running={kind === "run"} />
        <span className="agents-who">
          <b>@{e.m.alias}</b>
          <span>{kind === "wait" && e.m.waiting_for ? `${e.where} · ${e.m.waiting_for}` : e.where}</span>
        </span>
        {kind === "wait" && <span className="agents-tag wait">{t("agents.approve")}</span>}
        {fresh && <span className="agents-tag new">{t("agents.new")}</span>}
      </button>
    );
  };
  const group = (title: string, list: Entry[], kind: "wait" | "run" | "idle") => (
    <div className={`agents-grp ${kind}${list.length ? "" : " empty"}`}>
      <div className="agents-grp-head">
        {title}
        <span className="dim">{list.length}</span>
      </div>
      {list.map((e) => row(e, kind))}
    </div>
  );
  return (
    <section className="agents" aria-label={t("agents.title")}>
      <button className="agents-head" onClick={toggle} aria-expanded={open} title={open ? t("agents.collapse") : t("agents.expand")}>
        <Icon name="agents" size={14} /> {t("agents.title")}
        <span className="dim">{all.length}</span>
      </button>
      {open && (
        <div className="agents-list">
          {group(t("agents.waiting"), waiting, "wait")}
          {group(t("agents.running"), running, "run")}
          {group(t("agents.idle"), idle, "idle")}
        </div>
      )}
    </section>
  );
}
