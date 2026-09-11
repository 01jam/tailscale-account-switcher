import {
  Button,
  Dialog,
  Heading,
  Modal,
  ModalOverlay,
} from "react-aria-components";
import { IconCopy, IconLock, IconShieldCheck } from "@tabler/icons-react";

import type { Snapshot } from "../api";
import { copyText } from "../format";
import { useT, type Translate } from "../i18n";

type Props = {
  snapshot: Snapshot;
  busy: boolean;
  onGrant: () => void;
  onCopied: () => void;
};

/** The command that grants access by hand, for a desktop with no password
 *  prompt to go through — or a user who would rather see what runs. */
function operatorCommand(snapshot: Snapshot): string {
  return `sudo tailscale set --operator=${snapshot.user ?? "$USER"}`;
}

function explanation(t: Translate, snapshot: Snapshot): string {
  const active = snapshot.accounts.find((a) => a.active);
  const user = snapshot.user ?? t("access.this_user");
  return active
    ? t("access.body", { user, name: active.name })
    : t("access.body_generic", { user });
}

/**
 * Standing in the window while the daemon refuses this user the account in
 * front. It stays until the fix is in: a toast would be gone before anyone
 * found out why the switch buttons do nothing.
 */
export function AccessBanner({ snapshot, busy, onGrant, onCopied }: Props) {
  const t = useT();
  return (
    <section className="banner" role="status">
      <div className="banner-head">
        <IconLock size={18} />
        <div>
          <strong>{t("access.title")}</strong>
          <p>{explanation(t, snapshot)}</p>
        </div>
      </div>
      <GrantActions snapshot={snapshot} busy={busy} onGrant={onGrant} onCopied={onCopied} />
    </section>
  );
}

function GrantActions({ snapshot, busy, onGrant, onCopied }: Props) {
  const t = useT();
  const command = operatorCommand(snapshot);

  return (
    <div className="banner-actions">
      {snapshot.canElevate ? (
        <Button className="btn btn-primary btn-sm" isDisabled={busy} onPress={onGrant}>
          <IconShieldCheck size={15} />
          {t("access.grant")}
        </Button>
      ) : (
        <span className="muted">{t("access.manual")}</span>
      )}
      <div className="command command-row">
        <code>{command}</code>
        <Button
          className="btn btn-icon btn-sm"
          aria-label={t("app.copy")}
          onPress={() => copyText(command).then((ok) => ok && onCopied())}
        >
          <IconCopy size={15} />
        </Button>
      </div>
    </div>
  );
}

/**
 * The same offer, raised by a change the daemon just refused — with the change
 * retried as soon as access is in.
 */
export function GrantDialog({
  isOpen,
  onClose,
  ...props
}: Props & { isOpen: boolean; onClose: () => void }) {
  const t = useT();
  return (
    <ModalOverlay
      className="overlay"
      isOpen={isOpen}
      onOpenChange={(open) => !open && onClose()}
      isDismissable={!props.busy}
    >
      <Modal className="modal">
        <Dialog className="dialog" role="alertdialog">
          <Heading slot="title">{t("access.prompt_title")}</Heading>
          <p className="dialog-text">
            {explanation(t, props.snapshot)} {t("access.hint")}
          </p>
          <GrantActions {...props} />
          <div className="dialog-actions">
            <Button className="btn" isDisabled={props.busy} onPress={onClose}>
              {t("app.cancel")}
            </Button>
          </div>
        </Dialog>
      </Modal>
    </ModalOverlay>
  );
}
