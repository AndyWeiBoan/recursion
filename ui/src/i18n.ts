/// <reference types="vite/client" />
// The UI's text in the human's language. Each language is one file in `locales/` (shared with the
// daemon): adding `locales/ja.json` adds Japanese to Settings, no code change. A key missing from a
// language falls back to English, then to the key itself.
import { useSyncExternalStore } from "react";

type Table = Record<string, string>;
const files = import.meta.glob("../../locales/*.json", { eager: true, import: "default" }) as Record<string, Table>;
const tables: Record<string, Table> = Object.fromEntries(Object.entries(files).map(([path, t]) => [path.split("/").pop()!.replace(/\.json$/, ""), t]));
const FALLBACK = "en";

/** Built-in languages: code and own name ("English", "繁體中文"). */
export const languages = Object.entries(tables)
  .map(([code, t]) => ({ code, name: t._name ?? code }))
  .sort((a, b) => a.name.localeCompare(b.name));

/** The best available language for a list of wanted ones: exact, else same base language, else English. */
export function pick(wanted: readonly string[]): string {
  const have = Object.keys(tables);
  for (const w of wanted.map((x) => x.replace("_", "-").toLowerCase())) {
    const exact = have.find((c) => c.toLowerCase() === w);
    if (exact) return exact;
    const base = have.find((c) => c.toLowerCase().split("-")[0] === w.split("-")[0]);
    if (base) return base;
  }
  return FALLBACK;
}

const KEY = "recursion.lang";
// Remembered locally too, so the first paint (before the daemon answers) is already right.
let lang = (() => {
  try {
    return localStorage.getItem(KEY) || pick(navigator.languages);
  } catch {
    return pick(navigator.languages);
  }
})();
document.documentElement.lang = lang;
const subs = new Set<() => void>();

/** Called with the daemon's saved choice (`null`/absent: follow the system). */
export function syncLang(saved: string | null | undefined) {
  const next = saved ? pick([saved]) : pick(navigator.languages);
  if (next === lang) return;
  lang = next;
  document.documentElement.lang = lang;
  try {
    localStorage.setItem(KEY, lang);
  } catch {}
  subs.forEach((f) => f());
}

export function useLang() {
  return useSyncExternalStore(
    (f) => (subs.add(f), () => subs.delete(f)),
    () => lang,
  );
}

/**
 * `t("agent.joined", { alias })`: the text with each `{alias}` filled in. With a number `n`,
 * a `key_one` variant (English "1 vote") is used when n is 1.
 */
export function t(key: string, vars?: Record<string, string | number>): string {
  const own = tables[lang] ?? {};
  const en = tables[FALLBACK] ?? {};
  // The singular ("1 vote") only where the language that has the text has one: Chinese has none,
  // and must not borrow English's.
  const table = own[key] !== undefined ? own : en;
  const k = vars?.n === 1 && table[`${key}_one`] !== undefined ? `${key}_one` : key;
  let s = table[k] ?? key;
  if (vars) for (const [name, v] of Object.entries(vars)) s = s.split(`{${name}}`).join(String(v));
  return s;
}
