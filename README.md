# Vault

A small, native, keyboard-first Bitwarden frontend for Linux. Built with Rust and
[GPUI](https://gpui.rs), using [`rbw`](https://github.com/doy/rbw) as the vault
engine and the system clipboard for autofill. No Electron, no WebView, no browser
automation, no simulated typing.

## Why rbw is a subprocess

`rbw` is a binary with a private agent protocol, not a stable library. Vault runs
it with `std::process::Command`/`tokio::process::Command`, passes explicit
arguments (no shell), and parses JSON from `rbw list --raw` and `rbw get --raw`.
All of it is hidden behind the `backend::VaultProvider` trait so another provider
can be added later.

Vault never talks to Bitwarden directly and never implements cryptography or
synchronization. `rbw` remains the source of truth and owns locking.

## Requirements

- Arch Linux: `sudo pacman -S rbw`
- A working `rbw` setup (`rbw config set email ...`, then `rbw register` for the
  official server, `rbw sync`, `rbw unlock`)
- Wayland or X11 (GPUI picks the available backend)
- A pinentry program `rbw` can use. Unlocking from a GUI may need a graphical
  pinentry (`pinentry-gtk`, `pinentry-qt`, …) or launching Vault from a terminal.

## Build and install

```sh
cargo build --release
install -Dm755 target/release/vault ~/.local/bin/vault
install -Dm644 resources/vault.desktop ~/.local/share/applications/vault.desktop
install -Dm644 resources/vault.svg ~/.local/share/icons/hicolor/scalable/apps/vault.svg
```

Run with `vault`, or `cargo run` during development.

## Interface

The layout and tokens follow the Aster Qt client: a 64px top bar with brand,
search and one primary action; a 248px sidebar with account card, sections and
counts; a dense middle list with filter chips; a 480px reader-style details
panel; and a 30px status footer. Dark uses the Aster Dark tokens, light uses
Kestrel Light. Icons are embedded 16px line SVGs, not glyphs or emoji.

## Keyboard shortcuts

| Shortcut | Action |
|---|---|
| `Ctrl+K` / `Ctrl+F` | Focus search (clears the query) |
| `Ctrl+C` | Copy the default field (password by default) |
| `Ctrl+Shift+C` | Copy password |
| `Ctrl+Shift+U` | Copy username |
| `Ctrl+Shift+T` | Copy current TOTP code |
| `Ctrl+G` | Password generator |
| `Ctrl+,` | Settings |
| `Ctrl+D` | Toggle favorite |
| `Ctrl+R` | Synchronize vault |
| `Enter` | Open selected entry / unlock when locked |
| `↑` / `↓` / `Tab` | Move selection |
| `Escape` | Close details, or leave settings/generator, or clear search |

Shortcuts live in `config.toml` and are read at startup.

## Configuration

`$XDG_CONFIG_HOME/vault/config.toml` (usually `~/.config/vault/config.toml`):

```toml
theme = "system"               # system | dark | light
clipboard_timeout_secs = 30    # 0 disables timed clearing
clear_clipboard_on_exit = true
start_minimized = false        # stored; needs a tray/daemon to take effect
launch_at_login = false
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

Favorites are stored separately in `~/.config/vault/favorites.json` as item IDs
only. No vault contents are ever persisted.

## Security model

- Master passwords are never requested by Vault and never stored; `rbw`/pinentry
  handles unlocking.
- Only non-secret metadata (`id`, `name`, `username`, URLs, folder, type) is
  loaded up front. Passwords and notes are fetched on demand with
  `rbw get <id> --raw`.
- TOTP codes come from `rbw code <id>`; the TOTP secret is never retained.
- `Secret` values redact themselves in `Debug` output and are zeroized on drop.
- Passwords and notes are masked by default; revealing requires an explicit click.
- The clipboard is cleared after the configured timeout only if it still contains
  the exact value Vault copied. If you copied something else in the meantime,
  Vault leaves it alone. The same check runs on exit.
- No shell command strings are constructed and no credential is logged.

### Clipboard caveat

Clipboard clearing is best-effort. Wayland/X11 clipboard managers, history
tools, and the receiving application may retain a copy. Vault does not claim
secure erasure, and it does not restore previous clipboard contents by default
(restoration risks reintroducing stale secrets). Set `clipboard_timeout_secs = 0`
to disable clearing.

## Limitations

- `rbw` does not expose Bitwarden favorites, so favorites are local to Vault
  (IDs only).
- Cards, identities, and SSH keys are listed and searchable; this release shows
  their name, URLs, and notes, not every type-specific field.
- `theme = "system"` follows the desktop light/dark preference live, using the
  same GTK/GNOME/KDE sources as aster-qt. `dark` and `light` pin a slot.
- `start_minimized`, global quick-access shortcuts, and tray integration need a
  background daemon and are not part of this release. The `ipc` module reserves
  the boundary.
- Browser integration, focus monitoring, form detection, and typing are
  intentionally absent.

## Testing

```sh
cargo test
```

The tests cover fuzzy matching, rbw JSON parsing/redaction, error
classification, clipboard exact-match clearing, password generation, and config
round-tripping.

For manual UI testing without a real vault, point `rbw_path` at a small script
that answers `--version`, `unlocked`, `list --raw`, `get <id> --raw`, and
`code <id>` with fixtures, then start Vault with
`XDG_CONFIG_HOME=/tmp/vault-test`.
