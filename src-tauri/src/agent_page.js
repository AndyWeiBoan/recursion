// Runs inside a browser panel's page, in an isolated JS world (the page's own scripts can't
// see or tamper with it), as the body of an async function with `op` and `args` in scope.
// Returns a JSON string. Interactive elements get numbered refs that click/fill use.
const MAX = 30000;
const SKIP = new Set(["SCRIPT", "STYLE", "NOSCRIPT", "TEMPLATE", "SVG", "CANVAS", "IFRAME", "HEAD"]);
const BLOCK = new Set(["P", "DIV", "SECTION", "ARTICLE", "MAIN", "HEADER", "FOOTER", "NAV", "ASIDE", "UL", "OL", "LI", "TABLE", "TR", "FORM", "FIELDSET", "BLOCKQUOTE", "PRE", "H1", "H2", "H3", "H4", "H5", "H6", "DL", "DT", "DD", "FIGURE", "BR", "HR"]);
const ROLES = new Set(["button", "link", "checkbox", "radio", "tab", "menuitem", "option", "switch", "combobox", "textbox"]);

const info = () => ({ url: location.href, title: document.title });
const byRef = (n) => document.querySelector(`[data-rc-ref="${Number(n)}"]`);
const clip = (s, n) => (s.length > n ? s.slice(0, n - 1) + "…" : s);
const label = (el) =>
  clip(
    (el.getAttribute("aria-label") || el.innerText || el.value || el.getAttribute("title") || el.getAttribute("alt") || el.getAttribute("placeholder") || "")
      .replace(/\s+/g, " ")
      .trim(),
    80,
  );

function hidden(el) {
  if (el.checkVisibility) return !el.checkVisibility({ checkOpacity: false, checkVisibilityCSS: true });
  const s = getComputedStyle(el);
  return s.display === "none" || s.visibility === "hidden";
}

function interactive(el) {
  const t = el.tagName;
  if (t === "A") return el.hasAttribute("href");
  if (t === "BUTTON" || t === "SELECT" || t === "TEXTAREA" || t === "SUMMARY") return true;
  if (t === "INPUT") return el.type !== "hidden";
  if (el.isContentEditable && !el.parentElement?.isContentEditable) return true;
  return ROLES.has(el.getAttribute("role") || "");
}

function describe(el, n) {
  const t = el.tagName;
  if (t === "A") {
    let href = el.getAttribute("href") || "";
    try {
      href = new URL(href, location.href).href;
    } catch {}
    return `[#${n} link: ${label(el) || "(no text)"} → ${clip(href, 160)}]`;
  }
  if (t === "INPUT" && (el.type === "checkbox" || el.type === "radio")) {
    const box = el.labels?.[0]?.innerText?.trim() || el.name || "";
    return `[#${n} ${el.type} ${el.checked ? "☑" : "☐"} ${clip(box, 60)}]`;
  }
  if (t === "INPUT" || t === "TEXTAREA") {
    const name = el.labels?.[0]?.innerText?.trim() || el.getAttribute("aria-label") || el.name || el.placeholder || "";
    const v = el.type === "password" ? (el.value ? "••••" : "") : el.value;
    return `[#${n} ${t === "TEXTAREA" ? "textarea" : `input ${el.type}`}: ${clip(name, 60)}${v ? ` = "${clip(v, 80)}"` : ""}]`;
  }
  if (t === "SELECT") {
    const opts = [...el.options].slice(0, 15).map((o) => (o.selected ? `*${o.text.trim()}*` : o.text.trim()));
    return `[#${n} select: ${opts.join(" | ")}${el.options.length > 15 ? " | …" : ""}]`;
  }
  if (el.isContentEditable) return `[#${n} editable: ${clip(el.innerText.trim(), 80)}]`;
  return `[#${n} ${el.getAttribute("role") || "button"}: ${label(el) || "(no text)"}]`;
}

function read() {
  document.querySelectorAll("[data-rc-ref]").forEach((e) => e.removeAttribute("data-rc-ref"));
  let n = 0;
  const out = [];
  const walk = (node) => {
    if (node.nodeType === Node.TEXT_NODE) {
      const t = node.nodeValue.replace(/\s+/g, " ");
      if (t.trim()) out.push(t);
      return;
    }
    if (node.nodeType !== Node.ELEMENT_NODE) return;
    const el = node;
    if (SKIP.has(el.tagName.toUpperCase()) || hidden(el)) return;
    if (interactive(el)) {
      el.setAttribute("data-rc-ref", String(++n));
      out.push(" " + describe(el, n) + " ");
      // A button or link's own text is already in its description.
      if (["A", "BUTTON", "SELECT", "TEXTAREA", "INPUT", "SUMMARY"].includes(el.tagName) || el.getAttribute("role")) return;
    }
    const tag = el.tagName;
    const block = BLOCK.has(tag);
    if (block) out.push("\n");
    if (/^H[1-6]$/.test(tag)) out.push("#".repeat(Number(tag[1])) + " ");
    if (tag === "LI") out.push("- ");
    if (tag === "IMG" && el.alt) out.push(`[image: ${clip(el.alt, 80)}]`);
    if (tag === "TD" || tag === "TH") out.push(" | ");
    for (const c of el.shadowRoot ? el.shadowRoot.childNodes : el.childNodes) walk(c);
    if (block) out.push("\n");
  };
  walk(document.body || document.documentElement);
  const text = out
    .join("")
    .replace(/[ \t]+\n/g, "\n")
    .replace(/\n[ \t]+/g, "\n")
    .replace(/\n{3,}/g, "\n\n")
    .trim();
  return { text, refs: n };
}

const settle = () => new Promise((r) => setTimeout(r, 150));
// A click or submit may navigate away, which throws this world away mid-call and the answer never
// comes back. So the action runs just after we return; the app then waits to see where it landed.
const later = (f) => setTimeout(f, 0);

switch (op) {
  case "info":
    return JSON.stringify({ ...info(), ready: document.readyState });
  case "read": {
    const { text, refs } = read();
    const from = Math.max(0, Number(args.from) || 0);
    const part = text.slice(from, from + MAX);
    const more = from + MAX < text.length ? from + MAX : null;
    return JSON.stringify({ ...info(), text: part, refs, total: text.length, more });
  }
  case "click": {
    const el = byRef(args.ref);
    if (!el) return JSON.stringify({ error: `No element #${args.ref} on this page. Call browse_read to get fresh numbers.` });
    el.scrollIntoView({ block: "center" });
    el.focus?.();
    later(() => el.click());
    return JSON.stringify({ ...info(), did: `clicked ${describe(el, args.ref)}` });
  }
  case "fill": {
    const el = byRef(args.ref);
    if (!el) return JSON.stringify({ error: `No element #${args.ref} on this page. Call browse_read to get fresh numbers.` });
    const value = String(args.value ?? "");
    el.scrollIntoView({ block: "center" });
    el.focus?.();
    if (el.tagName === "SELECT") {
      const opt = [...el.options].find((o) => o.value === value || o.text.trim() === value);
      if (!opt) return JSON.stringify({ error: `#${args.ref} has no option "${value}".` });
      el.value = opt.value;
    } else if (el.type === "checkbox" || el.type === "radio") {
      const want = !/^(false|0|no|off|)$/i.test(value);
      if (el.checked !== want) el.click();
    } else if (el.isContentEditable) {
      el.textContent = value;
    } else {
      // Go through the prototype's setter so frameworks (React etc.) notice the change.
      const proto = el.tagName === "TEXTAREA" ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
      Object.getOwnPropertyDescriptor(proto, "value").set.call(el, value);
    }
    el.dispatchEvent(new Event("input", { bubbles: true }));
    el.dispatchEvent(new Event("change", { bubbles: true }));
    await settle();
    if (args.submit)
      later(() => {
        const enter = { key: "Enter", code: "Enter", keyCode: 13, bubbles: true, cancelable: true };
        const go = el.dispatchEvent(new KeyboardEvent("keydown", enter));
        el.dispatchEvent(new KeyboardEvent("keyup", enter));
        if (go && el.form) el.form.requestSubmit ? el.form.requestSubmit() : el.form.submit();
      });
    return JSON.stringify({ ...info(), did: `filled ${describe(el, args.ref)}${args.submit ? " and submitted" : ""}` });
  }
}
return JSON.stringify({ error: `unknown op ${op}` });
