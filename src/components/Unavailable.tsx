import { Button } from "react-aria-components";
import { IconDownload, IconPlugConnectedX, IconRefresh } from "@tabler/icons-react";

import type { Snapshot } from "../api";
import { useT } from "../i18n";

const DOWNLOAD_URL = "https://tailscale.com/download";

type Props = {
  snapshot: Snapshot;
  busy: boolean;
  onRetry: () => void;
  onOpenUrl: (url: string) => void;
};

/** In place of the list, while there is no daemon to list anything. */
export default function Unavailable({ snapshot, busy, onRetry, onOpenUrl }: Props) {
  const t = useT();
  const installed = snapshot.connection.state !== "notInstalled";

  return (
    <div className="empty" role="status">
      <IconPlugConnectedX size={30} stroke={1.5} />
      <strong>
        {t(installed ? "unavailable.daemon_down_title" : "unavailable.not_installed_title")}
      </strong>
      <p>{t(installed ? "unavailable.daemon_down_hint" : "unavailable.not_installed_hint")}</p>
      {snapshot.problem && <p className="mono">{snapshot.problem}</p>}
      <div className="banner-actions" style={{ justifyContent: "center" }}>
        {!installed && (
          <Button className="btn btn-primary" onPress={() => onOpenUrl(DOWNLOAD_URL)}>
            <IconDownload size={16} />
            {t("unavailable.download")}
          </Button>
        )}
        <Button className="btn" isDisabled={busy} onPress={onRetry}>
          <IconRefresh size={16} />
          {t("app.retry")}
        </Button>
      </div>
    </div>
  );
}
