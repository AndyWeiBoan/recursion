// The system clipboard. In the app it goes through Tauri: WebKitGTK's own clipboard reads
// are unreliable. In a plain browser (tests) it falls back to navigator.clipboard.
import { t } from "./i18n";
import { readImage, readText, writeText } from "@tauri-apps/plugin-clipboard-manager";

const isTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

/** Puts text on the clipboard; false if that failed. */
export async function copyText(text: string): Promise<boolean> {
  try {
    if (isTauri) await writeText(text);
    else await navigator.clipboard.writeText(text);
    return true;
  } catch (e) {
    console.warn("copy failed", e);
    return false;
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

/**
 * A picture on the system clipboard, as a PNG file; null when there is none. The webviews often
 * leave pictures out of a paste event (WKWebView with a screenshot), so the app reads it itself.
 */
export async function readClipboardImage(): Promise<File | null> {
  if (!isTauri) return null;
  try {
    const img = await readImage();
    const { width, height } = await img.size();
    const rgba = await img.rgba();
    if (!width || !height) return null;
    const canvas = document.createElement("canvas");
    canvas.width = width;
    canvas.height = height;
    canvas.getContext("2d")!.putImageData(new ImageData(new Uint8ClampedArray(rgba), width, height), 0, 0);
    const blob = await new Promise<Blob | null>((r) => canvas.toBlob(r, "image/png"));
    return blob ? new File([blob], "pasted.png", { type: "image/png" }) : null;
  } catch {
    return null; // nothing that is a picture
  }
}

/** Hands a picture to the focused field as a paste event, the way a native paste would. */
function pastePicture(el: Element, file: File) {
  const dt = new DataTransfer();
  dt.items.add(file);
  el.dispatchEvent(new ClipboardEvent("paste", { clipboardData: dt, bubbles: true, cancelable: true }));
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
        // No text: maybe a picture (a screenshot), for fields that take one (the message box).
        void pasteText().then(async (t) => {
          if (t) return void document.execCommand("insertText", false, t);
          const pic = await readClipboardImage();
          if (pic) pastePicture(el, pic);
        });
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

/** Saves a file a doc page downloaded: a save dialog in the app, a plain download in a browser. */
export async function saveDownload(name: string, data: ArrayBuffer) {
  if (!isTauri) {
    const a = document.createElement("a");
    a.href = URL.createObjectURL(new Blob([data]));
    a.download = name;
    a.click();
    setTimeout(() => URL.revokeObjectURL(a.href), 1000);
    return;
  }
  const [{ save }, { invoke }] = await Promise.all([import("@tauri-apps/plugin-dialog"), import("@tauri-apps/api/core")]);
  const path = await save({ defaultPath: name, title: t("common.saveAs") });
  if (!path) return;
  try {
    await invoke("save_bytes", new Uint8Array(data), { headers: { path: encodeURIComponent(path) } });
  } catch (e) {
    console.warn("save failed", e);
  }
}
