import type { Account, Snapshot } from "./api";
import type { Translate } from "./i18n";

/** Initials for the avatar, taken from the name the card leads with. An
 *  acronym stands on its own ("IT / Infrastructure" is IT, not II); otherwise
 *  the first letters of the first two words, or of the only one. */
export function initials(account: Account): string {
  const words = title(account).split(/[\s@._\-/()]+/).filter(Boolean);
  if (words.length === 0) return "?";
  const [first, second] = words;
  if (/^[A-Z0-9]{2,3}$/.test(first)) return first;
  const letters = second ? [...first][0] + [...second][0] : [...first][0];
  return letters.toUpperCase();
}

/** The name the card leads with: a nickname if there is one, the person's
 *  display name when it says more than the login, the login otherwise. */
export function title(account: Account): string {
  const nicknamed = account.loginName !== null && account.name !== account.loginName;
  if (nicknamed) return account.name;
  return account.displayName ?? account.name;
}

/** The host of a control server, or null for Tailscale's own. */
export function serverHost(url: string | null): string | null {
  if (!url) return null;
  try {
    const host = new URL(url).host;
    return host === "controlplane.tailscale.com" || host === "login.tailscale.com"
      ? null
      : host;
  } catch {
    return url;
  }
}

/** Login, tailnet and server, each only when it adds something the ones
 *  before it did not already say. */
export function accountMeta(account: Account): string {
  const shown = new Set([title(account)]);
  const parts: string[] = [];
  const push = (value: string | null) => {
    if (value && !shown.has(value)) {
      parts.push(value);
      shown.add(value);
    }
  };
  push(account.loginName);
  push(account.tailnet);
  push(serverHost(account.controlUrl));
  return parts.join(" · ");
}

/** How long the node key has left, in the largest unit that is still honest. */
export function keyExpiry(t: Translate, iso: string | null): string | null {
  if (!iso) return null;
  const ms = Date.parse(iso) - Date.now();
  if (Number.isNaN(ms)) return null;
  if (ms <= 0) return t("accounts.key_expired");
  const hours = Math.floor(ms / 3_600_000);
  if (hours < 48) return t("accounts.key_expires_hours", { hours: Math.max(hours, 1) });
  return t("accounts.key_expires_days", { days: Math.floor(hours / 24) });
}

/** Whether the daemon can be talked to at all. */
export function reachable(snapshot: Snapshot | null): boolean {
  return (
    snapshot !== null &&
    snapshot.connection.state !== "notInstalled" &&
    snapshot.connection.state !== "daemonDown"
  );
}

/** Copy to the clipboard, falling back to the old selection trick where the
 *  webview has not granted the async API. */
export async function copyText(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    const area = document.createElement("textarea");
    area.value = text;
    area.setAttribute("readonly", "");
    area.style.position = "fixed";
    area.style.opacity = "0";
    document.body.appendChild(area);
    area.select();
    const ok = document.execCommand("copy");
    area.remove();
    return ok;
  }
}
