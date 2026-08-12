//! Rendering: btm-style braille charts, gauges, buttons, full variable table.

use std::collections::VecDeque;

use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::symbols::Marker;
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Axis, Block, Chart, Clear, Dataset, Gauge, GraphType, List, ListItem, Paragraph, Row, Table,
};
use ratatui::Frame;

use crate::app::{fmt_dur, is_dangerous, App, Mode, HIST_SECS};

pub const BUTTONS: [(&str, char, &str); 4] = [
    ("Quick test", 't', "test.battery.start.quick"),
    ("Deep test", 'd', "test.battery.start.deep"),
    ("Stop test", 's', "test.battery.stop"),
    // Sentinel resolved at dispatch: most UPSes lack beeper.toggle and only
    // offer discrete beeper.enable / beeper.disable.
    ("Beeper", 'b', "beeper"),
];

/// Muted style for secondary text. Deliberately the DIM attribute rather
/// than Color::DarkGray: DarkGray is ANSI bright-black, which several
/// common palettes (e.g. Solarized dark) map to the background color,
/// rendering the text invisible.
fn gray() -> Style {
    Style::new().add_modifier(Modifier::DIM)
}

pub fn draw(f: &mut Frame, app: &mut App) {
    app.buttons.clear();
    if app.basic {
        // Basic mode: just the summary panel and the footer.
        let rows = Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).split(f.area());
        draw_status(f, app, rows[0]);
        draw_footer(f, app, rows[1]);
    } else {
        let rows = Layout::vertical([
            Constraint::Percentage(45),
            Constraint::Min(15),
            Constraint::Length(1),
        ])
        .split(f.area());
        let charts = Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(rows[0]);
        draw_battery_chart(f, app, charts[0]);
        draw_voltage_chart(f, app, charts[1]);
        let main = Layout::horizontal([Constraint::Percentage(46), Constraint::Percentage(54)])
            .split(rows[1]);
        draw_status(f, app, main[0]);
        draw_vars(f, app, main[1]);
        draw_footer(f, app, rows[2]);
    }
    // Not a match on &app.mode: the Menu arm needs app mutably.
    if matches!(app.mode, Mode::Help) {
        draw_help_popup(f);
    } else if matches!(app.mode, Mode::Menu) {
        draw_menu_popup(f, app);
    } else if let Mode::Confirm(cmd) = &app.mode {
        let cmd = cmd.clone();
        draw_confirm_popup(f, &cmd);
    } else if let Mode::SetVar { name, buffer } = &app.mode {
        let (name, buffer) = (name.clone(), buffer.clone());
        draw_setvar_popup(f, app, &name, &buffer);
    }
}

/// A centered popup area, cleared of whatever was underneath.
fn popup_area(f: &mut Frame, width: u16, height: u16) -> Rect {
    let area = f.area();
    let w = width.min(area.width.saturating_sub(2));
    let h = height.min(area.height.saturating_sub(2));
    let rect = Rect::new(
        area.x + (area.width.saturating_sub(w)) / 2,
        area.y + (area.height.saturating_sub(h)) / 2,
        w,
        h,
    );
    f.render_widget(Clear, rect);
    rect
}

fn draw_help_popup(f: &mut Frame) {
    let keys: [(&str, &str); 10] = [
        ("q / Esc", "quit (or close this popup)"),
        ("?", "toggle this help"),
        ("m", "menu: commands & settable variables"),
        ("t / d / s", "quick test · deep test · stop test"),
        ("b", "toggle beeper on/off"),
        ("↑ ↓ / k j", "scroll variables / menu"),
        ("g / G", "jump to top / bottom"),
        ("h / l", "in menu: close / run selected"),
        ("PgUp PgDn", "scroll faster"),
        ("mouse", "click the buttons"),
    ];
    let lines: Vec<Line> = keys
        .iter()
        .map(|(k, v)| {
            Line::from(vec![
                Span::styled(format!("  {k:<15}"), Style::new().fg(Color::Cyan).bold()),
                Span::raw(*v),
            ])
        })
        .collect();
    let rect = popup_area(f, 62, lines.len() as u16 + 2);
    f.render_widget(
        Paragraph::new(lines).block(Block::bordered().title(" Help — press ? or Esc to close ")),
        rect,
    );
}

fn draw_menu_popup(f: &mut Frame, app: &mut App) {
    if app.menu_len() == 0 {
        let rect = popup_area(f, 56, 3);
        f.render_widget(
            Paragraph::new("command list not loaded yet (server unreachable?)")
                .alignment(Alignment::Center)
                .block(Block::bordered().title(" Commands ")),
            rect,
        );
        return;
    }
    let mut items: Vec<ListItem> = app
        .cmds
        .iter()
        .map(|(name, desc)| {
            let name_style = if is_dangerous(name) {
                Style::new().fg(Color::Red)
            } else {
                Style::new().fg(Color::Cyan)
            };
            ListItem::new(Line::from(vec![
                Span::styled(format!(" {name:<28}"), name_style),
                Span::styled(desc.clone(), gray()),
            ]))
        })
        .collect();
    // Writable variables follow the commands; Enter opens the value editor.
    items.extend(app.rw.iter().map(|name| {
        ListItem::new(Line::from(vec![
            Span::styled(format!(" set {name:<24}"), Style::new().fg(Color::Yellow)),
            Span::styled(format!("current {} · Enter to edit", app.s(name)), gray()),
        ]))
    }));
    let rect = popup_area(f, 78, app.menu_len() as u16 + 2);
    let list = List::new(items)
        .block(Block::bordered().title(
            " Commands & settings — Enter runs or edits · red needs y/N confirm · Esc closes ",
        ))
        .highlight_style(Style::new().add_modifier(Modifier::REVERSED));
    f.render_stateful_widget(list, rect, &mut app.menu_state);
}

fn draw_setvar_popup(f: &mut Frame, app: &App, name: &str, buffer: &str) {
    let rect = popup_area(f, 46, 6);
    let lines = vec![
        Line::from(Span::styled(
            name.to_string(),
            Style::new().fg(Color::Yellow).bold(),
        )),
        Line::from(format!("current: {}", app.s(name))),
        Line::from(vec![
            Span::raw("new: "),
            Span::styled(format!("{buffer}_"), Style::new().fg(Color::Cyan).bold()),
        ]),
        Line::from(Span::styled("digits · Enter apply · Esc cancel", gray())),
    ];
    f.render_widget(
        Paragraph::new(lines).alignment(Alignment::Center).block(
            Block::bordered()
                .border_style(Style::new().fg(Color::Yellow))
                .title(" Set variable "),
        ),
        rect,
    );
}

fn draw_confirm_popup(f: &mut Frame, cmd: &str) {
    let rect = popup_area(f, 60, 5);
    let lines = vec![
        Line::from(vec![
            Span::raw("Really run "),
            Span::styled(cmd, Style::new().fg(Color::Red).bold()),
            Span::raw("?"),
        ]),
        Line::from("This can cut output power or shut the system down."),
        Line::from(vec![
            Span::styled("y", Style::new().fg(Color::Red).bold()),
            Span::raw(" to confirm · any other key to cancel"),
        ]),
    ];
    f.render_widget(
        Paragraph::new(lines).alignment(Alignment::Center).block(
            Block::bordered()
                .border_style(Style::new().fg(Color::Red))
                .title(" Confirm "),
        ),
        rect,
    );
}

/// Map absolute-time points to seconds-before-now for the chart x axis.
fn rel(points: &VecDeque<(f64, f64)>, now: f64) -> Vec<(f64, f64)> {
    points
        .iter()
        .filter(|(t, _)| now - t <= HIST_SECS)
        .map(|&(t, v)| (t - now, v))
        .collect()
}

fn draw_battery_chart(f: &mut Frame, app: &App, area: Rect) {
    let now = app.now();
    let data = rel(&app.charge, now);
    let low = app.f("battery.charge.low").unwrap_or(20.0);
    let cutoff = [(-HIST_SECS, low), (0.0, low)];
    let charge_name = app
        .f("battery.charge")
        .map(|c| format!("charge {c:.0}%"))
        .unwrap_or_else(|| "charge".into());
    let datasets = vec![
        Dataset::default()
            .name(format!("cutoff {low:.0}%"))
            .marker(Marker::Braille)
            .graph_type(GraphType::Line)
            .style(gray())
            .data(&cutoff),
        Dataset::default()
            .name(charge_name)
            .marker(Marker::Braille)
            .graph_type(GraphType::Line)
            .style(Style::new().fg(Color::Green))
            .data(&data),
    ];
    let chart = Chart::new(datasets)
        .block(Block::bordered().title(" Battery charge "))
        .x_axis(
            Axis::default()
                .style(gray())
                .bounds([-HIST_SECS, 0.0])
                .labels(["-15m", "-10m", "-5m", "now"]),
        )
        .y_axis(
            Axis::default()
                .style(gray())
                .bounds([0.0, 100.0])
                .labels(["0%", "50%", "100%"]),
        );
    f.render_widget(chart, area);
}

fn draw_voltage_chart(f: &mut Frame, app: &App, area: Rect) {
    let now = app.now();
    let vin = rel(&app.volt_in, now);
    let vout = rel(&app.volt_out, now);
    let mut lo = f64::INFINITY;
    let mut hi = f64::NEG_INFINITY;
    for &(_, v) in vin.iter().chain(vout.iter()) {
        lo = lo.min(v);
        hi = hi.max(v);
    }
    if !lo.is_finite() {
        (lo, hi) = (180.0, 260.0);
    }
    let (lo, hi) = ((lo - 5.0).floor(), (hi + 5.0).ceil());
    let name_in = app
        .f("input.voltage")
        .map(|v| format!("in {v:.1}V"))
        .unwrap_or_else(|| "in".into());
    let name_out = app
        .f("output.voltage")
        .map(|v| format!("out {v:.1}V"))
        .unwrap_or_else(|| "out".into());
    let datasets = vec![
        Dataset::default()
            .name(name_in)
            .marker(Marker::Braille)
            .graph_type(GraphType::Line)
            .style(Style::new().fg(Color::Yellow))
            .data(&vin),
        Dataset::default()
            .name(name_out)
            .marker(Marker::Braille)
            .graph_type(GraphType::Line)
            .style(Style::new().fg(Color::Cyan))
            .data(&vout),
    ];
    let title = match (app.f("input.transfer.low"), app.f("input.transfer.high")) {
        (Some(a), Some(b)) => format!(" Voltage · transfer window {a:.0}–{b:.0} V "),
        _ => " Voltage ".to_string(),
    };
    let chart = Chart::new(datasets)
        .block(Block::bordered().title(title))
        .x_axis(
            Axis::default()
                .style(gray())
                .bounds([-HIST_SECS, 0.0])
                .labels(["-15m", "-10m", "-5m", "now"]),
        )
        .y_axis(Axis::default().style(gray()).bounds([lo, hi]).labels([
            format!("{lo:.0}V"),
            format!("{:.0}V", (lo + hi) / 2.0),
            format!("{hi:.0}V"),
        ]));
    f.render_widget(chart, area);
}

fn status_token(tok: &str) -> (String, Color) {
    let (label, color) = match tok {
        "OL" => ("ONLINE", Color::Green),
        "OB" => ("ON BATTERY", Color::Yellow),
        "LB" => ("LOW BATTERY", Color::Red),
        "HB" => ("HIGH BATTERY", Color::Yellow),
        "CHRG" => ("CHARGING", Color::Cyan),
        "DISCHRG" => ("DISCHARGING", Color::Yellow),
        "RB" => ("REPLACE BATTERY", Color::Red),
        "BYPASS" => ("BYPASS", Color::Red),
        "OFF" => ("OUTPUT OFF", Color::Red),
        "OVER" => ("OVERLOAD", Color::Red),
        "TRIM" => ("TRIMMING", Color::Yellow),
        "BOOST" => ("BOOSTING", Color::Yellow),
        "CAL" => ("CALIBRATING", Color::Cyan),
        "FSD" => ("FORCED SHUTDOWN", Color::Red),
        other => (other, Color::White),
    };
    (label.to_string(), color)
}

fn kv(label: &str, value: impl Into<String>) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{label:<9}"), gray()),
        Span::raw(value.into()),
    ])
}

fn draw_status(f: &mut Frame, app: &mut App, area: Rect) {
    let title = format!(
        " {} {} · SN {} ",
        app.s("device.mfr").trim(),
        app.s("device.model").trim(),
        app.s("device.serial").trim()
    );
    let block = Block::bordered().title(title);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let secs = Layout::vertical([
        Constraint::Length(8), // info lines
        Constraint::Length(1), // charge gauge
        Constraint::Length(1), // load gauge
        Constraint::Length(3), // buttons
        Constraint::Min(2),    // log
    ])
    .split(inner);

    // Info lines
    let mut status_spans: Vec<Span> = vec![Span::styled("Status   ", gray())];
    if app.status().is_empty() {
        status_spans.push(Span::styled(
            "no data yet",
            gray().add_modifier(Modifier::ITALIC),
        ));
    } else {
        let toks: Vec<&str> = app.status().split_whitespace().collect();
        for (i, tok) in toks.iter().enumerate() {
            let (label, color) = status_token(tok);
            status_spans.push(Span::styled(label, Style::new().fg(color).bold()));
            if i + 1 < toks.len() {
                status_spans.push(Span::styled(" · ", gray()));
            }
        }
    }
    let runtime = app
        .f("battery.runtime")
        .map(fmt_dur)
        .unwrap_or_else(|| "–".into());
    let low_rt = app
        .f("battery.runtime.low")
        .map(|s| format!(" (shutdown below {})", fmt_dur(s)))
        .unwrap_or_default();
    let trend = app.trend_eta().unwrap_or_else(|| "steady".into());
    let batt = format!(
        "{} V (nom {} V) · {}",
        app.s("battery.voltage"),
        app.s("battery.voltage.nominal"),
        app.s("battery.type")
    );
    let mains = format!(
        "in {} V · {} Hz   out {} V · {} Hz",
        app.s("input.voltage"),
        app.s("input.frequency"),
        app.s("output.voltage"),
        app.s("output.frequency")
    );
    let misc = format!(
        "temp {} °C · beeper {}",
        app.s("ups.temperature"),
        app.s("ups.beeper.status")
    );
    let lines = vec![
        Line::from(status_spans),
        kv("Runtime", format!("{runtime}{low_rt}")),
        kv("Trend", trend),
        kv("Battery", batt),
        kv("Power", mains),
        kv("Misc", misc),
        kv("Test", app.s("ups.test.result").to_string()),
        Line::default(),
    ];
    f.render_widget(Paragraph::new(lines), secs[0]);

    // Gauges
    let charge = app.f("battery.charge").unwrap_or(0.0);
    let charge_color = if charge > 50.0 {
        Color::Green
    } else if charge > app.f("battery.charge.low").unwrap_or(20.0) {
        Color::Yellow
    } else {
        Color::Red
    };
    f.render_widget(
        Gauge::default()
            .ratio((charge / 100.0).clamp(0.0, 1.0))
            .label(format!("battery {charge:.0}%"))
            .gauge_style(Style::new().fg(charge_color).bg(Color::Black)),
        secs[1],
    );
    let load = app.f("ups.load").unwrap_or(0.0);
    let watts = app
        .f("ups.realpower.nominal")
        .map(|n| format!(" · ~{:.0} W", n * load / 100.0))
        .unwrap_or_default();
    let load_color = if load < 60.0 {
        Color::Cyan
    } else if load < 85.0 {
        Color::Yellow
    } else {
        Color::Red
    };
    f.render_widget(
        Gauge::default()
            .ratio((load / 100.0).clamp(0.0, 1.0))
            .label(format!("load {load:.0}%{watts}"))
            .gauge_style(Style::new().fg(load_color).bg(Color::Black)),
        secs[2],
    );

    // Buttons (clickable + keyboard)
    let bcols = Layout::horizontal([Constraint::Ratio(1, 4); 4]).split(secs[3]);
    for (i, (label, key, cmd)) in BUTTONS.iter().enumerate() {
        let p = Paragraph::new(format!("{label} ({key})"))
            .alignment(Alignment::Center)
            .style(Style::new().fg(Color::Cyan))
            .block(Block::bordered().border_style(gray()));
        f.render_widget(p, bcols[i]);
        app.buttons.push((bcols[i], cmd.to_string()));
    }

    // Log tail
    let visible = secs[4].height as usize;
    let text: Vec<Line> = app
        .log
        .iter()
        .rev()
        .take(visible)
        .rev()
        .map(|l| Line::from(l.as_str()).style(gray()))
        .collect();
    f.render_widget(Paragraph::new(text), secs[4]);
}

fn draw_vars(f: &mut Frame, app: &mut App, area: Rect) {
    let rows: Vec<Row> = app
        .vars
        .iter()
        .map(|(k, v)| Row::new([k.clone(), v.clone()]))
        .collect();
    let table = Table::new(rows, [Constraint::Fill(2), Constraint::Fill(3)])
        .header(Row::new(["variable", "value"]).style(gray().bold()))
        .block(Block::bordered().title(format!(" All UPS data · {} vars ", app.vars.len())))
        .row_highlight_style(Style::new().add_modifier(Modifier::REVERSED));
    f.render_stateful_widget(table, area, &mut app.table_state);
}

fn draw_footer(f: &mut Frame, app: &App, area: Rect) {
    let dot = if app.connected {
        Span::styled("●", Style::new().fg(Color::Green))
    } else {
        Span::styled("●", Style::new().fg(Color::Red))
    };
    let mut spans = vec![
        Span::raw(" "),
        dot,
        Span::raw(format!(" {}@{}:{}  ", app.ups, app.host, app.port)),
    ];
    if let Some(e) = &app.last_error {
        spans.push(Span::styled(format!("{e}  "), Style::new().fg(Color::Red)));
    }
    spans.push(Span::styled(
        "q quit · m menu · t/d/s tests · b beeper · ↑↓ scroll",
        gray(),
    ));
    let cols = Layout::horizontal([Constraint::Min(0), Constraint::Length(20)]).split(area);
    f.render_widget(Paragraph::new(Line::from(spans)), cols[0]);
    f.render_widget(
        Paragraph::new(Span::styled("Press '?' for help ", gray().italic()))
            .alignment(Alignment::Right),
        cols[1],
    );
}
