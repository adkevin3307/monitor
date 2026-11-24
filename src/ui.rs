use chrono::Local;
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Clear, List, ListItem, ListState, Paragraph, Scrollbar,
    ScrollbarOrientation, ScrollbarState,
};

use crate::app::App;
use crate::cli::format_interval;

const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const ACCENT: Color = Color::Cyan;

pub fn draw(f: &mut Frame, app: &mut App, hostname: &str) {
    let [title, main, footer] = Layout::vertical([
        Constraint::Length(if app.show_title { 1 } else { 0 }),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(f.area());

    let hist_w = if app.show_history && main.width >= 50 { 22 } else { 0 };
    let [body, hist] =
        Layout::horizontal([Constraint::Min(1), Constraint::Length(hist_w)]).areas(main);

    if app.show_title {
        draw_title(f, app, title, hostname);
    }
    draw_body(f, app, body);
    if hist_w > 0 {
        draw_history(f, app, hist);
    }
    draw_footer(f, app, footer);
    if app.show_help {
        draw_help(f);
    }
}

fn draw_title(f: &mut Frame, app: &App, area: Rect, hostname: &str) {
    let mut left = vec![
        Span::raw(" Every ").dim(),
        Span::raw(format_interval(app.interval)).fg(ACCENT).bold(),
    ];
    if app.precise {
        left.push(Span::raw(" (precise)").dim());
    }
    left.push(Span::raw(": ").dim());
    left.push(Span::raw(app.command.clone()).bold());

    let right = Line::from(vec![
        Span::raw(hostname.to_owned()).dim(),
        Span::raw("  "),
        Span::raw(Local::now().format("%Y-%m-%d %H:%M:%S").to_string()),
        Span::raw(" "),
    ])
    .alignment(Alignment::Right);

    let rw = right.width() as u16;
    let [l, r] = Layout::horizontal([Constraint::Min(0), Constraint::Length(rw)]).areas(area);
    f.render_widget(Paragraph::new(Line::from(left)), l);
    f.render_widget(Paragraph::new(right), r);
}

fn mode_badge(app: &App) -> Line<'static> {
    let mut spans = Vec::new();
    if !app.running.is_empty() {
        let i = (Local::now().timestamp_millis() / 100) as usize % SPINNER.len();
        spans.push(Span::raw(format!(" {} ", SPINNER[i])).fg(ACCENT));
    }
    let (label, color) = if app.time_machine() {
        (" TIME MACHINE ", Color::Magenta)
    } else {
        (" LIVE ", Color::Green)
    };
    spans.push(Span::styled(label, Style::new().fg(Color::Black).bg(color).bold()));
    if app.suspended {
        spans.push(Span::raw(" "));
        spans.push(Span::styled(" SUSPENDED ", Style::new().fg(Color::Black).bg(Color::Yellow).bold()));
    }
    spans.push(Span::raw(" "));
    Line::from(spans).alignment(Alignment::Right)
}

fn draw_body(f: &mut Frame, app: &mut App, area: Rect) {
    let mut block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(if app.time_machine() { Color::Magenta } else { Color::DarkGray }))
        .title_top(mode_badge(app));

    if let Some(s) = app.current() {
        let mut info = vec![
            Span::raw(" "),
            Span::raw(format!("#{}", s.id)).fg(ACCENT).bold(),
            Span::raw("  "),
            Span::raw(s.start.format("%H:%M:%S%.3f").to_string()),
            Span::raw("  "),
            Span::raw(format_duration(s.duration)).dim(),
            Span::raw("  "),
        ];
        info.push(match s.code {
            Some(0) => Span::raw("exit 0").fg(Color::Green),
            Some(c) => Span::raw(format!("exit {c}")).fg(Color::Red).bold(),
            None => Span::raw("failed").fg(Color::Red).bold(),
        });
        info.push(Span::raw(" "));
        block = block.title_top(Line::from(info));
    }

    let inner = block.inner(area);
    f.render_widget(block, area);
    app.view_height = inner.height as usize;
    app.view_width = inner.width as usize;

    if app.rows(inner.width).is_none() {
        let msg = if app.running.is_empty() { "No output yet" } else { "Waiting for first run…" };
        f.render_widget(Paragraph::new(msg).dim().alignment(Alignment::Center), inner);
        return;
    }
    app.after_layout();

    let rows = app.cached_rows().expect("rows built above");
    let total = rows.lines.len();
    let end = (app.scroll_y + inner.height as usize).min(total);
    let visible: Vec<Line> = rows.lines[app.scroll_y.min(end)..end].to_vec();
    f.render_widget(Paragraph::new(visible).scroll((0, app.scroll_x as u16)), inner);

    if total > inner.height as usize {
        let mut state = ScrollbarState::new(total.saturating_sub(inner.height as usize))
            .position(app.scroll_y)
            .viewport_content_length(inner.height as usize);
        f.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None)
                .thumb_style(Style::new().fg(ACCENT))
                .track_style(Style::new().fg(Color::DarkGray)),
            area.inner(ratatui::layout::Margin { vertical: 1, horizontal: 0 }),
            &mut state,
        );
        let pos = Line::from(format!(" {}-{}/{} ", app.scroll_y + 1, end, total))
            .dim()
            .alignment(Alignment::Right);
        f.render_widget(pos, Rect { y: area.bottom() - 1, height: 1, ..area }.inner(
            ratatui::layout::Margin { vertical: 0, horizontal: 1 },
        ));
    }
}

fn draw_history(f: &mut Frame, app: &App, area: Rect) {
    let n = app.history.len();
    let items: Vec<ListItem> = app
        .history
        .iter()
        .rev()
        .map(|s| {
            let id_style = if s.success() { Style::new().fg(ACCENT) } else { Style::new().fg(Color::Red) };
            let mut time_style = Style::new();
            if !s.changed {
                time_style = time_style.add_modifier(Modifier::DIM);
            }
            ListItem::new(Line::from(vec![
                Span::styled(if s.changed { "● " } else { "  " }, Style::new().fg(Color::Yellow)),
                Span::styled(format!("{:>5}", s.id), id_style),
                Span::raw(" "),
                Span::styled(s.start.format("%H:%M:%S").to_string(), time_style),
            ]))
        })
        .collect();

    let mut state = ListState::default();
    if let Some(cur) = app.current_index() {
        state.select(Some(n - 1 - cur));
    }
    let highlight = if app.time_machine() {
        Style::new().bg(Color::Magenta).fg(Color::Black)
    } else {
        Style::new().add_modifier(Modifier::REVERSED)
    };
    let list = List::new(items)
        .block(
            Block::bordered()
                .border_type(BorderType::Rounded)
                .border_style(Style::new().fg(Color::DarkGray))
                .title(Line::from(format!(" History ({n}) ")).bold()),
        )
        .highlight_style(highlight);
    f.render_stateful_widget(list, area, &mut state);
}

fn draw_footer(f: &mut Frame, app: &App, area: Rect) {
    let left = if let Some(input) = &app.search_input {
        Line::from(vec![
            Span::raw(" /").fg(ACCENT).bold(),
            Span::raw(input.clone()),
            Span::raw("█").fg(ACCENT),
        ])
    } else if let Some(msg) = &app.message {
        Line::from(Span::raw(format!(" {msg}")).fg(Color::Yellow))
    } else {
        let mut spans = Vec::new();
        for (k, d) in [
            ("q", "quit"),
            ("space", "time machine"),
            ("J/K", "history"),
            ("s", "suspend"),
            ("/", "search"),
            ("?", "help"),
        ] {
            spans.push(Span::raw(format!(" {k}")).fg(ACCENT).bold());
            spans.push(Span::raw(format!(" {d} ")).dim());
        }
        Line::from(spans)
    };

    let toggle = |name: &'static str, on: bool| {
        if on {
            Span::styled(format!(" {name} "), Style::new().fg(Color::Black).bg(ACCENT))
        } else {
            Span::styled(format!(" {name} "), Style::new().fg(Color::DarkGray))
        }
    };
    let mut right = vec![toggle("diff", app.diff), toggle("bell", app.bell), toggle("wrap", app.wrap)];
    if let Some(q) = &app.query {
        let count = app.cached_rows().map(|r| r.matches.len()).unwrap_or(0);
        right.insert(0, Span::raw(format!(" /{q} [{count}] ")).fg(Color::Yellow));
    }
    let right = Line::from(right).alignment(Alignment::Right);
    let rw = right.width() as u16;
    let [l, r] = Layout::horizontal([Constraint::Min(0), Constraint::Length(rw)]).areas(area);
    f.render_widget(Paragraph::new(left), l);
    f.render_widget(Paragraph::new(right), r);
}

fn draw_help(f: &mut Frame) {
    let sections: [(&str, &[(&str, &str)]); 4] = [
        ("General", &[
            ("q, Ctrl-C", "quit"),
            ("?", "toggle this help"),
            ("s", "suspend / resume running the command"),
            ("r", "run the command now"),
        ]),
        ("Time machine", &[
            ("space", "toggle time machine (pause the view)"),
            ("J / Shift-↓", "older result"),
            ("K / Shift-↑", "newer result"),
            ("< / >", "oldest / newest result"),
            ("Esc", "back to live view"),
        ]),
        ("View", &[
            ("j k ↑ ↓", "scroll"),
            ("Ctrl-D / Ctrl-U", "half page down / up"),
            ("PgDn PgUp Ctrl-F/B", "page down / up"),
            ("gg / G", "top / bottom"),
            ("h l ← → 0", "scroll horizontally (when unwrapped)"),
            ("d  b  w  t  H", "toggle diff / bell / wrap / title / history"),
        ]),
        ("Search", &[
            ("/", "search (smart case)"),
            ("n / N", "next / previous match"),
            ("Esc", "clear search"),
        ]),
    ];

    let mut lines = Vec::new();
    for (title, keys) in sections {
        if !lines.is_empty() {
            lines.push(Line::raw(""));
        }
        lines.push(Line::from(Span::raw(title).fg(ACCENT).bold()));
        for (k, d) in keys {
            lines.push(Line::from(vec![
                Span::raw(format!("  {k:<20}")).bold(),
                Span::raw(d.to_string()),
            ]));
        }
    }

    let area = f.area();
    let w = 66.min(area.width);
    let h = (lines.len() as u16 + 2).min(area.height);
    let rect = Rect { x: area.x + (area.width - w) / 2, y: area.y + (area.height - h) / 2, width: w, height: h };
    f.render_widget(Clear, rect);
    f.render_widget(
        Paragraph::new(lines).block(
            Block::bordered()
                .border_type(BorderType::Rounded)
                .border_style(Style::new().fg(ACCENT))
                .title(Line::from(" monitor — keys ").bold())
                .title_bottom(Line::from(" press any key ").dim().alignment(Alignment::Right)),
        ),
        rect,
    );
}

fn format_duration(d: std::time::Duration) -> String {
    let ms = d.as_secs_f64() * 1000.0;
    if ms < 1000.0 { format!("{ms:.0}ms") } else { format!("{:.2}s", ms / 1000.0) }
}
