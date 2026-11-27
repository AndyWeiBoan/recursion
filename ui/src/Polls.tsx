// Polls: the form that starts one, its card in the chat, and its row / full view in the 產出 drawer.
// While a poll is open nobody's pick is shown (only who has voted), so it can't sway the agents.
import { QuoteButton, startRefDrag, type Payload } from "./refs";
import { t } from "./i18n";
import { useState } from "react";
import { send } from "./client";
import { RemoveButton, showInChat } from "./ThreadDrawer";
import { closeThreads, setThreadView } from "./threadsUI";
import { Avatar } from "./WorkspaceView";
import type { Member, Poll } from "./types";
import { Icon } from "./icons";

const MAX = 8;
export const letter = (i: number) => String.fromCharCode(65 + i);

function time(ts: number) {
  const d = new Date(ts);
  return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
}

/** Most votes; several when tied, none when nobody voted. */
export function winners(p: Poll) {
  const n = p.options.map((_, i) => p.votes.filter((v) => v.option === i).length);
  const top = Math.max(...n);
  return top > 0 ? n.flatMap((c, i) => (c === top ? [i] : [])) : [];
}

/** "A 勝出（2 票）", "A、B 平手", "沒有人投票". */
export function outcome(p: Poll) {
  const w = winners(p);
  if (!w.length) return t("poll.noVotes");
  const votes = p.votes.filter((v) => v.option === w[0]).length;
  return w.length > 1 ? t("poll.tie", { options: w.map(letter).join(t("common.listSep")), n: votes }) : t("poll.won", { option: `${letter(w[0])} ${p.options[w[0]]}`, n: votes });
}

function Voter({ id, byId }: { id: string; byId: Map<string, Member> }) {
  if (id === "you") return <span className="poll-you">{t("common.you")}</span>;
  const m = byId.get(id);
  return (
    <span title={`@${m?.alias ?? t("agent.gone")}`}>
      <Avatar m={m} size={16} />
    </span>
  );
}

export function PollForm({ panel, onDone }: { panel: string; onDone: () => void }) {
  const [question, setQuestion] = useState("");
  const [description, setDescription] = useState("");
  const [options, setOptions] = useState(["", ""]);
  const [mine, setMine] = useState<number | null>(null);
  // An option added from the keyboard gets the caret.
  const [focusAt, setFocusAt] = useState<number | null>(null);
  const filled = options.map((o) => o.trim()).filter(Boolean);
  const ok = question.trim() && filled.length >= 2;
  const submit = () => {
    if (!ok) return;
    // Its own pick is an index into the filled options.
    const kept = options.map((o, i) => [o.trim(), i] as const).filter(([o]) => o);
    const vote = mine === null ? null : kept.findIndex(([, i]) => i === mine);
    send({ t: "poll_create", panel, question: question.trim(), description: description.trim(), options: kept.map(([o]) => o), vote: vote === -1 ? null : vote });
    onDone();
  };
  return (
    <form
      className="poll-form"
      onSubmit={(e) => {
        e.preventDefault();
        submit();
      }}
    >
      <h3>{t("poll.start")}</h3>
      <p className="dim">{t("poll.formHint")}</p>
      <label className="field">
        <span>{t("poll.title")}</span>
        <input autoFocus value={question} onChange={(e) => setQuestion(e.target.value)} placeholder={t("poll.titlePlaceholder")} />
      </label>
      <label className="field">
        <span>
          {t("poll.description")}<span className="dim">{t("common.optional")}</span>
        </span>
        <textarea value={description} onChange={(e) => setDescription(e.target.value)} rows={3} placeholder={t("poll.descriptionPlaceholder")} />
      </label>
      <div className="field">
        <span>{t("poll.options")}</span>
        {options.map((o, i) => (
          <div key={i} className="poll-opt-edit">
            <b>{letter(i)}</b>
            <input
              ref={(el) => {
                if (el && focusAt === i) {
                  el.focus();
                  setFocusAt(null);
                }
              }}
              value={o}
              onChange={(e) => setOptions(options.map((x, k) => (k === i ? e.target.value : x)))}
              onKeyDown={(e) => {
                // Enter on the last option adds another instead of sending.
                if (e.key === "Enter" && i === options.length - 1 && o.trim() && options.length < MAX && !e.nativeEvent.isComposing) {
                  e.preventDefault();
                  setOptions([...options, ""]);
                  setFocusAt(options.length);
                }
              }}
              placeholder={t("poll.option", { letter: letter(i) })}
            />
            {o.trim() ? (
              <button type="button" className={`poll-mine${mine === i ? " on" : ""}`} onClick={() => setMine(mine === i ? null : i)} title={t("poll.mineHint")}>
                {mine === i ? <><Icon name="check" size={12} /> {t("poll.mine")}</> : t("poll.mine")}
              </button>
            ) : (
              <span />
            )}
            {options.length > 2 ? (
              <button type="button" className="poll-del" onClick={() => (setOptions(options.filter((_, k) => k !== i)), setMine(mine === i ? null : mine !== null && mine > i ? mine - 1 : mine))} aria-label={t("poll.removeOption", { letter: letter(i) })}>
                <Icon name="close" />
              </button>
            ) : (
              <span />
            )}
          </div>
        ))}
        {options.length < MAX && (
          <button type="button" className="poll-add" onClick={() => (setOptions([...options, ""]), setFocusAt(options.length))}>
            {t("poll.addOption")}
          </button>
        )}
      </div>
      <button className="add-submit" type="submit" disabled={!ok}>
        {t("poll.start")}
      </button>
    </form>
  );
}

/** The poll in the chat (and, with `full`, in the drawer: reasons always shown). */
export function PollCard({ panel, p, byId, full }: { panel: string; p: Poll; byId: Map<string, Member>; full?: boolean }) {
  const [why, setWhy] = useState(false);
  const open = p.status === "open";
  const mine = p.votes.find((v) => v.voter === "you")?.option;
  const voted = new Set(p.votes.map((v) => v.voter));
  const agentsIn = p.voters.filter((v) => voted.has(v)).length;
  const top = new Set(open ? [] : winners(p));
  const most = Math.max(1, ...p.options.map((_, i) => p.votes.filter((v) => v.option === i).length));
  const reasons = p.votes.filter((v) => v.reason);
  return (
    <div className={`poll-card${open ? " open" : ""}`} data-poll={p.id}>
      <div className="poll-head">
        <span className="poll-ic"><Icon name="poll" size={14} /></span>
        <b>{p.question}</b>
        <span className="poll-state">
          #{p.num} · {open ? t("poll.agentsIn", { done: agentsIn, all: p.voters.length }) : `${t("poll.closed")} · ${t("poll.votes", { n: p.votes.length })}`}
        </span>
      </div>
      {p.description && <p className="poll-desc">{p.description}</p>}
      <div className="poll-opts">
        {p.options.map((o, i) => {
          const who = p.votes.filter((v) => v.option === i);
          return (
            <button
              key={i}
              className={`poll-opt${mine === i ? " mine" : ""}${top.has(i) ? " top" : ""}`}
              disabled={!open}
              onClick={() => send({ t: "poll_vote", panel, poll: p.id, option: i })}
              title={open ? (mine === i ? t("poll.youVotedThis") : t("poll.voteThis")) : undefined}
            >
              <span className="poll-letter">{letter(i)}</span>
              <span className="poll-text">{o}</span>
              {open ? (
                mine === i && <span className="poll-check"><Icon name="check" size={12} /> {t("common.you")}</span>
              ) : (
                <>
                  <span className="poll-bar">
                    <i style={{ width: `${(who.length / most) * 100}%` }} />
                  </span>
                  <span className="poll-n">{who.length}</span>
                  <span className="poll-who">
                    {who.map((v) => (
                      <Voter key={v.voter} id={v.voter} byId={byId} />
                    ))}
                  </span>
                </>
              )}
            </button>
          );
        })}
      </div>
      <div className="poll-foot">
        {open ? (
          <>
            <span className="poll-in">
              {t("poll.votedSoFar")}
              {p.voters.filter((v) => voted.has(v)).map((v) => (
                <Voter key={v} id={v} byId={byId} />
              ))}
              {!agentsIn && <span className="dim">{t("poll.nobodyYet")}</span>}
            </span>
            <span className="dim">{t("poll.hiddenUntilDone")}</span>
            <button className="poll-close" onClick={() => send({ t: "poll_close", panel, poll: p.id })}>
              {t("poll.end")}
            </button>
          </>
        ) : (
          <>
            <span className="poll-outcome">{outcome(p)}</span>
            {!full && reasons.length > 0 && (
              <button className="poll-why" onClick={() => setWhy((v) => !v)}>
                <Icon name={why ? "dropdown" : "expand"} size={12} /> {why ? t("poll.hideReasons") : t("poll.showReasons", { n: reasons.length })}
              </button>
            )}
          </>
        )}
      </div>
      {!open && (full || why) && reasons.length > 0 && (
        <ul className="poll-reasons">
          {reasons.map((v) => (
            <li key={v.voter}>
              <Voter id={v.voter} byId={byId} />
              <b>{v.voter === "you" ? t("common.you") : `@${byId.get(v.voter)?.alias ?? t("agent.gone")}`}</b>
              <span className="poll-letter sm">{letter(v.option)}</span>
              <span>{v.reason}</span>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

/** The drawer's row for a poll. */
export function PollRow({ panel, p }: { panel: string; p: Poll }) {
  const open = p.status === "open";
  const ref: Payload = { ref: { kind: "poll", id: p.id, title: p.question } };
  return (
    <div className={`td-row${open ? " st-waiting" : ""}`} onPointerDown={(e) => startRefDrag(e, panel, ref, p.question)}>
      <button className="td-open" onClick={() => setThreadView(panel, { kind: "poll", id: p.id })}>
        <span className="td-num">
          <span className="td-icon"><Icon name="poll" size={14} /></span>#{p.num}
        </span>
        <span className="td-main">
          <span className="td-quote">{p.question}</span>
          <span className="td-meta">
            {open ? (
              <>
                <i className="td-dot" />
                {t("poll.open")} · {t("poll.votedCount", { done: p.voters.filter((v) => p.votes.some((x) => x.voter === v)).length, all: p.voters.length })}
              </>
            ) : (
              <>
                {outcome(p)} · {time(p.closed ?? p.created)}
              </>
            )}
          </span>
        </span>
      </button>
      <QuoteButton panel={panel} p={ref} />
      <button className="td-goto" onClick={() => showInChat(p.msg)} title={t("poll.findInChat")} aria-label={t("poll.cardOf", { num: p.num })}>
        <Icon name="find" />
      </button>
      <RemoveButton panel={panel} id={p.id} name={t("poll.name", { num: p.num })} />
    </div>
  );
}

/** One poll in the drawer, reasons and all. */
export function PollView({ panel, p, byId }: { panel: string; p: Poll; byId: Map<string, Member> }) {
  return (
    <>
      <header className="td-head">
        <button className="td-back" onClick={() => setThreadView(panel, { kind: "list" })}>
          <Icon name="back" size={14} /> {t("common.back")}
        </button>
        <b>{t("poll.name", { num: p.num })}</b>
        <span className="dim">{p.status === "open" ? t("poll.open") : t("poll.closed")}</span>
        <button className="td-x" onClick={() => closeThreads(panel)} aria-label={t("common.close")} title={t("drawer.closeHint")}>
          <Icon name="close" />
        </button>
      </header>
      <div className="td-body">
        <PollCard panel={panel} p={p} byId={byId} full />
        <button className="td-find" onClick={() => showInChat(p.msg)}>
          <Icon name="find" size={14} /> {t("common.findInChat")}
        </button>
      </div>
    </>
  );
}
