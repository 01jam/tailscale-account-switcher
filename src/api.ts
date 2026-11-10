import { invoke } from "@tauri-apps/api/core";

import type { Lang } from "./i18n";

/** Where the daemon is in its life, plus the two states before there is a
 *  daemon to ask. */
export type DaemonState =
  | "notInstalled"
  | "daemonDown"
  | "noState"
  | "needsLogin"
  | "needsMachineAuth"
  | "stopped"
  | "starting"
  | "running";

/** One login profile on this machine. */
export type Account = {
  id: string;
  /** The nickname when one is set, the login name otherwise. */
  name: string;
  loginName: string | null;
  displayName: string | null;
  tailnet: string | null;
  controlUrl: string | null;
  active: boolean;
};

/** This device, as the account in front sees it. */
export type Connection = {
  state: DaemonState;
  version: string | null;
  hostname: string | null;
  dnsName: string | null;
  ipv4: string | null;
  ipv6: string | null;
  tailnet: string | null;
  loginName: string | null;
  exitNode: string | null;
  /** RFC 3339, when the tailnet expires node keys at all. */
  keyExpiry: string | null;
  /** Set while a login is waiting for someone to open it. */
  authUrl: string | null;
};

export type Snapshot = {
  connection: Connection;
  accounts: Account[];
  /** False when the daemon refuses this user changes on the account in front. */
  canManage: boolean;
  problem: string | null;
  /** Access can be granted from here, through the system's password prompt. */
  canElevate: boolean;
  /** Adding an account will bring up that prompt. */
  loginNeedsRoot: boolean;
  user: string | null;
};

/** A change that did not go through. Commands reject with this object. */
export type Failure = {
  message: string;
  /** Granting access to the account in front is the fix. */
  accessDenied: boolean;
  /** The password prompt was dismissed: an answer, not an error. */
  cancelled: boolean;
};

export type LoginProgress = {
  url: string;
  /** The link is an admin's device approval rather than a login. */
  approval: boolean;
};

export type LoginFinished = {
  ok: boolean;
  cancelled: boolean;
  error: string | null;
};

export type LoginStatus = {
  running: boolean;
  /** The login uses a pre-auth key: there is no link to wait for. */
  withKey: boolean;
  progress: LoginProgress | null;
};

/** A tray action refused for lack of access, for the window to retry once
 *  access is granted. */
export type Retry =
  | { kind: "switch"; id: string }
  | { kind: "connect" }
  | { kind: "disconnect" }
  | { kind: "logIn" };

/** A release newer than the running app, already narrowed to the one file this
 *  machine can use. */
export type UpdateAvailable = {
  version: string;
  notesUrl: string;
  /** `null` when the release carries nothing in this install's format: there is
   *  an update, but fetching it is a manual job. */
  assetName: string | null;
  assetUrl: string | null;
  assetSize: number | null;
  /** Whether this machine installs the package itself, or can only put the file
   *  somewhere and step back — a `.dmg` or an AppImage. */
  installs: boolean;
};

/** What pressing the button came to. A dismissed password dialog is not an
 *  error: the user answered, and the answer was no. */
export type InstallOutcome = {
  kind: "installed" | "cancelled" | "downloaded" | "openedPage";
  name: string | null;
  version: string | null;
};

export type UpdateStatus = {
  /** The version running right now, straight from the binary. */
  current: string;
  available: UpdateAvailable | null;
};

/** How loudly the window draws a notice. */
export type NoticeLevel = "info" | "error";

/** Something the app has to say that nobody asked for. Written on the Rust
 *  side, because when the window is down it goes to the desktop instead. */
export type Notice = {
  text: string;
  level: NoticeLevel;
};

export type Settings = {
  startHidden: boolean;
  autostart: boolean;
  /** The saved override; `null` means "follow the system". */
  language: Lang | null;
  systemLanguage: Lang;
  resolvedLanguage: Lang;
};

/** The settings that are plain on/off switches. */
export type ToggleKey = "autostart" | "startHidden";

export function isFailure(value: unknown): value is Failure {
  return (
    typeof value === "object" &&
    value !== null &&
    "message" in value &&
    "accessDenied" in value
  );
}

export const api = {
  state: () => invoke<Snapshot>("get_state"),
  switchAccount: (id: string) => invoke<void>("switch_account", { id }),
  removeAccount: (id: string) => invoke<void>("remove_account", { id }),
  logout: () => invoke<void>("logout"),
  connect: () => invoke<void>("connect"),
  disconnect: () => invoke<void>("disconnect"),
  renameAccount: (name: string) => invoke<void>("rename_account", { name }),
  /** Starts a login on the account in front and opens its link; resolves with
   *  the link so the window can offer it again. */
  reauthenticate: () => invoke<string | null>("reauthenticate"),
  /** Makes this user the operator of the account in front, through the
   *  system's password prompt. */
  grantAccess: () => invoke<void>("grant_access"),

  /** `authKey` authenticates with a pre-auth key instead of a browser login. */
  addAccount: (server: string | null, nickname: string | null, authKey: string | null) =>
    invoke<void>("add_account", { server, nickname, authKey }),
  cancelAddAccount: () => invoke<void>("cancel_add_account"),
  loginStatus: () => invoke<LoginStatus>("login_status"),
  /** True once after "Add account…" was picked from the tray. */
  takeAddRequest: () => invoke<boolean>("take_add_request"),
  knownServers: () => invoke<string[]>("known_servers"),

  openUrl: (url: string) => invoke<void>("open_url", { url }),
  reorderAccounts: (ids: string[]) => invoke<void>("reorder_accounts", { ids }),

  settings: () => invoke<Settings>("get_settings"),
  setStartHidden: (enabled: boolean) =>
    invoke<void>("set_start_hidden", { enabled }),
  setAutostart: (enabled: boolean) => invoke<void>("set_autostart", { enabled }),
  /** `null` hands the choice back to the system locale. */
  setLanguage: (tag: Lang | null) => invoke<void>("set_language", { tag }),
  pendingNotices: () => invoke<Notice[]>("pending_notices"),

  updateStatus: () => invoke<UpdateStatus>("update_status"),
  /** Asks GitHub now instead of waiting for the next scheduled check. */
  checkUpdate: () => invoke<UpdateStatus>("check_update"),
  /** Downloads the package and, where the machine has a package manager for
   *  it, installs it — the password prompt in the middle is polkit's own. */
  installUpdate: () => invoke<InstallOutcome>("install_update"),
  openReleaseNotes: () => invoke<void>("open_release_notes"),
};
