# AGENTS.md — working on nuttty

nuttty is a btm-inspired terminal dashboard for Network UPS Tools (NUT),
written in Rust with ratatui. It talks the `upsd` TCP line protocol directly.

## Commands

| Task | Command |
|------|---------|
| Full quality gate | `make check` (fmt-check + clippy `-D warnings` + tests + release build) |
| Format | `make fmt` |
| Lint | `make lint` |
| Tests | `make test` |
| Release build | `make build` |
| Fast smoke test (no TTY needed) | `./target/release/nuttty --once` — dumps all variables from a live `upsd` and exits |

CI (`.github/workflows/ci.yml`) runs the same gate as `make check`.

## Architecture

- `src/main.rs` — CLI/env/config parsing (precedence: CLI > env > config
  file), the worker thread that polls `upsd`, and the event loop with
  mode-based key routing (`Normal` / `Help` / `Menu` / `Confirm`).
- `src/nut.rs` — minimal NUT protocol client. Polling uses one persistent
  anonymous connection that is dropped and re-established on any error.
  Instant commands (`instcmd`) use a separate short-lived authenticated
  connection. All connects go through `connect()` which enforces a connect
  timeout — never use bare `TcpStream::connect` (unreachable hosts block for
  minutes and freeze the single polling thread).
- `src/app.rs` — application state: latest variables, chart history,
  trend fit (`charge_slope`/`trend_eta`), log, popup mode, menu state.
- `src/ui.rs` — all rendering. Popups are drawn last over the main layout.

Threading model: one worker thread owns the socket; it exchanges
`Update`/`WorkerCmd` messages with the UI thread over `std::sync::mpsc`.
Polling is paced by `recv_timeout` on the command channel so a queued
command also triggers an immediate re-poll.

## Hard rules

- **Dependencies:** ratatui + anyhow only. Do not add crates (no tokio, no
  serde/toml, no clap, no NUT client crates). The config parser is
  deliberately a flat `key = "value"` subset of TOML.
- **Never style text with `Color::DarkGray`** (or other bright-black). In
  common palettes (Solarized dark among them) bright-black equals the
  terminal background, making the text invisible. Use `ui::gray()` (the
  `DIM` attribute) for muted text.
- **No machine-specific strings** in code, docs, or examples: no real UPS
  names, hostnames, serial numbers, usernames, or passwords. Use `myups`,
  `nut-server`, `upsadmin`-style placeholders.
- Commands that can cut power (`load.off*`, `shutdown.*`,
  `driver.killpower`) must stay behind the y/N `Confirm` mode and be
  rendered red in the menu (`app::is_dangerous`).

## Release process

The version lives in three places — the git tag, `Cargo.toml`, and
`PKGBUILD` — and they must stay in sync. To release vX.Y.Z:

1. Bump `version` in `Cargo.toml`, then run `cargo build` so `Cargo.lock`
   follows.
2. Set `pkgver=X.Y.Z` in `PKGBUILD` and reset `pkgrel=1`.
3. Commit, create an annotated tag `vX.Y.Z`, push with `--follow-tags`.
4. The `sha512sums` in PKGBUILD hashes the tag's GitHub tarball, which only
   exists after step 3 (and can't hash itself, so the tagged PKGBUILD always
   lags one release). After pushing the tag: regenerate with `makepkg -g`,
   verify with `makepkg --verifysource`, then commit and push the checksum
   update to master — that's the PKGBUILD users clone.

## Testing against real hardware

Development often happens on a machine with a live UPS on `localhost:3493`.

- Safe to run freely: `--once`, variable polling, `beeper.enable` when the
  beeper is already enabled (idempotent), `test.battery.stop` when no test
  is running.
- Needs a reason: `test.battery.start.quick` (brief battery discharge).
- Never run in automated tests: `test.battery.start.deep` (drains the
  battery), `load.off*`, `shutdown.*`, `driver.killpower` (cut real power).
- Credentials for instant commands come from `NUTTTY_USER`/`NUTTTY_PASSWORD`
  env vars; never hardcode them.

Quirk to expect from real devices: some firmwares (CyberPower-platform OEMs)
refresh status variables only every driver `pollfreq` (~12 s), so a command's
effect (e.g. beeper status) shows up one poll cycle later, and
`ups.test.result` may reset to "No test initiated" instead of reporting a
pass. Code and tests must tolerate this lag.

## Verifying the TUI

There is no tmux here; use a Python `pty.fork()` harness (spawn the release
binary, set the winsize ioctl, drain output, write keys, assert on
escape-stripped landmarks). Two traps:

- Strip only SGR/cursor escapes and compare **space-collapsed** strings —
  cursor-positioning splits words mid-cell.
- Never send `\x1b` (Esc) immediately followed by another key: terminal
  input parsing coalesces them into an Alt-chord. Sleep between writes or
  close popups with `q`/`?` instead.

Landmark checks verify characters, not colors. If a report says an element
"doesn't show", suspect color-vs-background collisions before layout bugs.
