# Vault

Native Linux Bitwarden client built with Rust and [GPUI](https://gpui.rs). It
uses [`rbw`](https://github.com/doy/rbw) for vault operations and the system
clipboard for autofill. No Electron, no browser engine, no simulated typing.

## Requirements

- Arch Linux: `sudo pacman -S rbw`
- A configured rbw setup:
  ```sh
  rbw config set email you@example.com
  rbw sync
  rbw unlock
  ```
- Wayland or X11
- A pinentry program rbw can use. A graphical pinentry is easier when Vault is
  started from the desktop.

## Build and install

```sh
cargo build --release
install -Dm755 target/release/vault ~/.local/bin/vault
install -Dm644 resources/vault.desktop ~/.local/share/applications/vault.desktop
install -Dm644 resources/vault.svg ~/.local/share/icons/hicolor/scalable/apps/vault.svg
```

Run with `vault`, or use `cargo run` while developing.

## Interface

The interface follows Aster Qt: a top bar, a sidebar with account and filters,
a searchable list, a details reader, and a status footer. The default theme
follows the desktop light or dark preference and updates live. Dark uses Aster
Dark tokens and light uses Kestrel Light. Icons are embedded SVG line icons.

## Shortcuts

| Shortcut | Action |
|---|---|
| `Ctrl+K` or `Ctrl+F` | Focus search |
| `Ctrl+C` | Copy default field |
| `Ctrl+Shift+C` | Copy password |
| `Ctrl+Shift+U` | Copy username |
| `Ctrl+Shift+T` | Copy TOTP |
| `Ctrl+G` | Password generator |
| `Ctrl+,` | Settings |
| `Ctrl+D` | Toggle favorite |
| `Ctrl+R` | Sync vault |
| `Enter` | Open entry or unlock |
| `Up`, `Down`, `Tab` | Move selection |
| `Escape` | Close details or clear search |

Shortcuts are read from `config.toml`.

## Configuration

Config lives at `~/.config/vault/config.toml`.

```toml
theme = "system"                # system | dark | light
clipboard_timeout_secs = 30     # 0 disables clearing
clear_clipboard_on_exit = true
default_copy_field = "password" # username | password | totp
favorites_first = true
rbw_path = "rbw"
notifications = true

[shortcuts]
focus_search = "ctrl-k"
copy_default = "ctrl-c"
copy_password = "ctrl-shift-c"
copy_username = "ctrl-shift-u"
copy_totp = "ctrl-shift-t"
sync = "ctrl-r"
generator = "ctrl-g"
toggle_favorite = "ctrl-d"

[generator]
length = 20
uppercase = true
lowercase = true
numbers = true
symbols = true
exclude_ambiguous = false
passphrase = false
words = 5
separator = "-"
```

Favorites are stored in `~/.config/vault/favorites.json` as item IDs only. Vault
never writes vault contents to disk.

## Security

- Vault never asks for or stores the master password. rbw handles locking.
- Only metadata is loaded up front. Passwords and notes are fetched on demand.
- TOTP codes come from `rbw code`. The TOTP secret is not kept.
- Secrets are masked by default, redacted in debug output, and zeroized on drop.
- The clipboard is cleared only if it still holds the exact value Vault copied.
  A newer value from another app is never overwritten.
- Config lives in a 0700 directory with 0600 files. Favorites store IDs only.
- rbw output is capped at 32 MiB, entry IDs are validated, and error text is
  stripped of control characters and truncated.
- Core dumps are disabled and the process is marked non-dumpable at startup.
- No shell strings are built, no network calls are made, and no credentials are
  logged.

Clipboard clearing is best effort. Clipboard managers, history tools, and the
receiving app may keep a copy. Vault does not claim secure erasure. Set
`clipboard_timeout_secs = 0` to disable clearing.

## Limits

- rbw does not expose favorites, so Vault keeps local favorite IDs.
- Cards, identities, and SSH keys are listed and searchable. Type-specific
  fields are not shown yet.
- `system` uses GTK, GNOME, or KDE settings and updates live. `dark` and `light`
  pin a theme.
- Tray, global quick access, and start minimized need a background daemon.
- Browser integration, focus monitoring, form detection, and typing are out of
  scope.
- A compromised rbw binary, clipboard manager, or desktop session can still read
  what the session can read. Vault reduces exposure, it cannot make a
  compromised machine safe.

## Tests

```sh
cargo test
```

Tests cover fuzzy search, rbw JSON parsing and redaction, error mapping,
clipboard exact-match clearing, password generation, and config round trips.

To try the UI without a real vault, point `rbw_path` at a script that answers
`--version`, `unlocked`, `list --raw`, `get <id> --raw`, and `code <id>`, then
run with `XDG_CONFIG_HOME=/tmp/vault-test`.
