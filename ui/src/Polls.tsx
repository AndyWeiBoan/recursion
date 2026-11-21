// Polls: the form that starts one, its card in the chat, and its row / full view in the 產出 drawer.
// While a poll is open nobody's pick is shown (only who has voted), so it can't sway the agents.
import { useState } from "react";
import { send } from "./client";
import { RemoveButton, showInChat } from "./ThreadDrawer";
import { closeThreads, setThreadView } from "./threadsUI";
import { Avatar } from "./WorkspaceView";
import type { Member, Poll } from "./types";

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
  if (!w.length) return "沒有人投票";
  const votes = p.votes.filter((v) => v.option === w[0]).length;
  return w.length > 1 ? `${w.map(letter).join("、")} 平手（各 ${votes} 票）` : `${letter(w[0])} ${p.options[w[0]]} 勝出（${votes} 票）`;
}

function Voter({ id, byId }: { id: string; byId: Map<string, Member> }) {
  if (id === "you") return <span className="poll-you">你</span>;
  const m = byId.get(id);
  return (
    <span title={`@${m?.alias ?? "已離開"}`}>
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
      <h3>發起投票</h3>
      <p className="dim">每個 agent 都會選一個並附理由。全部投完才公開誰選了什麼。</p>
      <label className="field">
        <span>標題</span>
        <input autoFocus value={question} onChange={(e) => setQuestion(e.target.value)} placeholder="例如：下一步先做哪個？" />
      </label>
      <label className="field">
        <span>
          描述<span className="dim">（可不填）</span>
        </span>
        <textarea value={description} onChange={(e) => setDescription(e.target.value)} rows={3} placeholder="背景、限制、希望大家考慮什麼…" />
      </label>
      <div className="field">
        <span>選項</span>
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
              placeholder={`選項 ${letter(i)}`}
            />
            {o.trim() ? (
              <button type="button" className={`poll-mine${mine === i ? " on" : ""}`} onClick={() => setMine(mine === i ? null : i)} title="你自己也可以投一票（可不選）">
                {mine === i ? "✓ 我投這個" : "我投這個"}
              </button>
            ) : (
              <span />
            )}
            {options.length > 2 ? (
              <button type="button" className="poll-del" onClick={() => (setOptions(options.filter((_, k) => k !== i)), setMine(mine === i ? null : mine !== null && mine > i ? mine - 1 : mine))} aria-label={`刪除選項 ${letter(i)}`}>
                ✕
              </button>
            ) : (
              <span />
            )}
          </div>
        ))}
        {options.length < MAX && (
          <button type="button" className="poll-add" onClick={() => (setOptions([...options, ""]), setFocusAt(options.length))}>
            ＋ 加選項
          </button>
        )}
      </div>
      <button className="add-submit" type="submit" disabled={!ok}>
        發起投票
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
        <span className="poll-ic">📊</span>
        <b>{p.question}</b>
        <span className="poll-state">
          #{p.num} · {open ? `${agentsIn}/${p.voters.length} 個 agent 已投` : `已結束 · ${p.votes.length} 票`}
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
              title={open ? (mine === i ? "你投了這個" : "投這個") : undefined}
            >
              <span className="poll-letter">{letter(i)}</span>
              <span className="poll-text">{o}</span>
              {open ? (
                mine === i && <span className="poll-check">✓ 你</span>
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
              已投：
              {p.voters.filter((v) => voted.has(v)).map((v) => (
                <Voter key={v} id={v} byId={byId} />
              ))}
              {!agentsIn && <span className="dim">還沒有</span>}
            </span>
            <span className="dim">全部投完才公開</span>
            <button className="poll-close" onClick={() => send({ t: "poll_close", panel, poll: p.id })}>
              結束投票
            </button>
          </>
        ) : (
          <>
            <span className="poll-outcome">{outcome(p)}</span>
            {!full && reasons.length > 0 && (
              <button className="poll-why" onClick={() => setWhy((v) => !v)}>
                {why ? "▾ 收起理由" : `▸ 看理由（${reasons.length}）`}
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
              <b>{v.voter === "you" ? "你" : `@${byId.get(v.voter)?.alias ?? "已離開"}`}</b>
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
  return (
    <div className={`td-row${open ? " st-waiting" : ""}`}>
      <button className="td-open" onClick={() => setThreadView(panel, { kind: "poll", id: p.id })}>
        <span className="td-num">
          <span className="td-icon">📊</span>#{p.num}
        </span>
        <span className="td-main">
          <span className="td-quote">{p.question}</span>
          <span className="td-meta">
            {open ? (
              <>
                <i className="td-dot" />
                進行中 · {p.voters.filter((v) => p.votes.some((x) => x.voter === v)).length}/{p.voters.length} 已投
              </>
            ) : (
              <>
                {outcome(p)} · {time(p.closed ?? p.created)}
              </>
            )}
          </span>
        </span>
      </button>
      <button className="td-goto" onClick={() => showInChat(p.msg)} title="在主對話中找到這次投票" aria-label={`#${p.num} 的投票卡`}>
        ↖
      </button>
      <RemoveButton panel={panel} id={p.id} name={`投票 #${p.num}`} />
    </div>
  );
}

/** One poll in the drawer, reasons and all. */
export function PollView({ panel, p, byId }: { panel: string; p: Poll; byId: Map<string, Member> }) {
  return (
    <>
      <header className="td-head">
        <button className="td-back" onClick={() => setThreadView(panel, { kind: "list" })}>
          ← 返回
        </button>
        <b>投票 #{p.num}</b>
        <span className="dim">{p.status === "open" ? "進行中" : "已結束"}</span>
        <button className="td-x" onClick={() => closeThreads(panel)} aria-label="關閉" title="關閉（Esc 返回）">
          ✕
        </button>
      </header>
      <div className="td-body">
        <PollCard panel={panel} p={p} byId={byId} full />
        <button className="td-find" onClick={() => showInChat(p.msg)}>
          ↖ 在對話中找到
        </button>
      </div>
    </>
  );
}
