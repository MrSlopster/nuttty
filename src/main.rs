// nuttty — a btm-inspired TUI dashboard for Network UPS Tools (NUT)
// Copyright (C) 2026
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

mod app;
mod nut;
mod ui;

use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use app::{App, Mode, Update, WorkerCmd};
use ratatui::crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers,
    MouseButton, MouseEventKind,
};
use ratatui::crossterm::execute;
use ratatui::layout::Position;
use ratatui::DefaultTerminal;

const USAGE: &str = "\
nuttty — a TUI dashboard for Network UPS Tools

USAGE: nuttty [OPTIONS] [UPS[@HOST[:PORT]]]

Defaults to the first UPS reported by upsd on localhost:3493.

OPTIONS:
  --user USER       NUT username for instant commands (env NUTTTY_USER / NUT_USER)
  --password PASS   NUT password (env NUTTTY_PASSWORD / NUT_PASSWORD;
                    prefer env or config: argv is visible in process lists)
  --interval MS     poll interval in milliseconds (default 2000)
  -b, --basic       summary panel only, no charts or variable table
  --once            print all UPS variables and exit
  -h, --help        show this help

CONFIG: $XDG_CONFIG_HOME/nuttty/config.toml, key = \"value\" lines
(ups, host, port, user, password, interval).
Precedence: command line > environment > config file.

KEYS: q quit · ? help · m command menu · t/d/s battery tests · b beeper
      arrows or j/k/g/G scroll · mouse clicks buttons
";

struct Config {
    ups: Option<String>,
    host: String,
    port: u16,
    user: Option<String>,
    pass: Option<String>,
    interval_ms: u64,
    once: bool,
    basic: bool,
}

/// $XDG_CONFIG_HOME/nuttty/config.toml, falling back to ~/.config per the
/// XDG base directory spec (which says a relative XDG_CONFIG_HOME is invalid
/// and must be ignored).
fn config_path() -> Option<std::path::PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config")))
        .map(|d| d.join("nuttty").join("config.toml"))
}

/// Parse config text: flat TOML-style `key = "value"` lines. `origin` is
/// only used to prefix error messages (the file path in real use).
fn parse_config(text: &str, origin: &str, cfg: &mut Config) -> Result<()> {
    for (no, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            bail!("{origin}:{}: expected `key = value`", no + 1);
        };
        // Quoted values keep everything up to the closing quote (so '#' is
        // literal inside quotes); unquoted values stop at an inline comment.
        let v = v.trim();
        let v = if let Some(rest) = v.strip_prefix('"') {
            rest.split_once('"').map_or(rest, |(inner, _)| inner)
        } else {
            v.split_once('#').map_or(v, |(x, _)| x).trim_end()
        };
        match k.trim() {
            "ups" => cfg.ups = Some(v.into()),
            "host" => cfg.host = v.into(),
            "port" => {
                cfg.port = v
                    .parse()
                    .with_context(|| format!("{origin}:{}: bad port", no + 1))?;
            }
            "user" => cfg.user = Some(v.into()),
            "password" => cfg.pass = Some(v.into()),
            "interval" => {
                cfg.interval_ms = v.parse().with_context(|| {
                    format!("{origin}:{}: interval must be milliseconds", no + 1)
                })?;
            }
            other => bail!("{origin}:{}: unknown key '{other}'", no + 1),
        }
    }
    Ok(())
}

fn load_config_file(cfg: &mut Config) -> Result<()> {
    let Some(path) = config_path() else {
        return Ok(());
    };
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e).context(format!("reading {}", path.display())),
    };
    parse_config(&text, &path.display().to_string(), cfg)
}

/// Parse a positional `UPS[@HOST[:PORT]]` argument into the config.
fn apply_spec(cfg: &mut Config, spec: &str) -> Result<()> {
    let (ups, hostpart) = match spec.split_once('@') {
        Some((u, h)) => (u, Some(h)),
        None => (spec, None),
    };
    if !ups.is_empty() {
        cfg.ups = Some(ups.to_string());
    }
    if let Some(h) = hostpart {
        match h.split_once(':') {
            Some((host, port)) => {
                cfg.host = host.to_string();
                cfg.port = port.parse().context("bad port")?;
            }
            None => cfg.host = h.to_string(),
        }
    }
    Ok(())
}

fn parse_args() -> Result<Config> {
    let mut cfg = Config {
        ups: None,
        host: "localhost".into(),
        port: 3493,
        user: None,
        pass: None,
        interval_ms: 2000,
        once: false,
        basic: false,
    };
    // Precedence: command line > environment > config file > defaults.
    load_config_file(&mut cfg)?;
    // A defined-but-empty env var must not clobber a config-file value.
    let env_nonempty = |names: [&str; 2]| {
        names
            .iter()
            .find_map(|n| std::env::var(n).ok().filter(|s| !s.is_empty()))
    };
    if let Some(u) = env_nonempty(["NUTTTY_USER", "NUT_USER"]) {
        cfg.user = Some(u);
    }
    if let Some(p) = env_nonempty(["NUTTTY_PASSWORD", "NUT_PASSWORD"]) {
        cfg.pass = Some(p);
    }
    // args_os: a non-UTF-8 argument must not panic us out of the process.
    let mut args = std::env::args_os()
        .skip(1)
        .map(|a| a.to_string_lossy().into_owned());
    while let Some(a) = args.next() {
        match a.as_str() {
            "-h" | "--help" => {
                print!("{USAGE}");
                std::process::exit(0);
            }
            "--once" => cfg.once = true,
            "-b" | "--basic" => cfg.basic = true,
            "--user" => cfg.user = Some(args.next().context("--user needs a value")?),
            "--password" => cfg.pass = Some(args.next().context("--password needs a value")?),
            "--interval" => {
                cfg.interval_ms = args
                    .next()
                    .context("--interval needs a value")?
                    .parse()
                    .context("--interval must be milliseconds")?;
            }
            s if s.starts_with('-') => bail!("unknown option '{s}', see --help"),
            spec => apply_spec(&mut cfg, spec)?,
        }
    }
    Ok(cfg)
}

fn main() -> Result<()> {
    let cfg = parse_args()?;
    let mut client = nut::NutClient::new(&cfg.host, cfg.port);
    let ups = match &cfg.ups {
        Some(u) => u.clone(),
        None => client
            .list_ups()
            .context("cannot list UPSes on server")?
            .into_iter()
            .next()
            .map(|(name, _)| name)
            .context("upsd reports no UPS configured")?,
    };

    if cfg.once {
        use std::io::Write;
        let mut out = std::io::stdout().lock();
        for (k, v) in client.list_vars(&ups)? {
            // Exit quietly instead of panicking when piped into `head` etc.
            if writeln!(out, "{k}: {v}").is_err() {
                break;
            }
        }
        return Ok(());
    }

    let (update_tx, update_rx) = mpsc::channel::<Update>();
    let (cmd_tx, cmd_rx) = mpsc::channel::<WorkerCmd>();
    {
        let ups = ups.clone();
        let host = cfg.host.clone();
        let (port, user, pass) = (cfg.port, cfg.user.clone(), cfg.pass.clone());
        let interval = Duration::from_millis(cfg.interval_ms.max(250));
        thread::spawn(move || {
            worker(
                client, ups, host, port, user, pass, interval, update_tx, cmd_rx,
            )
        });
    }

    // Restores the terminal on every exit path, including panics — ratatui's
    // own panic hook undoes raw mode but not our explicit mouse capture.
    struct TerminalGuard;
    impl Drop for TerminalGuard {
        fn drop(&mut self) {
            let _ = execute!(std::io::stdout(), DisableMouseCapture);
            ratatui::restore();
        }
    }

    let mut terminal = ratatui::init();
    let _guard = TerminalGuard;
    let _ = execute!(std::io::stdout(), EnableMouseCapture);
    let mut app = App::new(ups, cfg.host, cfg.port);
    app.basic = cfg.basic;
    run(&mut terminal, &mut app, update_rx, cmd_tx)
}

#[allow(clippy::too_many_arguments)]
fn worker(
    mut client: nut::NutClient,
    ups: String,
    host: String,
    port: u16,
    user: Option<String>,
    pass: Option<String>,
    interval: Duration,
    tx: mpsc::Sender<Update>,
    rx: mpsc::Receiver<WorkerCmd>,
) {
    let mut cmds_sent = false;
    loop {
        let update = match client.list_vars(&ups) {
            Ok(v) => Update::Vars(v),
            Err(e) => Update::Error(e.to_string()),
        };
        let polled_ok = matches!(update, Update::Vars(_));
        if tx.send(update).is_err() {
            return; // UI is gone
        }
        // The supported command set is static per device: fetch it once,
        // after the first successful poll proves the connection works.
        if polled_ok && !cmds_sent {
            if let Ok(cmds) = client.list_cmds(&ups) {
                cmds_sent = true;
                if tx.send(Update::Cmds(cmds)).is_err() {
                    return;
                }
            }
        }
        // Pace polling on the command channel so a button press both runs
        // immediately and triggers a fresh poll right after.
        match rx.recv_timeout(interval) {
            Ok(WorkerCmd::Inst(cmd)) => {
                let result = match (&user, &pass) {
                    (Some(u), Some(p)) => nut::instcmd(&host, port, &ups, &cmd, u, p)
                        .unwrap_or_else(|e| format!("ERR {e}")),
                    _ => "ERR no credentials — pass --user/--password or set NUTTTY_USER/NUTTTY_PASSWORD".into(),
                };
                if tx.send(Update::CmdResult { cmd, result }).is_err() {
                    return;
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        }
    }
}

fn run(
    terminal: &mut DefaultTerminal,
    app: &mut App,
    update_rx: mpsc::Receiver<Update>,
    cmd_tx: mpsc::Sender<WorkerCmd>,
) -> Result<()> {
    let send_cmd = |app: &mut App, cmd: &str| {
        let cmd = if cmd == "beeper" {
            if app.s("ups.beeper.status") == "enabled" {
                "beeper.disable"
            } else {
                "beeper.enable"
            }
        } else {
            cmd
        };
        app.note(format!("→ {cmd}"));
        let _ = cmd_tx.send(WorkerCmd::Inst(cmd.to_string()));
    };
    loop {
        while let Ok(u) = update_rx.try_recv() {
            app.apply(u);
        }
        terminal.draw(|f| ui::draw(f, app))?;
        if !event::poll(Duration::from_millis(200))? {
            continue;
        }
        match event::read()? {
            Event::Key(k) if k.kind == KeyEventKind::Press => {
                if k.code == KeyCode::Char('c') && k.modifiers.contains(KeyModifiers::CONTROL) {
                    return Ok(());
                }
                match &app.mode {
                    Mode::Help => match k.code {
                        KeyCode::Char('q')
                        | KeyCode::Char('?')
                        | KeyCode::Char('h')
                        | KeyCode::Esc
                        | KeyCode::Enter => {
                            app.mode = Mode::Normal;
                        }
                        _ => {}
                    },
                    Mode::Menu => match k.code {
                        KeyCode::Char('q')
                        | KeyCode::Char('m')
                        | KeyCode::Char('h')
                        | KeyCode::Esc => {
                            app.mode = Mode::Normal;
                        }
                        KeyCode::Char('?') => app.mode = Mode::Help,
                        KeyCode::Up | KeyCode::Char('k') => app.menu_scroll(-1),
                        KeyCode::Down | KeyCode::Char('j') => app.menu_scroll(1),
                        KeyCode::PageUp => app.menu_scroll(-10),
                        KeyCode::PageDown => app.menu_scroll(10),
                        KeyCode::Char('g') => app.menu_scroll(i64::MIN),
                        KeyCode::Char('G') => app.menu_scroll(i64::MAX),
                        KeyCode::Enter | KeyCode::Char('l') => {
                            if let Some(cmd) = app.selected_cmd() {
                                if app::is_dangerous(&cmd) {
                                    app.mode = Mode::Confirm(cmd);
                                } else {
                                    send_cmd(app, &cmd);
                                    app.mode = Mode::Normal;
                                }
                            }
                        }
                        _ => {}
                    },
                    Mode::Confirm(cmd) => match k.code {
                        KeyCode::Char('y') | KeyCode::Char('Y') => {
                            let cmd = cmd.clone();
                            send_cmd(app, &cmd);
                            app.mode = Mode::Normal;
                        }
                        // Anything else backs out to the menu, not straight
                        // to Normal, so a slip doesn't lose the user's place.
                        _ => app.mode = Mode::Menu,
                    },
                    Mode::Normal => match k.code {
                        KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                        KeyCode::Char('?') => app.mode = Mode::Help,
                        KeyCode::Char('m') => app.mode = Mode::Menu,
                        KeyCode::Up | KeyCode::Char('k') => app.scroll(-1),
                        KeyCode::Down | KeyCode::Char('j') => app.scroll(1),
                        KeyCode::PageUp => app.scroll(-10),
                        KeyCode::PageDown => app.scroll(10),
                        KeyCode::Char('g') => app.scroll(i64::MIN),
                        KeyCode::Char('G') => app.scroll(i64::MAX),
                        KeyCode::Char(c) => {
                            if let Some((_, _, cmd)) =
                                ui::BUTTONS.iter().find(|(_, key, _)| *key == c)
                            {
                                send_cmd(app, cmd);
                            }
                        }
                        _ => {}
                    },
                }
            }
            // Buttons are only clickable when no popup covers them.
            Event::Mouse(m)
                if matches!(m.kind, MouseEventKind::Down(MouseButton::Left))
                    && matches!(app.mode, Mode::Normal) =>
            {
                let pos = Position::new(m.column, m.row);
                let hit = app
                    .buttons
                    .iter()
                    .find(|(rect, _)| rect.contains(pos))
                    .map(|(_, cmd)| cmd.clone());
                if let Some(cmd) = hit {
                    send_cmd(app, &cmd);
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Config {
        Config {
            ups: None,
            host: "localhost".into(),
            port: 3493,
            user: None,
            pass: None,
            interval_ms: 2000,
            once: false,
            basic: false,
        }
    }

    #[test]
    fn config_full_and_comments() {
        let mut cfg = base();
        let text = "\
# leading comment
ups = \"myups\"
host = nut-server
port = 3494 # inline comment
user = \"admin\"
password = \"se#ret\"
interval = 1500

";
        parse_config(text, "test", &mut cfg).unwrap();
        assert_eq!(cfg.ups.as_deref(), Some("myups"));
        assert_eq!(cfg.host, "nut-server");
        assert_eq!(cfg.port, 3494);
        assert_eq!(cfg.user.as_deref(), Some("admin"));
        // '#' inside a quoted value is literal, not a comment.
        assert_eq!(cfg.pass.as_deref(), Some("se#ret"));
        assert_eq!(cfg.interval_ms, 1500);
    }

    #[test]
    fn config_rejects_unknown_key_and_bad_values() {
        let mut cfg = base();
        let err = parse_config("bogus = 1", "test", &mut cfg).unwrap_err();
        assert!(err.to_string().contains("unknown key 'bogus'"));
        let err = parse_config("port = zap", "test", &mut cfg).unwrap_err();
        assert!(err.to_string().contains("bad port"));
        let err = parse_config("just words", "test", &mut cfg).unwrap_err();
        assert!(err.to_string().contains("expected `key = value`"));
    }

    #[test]
    fn spec_variants() {
        let mut cfg = base();
        apply_spec(&mut cfg, "myups").unwrap();
        assert_eq!(cfg.ups.as_deref(), Some("myups"));
        assert_eq!((cfg.host.as_str(), cfg.port), ("localhost", 3493));

        let mut cfg = base();
        apply_spec(&mut cfg, "myups@server").unwrap();
        assert_eq!(cfg.ups.as_deref(), Some("myups"));
        assert_eq!((cfg.host.as_str(), cfg.port), ("server", 3493));

        let mut cfg = base();
        apply_spec(&mut cfg, "myups@server:1234").unwrap();
        assert_eq!((cfg.host.as_str(), cfg.port), ("server", 1234));

        // Host-only: keep the UPS unset so the first server UPS is used.
        let mut cfg = base();
        apply_spec(&mut cfg, "@server").unwrap();
        assert_eq!(cfg.ups, None);
        assert_eq!(cfg.host, "server");

        let mut cfg = base();
        assert!(apply_spec(&mut cfg, "u@h:notaport").is_err());
    }
}
