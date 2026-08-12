//! Application state: latest UPS variables, time-series history, log, UI state.

use std::collections::{BTreeMap, VecDeque};
use std::time::Instant;

use ratatui::layout::Rect;
use ratatui::widgets::{ListState, TableState};

/// How much history the charts display, in seconds.
pub const HIST_SECS: f64 = 900.0;

pub enum Update {
    Vars(BTreeMap<String, String>),
    /// Instant commands the device supports: (name, description).
    Cmds(Vec<(String, String)>),
    /// Names of variables the device reports as writable.
    Rw(Vec<String>),
    Error(String),
    CmdResult {
        cmd: String,
        result: String,
    },
}

pub enum WorkerCmd {
    Inst(String),
    Set { name: String, value: String },
}

/// Which overlay currently owns the keyboard.
pub enum Mode {
    Normal,
    Help,
    Menu,
    /// Waiting for y/N on a destructive command.
    Confirm(String),
    /// Editing a new value for a writable variable.
    SetVar {
        name: String,
        buffer: String,
    },
}

/// What the currently selected menu row does when activated.
pub enum MenuAction {
    Cmd(String),
    Set(String),
}

/// Commands that can cut output power or shut things down get a
/// confirmation popup before being sent.
pub fn is_dangerous(cmd: &str) -> bool {
    cmd.starts_with("shutdown") || cmd.starts_with("load.off") || cmd == "driver.killpower"
}

pub struct App {
    pub ups: String,
    pub host: String,
    pub port: u16,
    pub vars: BTreeMap<String, String>,
    pub charge: VecDeque<(f64, f64)>,
    pub volt_in: VecDeque<(f64, f64)>,
    pub volt_out: VecDeque<(f64, f64)>,
    pub start: Instant,
    pub connected: bool,
    pub last_error: Option<String>,
    pub log: VecDeque<String>,
    pub table_state: TableState,
    /// Clickable button hitboxes, rebuilt every frame: (area, instant command).
    pub buttons: Vec<(Rect, String)>,
    pub mode: Mode,
    /// Instant commands supported by the device: (name, description).
    pub cmds: Vec<(String, String)>,
    /// Writable variable names; listed in the menu after the commands.
    pub rw: Vec<String>,
    pub menu_state: ListState,
    /// Basic mode: only the summary panel, no charts or variable table.
    pub basic: bool,
}

impl App {
    pub fn new(ups: String, host: String, port: u16) -> Self {
        Self {
            ups,
            host,
            port,
            vars: BTreeMap::new(),
            charge: VecDeque::new(),
            volt_in: VecDeque::new(),
            volt_out: VecDeque::new(),
            start: Instant::now(),
            connected: false,
            last_error: None,
            log: VecDeque::new(),
            table_state: TableState::default(),
            buttons: Vec::new(),
            mode: Mode::Normal,
            cmds: Vec::new(),
            rw: Vec::new(),
            menu_state: ListState::default(),
            basic: false,
        }
    }

    pub fn now(&self) -> f64 {
        self.start.elapsed().as_secs_f64()
    }

    pub fn apply(&mut self, u: Update) {
        match u {
            Update::Vars(v) => {
                self.connected = true;
                self.last_error = None;
                let t = self.now();
                if let Some(c) = get_f(&v, "battery.charge") {
                    push(&mut self.charge, t, c);
                }
                if let Some(x) = get_f(&v, "input.voltage") {
                    push(&mut self.volt_in, t, x);
                }
                if let Some(x) = get_f(&v, "output.voltage") {
                    push(&mut self.volt_out, t, x);
                }
                self.vars = v;
                // Keep the table cursor in bounds if the variable set shrank.
                if let Some(i) = self.table_state.selected() {
                    if i >= self.vars.len() {
                        self.table_state.select(self.vars.len().checked_sub(1));
                    }
                }
            }
            Update::Cmds(c) => {
                self.cmds = c;
                if self.menu_state.selected().is_none() && self.menu_len() > 0 {
                    self.menu_state.select(Some(0));
                }
            }
            Update::Rw(r) => {
                self.rw = r;
                if self.menu_state.selected().is_none() && self.menu_len() > 0 {
                    self.menu_state.select(Some(0));
                }
            }
            Update::Error(e) => {
                self.connected = false;
                if self.last_error.as_deref() != Some(e.as_str()) {
                    let msg = format!("poll error: {e}");
                    self.note(msg);
                }
                self.last_error = Some(e);
            }
            Update::CmdResult { cmd, result } => self.note(format!("{cmd} → {result}")),
        }
    }

    pub fn f(&self, key: &str) -> Option<f64> {
        get_f(&self.vars, key)
    }

    pub fn s(&self, key: &str) -> &str {
        self.vars.get(key).map(|s| s.as_str()).unwrap_or("–")
    }

    pub fn status(&self) -> &str {
        self.vars
            .get("ups.status")
            .map(|s| s.as_str())
            .unwrap_or("")
    }

    pub fn on_battery(&self) -> bool {
        self.status().split_whitespace().any(|t| t == "OB")
    }

    pub fn note(&mut self, msg: impl Into<String>) {
        // '+' marks these as elapsed-since-start, not wall-clock times.
        let t = self.now() as u64;
        self.log.push_back(format!(
            "[+{:02}:{:02}:{:02}] {}",
            t / 3600,
            t / 60 % 60,
            t % 60,
            msg.into()
        ));
        while self.log.len() > 100 {
            self.log.pop_front();
        }
    }

    pub fn scroll(&mut self, delta: i64) {
        let len = self.vars.len();
        if len == 0 {
            return;
        }
        let cur = self.table_state.selected().unwrap_or(0) as i64;
        // saturating_add so g/G can be sent as i64::MIN / i64::MAX deltas.
        let next = cur.saturating_add(delta).clamp(0, len as i64 - 1) as usize;
        self.table_state.select(Some(next));
    }

    /// Menu rows: instant commands first, then writable variables.
    pub fn menu_len(&self) -> usize {
        self.cmds.len() + self.rw.len()
    }

    pub fn menu_scroll(&mut self, delta: i64) {
        let len = self.menu_len();
        if len == 0 {
            return;
        }
        let cur = self.menu_state.selected().unwrap_or(0) as i64;
        let next = cur.saturating_add(delta).clamp(0, len as i64 - 1) as usize;
        self.menu_state.select(Some(next));
    }

    pub fn selected_action(&self) -> Option<MenuAction> {
        let i = self.menu_state.selected()?;
        if let Some((name, _)) = self.cmds.get(i) {
            return Some(MenuAction::Cmd(name.clone()));
        }
        self.rw
            .get(i - self.cmds.len())
            .map(|name| MenuAction::Set(name.clone()))
    }

    /// Charge slope in %/s from a least-squares fit over the last 5 minutes.
    fn charge_slope(&self) -> Option<f64> {
        let now = self.now();
        let pts: Vec<(f64, f64)> = self
            .charge
            .iter()
            .copied()
            .filter(|(t, _)| now - t <= 300.0)
            .collect();
        if pts.len() < 5 {
            return None;
        }
        if pts.last()?.0 - pts.first()?.0 < 60.0 {
            return None;
        }
        let n = pts.len() as f64;
        let mx = pts.iter().map(|p| p.0).sum::<f64>() / n;
        let my = pts.iter().map(|p| p.1).sum::<f64>() / n;
        let cov: f64 = pts.iter().map(|p| (p.0 - mx) * (p.1 - my)).sum();
        let var: f64 = pts.iter().map(|p| (p.0 - mx).powi(2)).sum();
        (var > 0.0).then(|| cov / var)
    }

    /// Trend-based prediction from observed charge slope, e.g.
    /// "~34m 10s to 20% cutoff" on battery, "~1h 05m to full" while charging.
    pub fn trend_eta(&self) -> Option<String> {
        let slope = self.charge_slope()?;
        let charge = self.f("battery.charge")?;
        if self.on_battery() && slope < -1e-4 {
            let low = self.f("battery.charge.low").unwrap_or(20.0);
            if charge <= low {
                return Some("below cutoff".into());
            }
            Some(format!(
                "~{} to {low:.0}% cutoff",
                fmt_dur((charge - low) / -slope)
            ))
        } else if slope > 1e-4 && charge < 100.0 {
            Some(format!("~{} to full", fmt_dur((100.0 - charge) / slope)))
        } else {
            None
        }
    }
}

fn get_f(m: &BTreeMap<String, String>, k: &str) -> Option<f64> {
    m.get(k)?.trim().parse().ok()
}

fn push(dq: &mut VecDeque<(f64, f64)>, t: f64, v: f64) {
    dq.push_back((t, v));
    while dq.front().is_some_and(|(ft, _)| t - ft > HIST_SECS + 60.0) {
        dq.pop_front();
    }
}

pub fn fmt_dur(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    if s >= 3600 {
        format!("{}h {:02}m", s / 3600, s / 60 % 60)
    } else {
        format!("{}m {:02}s", s / 60, s % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fmt_dur_ranges() {
        assert_eq!(fmt_dur(0.0), "0m 00s");
        assert_eq!(fmt_dur(59.0), "0m 59s");
        assert_eq!(fmt_dur(2040.0), "34m 00s");
        assert_eq!(fmt_dur(3600.0), "1h 00m");
        assert_eq!(fmt_dur(3900.0), "1h 05m");
        // Negative and NaN must not panic or underflow.
        assert_eq!(fmt_dur(-5.0), "0m 00s");
        assert_eq!(fmt_dur(f64::NAN), "0m 00s");
    }

    #[test]
    fn dangerous_commands() {
        assert!(is_dangerous("shutdown.default"));
        assert!(is_dangerous("shutdown.return"));
        assert!(is_dangerous("load.off"));
        assert!(is_dangerous("load.off.delay"));
        assert!(is_dangerous("driver.killpower"));
        assert!(!is_dangerous("load.on"));
        assert!(!is_dangerous("beeper.disable"));
        assert!(!is_dangerous("test.battery.start.deep"));
    }

    fn app_with(status: &str, charge: f64, history: &[(f64, f64)]) -> App {
        let mut app = App::new("ups".into(), "localhost".into(), 3493);
        app.vars.insert("ups.status".into(), status.into());
        app.vars
            .insert("battery.charge".into(), format!("{charge}"));
        app.vars.insert("battery.charge.low".into(), "20".into());
        for &(t, v) in history {
            app.charge.push_back((t, v));
        }
        app
    }

    #[test]
    fn trend_discharging_predicts_cutoff() {
        // 0.1 %/s discharge: 62% -> 50% over 120s; 50% to the 20% cutoff = 300s.
        let hist = [
            (0.0, 62.0),
            (30.0, 59.0),
            (60.0, 56.0),
            (90.0, 53.0),
            (120.0, 50.0),
        ];
        let app = app_with("OB DISCHRG", 50.0, &hist);
        let eta = app.trend_eta().expect("slope should produce a prediction");
        assert!(eta.contains("5m 00s"), "got: {eta}");
        assert!(eta.contains("cutoff"), "got: {eta}");
    }

    #[test]
    fn trend_charging_predicts_full() {
        // 0.1 %/s charge: 50% to 100% = 500s = 8m 20s.
        let hist = [
            (0.0, 38.0),
            (30.0, 41.0),
            (60.0, 44.0),
            (90.0, 47.0),
            (120.0, 50.0),
        ];
        let app = app_with("OL CHRG", 50.0, &hist);
        let eta = app.trend_eta().expect("slope should produce a prediction");
        assert!(eta.contains("8m 20s"), "got: {eta}");
        assert!(eta.contains("full"), "got: {eta}");
    }

    #[test]
    fn trend_needs_enough_history() {
        // Too few samples.
        let app = app_with("OB", 50.0, &[(0.0, 51.0), (30.0, 50.0)]);
        assert!(app.trend_eta().is_none());
        // Enough samples but too short a time span.
        let hist = [
            (0.0, 55.0),
            (5.0, 54.0),
            (10.0, 53.0),
            (15.0, 52.0),
            (20.0, 51.0),
        ];
        let app = app_with("OB", 51.0, &hist);
        assert!(app.trend_eta().is_none());
    }

    #[test]
    fn trend_flat_is_none() {
        let hist = [
            (0.0, 50.0),
            (30.0, 50.0),
            (60.0, 50.0),
            (90.0, 50.0),
            (120.0, 50.0),
        ];
        let app = app_with("OL CHRG", 50.0, &hist);
        assert!(app.trend_eta().is_none());
    }

    #[test]
    fn scroll_clamps_and_jumps() {
        let mut app = App::new("ups".into(), "localhost".into(), 3493);
        for i in 0..10 {
            app.vars.insert(format!("var{i:02}"), "x".into());
        }
        app.scroll(1);
        assert_eq!(app.table_state.selected(), Some(1));
        app.scroll(i64::MAX); // vim G
        assert_eq!(app.table_state.selected(), Some(9));
        app.scroll(i64::MIN); // vim g
        assert_eq!(app.table_state.selected(), Some(0));
        app.scroll(-1); // already at top: stays clamped
        assert_eq!(app.table_state.selected(), Some(0));
    }

    #[test]
    fn menu_action_indexing() {
        let mut app = App::new("ups".into(), "localhost".into(), 3493);
        app.cmds = vec![
            ("beeper.enable".into(), String::new()),
            ("beeper.disable".into(), String::new()),
        ];
        app.rw = vec!["input.transfer.low".into()];
        assert_eq!(app.menu_len(), 3);
        app.menu_state.select(Some(1));
        assert!(matches!(app.selected_action(), Some(MenuAction::Cmd(c)) if c == "beeper.disable"));
        app.menu_state.select(Some(2));
        assert!(
            matches!(app.selected_action(), Some(MenuAction::Set(n)) if n == "input.transfer.low")
        );
        app.menu_state.select(Some(3));
        assert!(app.selected_action().is_none());
        // menu_scroll must clamp within the combined command+rw range.
        app.menu_scroll(i64::MAX);
        assert_eq!(app.menu_state.selected(), Some(2));
    }

    #[test]
    fn on_battery_token_match() {
        // "OB" must match as a whole token, not as a substring.
        let app = app_with("OB DISCHRG", 50.0, &[]);
        assert!(app.on_battery());
        let app = app_with("OL CHRG", 50.0, &[]);
        assert!(!app.on_battery());
    }
}
