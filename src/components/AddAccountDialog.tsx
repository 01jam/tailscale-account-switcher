import { useState } from "react";
import {
  Button,
  Dialog,
  FieldError,
  Heading,
  Input,
  Label,
  ListBox,
  ListBoxItem,
  Modal,
  ModalOverlay,
  Popover,
  Radio,
  RadioGroup,
  Select,
  SelectValue,
  Text,
  TextField,
} from "react-aria-components";
import {
  IconChevronDown,
  IconCopy,
  IconExternalLink,
  IconEye,
  IconEyeOff,
  IconKey,
  IconLoader2,
  IconLock,
  IconPlus,
  IconUserCheck,
} from "@tabler/icons-react";

import type { Account, LoginProgress } from "../api";
import { copyText, serverHost } from "../format";
import { useT } from "../i18n";

/** Where an add is, from the form to the browser and back. */
export type AddPhase =
  | { phase: "form" }
  | { phase: "starting"; withKey: boolean }
  | { phase: "waiting"; withKey: boolean; progress: LoginProgress }
  | { phase: "failed"; error: string };

export type AddMethod = "browser" | "key";

/** What the form holds. Kept by the caller rather than the form, so that
 *  "Try again" after a failed login goes back to what was typed. */
export type AddDraft = {
  server: typeof TAILSCALE | typeof OTHER;
  /** Under `OTHER`: a server used before, or `NEW_SERVER` to type one in. Empty
   *  until chosen, and then the most recent server stands in. */
  pick: string;
  url: string;
  nickname: string;
  method: AddMethod;
  key: string;
  /** Opened for an account that needs logging in again, not to add one. */
  reauth: boolean;
};

export const TAILSCALE = "tailscale";
export const OTHER = "other";
/** The dropdown's own entry for a server not used before. Not a URL, so it
 *  cannot collide with one. */
export const NEW_SERVER = "new";

export const EMPTY_DRAFT: AddDraft = {
  server: TAILSCALE,
  pick: "",
  url: "",
  nickname: "",
  method: "browser",
  key: "",
  reauth: false,
};

/** A key login for an account already on this machine. The daemon folds it
 *  into that account rather than adding a second one, so this is how a key
 *  gets an expired account back — with its server and nickname kept. */
export function reauthDraft(account: Account): AddDraft {
  return {
    ...EMPTY_DRAFT,
    ...(account.controlUrl && serverHost(account.controlUrl)
      ? { server: OTHER, pick: account.controlUrl }
      : { server: TAILSCALE }),
    nickname: account.name !== account.loginName ? account.name : "",
    method: "key",
    reauth: true,
  };
}

type Props = {
  isOpen: boolean;
  state: AddPhase;
  draft: AddDraft;
  /** Merged into the draft the caller holds — as an update, not a copy, so two
   *  changes landing in one render do not undo each other. */
  onDraft: (change: Partial<AddDraft>) => void;
  /** Custom control servers seen or typed before. */
  servers: string[];
  /** On Linux the login runs as root, behind the system's password prompt. */
  needsRoot: boolean;
  onStart: (server: string | null, nickname: string | null, authKey: string | null) => void;
  onCancel: () => void;
  onClose: () => void;
  onBack: () => void;
  onOpenUrl: (url: string) => void;
  onCopied: () => void;
};

/**
 * Adding an account is the CLI's `tailscale login`. The dialog asks what the
 * CLI cannot guess — which control server, how to authenticate, and what to
 * call the account — and then stays up until the login is done.
 *
 * Two ways in, because they suit different tailnets: a login in the browser,
 * which on Headscale ends with an admin registering the device on the server,
 * and a pre-auth key, which the server accepts on its own.
 *
 * It cannot be dismissed while a login runs: closing it would leave the daemon
 * on a half-made account with nothing on screen saying so. Cancel is the way
 * out, and it puts the previous account back.
 */
export default function AddAccountDialog(props: Props) {
  const { isOpen, state, onClose } = props;
  const running = state.phase === "starting" || state.phase === "waiting";

  return (
    <ModalOverlay
      className="overlay"
      isOpen={isOpen}
      onOpenChange={(open) => !open && !running && onClose()}
      isDismissable={!running}
      isKeyboardDismissDisabled={running}
    >
      <Modal className="modal">
        <Dialog className="dialog">
          {state.phase === "form" && <Form {...props} />}
          {running && <Waiting {...props} />}
          {state.phase === "failed" && <Failed {...props} error={state.error} />}
        </Dialog>
      </Modal>
    </ModalOverlay>
  );
}

function Form({ draft, onDraft, servers, needsRoot, onStart, onClose }: Props) {
  const t = useT();
  const [tried, setTried] = useState(false);
  const [revealed, setRevealed] = useState(false);
  const set = onDraft;

  const other = draft.server === OTHER;
  // A preset's server gets its entry even before the list comes back with it.
  const preset = draft.pick && draft.pick !== NEW_SERVER ? draft.pick : null;
  const known = preset && !servers.includes(preset) ? [preset, ...servers] : servers;
  const pick =
    draft.pick === NEW_SERVER || known.includes(draft.pick)
      ? draft.pick
      : (known[0] ?? NEW_SERVER);
  const typing = other && pick === NEW_SERVER;

  const withKey = draft.method === "key";
  const urlMissing = typing && draft.url.trim() === "";
  const keyMissing = withKey && draft.key.trim() === "";

  return (
    <form
      onSubmit={(e) => {
        e.preventDefault();
        setTried(true);
        if (urlMissing || keyMissing) return;
        const server = !other ? null : typing ? draft.url.trim() : pick;
        onStart(server, draft.nickname.trim() || null, withKey ? draft.key.trim() : null);
      }}
    >
      <Heading slot="title">{t(draft.reauth ? "add.reauth_title" : "add.title")}</Heading>
      {/* The daemon folds the login into the existing account but keeps this
          login's preferences, not the account's old ones. */}
      {draft.reauth && <p className="dialog-text">{t("add.reauth_note")}</p>}

      <div className="dialog-stack">
        <div className="field">
          <RadioGroup
            className="field"
            value={draft.server}
            onChange={(server) => set({ server: server as AddDraft["server"] })}
          >
            <Label className="field-label">{t("add.server_label")}</Label>
            <div className="choices">
              <Choice
                value={TAILSCALE}
                title={t("add.server_tailscale")}
                hint={t("add.server_tailscale_hint")}
              />
              <Choice value={OTHER} title={t("add.server_other")} hint={t("add.server_other_hint")} />
            </div>
          </RadioGroup>

          {/* Outside the radio group, so arrow keys in the dropdown stay the
              dropdown's. However many servers have been used, this is one row:
              the list is inside it. */}
          {other && (
            <div className="nested">
              {known.length > 0 && (
                <Select
                  aria-label={t("add.server_pick")}
                  selectedKey={pick}
                  onSelectionChange={(key) => set({ pick: String(key) })}
                >
                  <Button className="select-trigger">
                    <SelectValue className="select-value" />
                    <IconChevronDown size={16} />
                  </Button>
                  <Popover className="popover popover-select">
                    <ListBox className="menu">
                      {known.map((server) => (
                        <ListBoxItem
                          key={server}
                          id={server}
                          className="menu-item"
                          textValue={serverHost(server) ?? server}
                        >
                          <span className="option-title">{serverHost(server) ?? server}</span>
                          {beyondHost(server) && (
                            <span className="option-hint">{beyondHost(server)}</span>
                          )}
                        </ListBoxItem>
                      ))}
                      <ListBoxItem
                        id={NEW_SERVER}
                        className="menu-item menu-item-new"
                        textValue={t("add.server_new")}
                      >
                        <IconPlus size={15} />
                        {t("add.server_new")}
                      </ListBoxItem>
                    </ListBox>
                  </Popover>
                </Select>
              )}

              {typing && (
                <TextField
                  className="field"
                  autoFocus
                  value={draft.url}
                  onChange={(url) => set({ url })}
                  isInvalid={tried && urlMissing}
                  aria-label={t("add.server_url")}
                  type="url"
                >
                  <Input className="input mono" placeholder={t("add.server_placeholder")} />
                  <FieldError>{t("errors.invalid_server")}</FieldError>
                </TextField>
              )}
            </div>
          )}
        </div>

        <RadioGroup
          className="field"
          orientation="horizontal"
          value={draft.method}
          onChange={(method) => set({ method: method as AddMethod })}
        >
          <Label className="field-label">{t("add.method_label")}</Label>
          <div className="segmented">
            <Radio className="segment" value="browser">
              <IconExternalLink size={15} />
              {t("add.method_browser")}
            </Radio>
            <Radio className="segment" value="key">
              <IconKey size={15} />
              {t("add.method_key")}
            </Radio>
          </div>
          {/* Only the browser needs explaining here; the key field below says
              what it takes. */}
          {!withKey && <Text slot="description">{t("add.method_browser_hint")}</Text>}
        </RadioGroup>

        {withKey && (
          <TextField
            className="field"
            autoFocus={!typing}
            value={draft.key}
            onChange={(key) => set({ key })}
            isInvalid={tried && keyMissing}
            type={revealed ? "text" : "password"}
          >
            <Label>{t("add.key_label")}</Label>
            <div className="input-group">
              <Input
                className="input mono"
                placeholder={t("add.key_placeholder")}
                autoComplete="off"
                spellCheck={false}
              />
              <Button
                className="btn btn-icon btn-sm"
                aria-label={t(revealed ? "add.key_hide" : "add.key_show")}
                onPress={() => setRevealed((r) => !r)}
              >
                {revealed ? <IconEyeOff size={16} /> : <IconEye size={16} />}
              </Button>
            </div>
            <Text slot="description">{t("add.key_description")}</Text>
            <FieldError>{t("add.key_missing")}</FieldError>
          </TextField>
        )}

        <TextField
          className="field"
          value={draft.nickname}
          onChange={(nickname) => set({ nickname })}
        >
          <Label>{t("add.nickname")}</Label>
          <Input className="input" placeholder={t("add.nickname_placeholder")} />
        </TextField>

        {needsRoot && (
          <p className="note">
            <IconLock size={14} />
            <span>{t("add.admin_note")}</span>
          </p>
        )}
      </div>

      <div className="dialog-actions">
        <Button className="btn" onPress={onClose}>
          {t("app.cancel")}
        </Button>
        <Button className="btn btn-primary" type="submit">
          {withKey ? <IconKey size={15} /> : <IconExternalLink size={15} />}
          {withKey
            ? t(draft.reauth ? "add.start_reauth" : "add.start_key")
            : t("add.start")}
        </Button>
      </div>
    </form>
  );
}

/** What a server's URL says beyond its host — a port, a path, plain http — or
 *  nothing, when the host already said it all. */
function beyondHost(url: string): string {
  const host = serverHost(url);
  return host && url.replace(/\/+$/, "") === `https://${host}` ? "" : url;
}

function Choice({ value, title, hint }: { value: string; title: string; hint: string }) {
  return (
    <Radio className="choice" value={value}>
      <span className="choice-mark" aria-hidden />
      <span className="choice-body">
        <span className="choice-title">{title}</span>
        <span className="choice-hint">{hint}</span>
      </span>
    </Radio>
  );
}

function Waiting({ state, draft, onCancel, onOpenUrl, onCopied }: Props) {
  const t = useT();
  const progress = state.phase === "waiting" ? state.progress : null;
  const withKey = state.phase !== "failed" && state.phase !== "form" && state.withKey;
  const approval = progress?.approval ?? false;

  const heading = progress
    ? t(approval ? "add.approval" : "add.waiting")
    : t(withKey ? "add.key_starting" : "add.starting");

  return (
    <>
      <Heading slot="title">{t(draft.reauth ? "add.reauth_title" : "add.title")}</Heading>
      <div className="waiting" aria-live="polite">
        {approval ? (
          <IconUserCheck size={28} stroke={1.5} />
        ) : (
          <IconLoader2 size={28} stroke={1.5} className="spin" />
        )}
        <strong>{heading}</strong>
        {progress && (
          <>
            <p>{t(approval ? "add.approval_hint" : "add.waiting_hint")}</p>
            <div className="command">
              <code title={progress.url}>{progress.url}</code>
              <Button
                className="btn btn-icon btn-sm"
                aria-label={t("add.copy_link")}
                onPress={() => copyText(progress.url).then((ok) => ok && onCopied())}
              >
                <IconCopy size={15} />
              </Button>
              <Button
                className="btn btn-icon btn-sm"
                aria-label={t("add.open_again")}
                onPress={() => onOpenUrl(progress.url)}
              >
                <IconExternalLink size={15} />
              </Button>
            </div>
          </>
        )}
      </div>
      <div className="dialog-actions">
        <Button className="btn" onPress={onCancel}>
          {t("app.cancel")}
        </Button>
      </div>
    </>
  );
}

function Failed({ error, onBack, onClose }: Props & { error: string }) {
  const t = useT();
  return (
    <>
      <Heading slot="title">{t("add.failed_title")}</Heading>
      <p className="error-text">{error}</p>
      <div className="dialog-actions">
        <Button className="btn" onPress={onClose}>
          {t("app.close")}
        </Button>
        <Button className="btn btn-primary" onPress={onBack}>
          {t("app.retry")}
        </Button>
      </div>
    </>
  );
}
