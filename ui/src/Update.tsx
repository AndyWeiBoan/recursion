// App updates. The app asks GitHub for a newer release at start and every few hours; a small card
// says when one is out, and Settings › 軟體資訊 is where the human checks and installs (which
// downloads it, swaps the app in place and restarts into it).
import { useEffect, useState, useSyncExternalStore } from "react";
import { invoke } from "@tauri-apps/api/core";
import { t } from "./i18n";
import { openSettings } from "./Settings";

const EVERY = 6 * 60 * 60 * 1000;
const desktop = "__TAURI_INTERNALS__" in window;

export type UpdateState =
  | { s: "idle" }
  | { s: "checking" }
  | { s: "latest" }
  | { s: "available"; version: string }
  | { s: "installing"; version: string }
  | { s: "failed"; version?: string };

let state: UpdateState = { s: "idle" };
const subs = new Set<() => void>();
function set(next: UpdateState) {
  state = next;
  subs.forEach((f) => f());
}
export function useUpdate() {
  return useSyncExternalStore(
    (f) => {
      subs.add(f);
      return () => subs.delete(f);
    },
    () => state,
  );
}

export async function checkUpdate() {
  if (!desktop || state.s === "checking" || state.s === "installing") return;
  set({ s: "checking" });
  try {
    const v = await invoke<string | null>("update_check");
    set(v ? { s: "available", version: v } : { s: "latest" });
  } catch {
    set({ s: "failed" });
  }
}

export function installUpdate() {
  if (state.s !== "available") return;
  const version = state.version;
  set({ s: "installing", version });
  invoke("update_install").catch(() => set({ s: "failed", version }));
}

/** The running app's version, as built. */
export function useAppVersion() {
  const [v, setV] = useState<string | null>(null);
  useEffect(() => {
    if (desktop) import("@tauri-apps/api/app").then((m) => m.getVersion()).then(setV, () => {});
  }, []);
  return v;
}

/** Checks in the background and shows a card when a new version is out. */
export function UpdateNotice() {
  const u = useUpdate();
  const [dismissed, setDismissed] = useState<string | null>(null);
  useEffect(() => {
    checkUpdate();
    const id = setInterval(checkUpdate, EVERY);
    return () => clearInterval(id);
  }, []);
  if (u.s !== "available" || dismissed === u.version) return null;
  return (
    <div className="update-bar" role="status">
      <span>{t("update.ready", { version: u.version })}</span>
      <button className="primary" onClick={() => (setDismissed(u.version), openSettings("about"))}>
        {t("update.view")}
      </button>
      <button className="update-later" onClick={() => setDismissed(u.version)}>
        {t("update.later")}
      </button>
    </div>
  );
}

/** The update section of Settings › 軟體資訊. */
export function UpdateSection() {
  const u = useUpdate();
  if (!desktop) return null;
  const line =
    u.s === "checking" ? t("update.checking")
    : u.s === "latest" ? t("update.latest")
    : u.s === "available" ? t("update.ready", { version: u.version })
    : u.s === "installing" ? t("update.installingHint", { version: u.version })
    : u.s === "failed" ? t("update.failed")
    : t("update.hint");
  return (
    <div className="set-restart">
      <div>
        <b>{t("update.title")}</b>
        <p className="dim">{line}</p>
      </div>
      {u.s === "available" || u.s === "installing" ? (
        <button className="primary" disabled={u.s === "installing"} onClick={installUpdate}>
          {u.s === "installing" ? t("update.installing") : t("update.install")}
        </button>
      ) : (
        <button disabled={u.s === "checking"} onClick={checkUpdate}>
          {t("update.check")}
        </button>
      )}
    </div>
  );
}
