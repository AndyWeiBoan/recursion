// Markdown helpers for the chat box: each takes the text and the selection, and says which
// range to replace with what and where the selection goes after. Applied through the browser's
// own editing (see `apply`), so Ctrl+Z undoes them like typing.

export interface Edit {
  /** Range of the current text to replace. */
  from: number;
  to: number;
  insert: string;
  /** Selection afterwards. */
  selStart: number;
  selEnd: number;
}

/** `**bold**`, `*italic*`, `~~strike~~`, `` `code` ``: wraps the selection, or unwraps it if it already is. */
export function wrap(text: string, a: number, b: number, mark: string): Edit {
  const n = mark.length;
  if (text.slice(a - n, a) === mark && text.slice(b, b + n) === mark) {
    return { from: a - n, to: b + n, insert: text.slice(a, b), selStart: a - n, selEnd: b - n };
  }
  const inner = text.slice(a, b);
  return { from: a, to: b, insert: mark + inner + mark, selStart: a + n, selEnd: b + n };
}

/** A fenced code block around the selection (or an empty one), on lines of its own. */
export function codeBlock(text: string, a: number, b: number): Edit {
  const before = a > 0 && text[a - 1] !== "\n" ? "\n" : "";
  const after = b < text.length && text[b] !== "\n" ? "\n" : "";
  const inner = text.slice(a, b);
  const insert = `${before}\`\`\`\n${inner}\n\`\`\`${after}`;
  const start = a + before.length + 4;
  return { from: a, to: b, insert, selStart: start, selEnd: start + inner.length };
}

/** The whole lines the selection touches. */
function lines(text: string, a: number, b: number) {
  const from = text.lastIndexOf("\n", a - 1) + 1;
  let to = text.indexOf("\n", b);
  if (to < 0) to = text.length;
  return { from, to, list: text.slice(from, to).split("\n") };
}

const BULLET = /^(\s*)[-*] /;
const NUMBER = /^(\s*)\d+\. /;
const QUOTE = /^> ?/;

/** Bulleted / numbered list or quote on every selected line; again on a list removes it. */
export function prefix(text: string, a: number, b: number, kind: "bullet" | "number" | "quote"): Edit {
  const { from, to, list } = lines(text, a, b);
  const re = kind === "bullet" ? BULLET : kind === "number" ? NUMBER : QUOTE;
  const all = list.every((l) => re.test(l) || !l.trim());
  const out = list.map((l, i) => {
    if (all) return l.replace(re, "$1");
    if (!l.trim() && list.length > 1) return l;
    const bare = l.replace(BULLET, "$1").replace(NUMBER, "$1").replace(QUOTE, "");
    return kind === "bullet" ? `- ${bare}` : kind === "number" ? `${i + 1}. ${bare}` : `> ${bare}`;
  });
  const insert = out.join("\n");
  return { from, to, insert, selStart: from + insert.length, selEnd: from + insert.length };
}

/** Is the caret inside an open ``` block? (Enter then adds a line instead of sending.) */
export function inCodeBlock(text: string, caret: number) {
  return (text.slice(0, caret).match(/^```/gm)?.length ?? 0) % 2 === 1;
}

/**
 * Enter on a list line: continue the list ("- ", "3. "), or end it when the item is empty.
 * `null` when the line isn't a list item.
 */
export function continueList(text: string, caret: number): Edit | null {
  const start = text.lastIndexOf("\n", caret - 1) + 1;
  const line = text.slice(start, caret);
  const m = /^(\s*)([-*]|\d+\.) (.*)$/.exec(line) ?? /^(>) ?(.*)$/.exec(line);
  if (!m) return null;
  const quote = m[1] === ">";
  const body = quote ? m[2] : m[3];
  if (!body.trim()) return { from: start, to: caret, insert: "", selStart: start, selEnd: start };
  const next = quote ? "> " : `${m[1]}${/\d/.test(m[2]) ? `${parseInt(m[2]) + 1}.` : m[2]} `;
  return { from: caret, to: caret, insert: "\n" + next, selStart: caret + 1 + next.length, selEnd: caret + 1 + next.length };
}

/** Applies an edit to a text field so it lands in its undo history and fires `input`. */
export function apply(el: HTMLTextAreaElement, e: Edit) {
  el.focus();
  el.setSelectionRange(e.from, e.to);
  // execCommand is the only way to keep native undo; fall back to setRangeText.
  const ok = e.insert ? document.execCommand("insertText", false, e.insert) : e.from === e.to || document.execCommand("delete");
  if (!ok) {
    el.setRangeText(e.insert, e.from, e.to, "end");
    el.dispatchEvent(new Event("input", { bubbles: true }));
  }
  el.setSelectionRange(e.selStart, e.selEnd);
}
