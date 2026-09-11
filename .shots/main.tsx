import React from "react";
import ReactDOM from "react-dom/client";
import "@fontsource-variable/inter";
import App from "../src/App";
import "../src/styles.css";

const params = new URLSearchParams(location.search);
const scene = params.get("scene") ?? "window";

// The stylesheet carries its dark palette behind a media query, and headless
// Chrome decides for itself which way that query answers. So the query is
// rewritten to match, or never to — the rules that apply are exactly the ones
// the app ships.
function forceTheme(dark: boolean) {
  document.documentElement.style.colorScheme = dark ? "dark" : "light";
  for (const sheet of Array.from(document.styleSheets)) {
    let rules: CSSRuleList;
    try {
      rules = sheet.cssRules;
    } catch {
      continue;
    }
    for (const rule of Array.from(rules)) {
      if (
        rule instanceof CSSMediaRule &&
        rule.conditionText.includes("prefers-color-scheme: dark")
      ) {
        rule.media.mediaText = dark ? "all" : "not all";
      }
    }
  }
}

// Before the first render, and with transitions off: a screenshot is taken in
// virtual time, where a transition started by the theme switch never finishes
// and leaves whatever it touched halfway between the two palettes.
forceTheme(params.get("theme") === "dark");
const still = document.createElement("style");
still.textContent =
  "*, *::before, *::after { transition: none !important; animation: none !important; }";
document.head.appendChild(still);

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);

function press(label: RegExp) {
  const button = Array.from(document.querySelectorAll("button")).find((b) =>
    label.test(b.getAttribute("aria-label") ?? b.textContent ?? ""),
  );
  button?.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, button: 0 }));
  button?.dispatchEvent(new PointerEvent("pointerup", { bubbles: true, button: 0 }));
  button?.click();
}

/** A React Aria radio is a label around a hidden input: clicking the label is
 *  what a user does. */
function choose(label: RegExp) {
  const target = Array.from(document.querySelectorAll("label")).find((l) =>
    label.test(l.textContent ?? ""),
  );
  target?.click();
}

setTimeout(() => {
  if (scene === "settings") press(/^(Impostazioni|Settings)$/);
  if (scene === "menu") press(/^(Azioni per|Actions for) Alex/);
  if (["add-key", "add-other", "add-first", "add-open"].includes(scene)) {
    choose(/^(Altro server|Another server)/);
  }
  if (scene === "add-open") {
    setTimeout(() => {
      const trigger = document.querySelector<HTMLElement>(".dialog .select-trigger");
      trigger?.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, button: 0 }));
      trigger?.dispatchEvent(new PointerEvent("pointerup", { bubbles: true, button: 0 }));
      trigger?.click();
    }, 150);
  }
  if (scene === "add-key") choose(/^(Chiave pre-auth|Pre-auth key)$/);
  if (scene === "reauth-key") press(/^(Usa una chiave|Use a key)$/);
  setTimeout(() => {
    if (scene !== "menu" && scene !== "add-open") (document.activeElement as HTMLElement | null)?.blur();
    document.body.dataset.ready = "1";
  }, 400);
}, 500);

