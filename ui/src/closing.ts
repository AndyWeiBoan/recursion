// Closing asks first only when something would be lost for good: a program running in a
// terminal (a bare shell has nothing to lose). A chat goes to Recently closed, a page or a
// document opens again the same.
import { getSnapshot, toast } from "./client";
import { t } from "./i18n";

const SHELLS = new Set(["bash", "zsh", "fish", "sh", "dash", "nu", "shell", "login"]);

/** The program closing these panels would stop, if any. */
export function wouldStop(panels: string[]): string | null {
  const { state, terms } = getSnapshot();
  for (const id of panels) {
    if (state?.panels[id]?.kind !== "terminal") continue;
    const fg = terms[id]?.fg;
    if (fg && !SHELLS.has(fg)) return fg;
  }
  return null;
}

// A close that needs a second go: which one is armed, until when.
let armed: { key: string; until: number } | null = null;

/** Close now, or, when it would stop a program, only on a second try within 3 seconds (saying
 *  so the first time). `key` tells the two tries apart from closing something else. */
export function closeChecked(key: string, panels: string[], close: () => void): boolean {
  const prog = wouldStop(panels);
  if (prog && !(armed?.key === key && armed.until > Date.now())) {
    armed = { key, until: Date.now() + 3000 };
    toast(t("close.confirm", { prog }));
    return false;
  }
  armed = null;
  close();
  return true;
}
