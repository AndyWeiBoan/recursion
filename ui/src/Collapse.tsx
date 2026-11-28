// Folds its children away and back with a slide, at the sidebar's own pace: their height eases
// between none and all of it. Measured in pixels rather than with CSS grid rows, which older
// WebKit (macOS Ventura's) can't animate. Still mounted while it slides, gone once folded.
import { useLayoutEffect, useRef, useState } from "react";

export function Collapse({ open, className, children }: { open: boolean; className?: string; children: React.ReactNode }) {
  const [mounted, setMounted] = useState(open);
  const box = useRef<HTMLDivElement>(null);
  const first = useRef(true);
  const last = useRef<HTMLDivElement | null>(null);
  if (open && !mounted) setMounted(true);
  useLayoutEffect(() => {
    const el = box.current;
    // Just put in to open: it starts from nothing.
    const fresh = el !== last.current;
    last.current = el;
    if (first.current) {
      first.current = false;
      return;
    }
    if (!el || matchMedia("(prefers-reduced-motion: reduce)").matches) {
      if (!open) setMounted(false);
      return;
    }
    // From where it is now to where it goes, then back to its own height (or gone).
    // Its own height laid out where it is (a list that scrolls inside a capped box is as tall as
    // the box lets it be, not as its content).
    const from = fresh ? 0 : el.getBoundingClientRect().height;
    el.style.height = "";
    const full = el.getBoundingClientRect().height;
    el.style.height = `${from}px`;
    void el.offsetHeight;
    el.classList.add("sliding");
    el.style.height = open ? `${full}px` : "0px";
    const done = (e?: TransitionEvent) => {
      if (e && (e.target !== el || e.propertyName !== "height")) return;
      el.removeEventListener("transitionend", done);
      // Shut: it stays shut (height 0, clipped) until it is gone, or it would flash back for a frame.
      if (!open) return setMounted(false);
      el.classList.remove("sliding");
      el.style.height = "";
    };
    el.addEventListener("transitionend", done);
    return () => el.removeEventListener("transitionend", done);
  }, [open]);
  if (!mounted) return null;
  return (
    <div ref={box} className={`collapse${open ? " open" : ""}${className ? ` ${className}` : ""}`}>
      {children}
    </div>
  );
}
