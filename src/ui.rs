use chrono::Local;
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Margin, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph, Scrollbar,
    ScrollbarOrientation, ScrollbarState,
};

use unicode_width::UnicodeWidthChar;

use crate::app::{App, Selection};
use crate::cli::{format_bytes, format_interval};

// Styling follows git-graph: fixed RGB colors (not terminal theme colors,
// which vary between terminals) chosen to stay legible on dark backgrounds,
// a cyan accent for focus and keys, and dark gray for secondary text.
const ACCENT: Color = Color::Cyan;
const DIM: Color = Color::DarkGray;
/// A result's start time identifies it, like a commit hash in git-graph.
const TIME: Color = Color::Yellow;
const BLUE: Color = Color::Rgb(94, 149, 217);
const GREEN: Color = Color::Rgb(102, 187, 102);
const RED: Color = Color::Rgb(217, 106, 106);
const MAGENTA: Color = Color::Rgb(200, 115, 200);
const YELLOW: Color = Color::Rgb(204, 178, 85);
const SELECTED_BG: Color = Color::Rgb(45, 55, 75);
const SELECTION_BG: Color = Color::Rgb(70, 90, 130);

const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const HISTORY_WIDTH: u16 = 20;

pub fn draw(f: &mut Frame, app: &mut App, hostname: &str) {
    let [header, main, footer] = Layout::vertical([
        Constraint::Length(if app.show_title { 1 } else { 0 }),
        Constraint::Min(3),
        Constraint::Length(1),
    ])
    .areas(f.area());

    // The history panel is only useful while browsing it.
    let hist_w = if app.time_machine() && main.width >= 50 { HISTORY_WIDTH } else { 0 };
    let [output, history] =
        Layout::horizontal([Constraint::Min(1), Constraint::Length(hist_w)]).areas(main);

    if app.show_title {
        draw_header(f, app, header, hostname);
    }
    draw_output(f, app, output);
    app.history_area = if hist_w > 0 { history } else { Rect::default() };
    if hist_w > 0 {
        draw_history(f, app, history);
    }
    draw_footer(f, app, footer);
    if app.show_help {
        draw_help(f);
    }
}

fn draw_header(f: &mut Frame, app: &App, area: Rect, hostname: &str) {
    let mut spans = vec![
        Span::styled(" monitor ", Style::new().bg(ACCENT).fg(Color::Black)),
        Span::raw(" "),
        Span::raw(app.command.clone()),
        Span::styled("  every ", Style::new().fg(DIM)),
        Span::styled(format_interval(app.interval), Style::new().fg(BLUE)),
    ];
    if app.precise {
        spans.push(Span::styled(" (precise)", Style::new().fg(DIM)));
    }
    let right = format!("{hostname}  {} ", Local::now().format("%Y-%m-%d %H:%M:%S"));
    let used: usize = spans.iter().map(|s| s.width()).sum();
    let pad = (area.width as usize).saturating_sub(used + right.chars().count());
    spans.push(Span::raw(" ".repeat(pad)));
    spans.push(Span::styled(right, Style::new().fg(DIM)));
    f.render_widget(Line::from(spans), area);
}

fn panel<'a>(title: impl Into<Line<'a>>, focused: bool) -> Block<'a> {
    let mut title: Line = title.into();
    if focused && title.spans.len() == 1 {
        title = title.style(Style::new().fg(ACCENT));
    }
    Block::new()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(if focused { ACCENT } else { DIM }))
        .title(title)
}

/// Live / time machine / suspended, plus a spinner while a run is in progress.
fn mode_line(app: &App) -> Line<'static> {
    let mut spans = vec![];
    if !app.running.is_empty() {
        let i = (Local::now().timestamp_millis() / 100) as usize % SPINNER.len();
        spans.push(Span::styled(format!(" {}", SPINNER[i]), Style::new().fg(ACCENT)));
    }
    if app.time_machine() {
        spans.push(Span::styled(" ◆ time machine", Style::new().fg(MAGENTA)));
    } else {
        spans.push(Span::styled(" ● live", Style::new().fg(GREEN)));
    }
    if app.suspended {
        spans.push(Span::styled(" · suspended", Style::new().fg(YELLOW)));
    }
    spans.push(Span::raw(" "));
    Line::from(spans).alignment(Alignment::Right)
}

fn draw_output(f: &mut Frame, app: &mut App, area: Rect) {
    let title = match app.current() {
        Some(s) => {
            let sep = || Span::styled(" · ", Style::new().fg(DIM));
            let mut spans = vec![
                Span::raw(" "),
                Span::styled(s.start.format("%H:%M:%S%.3f").to_string(), Style::new().fg(TIME)),
                sep(),
                Span::styled(format_duration(s.duration), Style::new().fg(DIM)),
                sep(),
                match s.code {
                    Some(0) => Span::styled("exit 0", Style::new().fg(GREEN)),
                    Some(c) => Span::styled(format!("exit {c}"), Style::new().fg(RED)),
                    None => Span::styled("failed", Style::new().fg(RED)),
                },
            ];
            if s.runs > 1 {
                // Unchanged runs were merged into this result.
                spans.push(sep());
                spans.push(Span::styled(
                    format!("×{} until {}", s.runs, s.last_seen.format("%H:%M:%S")),
                    Style::new().fg(DIM),
                ));
            }
            spans.push(Span::raw(" "));
            Line::from(spans)
        }
        None => Line::raw(" Output "),
    };
    let block = panel(title, !app.time_machine()).title(mode_line(app));
    let inner = block.inner(area);
    f.render_widget(block, area);
    app.view_height = inner.height as usize;
    app.view_width = inner.width as usize;
    app.output_area = inner;

    if app.rows(inner.width).is_none() {
        let msg = if app.running.is_empty() { "No output yet" } else { "Waiting for first run…" };
        f.render_widget(
            Paragraph::new(msg).style(Style::new().fg(DIM)).alignment(Alignment::Center),
            inner,
        );
        return;
    }
    app.after_layout();

    let rows = app.cached_rows().expect("rows built above");
    let total = rows.lines.len();
    let height = inner.height as usize;
    let end = (app.scroll_y + height).min(total);
    let first = app.scroll_y.min(end);
    let visible: Vec<Line> = rows.lines[first..end]
        .iter()
        .enumerate()
        .map(|(i, line)| match app.selection {
            Some(sel) => highlight_selection(line, &sel, first + i),
            None => line.clone(),
        })
        .collect();
    let hscroll = u16::try_from(app.scroll_x).unwrap_or(u16::MAX);
    f.render_widget(Paragraph::new(visible).scroll((0, hscroll)), inner);

    if total > height {
        let mut state = ScrollbarState::new(total.saturating_sub(height))
            .position(app.scroll_y)
            .viewport_content_length(height);
        f.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None)
                .thumb_style(Style::new().fg(ACCENT))
                .track_style(Style::new().fg(DIM)),
            area.inner(Margin { vertical: 1, horizontal: 0 }),
            &mut state,
        );
        let pos = Line::styled(format!(" {}-{}/{} ", app.scroll_y + 1, end, total), Style::new().fg(DIM))
            .alignment(Alignment::Right);
        let bottom = Rect { y: area.bottom() - 1, height: 1, ..area };
        f.render_widget(pos, bottom.inner(Margin { vertical: 0, horizontal: 1 }));
    }
}

/// The style of selected text. Reverse video (used for differences) is dropped,
/// or it would swap the selection background into the text color.
fn selected_style(style: Style) -> Style {
    style.remove_modifier(Modifier::REVERSED).bg(SELECTION_BG)
}

/// Gives the selected columns of `line` (content row `row`) a selection background.
fn highlight_selection(line: &Line<'static>, sel: &Selection, row: usize) -> Line<'static> {
    let Some((from, to)) = sel.columns(row) else { return line.clone() };
    let mut spans = Vec::new();
    let mut col = 0;
    for span in &line.spans {
        let mut plain = String::new();
        let mut marked = String::new();
        for c in span.content.chars() {
            let w = c.width().unwrap_or(0);
            let inside = col < to && col + w.max(1) > from;
            // Flush when switching between selected and unselected text.
            if inside && !plain.is_empty() {
                spans.push(Span::styled(std::mem::take(&mut plain), span.style));
            } else if !inside && !marked.is_empty() {
                spans.push(Span::styled(std::mem::take(&mut marked), selected_style(span.style)));
            }
            if inside { marked.push(c) } else { plain.push(c) }
            col += w;
        }
        if !plain.is_empty() {
            spans.push(Span::styled(plain, span.style));
        }
        if !marked.is_empty() {
            spans.push(Span::styled(marked, selected_style(span.style)));
        }
    }
    Line::from(spans)
}

fn draw_history(f: &mut Frame, app: &mut App, area: Rect) {
    let n = app.history.len();
    let items: Vec<ListItem> = app
        .history
        .iter()
        .rev()
        .map(|s| {
            // Failed runs are red; with --keep-unchanged, repeats of the
            // previous output are dimmed.
            let time = if !s.success() {
                Style::new().fg(RED)
            } else if s.changed || s.runs > 1 {
                Style::new().fg(TIME)
            } else {
                Style::new().fg(DIM)
            };
            let runs = if s.runs > 1 { format!(" ×{}", s.runs) } else { String::new() };
            ListItem::new(Line::from(vec![
                Span::styled(s.start.format("%H:%M:%S").to_string(), time),
                Span::styled(runs, Style::new().fg(DIM)),
            ]))
        })
        .collect();

    let block = panel(format!(" History ({n}) "), true).title_bottom(
        Line::styled(format!(" {} ", format_bytes(app.history_bytes)), Style::new().fg(DIM))
            .alignment(Alignment::Right),
    );
    let list = List::new(items)
        .block(block)
        .highlight_style(Style::new().bg(SELECTED_BG))
        .highlight_symbol("▶ ");
    let mut state = ListState::default()
        .with_selected(app.current_index().map(|cur| n - 1 - cur))
        .with_offset(app.history_offset);
    f.render_stateful_widget(list, area, &mut state);
    // Keep the scroll position so the list doesn't jump and clicks map to rows.
    app.history_offset = state.offset();
}

fn key_hints(keys: &[(&str, &str)]) -> Vec<Span<'static>> {
    let mut spans = vec![Span::raw(" ")];
    for (k, v) in keys {
        spans.push(Span::styled(k.to_string(), Style::new().fg(ACCENT)));
        spans.push(Span::styled(format!(" {v}  "), Style::new().fg(DIM)));
    }
    spans
}

fn draw_footer(f: &mut Frame, app: &App, area: Rect) {
    let left = if let Some(input) = &app.search_input {
        Line::from(vec![
            Span::styled(" / ", Style::new().fg(ACCENT)),
            Span::raw(input.clone()),
            Span::styled("█", Style::new().fg(ACCENT)),
            Span::styled("   enter search  esc cancel", Style::new().fg(DIM)),
        ])
    } else if let Some(msg) = &app.message {
        Line::styled(format!(" {msg}"), Style::new().fg(Color::Yellow))
    } else if app.time_machine() {
        Line::from(key_hints(&[
            ("J/K", "older/newer"),
            ("</>", "oldest/newest"),
            ("j/k", "scroll"),
            ("/", "search"),
            ("esc", "live"),
            ("?", "help"),
        ]))
    } else {
        Line::from(key_hints(&[
            ("space", "time machine"),
            ("j/k", "scroll"),
            ("/", "search"),
            ("s", if app.suspended { "resume" } else { "suspend" }),
            ("?", "help"),
            ("q", "quit"),
        ]))
    };

    let toggle = |name: &'static str, on: bool| {
        Span::styled(format!("{name} "), Style::new().fg(if on { ACCENT } else { DIM }))
    };
    let mut right = vec![];
    if let Some(q) = &app.query {
        let count = app.cached_rows().map(|r| r.matches.len()).unwrap_or(0);
        right.push(Span::styled(format!("  /{q} [{count}]  "), Style::new().fg(Color::Yellow)));
    }
    right.extend([toggle("diff", app.diff), toggle("bell", app.bell), toggle("wrap", app.wrap)]);
    let right = Line::from(right);
    let rw = right.width() as u16;
    let [l, r] = Layout::horizontal([Constraint::Min(0), Constraint::Length(rw)]).areas(area);
    f.render_widget(left, l);
    f.render_widget(right, r);
}

fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width.saturating_sub(2));
    let h = h.min(area.height.saturating_sub(2));
    Rect::new(area.x + (area.width - w) / 2, area.y + (area.height - h) / 2, w, h)
}

fn draw_help(f: &mut Frame) {
    let rows: &[(&str, &str)] = &[
        ("General", ""),
        ("q, ctrl-c", "quit"),
        ("?", "toggle this help"),
        ("s", "suspend / resume running the command"),
        ("r", "run the command now"),
        ("Time machine", ""),
        ("space", "toggle time machine (pause the view, show history)"),
        ("J / shift-↓", "older result"),
        ("K / shift-↑", "newer result"),
        ("< / >", "oldest / newest result"),
        ("esc", "back to the live view"),
        ("Output", ""),
        ("j/k ↑/↓", "scroll"),
        ("ctrl-d/u", "half page down / up"),
        ("PgDn/PgUp, ctrl-f/b", "page down / up"),
        ("g / G", "top / bottom"),
        ("mouse wheel", "scroll; over the history, older / newer result"),
        ("click", "select a result in the history"),
        ("drag", "select output text; copied to the clipboard on release"),
        ("shift-drag", "the terminal's own selection"),
        ("h/l ←/→", "scroll sideways (when not wrapping)"),
        ("0/$, home/end", "line start / end (when not wrapping)"),
        ("d / b / w / t", "toggle diff / bell / wrap / title"),
        ("Search", ""),
        ("/", "search (smart case)"),
        ("n / N", "next / previous match"),
        ("esc", "clear the search"),
    ];
    let lines: Vec<Line> = rows
        .iter()
        .map(|(k, v)| {
            if v.is_empty() {
                Line::styled(format!(" {k}"), Style::new().fg(ACCENT))
            } else {
                Line::from(vec![
                    Span::styled(format!("   {k:<22}"), Style::new().fg(Color::Yellow)),
                    Span::raw(*v),
                ])
            }
        })
        .collect();
    let area = centered(f.area(), 80, lines.len() as u16 + 2);
    f.render_widget(Clear, area);
    f.render_widget(Paragraph::new(lines).block(panel(" Help — any key to close ", true)), area);
}

fn format_duration(d: std::time::Duration) -> String {
    let ms = d.as_secs_f64() * 1000.0;
    if ms < 1000.0 { format!("{ms:.0}ms") } else { format!("{:.2}s", ms / 1000.0) }
}
