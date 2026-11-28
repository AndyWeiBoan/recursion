// How far the human has looked in each chat (the `seq` of the newest message on screen when they
// last had it in front of them), so that an agent's reply after that is a new one in the sidebar's
// Agents list. Kept per computer.
import { useSyncExternalStore } from "react";

const KEY = "recursion.seenUpTo";
let upTo: Record<string, number> = (() => {
  try {
    return JSON.parse(localStorage.getItem(KEY) ?? "{}") ?? {};
  } catch {
    return {};
  }
})();
const subs = new Set<() => void>();

/** The chat was looked at up to message `seq`. */
export function markSeen(panel: string, seq: number) {
  if ((upTo[panel] ?? -1) >= seq) return;
  upTo = { ...upTo, [panel]: seq };
  try {
    localStorage.setItem(KEY, JSON.stringify(upTo));
  } catch {}
  subs.forEach((f) => f());
}

/** A chat never looked at here starts as looked at up to now: old replies aren't news. */
export function seenBaseline(panel: string, seq: number) {
  if (upTo[panel] === undefined) markSeen(panel, seq);
}

export const useSeen = () =>
  useSyncExternalStore(
    (f) => (subs.add(f), () => subs.delete(f)),
    () => upTo,
  );
