// Stands in for `@tauri-apps/api/core` so the real components can be rendered
// in a browser, against fixed data, for screenshots.
import type { Account, LoginStatus, Settings, Snapshot } from "../src/api";

const scene = new URLSearchParams(location.search).get("scene") ?? "window";

const accounts: Account[] = [
  {
    id: "4e04",
    name: "alex",
    loginName: "alex",
    displayName: "Alex Morgan",
    tailnet: "tailnet.example.net",
    controlUrl: "https://headscale.example.net",
    active: true,
  },
  {
    id: "9a1f",
    name: "Work",
    loginName: "alex@example.com",
    displayName: "Alex Morgan",
    tailnet: "example.com",
    controlUrl: "https://controlplane.tailscale.com",
    active: false,
  },
  {
    id: "77aa",
    name: "it",
    loginName: "it",
    displayName: "IT / Infrastructure",
    tailnet: "tailnet.example.net",
    controlUrl: "https://headscale.example.net",
    active: false,
  },
];

function snapshot(): Snapshot {
  const base: Snapshot = {
    connection: {
      state: "running",
      version: "1.102.3",
      hostname: "alex-laptop",
      dnsName: "alex-laptop.ts.example.net",
      ipv4: "100.64.0.2",
      ipv6: "fd7a:115c:a1e0::2",
      tailnet: "tailnet.example.net",
      loginName: "alex",
      exitNode: null,
      keyExpiry: new Date(Date.now() + 23 * 86_400_000).toISOString(),
      authUrl: null,
    },
    accounts,
    canManage: true,
    problem: null,
    canElevate: true,
    loginNeedsRoot: true,
    user: "alex",
  };
  switch (scene) {
    case "limited":
      return { ...base, canManage: false };
    case "stopped":
      return {
        ...base,
        connection: { ...base.connection, state: "stopped", ipv4: null, dnsName: null, keyExpiry: null },
      };
    case "needs-login":
    case "reauth-key":
      return {
        ...base,
        connection: {
          ...base.connection,
          state: "needsLogin",
          ipv4: null,
          dnsName: null,
          keyExpiry: null,
          authUrl: "https://headscale.example.net/register/nodekey:3f9c",
        },
      };
    case "unavailable":
      return {
        ...base,
        accounts: [],
        connection: { ...base.connection, state: "daemonDown", version: null },
        problem: "failed to connect to local tailscaled; it doesn't appear to be running",
      };
    case "empty":
      return {
        ...base,
        accounts: [],
        connection: { ...base.connection, state: "needsLogin", hostname: null, ipv4: null, dnsName: null },
      };
    default:
      return base;
  }
}

const settings: Settings = {
  startHidden: false,
  autostart: true,
  language: null,
  systemLanguage: "it",
  resolvedLanguage: (new URLSearchParams(location.search).get("lang") as "it" | "en") ?? "it",
  version: "0.1.0",
};

const login: LoginStatus =
  scene === "waiting"
    ? {
        running: true,
        withKey: false,
        progress: { url: "https://headscale.example.net/register/nodekey:8b2d41e0c7", approval: false },
      }
    : scene === "waiting-key"
      ? { running: true, withKey: true, progress: null }
      : { running: false, withKey: false, progress: null };

const update =
  (scene === "update" || scene === "update-settings")
    ? {
        current: "0.1.0",
        available: {
          version: "0.2.0",
          notesUrl: "https://github.com/01jam/tailscale-account-switcher/releases/tag/v0.2.0",
          assetName: "tailscale-account-switcher_0.2.0_amd64.deb",
          assetUrl: "https://example.invalid/tailscale-account-switcher_0.2.0_amd64.deb",
          assetSize: 2581408,
          installs: true,
        },
      }
    : { current: "0.1.0", available: null };

const answers: Record<string, unknown> = {
  update_status: update,
  get_state: snapshot(),
  get_settings: settings,
  pending_notices: [],
  login_status: login,
  take_add_request: scene.startsWith("add"),
  known_servers:
    scene === "add-first"
      ? []
      : ["https://headscale.example.net", "https://vpn.example.org", "http://10.0.0.2:8080"],
};

export function invoke<T>(cmd: string): Promise<T> {
  return Promise.resolve((answers[cmd] ?? null) as T);
}
