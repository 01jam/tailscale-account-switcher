import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { Button } from "react-aria-components";
import { IconPlus, IconRefresh, IconSettings } from "@tabler/icons-react";

import {
  api,
  isFailure,
  type Account,
  type LoginFinished,
  type LoginProgress,
  type Notice,
  type NoticeLevel,
  type Retry,
  type Settings,
  type Snapshot,
  type ToggleKey,
  type UpdateAvailable,
  type UpdateStatus,
} from "./api";
import { reachable, title } from "./format";
import { LangProvider, resolveLang, useT, type Lang, type Translate } from "./i18n";
import { useWindowDrag } from "./useWindowDrag";

import { AccessBanner, GrantDialog } from "./components/Access";
import AccountList from "./components/AccountList";
import AddAccountDialog, {
  EMPTY_DRAFT,
  reauthDraft,
  type AddDraft,
  type AddPhase,
} from "./components/AddAccountDialog";
import ConnectionBar from "./components/ConnectionBar";
import ResizeGrip from "./components/ResizeGrip";
import SettingsDialog from "./components/SettingsDialog";
import Toasts, { type Toast } from "./components/Toasts";
import Unavailable from "./components/Unavailable";
import WindowControls from "./components/WindowControls";

/** How many toasts may stand at once. Past this the window is a noticeboard. */
const MAX_TOASTS = 3;

const SETTERS: Record<ToggleKey, (enabled: boolean) => Promise<void>> = {
  autostart: api.setAutostart,
  startHidden: api.setStartHidden,
};

/** What is running, so the right control can show it and the rest wait. */
type Busy = { kind: "switch"; id: string } | { kind: "connection" } | { kind: "grant" } | { kind: "other" };

export default function App() {
  // The backend is the authority on the language, but it takes a round-trip to
  // answer; starting from the browser's own locale keeps the first paint from
  // being in the wrong one.
  const [lang, setLang] = useState<Lang>(() => resolveLang(navigator.language));

  return (
    <LangProvider value={lang}>
      <Switcher onLanguage={setLang} />
    </LangProvider>
  );
}

function Switcher({ onLanguage }: { onLanguage: (lang: Lang) => void }) {
  const t = useT();
  const drag = useWindowDrag();
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null);
  const [settings, setSettings] = useState<Settings | null>(null);
  const [toasts, setToasts] = useState<Toast[]>([]);
  const [busy, setBusy] = useState<Busy | null>(null);
  const [showSettings, setShowSettings] = useState(false);
  const [adding, setAdding] = useState(false);
  const [add, setAdd] = useState<AddPhase>({ phase: "form" });
  const [draft, setDraft] = useState<AddDraft>(EMPTY_DRAFT);
  // Read by the `login-finished` listener, which outlives any one render.
  const reauthing = useRef(false);
  const [servers, setServers] = useState<string[]>([]);
  const [update, setUpdate] = useState<UpdateStatus | null>(null);
  // The change the daemon refused, to run again once access is granted.
  const [grant, setGrant] = useState<{ retry: (() => Promise<unknown>) | null } | null>(null);

  const nextToastId = useRef(0);

  const say = useCallback((text: string, level: NoticeLevel = "info") => {
    setToasts((current) =>
      [...current, { id: nextToastId.current++, text, level }].slice(-MAX_TOASTS),
    );
  }, []);

  const dismiss = useCallback((id: number) => {
    setToasts((current) => current.filter((toast) => toast.id !== id));
  }, []);

  const reload = useCallback(async () => {
    try {
      const [state, config] = await Promise.all([api.state(), api.settings()]);
      setSnapshot(state);
      setSettings(config);
      onLanguage(config.resolvedLanguage);
    } catch (e) {
      say(isFailure(e) ? e.message : String(e), "error");
    }
  }, [onLanguage, say]);

  /** Open the add dialog, blank or preset — a key login for an account that
   *  needs logging in again. */
  const openAdd = useCallback((preset?: AddDraft) => {
    const next = preset ?? EMPTY_DRAFT;
    reauthing.current = next.reauth;
    setDraft(next);
    setAdd({ phase: "form" });
    api.knownServers().then(setServers).catch(() => {});
    setAdding(true);
  }, []);

  /** Close the dialog and forget what was typed — a pasted key included. */
  const closeAdd = useCallback(() => {
    setAdding(false);
    setAdd({ phase: "form" });
    setDraft(EMPTY_DRAFT);
  }, []);

  /** Run a change: busy while it runs, its failure said where it belongs. A
   *  refusal the user can fix opens the way to fix it, with the change queued
   *  behind; a dismissed password prompt is an answer and says nothing. */
  const run = useCallback(
    async (kind: Busy, change: () => Promise<unknown>, done?: string) => {
      setBusy(kind);
      try {
        await change();
        if (done) say(done);
      } catch (e) {
        if (!isFailure(e)) say(String(e), "error");
        else if (e.accessDenied) setGrant({ retry: change });
        else if (!e.cancelled) say(e.message, "error");
      } finally {
        setBusy(null);
      }
    },
    [say],
  );

  // What was raised while there was nowhere to show it, a login already
  // waiting, and an "Add account…" picked from the tray.
  const catchUp = useCallback(() => {
    api
      .pendingNotices()
      .then((held) => held.forEach((n) => say(n.text, n.level)))
      .catch(() => {});
    api
      .loginStatus()
      .then((status) => {
        if (!status.running) return;
        const withKey = status.withKey;
        setAdd(
          status.progress
            ? { phase: "waiting", withKey, progress: status.progress }
            : { phase: "starting", withKey },
        );
        setAdding(true);
      })
      .catch(() => {});
    api
      .takeAddRequest()
      .then((requested) => requested && openAdd())
      .catch(() => {});
  }, [say, openAdd]);

  useEffect(() => {
    reload();
    catchUp();
    // The check runs on the Rust side from launch, so by the time a window
    // opens the answer is usually already there. Merged rather than assigned:
    // an event that beat this round-trip home must not be overwritten by the
    // empty answer it raced.
    api
      .updateStatus()
      .then((status) =>
        setUpdate((prev) => (prev?.available ? { ...status, available: prev.available } : status)),
      )
      .catch(() => {});

    const unlisteners = [
      listen<UpdateAvailable>("update-available", (e) => {
        setUpdate((prev) => ({ current: prev?.current ?? "", available: e.payload }));
      }),
      listen<Snapshot>("state-changed", (e) => setSnapshot(e.payload)),
      listen<Notice>("notice", (e) => say(e.payload.text, e.payload.level)),
      listen<LoginProgress>("login-progress", (e) => {
        setAdd((current) => ({
          phase: "waiting",
          withKey: current.phase === "starting" || current.phase === "waiting"
            ? current.withKey
            : false,
          progress: e.payload,
        }));
      }),
      listen<LoginFinished>("login-finished", (e) => {
        const { ok, cancelled, error } = e.payload;
        if (ok || cancelled) {
          const done = reauthing.current ? "add.reauth_done" : "add.done";
          closeAdd();
          say(t(ok ? done : "add.cancelled"));
        } else {
          setAdd({ phase: "failed", error: error ?? t("add.failed_title") });
        }
      }),
      listen("open-add-account", () => {
        api
          .takeAddRequest()
          .then((requested) => requested && openAdd())
          .catch(() => {});
      }),
      // A tray action the daemon refused: the window came up to offer access.
      listen<Retry>("access-needed", (e) => {
        setGrant({ retry: retryFor(e.payload) });
      }),
    ];

    // A push that landed while the window was hidden is not replayed, so the
    // state is read again on the way back in. `focus` alone misses being shown
    // from the tray, which on Linux does not reliably reach the page as one.
    const onWake = () => {
      if (document.visibilityState !== "visible") return;
      reload();
      catchUp();
    };
    window.addEventListener("focus", onWake);
    document.addEventListener("visibilitychange", onWake);

    return () => {
      unlisteners.forEach((unlisten) => unlisten.then((f) => f()));
      window.removeEventListener("focus", onWake);
      document.removeEventListener("visibilitychange", onWake);
    };
  }, [reload, catchUp, say, t, openAdd, closeAdd]);

  const openUrl = (url: string) => {
    api.openUrl(url).catch((e) => say(isFailure(e) ? e.message : String(e), "error"));
  };
  const copied = () => say(t("app.copied"));

  const active = snapshot?.accounts.find((a) => a.active) ?? null;
  const isBusy = busy !== null;

  return (
    <main className="app">
      <div className="titlebar" {...drag}>
        <Button
          className={`win-control${update?.available ? " win-control-dot" : ""}`}
          aria-label={
            update?.available ? `${t("app.settings")} — ${t("update.badge")}` : t("app.settings")
          }
          onPress={() => setShowSettings(true)}
        >
          <IconSettings size={16} />
        </Button>
        <WindowControls />
      </div>

      <ResizeGrip />

      <header className="app-header">
        <div style={{ minWidth: 0 }}>
          <h1>{t("app.title")}</h1>
          <p className="subtitle">{statusLine(t, snapshot, active)}</p>
        </div>
        <Button
          className="btn btn-icon"
          aria-label={t("app.refresh")}
          isDisabled={isBusy}
          onPress={() => run({ kind: "other" }, reload)}
        >
          <IconRefresh size={18} />
        </Button>
      </header>

      {snapshot && reachable(snapshot) && (
        <>
          <ConnectionBar
            snapshot={snapshot}
            busy={isBusy}
            onConnect={() => run({ kind: "connection" }, api.connect)}
            onDisconnect={() => run({ kind: "connection" }, api.disconnect)}
            onLogIn={() => run({ kind: "connection" }, api.reauthenticate)}
            onLogInWithKey={() => active && openAdd(reauthDraft(active))}
            onOpenUrl={openUrl}
          />

          {!snapshot.canManage && (
            <AccessBanner
              snapshot={snapshot}
              busy={isBusy}
              onGrant={() =>
                run({ kind: "grant" }, api.grantAccess, t("access.granted"))
              }
              onCopied={copied}
            />
          )}

          <div className="section-label">
            <span>{t("accounts.list_label")}</span>
            <span>{snapshot.accounts.length}</span>
          </div>
        </>
      )}

      <div className="list-wrap">
        {snapshot && !reachable(snapshot) ? (
          <Unavailable
            snapshot={snapshot}
            busy={isBusy}
            onRetry={() => run({ kind: "other" }, reload)}
            onOpenUrl={openUrl}
          />
        ) : (
          snapshot && (
            <AccountList
              accounts={snapshot.accounts}
              connection={snapshot.connection}
              busy={isBusy}
              switchingTo={busy?.kind === "switch" ? busy.id : null}
              onSwitch={(account: Account) =>
                run(
                  { kind: "switch", id: account.id },
                  () => api.switchAccount(account.id),
                  t("accounts.switched", { name: title(account) }),
                )
              }
              onRename={(name) =>
                run({ kind: "other" }, () => api.renameAccount(name), t("accounts.renamed"))
              }
              onLogout={() =>
                run({ kind: "other" }, api.logout, t("accounts.logged_out"))
              }
              onRemove={(account) =>
                run(
                  { kind: "other" },
                  () => api.removeAccount(account.id),
                  t("accounts.removed", { name: title(account) }),
                )
              }
              onReorder={(ids) => run({ kind: "other" }, () => api.reorderAccounts(ids))}
              onCopied={copied}
            />
          )
        )}
      </div>

      <footer className="app-footer">
        <Button
          className="btn btn-primary"
          isDisabled={isBusy || !snapshot || !reachable(snapshot)}
          onPress={() => openAdd()}
        >
          <IconPlus size={16} />
          {t("add.button")}
        </Button>
        {snapshot?.connection.version && (
          <span className="footer-note">Tailscale {snapshot.connection.version}</span>
        )}
      </footer>

      <Toasts toasts={toasts} onDismiss={dismiss} />

      {snapshot && (
        <AddAccountDialog
          isOpen={adding}
          state={add}
          servers={servers}
          needsRoot={snapshot.loginNeedsRoot}
          onStart={(server, nickname, authKey) => {
            setAdd({ phase: "starting", withKey: authKey !== null });
            api.addAccount(server, nickname, authKey).catch((e) => {
              setAdd({ phase: "failed", error: isFailure(e) ? e.message : String(e) });
            });
          }}
          onCancel={() => {
            api.cancelAddAccount().catch(() => {});
          }}
          draft={draft}
          onDraft={(change) => setDraft((current) => ({ ...current, ...change }))}
          onClose={closeAdd}
          onBack={() => setAdd({ phase: "form" })}
          onOpenUrl={openUrl}
          onCopied={copied}
        />
      )}

      {snapshot && (
        <GrantDialog
          isOpen={grant !== null}
          snapshot={snapshot}
          busy={busy?.kind === "grant"}
          onClose={() => setGrant(null)}
          onCopied={copied}
          onGrant={() => {
            const retry = grant?.retry ?? null;
            run({ kind: "grant" }, async () => {
              await api.grantAccess();
              setGrant(null);
              say(t("access.granted"));
              if (retry) await retry();
            });
          }}
        />
      )}

      <SettingsDialog
        isOpen={showSettings}
        settings={settings}
        busy={isBusy}
        onClose={() => setShowSettings(false)}
        update={update}
        onInstallUpdate={() =>
          run({ kind: "other" }, async () => {
            const outcome = await api.installUpdate();
            // The browser opening on the release page is its own report; the
            // rest earn a line, cancelling included — a button that answers
            // silence reads as a broken one.
            if (outcome.kind === "installed") {
              say(t("update.installed", { version: outcome.version ?? "" }));
            } else if (outcome.kind === "cancelled") {
              say(t("update.cancelled"));
            } else if (outcome.kind === "downloaded") {
              say(t("update.done", { name: outcome.name ?? "" }));
            }
          })
        }
        onChange={(key, value) => {
          // Optimistic: the toggle should not lag behind the pointer.
          setSettings((s) => (s ? { ...s, [key]: value } : s));
          run({ kind: "other" }, () => SETTERS[key](value));
        }}
        onLanguageChange={(tag) => {
          setSettings((s) =>
            s ? { ...s, language: tag, resolvedLanguage: tag ?? s.systemLanguage } : s,
          );
          onLanguage(tag ?? settings?.systemLanguage ?? "en");
          run({ kind: "other" }, () => api.setLanguage(tag));
        }}
      />
    </main>
  );
}

function retryFor(retry: Retry): () => Promise<unknown> {
  switch (retry.kind) {
    case "switch":
      return () => api.switchAccount(retry.id);
    case "connect":
      return api.connect;
    case "disconnect":
      return api.disconnect;
    case "logIn":
      return api.reauthenticate;
  }
}

function statusLine(t: Translate, snapshot: Snapshot | null, active: Account | null): string {
  if (!snapshot) return t("status.loading");
  if (!reachable(snapshot)) return t("status.unavailable");
  if (!active) return t("status.none");
  return [title(active), active.tailnet].filter(Boolean).join(" · ");
}
