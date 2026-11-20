// Tiny bridge so any component can ask "which panel kind?" before creating one.
import type { PanelKind } from "./types";
import type { PickRequest } from "./KindPicker";

let show: ((r: PickRequest | null) => void) | null = null;
let open = false;

export function registerPicker(fn: (r: PickRequest | null) => void) {
  show = (r) => {
    open = !!r;
    fn(r);
  };
}

export function requestKind(title: string, anchor: DOMRect | null, onPick: (k: PanelKind) => void) {
  show?.({ title, anchor, onPick });
}

export function closePicker() {
  show?.(null);
}

export function pickerOpen() {
  return open;
}
