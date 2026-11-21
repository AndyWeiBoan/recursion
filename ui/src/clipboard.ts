// The system clipboard. In the app it goes through Tauri: WebKitGTK's own clipboard reads
// are unreliable. In a plain browser (tests) it falls back to navigator.clipboard.
import { readText, writeText } from "@tauri-apps/plugin-clipboard-manager";

const isTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

export async function copyText(text: string) {
  try {
    if (isTauri) await writeText(text);
    else await navigator.clipboard.writeText(text);
  } catch (e) {
    console.warn("copy failed", e);
  }
}

export async function pasteText(): Promise<string> {
  try {
    return (isTauri ? await readText() : await navigator.clipboard.readText()) ?? "";
  } catch (e) {
    console.warn("paste failed", e);
    return "";
  }
}

const editable = (el: Element | null): el is HTMLInputElement | HTMLTextAreaElement =>
  (el instanceof HTMLTextAreaElement || (el instanceof HTMLInputElement && /^(text|search|url|email|)$/.test(el.type))) && !el.readOnly && !el.disabled;

/** What Ctrl+C would copy: the selected part of a text field, or the selected page text. */
function selected(el: Element | null) {
  if (el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement) return el.value.slice(el.selectionStart ?? 0, el.selectionEnd ?? 0);
  return window.getSelection()?.toString() ?? "";
}

/**
 * Ctrl+A / C / X / V / Z for text fields and page text. In the Linux app the webview's own
 * versions do nothing, so the app does them. Terminals handle their own keys (Ctrl+Shift+C/V).
 */
export function installClipboardKeys() {
  if (!isTauri || /Mac/.test(navigator.platform)) return;
  window.addEventListener(
    "keydown",
    (e) => {
      if (!e.ctrlKey || e.altKey || e.metaKey) return;
      const k = e.key.toLowerCase();
      // Undo / redo: Ctrl+Z, Ctrl+Shift+Z, Ctrl+Y.
      if ((k === "z" || k === "y") && editable(document.activeElement) && !document.activeElement?.closest(".xterm")) {
        e.preventDefault();
        document.execCommand(k === "y" || e.shiftKey ? "redo" : "undo");
        return;
      }
      if (e.shiftKey) return;
      if (k !== "a" && k !== "c" && k !== "x" && k !== "v") return;
      const el = document.activeElement;
      if (el?.closest(".xterm")) return;
      if (k === "a") {
        if (!editable(el)) return;
        e.preventDefault();
        el.select();
        return;
      }
      if (k === "v") {
        if (!editable(el)) return;
        e.preventDefault();
        // insertText goes through the field's own editing, so React sees it and Ctrl+Z undoes it.
        void pasteText().then((t) => t && document.execCommand("insertText", false, t));
        return;
      }
      const text = selected(el);
      if (!text) return;
      e.preventDefault();
      void copyText(text);
      if (k === "x" && editable(el)) document.execCommand("delete");
    },
    true,
  );
}
