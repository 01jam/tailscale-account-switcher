import { useState, type ReactNode } from "react";
import {
  Button,
  Dialog,
  GridList,
  GridListItem,
  Heading,
  Input,
  Label,
  Menu,
  MenuItem,
  MenuTrigger,
  Modal,
  ModalOverlay,
  Popover,
  Separator,
  Text,
  TextField,
  useDragAndDrop,
} from "react-aria-components";
import {
  IconCopy,
  IconDotsVertical,
  IconGripVertical,
  IconLoader2,
  IconLogout,
  IconPencil,
  IconTrash,
  IconUsersGroup,
} from "@tabler/icons-react";

import type { Account, Connection } from "../api";
import { accountMeta, copyText, initials, keyExpiry, title } from "../format";
import { useT } from "../i18n";

type Props = {
  accounts: Account[];
  connection: Connection;
  /** Nothing can be pressed while a change is running. */
  busy: boolean;
  /** The account a switch is on its way to, for the spinner on its button. */
  switchingTo: string | null;
  onSwitch: (account: Account) => void;
  onRename: (name: string) => void;
  onLogout: (account: Account) => void;
  onRemove: (account: Account) => void;
  onReorder: (ids: string[]) => void;
  onCopied: () => void;
};

type Confirm = { kind: "logout" | "remove"; account: Account };

export default function AccountList({
  accounts,
  connection,
  busy,
  switchingTo,
  onSwitch,
  onRename,
  onLogout,
  onRemove,
  onReorder,
  onCopied,
}: Props) {
  const t = useT();
  const [renaming, setRenaming] = useState<Account | null>(null);
  const [confirming, setConfirming] = useState<Confirm | null>(null);

  const { dragAndDropHooks } = useDragAndDrop({
    getItems: (keys) => [...keys].map((key) => ({ "text/plain": String(key) })),
    onReorder(e) {
      const ids = accounts.map((a) => a.id);
      const moving = [...e.keys].map(String);
      const rest = ids.filter((id) => !moving.includes(id));
      const at = rest.indexOf(String(e.target.key));
      if (at < 0) return;
      const cut = e.target.dropPosition === "before" ? at : at + 1;
      onReorder([...rest.slice(0, cut), ...moving, ...rest.slice(cut)]);
    },
  });

  const copy = (text: string) => copyText(text).then((ok) => ok && onCopied());

  if (accounts.length === 0) {
    return (
      <div className="empty">
        <IconUsersGroup size={30} stroke={1.5} />
        <strong>{t("accounts.empty_title")}</strong>
        <p>{t("accounts.empty_hint")}</p>
      </div>
    );
  }

  return (
    <>
      <GridList
        className="accounts"
        aria-label={t("accounts.list_label")}
        selectionMode="none"
        items={accounts}
        dragAndDropHooks={dragAndDropHooks}
      >
        {(account) => {
          const name = title(account);
          const meta = accountMeta(account);
          const switching = switchingTo === account.id;

          return (
            <GridListItem
              className={`account${account.active ? " account-active" : ""}`}
              id={account.id}
              textValue={name}
            >
              <div className="account-row">
                <Button
                  slot="drag"
                  className="account-grip"
                  aria-label={t("accounts.reorder", { name })}
                >
                  <IconGripVertical size={16} />
                </Button>

                <span className="avatar" aria-hidden>
                  {initials(account)}
                </span>

                <span className="account-body">
                  <span className="account-title">
                    <span className="account-name">{name}</span>
                    {account.active && <span className="pill">{t("accounts.in_use")}</span>}
                  </span>
                  {meta && <span className="account-meta">{meta}</span>}
                </span>

                <span className="account-actions">
                  {!account.active && (
                    <Button
                      className="btn btn-sm"
                      aria-label={t("accounts.switch_to_label", { name })}
                      isDisabled={busy}
                      onPress={() => onSwitch(account)}
                    >
                      {switching && <IconLoader2 size={14} className="spin" />}
                      {switching ? t("accounts.switching") : t("accounts.switch_to")}
                    </Button>
                  )}

                  <MenuTrigger>
                    <Button
                      className="btn btn-icon btn-sm"
                      aria-label={t("accounts.actions_for", { name })}
                      isDisabled={busy}
                    >
                      <IconDotsVertical size={16} />
                    </Button>
                    <Popover className="popover" placement="bottom end">
                      <Menu
                        className="menu"
                        onAction={(key) => {
                          if (key === "rename") setRenaming(account);
                          if (key === "copy-ip" && connection.ipv4) copy(connection.ipv4);
                          if (key === "copy-dns" && connection.dnsName) copy(connection.dnsName);
                          if (key === "logout") setConfirming({ kind: "logout", account });
                          if (key === "remove") setConfirming({ kind: "remove", account });
                        }}
                      >
                        {account.active ? (
                          <>
                            <MenuItem className="menu-item" id="rename">
                              <IconPencil size={16} />
                              {t("accounts.rename")}
                            </MenuItem>
                            {connection.ipv4 && (
                              <MenuItem className="menu-item" id="copy-ip">
                                <IconCopy size={16} />
                                {t("accounts.copy_ip")}
                              </MenuItem>
                            )}
                            {connection.dnsName && (
                              <MenuItem className="menu-item" id="copy-dns">
                                <IconCopy size={16} />
                                {t("accounts.copy_dns")}
                              </MenuItem>
                            )}
                            <Separator className="menu-separator" />
                            <MenuItem className="menu-item menu-item-danger" id="logout">
                              <IconLogout size={16} />
                              {t("accounts.logout")}
                            </MenuItem>
                          </>
                        ) : (
                          <MenuItem className="menu-item menu-item-danger" id="remove">
                            <IconTrash size={16} />
                            {t("accounts.remove")}
                          </MenuItem>
                        )}
                      </Menu>
                    </Popover>
                  </MenuTrigger>
                </span>
              </div>

              {account.active && <Details connection={connection} onCopy={copy} />}
            </GridListItem>
          );
        }}
      </GridList>

      <RenameDialog
        account={renaming}
        onClose={() => setRenaming(null)}
        onSubmit={(name) => {
          onRename(name);
          setRenaming(null);
        }}
      />

      <ConfirmDialog
        confirm={confirming}
        onClose={() => setConfirming(null)}
        onConfirm={(confirm) => {
          setConfirming(null);
          if (confirm.kind === "logout") onLogout(confirm.account);
          else onRemove(confirm.account);
        }}
      />
    </>
  );
}

/** Where this device sits on the tailnet of the account in use. Only what the
 *  daemon actually reported: a stopped device has no address to show. */
function Details({
  connection,
  onCopy,
}: {
  connection: Connection;
  onCopy: (text: string) => void;
}) {
  const t = useT();
  const rows: { label: string; value: string; copy?: boolean }[] = [];
  if (connection.ipv4) rows.push({ label: t("accounts.ip"), value: connection.ipv4, copy: true });
  if (connection.dnsName) rows.push({ label: t("accounts.dns"), value: connection.dnsName, copy: true });
  if (connection.exitNode) rows.push({ label: t("accounts.exit_node"), value: connection.exitNode });
  const expiry = keyExpiry(t, connection.keyExpiry);
  if (expiry) rows.push({ label: t("accounts.key"), value: expiry });
  if (rows.length === 0) return null;

  return (
    <div className="account-details">
      {rows.map((row) => (
        <Row key={row.label} label={row.label}>
          <span className={`detail-value${row.copy ? " mono" : ""}`}>{row.value}</span>
          {row.copy ? (
            <Button
              className="btn btn-icon btn-sm detail-copy"
              aria-label={`${t("app.copy")} ${row.label}`}
              onPress={() => onCopy(row.value)}
            >
              <IconCopy size={14} />
            </Button>
          ) : (
            <span />
          )}
        </Row>
      ))}
    </div>
  );
}

function Row({ label, children }: { label: string; children: ReactNode }) {
  return (
    <>
      <span className="detail-label">{label}</span>
      {children}
    </>
  );
}

function RenameDialog({
  account,
  onClose,
  onSubmit,
}: {
  account: Account | null;
  onClose: () => void;
  onSubmit: (name: string) => void;
}) {
  const t = useT();

  return (
    <ModalOverlay
      className="overlay"
      isOpen={account !== null}
      onOpenChange={(open) => !open && onClose()}
      isDismissable
    >
      <Modal className="modal">
        <Dialog className="dialog">
          <Heading slot="title">{t("accounts.rename_title")}</Heading>
          {/* Keyed, so the field starts from this account's name every time
              the dialog opens rather than from whatever was typed last. */}
          {account && (
            <RenameForm
              key={account.id}
              account={account}
              onClose={onClose}
              onSubmit={onSubmit}
            />
          )}
        </Dialog>
      </Modal>
    </ModalOverlay>
  );
}

function RenameForm({
  account,
  onClose,
  onSubmit,
}: {
  account: Account;
  onClose: () => void;
  onSubmit: (name: string) => void;
}) {
  const t = useT();
  // A name that is only the login is no nickname: the field starts empty, and
  // saving it empty keeps it that way.
  const nickname = account.name !== account.loginName ? account.name : "";
  const [value, setValue] = useState(nickname);

  return (
    <form
      onSubmit={(e) => {
        e.preventDefault();
        onSubmit(value.trim());
      }}
    >
      <TextField className="field" autoFocus value={value} onChange={setValue}>
        <Label>{t("accounts.name_field")}</Label>
        <Input className="input" placeholder={account.loginName ?? ""} />
        <Text slot="description">{t("accounts.name_hint")}</Text>
      </TextField>
      <div className="dialog-actions">
        <Button className="btn" onPress={onClose}>
          {t("app.cancel")}
        </Button>
        <Button className="btn btn-primary" type="submit">
          {t("app.save")}
        </Button>
      </div>
    </form>
  );
}

function ConfirmDialog({
  confirm,
  onClose,
  onConfirm,
}: {
  confirm: Confirm | null;
  onClose: () => void;
  onConfirm: (confirm: Confirm) => void;
}) {
  const t = useT();
  const name = confirm ? title(confirm.account) : "";
  const logout = confirm?.kind === "logout";

  return (
    <ModalOverlay
      className="overlay"
      isOpen={confirm !== null}
      onOpenChange={(open) => !open && onClose()}
      isDismissable
    >
      <Modal className="modal">
        <Dialog className="dialog" role="alertdialog">
          <Heading slot="title">
            {t(logout ? "accounts.logout_title" : "accounts.remove_title", { name })}
          </Heading>
          <p className="dialog-text">
            {t(logout ? "accounts.logout_body" : "accounts.remove_body")}
          </p>
          <div className="dialog-actions">
            <Button className="btn" onPress={onClose}>
              {t("app.cancel")}
            </Button>
            <Button
              className="btn btn-danger"
              onPress={() => confirm && onConfirm(confirm)}
            >
              {t(logout ? "accounts.logout_confirm" : "accounts.remove_confirm")}
            </Button>
          </div>
        </Dialog>
      </Modal>
    </ModalOverlay>
  );
}
