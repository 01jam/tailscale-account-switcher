import { Button, Switch } from "react-aria-components";
import { IconExternalLink, IconKey, IconLogin2 } from "@tabler/icons-react";

import type { Snapshot } from "../api";
import { useT } from "../i18n";

type Props = {
  snapshot: Snapshot;
  busy: boolean;
  onConnect: () => void;
  onDisconnect: () => void;
  onLogIn: () => void;
  onLogInWithKey: () => void;
  onOpenUrl: (url: string) => void;
};

/**
 * This device, not an account: whether it is on the tailnet right now, and the
 * one control that changes that.
 *
 * A switch while the account is logged in, because connected is a state you
 * flip. A button when it needs logging in, because that is a thing you go and
 * do — in the browser.
 */
export default function ConnectionBar({
  snapshot,
  busy,
  onConnect,
  onDisconnect,
  onLogIn,
  onLogInWithKey,
  onOpenUrl,
}: Props) {
  const t = useT();
  const { connection } = snapshot;
  const hasAccount = snapshot.accounts.some((a) => a.active);
  const device = connection.hostname
    ? t("connection.device", { name: connection.hostname })
    : null;

  if (!hasAccount) {
    return (
      <section className="connection" aria-live="polite">
        <span className="dot dot-off" aria-hidden />
        <div className="connection-body">
          <span className="connection-title">{t("connection.no_account")}</span>
          <span className="connection-hint">{t("connection.no_account_hint")}</span>
        </div>
      </section>
    );
  }

  switch (connection.state) {
    // Two ways back in, as there are two ways in: the browser, or a pre-auth
    // key for a server where the browser login ends at an admin's desk.
    case "needsLogin":
      return (
        <section className="connection connection-wrap" aria-live="polite">
          <span className="dot dot-alert" aria-hidden />
          <div className="connection-body">
            <span className="connection-title">{t("connection.needs_login")}</span>
            <span className="connection-hint">{t("connection.needs_login_hint")}</span>
          </div>
          <div className="connection-actions">
            <Button className="btn btn-primary btn-sm" isDisabled={busy} onPress={onLogIn}>
              <IconLogin2 size={15} />
              {t("connection.log_in")}
            </Button>
            <Button className="btn btn-sm" isDisabled={busy} onPress={onLogInWithKey}>
              <IconKey size={15} />
              {t("connection.use_key")}
            </Button>
            {connection.authUrl && (
              <Button
                className="btn btn-icon btn-sm"
                aria-label={t("connection.open_login")}
                onPress={() => onOpenUrl(connection.authUrl!)}
              >
                <IconExternalLink size={16} />
              </Button>
            )}
          </div>
        </section>
      );

    case "needsMachineAuth":
      return (
        <section className="connection" aria-live="polite">
          <span className="dot dot-wait" aria-hidden />
          <div className="connection-body">
            <span className="connection-title">{t("connection.needs_machine_auth")}</span>
            <span className="connection-hint">{t("connection.needs_machine_auth_hint")}</span>
          </div>
        </section>
      );

    default: {
      const on = connection.state === "running" || connection.state === "starting";
      const settling = connection.state === "starting" || connection.state === "noState";
      const label = settling
        ? t("connection.starting")
        : on
          ? t("connection.running")
          : t("connection.stopped");
      return (
        <section className="connection" aria-live="polite">
          <span
            className={`dot ${settling ? "dot-wait" : on ? "dot-on" : "dot-off"}`}
            aria-hidden
          />
          <div className="connection-body">
            <span className="connection-title">{label}</span>
            {device && <span className="connection-meta">{device}</span>}
          </div>
          <Switch
            className="switch"
            aria-label={t("connection.toggle")}
            isSelected={on}
            isDisabled={busy}
            onChange={(value) => (value ? onConnect() : onDisconnect())}
          >
            <span className="switch-indicator" />
          </Switch>
        </section>
      );
    }
  }
}
