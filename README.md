# Tailscale Account Switcher

> **Fair warning: this thing is entirely vibe coded.**
>
> Not one line of it was typed by hand. It was prompted into existence with
> Claude Code, and reviewed mostly by using it. It works, though — so, honestly:
> who cares.

A desktop app (Tauri 2 + React + React Aria Components) that lists the Tailscale
accounts logged in on this computer and switches between them — from its window
or from the system tray. It is the sibling of
[Claude Account Switcher](https://github.com/01jam/claude-account-switcher), in black and white
after Tailscale's own palette.

The interface speaks English and Italian: it follows the system language, with
an override in settings.

## How it works

Tailscale already keeps several accounts side by side. Each login is a
*profile* held by the daemon — "fast user switching" — so, unlike its Claude
sibling, this app stores no credentials of its own. It reads the daemon and
drives the `tailscale` CLI.

- **Reads** go to the daemon's LocalAPI over its Unix socket
  (`/var/run/tailscale/tailscaled.sock`): the same JSON `tailscale status
  --json` prints, for a round-trip instead of a process. Where there is no
  socket to talk to — the macOS app keeps its API behind a token — the app
  falls back to the CLI's JSON output (`tailscale status --json`,
  `tailscale switch --list --json`, `tailscale debug prefs`).
- **Changes** always go through the CLI: `tailscale switch`, `switch remove`,
  `up`, `down`, `set --nickname`, `logout`, `login`.

The daemon is read every 3 seconds over the socket (6 over the CLI, 1 while an
account is coming up or a login is waiting), so a `tailscale switch` typed in a
terminal shows up in the tray within moments. The read is pushed to the window
rather than fetched by it: the webview spends most of its life hidden in the
tray, where its timers are throttled.

The app keeps a small file of its own, `~/.config/tailscale-switch/state.json`:
the order the list was dragged into, the window preferences, the custom control
servers typed before, and the control server last seen for each account (the
CLI only reports it for the account in front).

### Operator, account by account (Linux)

On Linux tailscaled runs as root and lets a normal user change anything only if
that user is the **operator** of the account in front. The operator is a
preference *of each account*, not of the machine. Two consequences, both easy to
trip over by hand:

- switching onto an account that has no operator works — and leaves you unable
  to switch back, or even to list the accounts, without `sudo`;
- `tailscale login` run as a normal user moves the daemon onto a brand-new
  account (which has no operator), then fails to start it. The daemon is left on
  an empty account, disconnected from the one you were on.

So the app does this:

- When the daemon refuses this user the account in front, the window says so
  and offers **Grant access**, which runs `tailscale set --operator=$USER`
  through `pkexec`. polkit draws the password prompt and keeps the password; the
  app only learns whether it was given. Once per account — the setting stays
  with it. The equivalent command is shown next to the button, to copy or to run
  yourself.
- A change the daemon refuses (from the window or the tray) opens the same offer
  and is retried as soon as access is in.
- **Adding an account** runs `tailscale login` as root, through the same prompt,
  with `--operator=$USER`: the new account is born drivable without root.

None of this applies where the daemon runs as your own user, or on macOS.

### Adding an account

**Add account** asks what the CLI cannot guess: which control server
(Tailscale's, one used before, or another URL such as a Headscale instance), how
to authenticate, and an optional nickname. Then it runs `tailscale login`, one
of two ways:

- **In the browser.** The login link the CLI prints is opened in the browser
  and kept in the dialog, with buttons to open it again or copy it. On Headscale
  that page ends with a `headscale nodes register` command an admin has to run
  on the server; if the tailnet wants an admin to approve the device, the dialog
  says that instead.
- **With a pre-auth key** — a `tskey-auth-…` key from Tailscale's admin console,
  or one from `headscale preauthkeys create`. The server accepts the device on
  the key, with no browser and nobody on the server. The key reaches the CLI
  through `--auth-key=file:`, in a file only this user can read in the per-user
  runtime directory, deleted as soon as the login ends: on the command line it
  would be readable by anyone on the machine in the process list. The app does
  not keep it.

The dialog cannot be dismissed while the login runs. **Cancel** stops it and
switches the daemon back to the account you were on, because an abandoned login
leaves the daemon on the half-made account. A login not completed within 10
minutes is abandoned the same way.

### Logging in again

An account whose node key expired shows **Login required**, with the same two
ways back in:

- **Log in via browser** goes through the socket's `login-interactive` rather
  than `tailscale up`: on an account that needs logging in, `up` insists on
  being given back every non-default setting the account has (`--login-server`
  first of all, for anyone on Headscale) and refuses otherwise.
- **Use a key** opens the add dialog preset with the account's server and
  nickname. A login for an account already on this machine does not add a
  second one: the daemon folds it into the existing account. It keeps the
  preferences of the new login, though, not the account's old ones — an exit
  node or accepted routes need setting again, and the dialog says so.

### Remove, or log out

- **Remove from this computer** (any account but the one in front) is
  `tailscale switch remove`: the account disappears from this machine, and
  nothing changes on the control server.
- **Log out** (the account in front) is `tailscale logout`: the device
  disconnects and its node key is invalidated on the server.

The CLI silently does nothing when asked to remove the account in front, so the
app does not offer it.

### Where the app says it

Anything that answers a button just pressed is a toast in the window. The one
thing that happens on its own — the account in front losing its login, typically
a key expiring hours into a session — goes to a desktop notification when the
window is not in front, and is kept for the window when the desktop has nowhere
to put it.

## Requirements

Node 20+ and npm, Tailscale installed and running.

### Linux

```bash
# toolchain
sudo apt install -y build-essential curl file libssl-dev pkg-config
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
# Tauri 2 runtime + tray, Ubuntu 24.04+
sudo apt install -y libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev \
  libayatana-appindicator3-dev
```

`pkexec` (package `pkexec`, or `policykit-1` on older releases) is what grants
access and adds accounts without a terminal. Without it the app still works, and
shows the commands to run with `sudo` instead.

**GNOME** shows no tray icons without the
[AppIndicator and KStatusNotifierItem Support](https://extensions.gnome.org/extension/615/appindicator-support/)
extension. **Fractional scaling** under Wayland and the **NVIDIA** driver get
the same workarounds as in the Claude sibling (`TAILSCALE_SWITCH_KEEP_WAYLAND=1`
keeps the native Wayland backend).

### macOS

Written for, not yet tried on. The CLI is looked for in
`/Applications/Tailscale.app/Contents/MacOS/Tailscale` as well as on the `PATH`.

## Development

```bash
npm install
npm run app        # tauri dev
npm run app:build  # .deb / AppImage / .rpm on Linux, .app / .dmg on macOS
```

```bash
cd src-tauri
cargo test                                     # parsing, link detection, i18n
cargo test live_snapshot -- --ignored --nocapture   # reads this machine's daemon
```

The live test only reads. `TAILSCALE_SWITCH_SOCKET=/nonexistent` forces the CLI
fallback; `TAILSCALE_SWITCH_CLI` points the app at another `tailscale` binary.

Side tools:

```bash
npm run shots                          # the real UI in a browser, with fixed data
python3 scripts/generate-icons.py      # regenerate the tray and app icons
./scripts/install-desktop-entry.sh     # a real dock icon while developing
```

`npm run shots` serves the window at `http://localhost:1431/` against mocked
Tauri APIs; `?scene=` picks a state (`window`, `limited`, `needs-login`,
`stopped`, `unavailable`, `empty`, `add`, `waiting`, `settings`, `menu`) and
`&theme=dark` the dark palette.

`generate-icons.py` draws everything with no imaging library: Tabler's
`arrows-exchange` in white on a black squircle, the sibling's mark on
Tailscale's ink. Nothing is borrowed from Tailscale's own logo.

## Caveats

- `tailscale switch` and `tailscale login` are marked *alpha* by Tailscale, and
  the LocalAPI is not a stable interface. Both have been steady for years; what
  would mean this needs revisiting is a change in `tailscale switch --list
  --json` or in the socket's `profiles/` endpoint.
- Switching accounts reconnects the device: open connections over the tailnet
  drop, as they would with the CLI.

## Licence

MIT — see [LICENSE](LICENSE).

Tailscale Account Switcher is an independent tool. It is not affiliated with,
endorsed by, or supported by Tailscale Inc.; Tailscale is their trademark.
