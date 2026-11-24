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
    let hist_w = if app.time_machine() && main.width >= 50 {
        HISTORY_WIDTH
    } else {
        0
    };
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
    if app.search_open {
        draw_search(f, app);
    }
    if app.show_help {
        draw_help(f, app);
    }
}

/// The search box, the same as in track and git-graph.
fn draw_search(f: &mut Frame, app: &App) {
    let info = (!app.editor.text().is_empty()).then(|| match app.match_position().1 {
        0 => Span::styled("no match", Style::new().fg(RED)),
        1 => Span::styled("1 match", Style::new().fg(DIM)),
        n => Span::styled(format!("{n} matches"), Style::new().fg(DIM)),
    });
    draw_input(
        f,
        "Search",
        "\u{f002}",
        app.editor.text(),
        app.editor.cursor(),
        info,
        &[("enter", "search"), ("↑/↓", "history"), ("esc", "cancel")],
    );
}

/// A one-line input box: rounded border, centered title, icon, block cursor,
/// optional info on the right and key hints in the bottom border.
fn draw_input(
    f: &mut Frame,
    title: &str,
    icon: &str,
    text: &str,
    cursor: usize,
    info: Option<Span<'static>>,
    hints: &[(&str, &str)],
) -> Option<Rect> {
    let full = f.area();
    let width = full.width.saturating_sub(4).min(60);
    if width < 10 || full.height < 3 {
        return None;
    }
    let area = Rect {
        x: full.x + (full.width - width) / 2,
        y: full.y + 2.min(full.height - 3),
        width,
        height: 3,
    };
    let mut hint_spans = vec![Span::raw(" ")];
    for (i, (k, v)) in hints.iter().enumerate() {
        if i > 0 {
            hint_spans.push(Span::styled(" · ", Style::new().fg(DIM)));
        }
        hint_spans.push(Span::styled(k.to_string(), Style::new().fg(ACCENT)));
        hint_spans.push(Span::styled(format!(" {v}"), Style::new().fg(DIM)));
    }
    hint_spans.push(Span::raw(" "));
    let block = Block::new()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(ACCENT))
        .title(Line::styled(format!(" {title} "), Style::new().fg(ACCENT)).centered())
        .title_bottom(Line::from(hint_spans).centered());
    let inner = block.inner(area);
    f.render_widget(Clear, area);
    f.render_widget(block, area);

    let info_width = info.as_ref().map_or(0, |s| s.width() as u16 + 1);
    let [input, info_area] =
        Layout::horizontal([Constraint::Min(1), Constraint::Length(info_width)]).areas(inner);
    // Keep the cursor in view: drop characters from the start of long input
    let room = (input.width as usize).saturating_sub(4);
    let (mut before, after) = text.split_at(cursor);
    while Line::raw(before).width() > room {
        let mut chars = before.chars();
        chars.next();
        before = chars.as_str();
    }
    let mut after_chars = after.chars();
    let spans = vec![
        Span::styled(format!(" {icon} "), Style::new().fg(ACCENT)),
        Span::raw(before.to_string()),
        match after_chars.next() {
            Some(c) => Span::styled(c.to_string(), Style::new().reversed()),
            None => Span::styled("█", Style::new().fg(ACCENT)),
        },
        Span::raw(after_chars.as_str().to_string()),
    ];
    f.render_widget(Line::from(spans), input);
    if let Some(info) = info {
        f.render_widget(Line::from(info).right_aligned(), info_area);
    }
    Some(area)
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
    // The toggles that are on, like track's header (d / b / w turn them on and off).
    let flags: Vec<&str> = [(app.diff, "diff"), (app.bell, "bell"), (app.wrap, "wrap")]
        .into_iter()
        .filter_map(|(on, name)| on.then_some(name))
        .collect();
    if !flags.is_empty() {
        spans.push(Span::styled(
            format!("  {}", flags.join(" · ")),
            Style::new().fg(DIM),
        ));
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
        spans.push(Span::styled(
            format!(" {}", SPINNER[i]),
            Style::new().fg(ACCENT),
        ));
    }
    if app.time_machine() {
        spans.push(Span::styled(" ◆ time machine", Style::new().fg(MAGENTA)));
    } else {
        spans.push(Span::styled(" ● live", Style::new().fg(GREEN)));
    }
    if app.suspended {
        spans.push(Span::styled(" · suspended", Style::new().fg(YELLOW)));
    }
    // The search and the current match; dimmed while its highlights are hidden.
    if let (Some(search), false) = (&app.search, app.search_open) {
        let count = match app.match_position() {
            _ if app.search_hidden => "hidden".to_string(),
            (_, 0) => "no match".to_string(),
            (Some(i), n) => format!("{}/{n}", i + 1),
            (None, 1) => "1 match".to_string(),
            (None, n) => format!("{n} matches"),
        };
        let style = if app.search_hidden { DIM } else { YELLOW };
        spans.insert(
            0,
            Span::styled(format!(" /{}", search.label()), Style::new().fg(style)),
        );
        spans.insert(1, Span::styled(format!("  {count} "), Style::new().fg(DIM)));
    }
    spans.push(Span::raw(" "));
    Line::from(spans).alignment(Alignment::Right)
}

fn draw_output(f: &mut Frame, app: &mut App, area: Rect) {
    // Lay out the rows (and do pending search jumps) first, so the title's
    // match count describes this frame.
    let inner = area.inner(Margin {
        vertical: 1,
        horizontal: 1,
    });
    app.view_height = inner.height as usize;
    app.view_width = inner.width as usize;
    app.output_area = inner;
    let has_rows = app.rows(inner.width).is_some();
    if has_rows {
        app.after_layout();
    }

    let change = app.change();
    let title = match app.current() {
        Some(s) => {
            let sep = || Span::styled(" · ", Style::new().fg(DIM));
            let mut spans = vec![
                Span::raw(" "),
                Span::styled(
                    s.start.format("%H:%M:%S%.3f").to_string(),
                    Style::new().fg(TIME),
                ),
                sep(),
                Span::styled(format_duration(s.duration), Style::new().fg(DIM)),
                sep(),
                match s.code {
                    Some(0) => Span::styled("exit 0", Style::new().fg(GREEN)),
                    Some(c) => Span::styled(format!("exit {c}"), Style::new().fg(RED)),
                    None => Span::styled("failed", Style::new().fg(RED)),
                },
            ];
            if let Some(c) = change {
                // Say where it changed: the change may be scrolled out of view.
                let text = match c.lines {
                    1 => format!("changed: line {}", c.first),
                    n => format!("changed: {n} lines from line {}", c.first),
                };
                spans.push(sep());
                spans.push(Span::styled(text, Style::new().fg(YELLOW)));
            }
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
    f.render_widget(block, area);

    if !has_rows {
        let msg = if app.running.is_empty() {
            "No output yet"
        } else {
            "Waiting for first run…"
        };
        f.render_widget(
            Paragraph::new(msg)
                .style(Style::new().fg(DIM))
                .alignment(Alignment::Center),
            inner,
        );
        return;
    }

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

    draw_scroll(f, area, !app.time_machine(), app.scroll_y, height, total);
}

/// The style of selected text. Reverse video (used for differences) is dropped,
/// or it would swap the selection background into the text color.
fn selected_style(style: Style) -> Style {
    style.remove_modifier(Modifier::REVERSED).bg(SELECTION_BG)
}

/// Gives the selected columns of `line` (content row `row`) a selection background.
fn highlight_selection(line: &Line<'static>, sel: &Selection, row: usize) -> Line<'static> {
    let Some((from, to)) = sel.columns(row) else {
        return line.clone();
    };
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
                spans.push(Span::styled(
                    std::mem::take(&mut marked),
                    selected_style(span.style),
                ));
            }
            if inside {
                marked.push(c)
            } else {
                plain.push(c)
            }
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
            let runs = if s.runs > 1 {
                format!(" ×{}", s.runs)
            } else {
                String::new()
            };
            ListItem::new(Line::from(vec![
                Span::styled(s.start.format("%H:%M:%S").to_string(), time),
                Span::styled(runs, Style::new().fg(DIM)),
            ]))
        })
        .collect();

    let block = panel(format!(" History ({n}) "), true).title_bottom(
        Line::styled(
            format!(" {} ", format_bytes(app.history_bytes)),
            Style::new().fg(DIM),
        )
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

    // Highlight selected text (columns start after the 2-cell "▶ " marker).
    let Some(sel) = app.history_selection.filter(|s| s.dragged) else {
        return;
    };
    let inner = area.inner(Margin {
        vertical: 1,
        horizontal: 1,
    });
    let buf = f.buffer_mut();
    for y in inner.y..inner.bottom() {
        let row = app.history_offset + (y - inner.y) as usize;
        let (Some((from, to)), Some(text)) = (sel.columns(row), app.history_row_text(row)) else {
            continue;
        };
        let to = to.min(Line::raw(text).width());
        for col in from..to {
            let x = inner.x + 2 + col as u16;
            if x < inner.right() {
                let cell = &mut buf[(x, y)];
                cell.set_style(selected_style(cell.style()));
            }
        }
    }
}

/// Footer key hints, the same in track, monitor and git-graph: movement, search,
/// the tool's own keys, then `? help` and `q quit` last. When the line is too
/// narrow, the tool's own keys are dropped from the end first, so help and quit
/// always stay visible.
fn footer_hints(keys: &[(&str, &str)], width: u16) -> Line<'static> {
    let hint_width = |(k, v): &(&str, &str)| Line::raw(*k).width() + Line::raw(*v).width() + 3;
    let mut keys = keys.to_vec();
    while keys.len() > 2 && 1 + keys.iter().map(hint_width).sum::<usize>() > width as usize {
        keys.remove(keys.len() - 3);
    }
    let mut spans = vec![Span::raw(" ")];
    for (k, v) in keys {
        spans.push(Span::styled(k.to_string(), Style::new().fg(ACCENT)));
        spans.push(Span::styled(format!(" {v}  "), Style::new().fg(DIM)));
    }
    Line::from(spans)
}

/// A scrollbar on the right border of a panel, the same in track, monitor and
/// git-graph. It takes the border's color (cyan when the panel is focused) and
/// is drawn only when the content doesn't fit.
fn draw_scroll(
    f: &mut Frame,
    panel: Rect,
    focused: bool,
    offset: usize,
    visible: usize,
    total: usize,
) {
    if total <= visible || panel.height < 3 {
        return;
    }
    let style = Style::new().fg(if focused { ACCENT } else { DIM });
    let mut state = ScrollbarState::new(total - visible)
        .position(offset)
        .viewport_content_length(visible);
    f.render_stateful_widget(
        Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(None)
            .end_symbol(None)
            .thumb_style(style)
            .track_style(style),
        panel.inner(Margin {
            vertical: 1,
            horizontal: 0,
        }),
        &mut state,
    );
}

fn draw_footer(f: &mut Frame, app: &App, area: Rect) {
    let line = if let Some(msg) = &app.message {
        Line::styled(format!(" {msg}"), Style::new().fg(YELLOW))
    } else if app.time_machine() {
        // The same words as track's and git-graph's footers.
        footer_hints(
            &[
                ("j/k", "down/up"),
                ("g/G", "top/bottom"),
                ("/", "search"),
                ("n/N", "next/prev"),
                ("J/K", "older/newer"),
                ("esc", "back"),
                ("?", "help"),
                ("q", "quit"),
            ],
            area.width,
        )
    } else {
        footer_hints(
            &[
                ("j/k", "down/up"),
                ("g/G", "top/bottom"),
                ("/", "search"),
                ("n/N", "next/prev"),
                ("space", "time machine"),
                ("s", if app.suspended { "resume" } else { "suspend" }),
                ("?", "help"),
                ("q", "quit"),
            ],
            area.width,
        )
    };
    f.render_widget(line, area);
}

fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width.saturating_sub(2));
    let h = h.min(area.height.saturating_sub(2));
    Rect::new(
        area.x + (area.width - w) / 2,
        area.y + (area.height - h) / 2,
        w,
        h,
    )
}

fn draw_help(f: &mut Frame, app: &mut App) {
    // The same sections and wording as track's and git-graph's help.
    let rows: &[(&str, &str)] = &[
        ("Navigation", ""),
        ("j/k ↓/↑", "down / up"),
        ("ctrl-d/u", "half page down / up"),
        ("PgDn/PgUp, ctrl-f/b", "page down / up"),
        ("g/G", "top / bottom"),
        ("h/l ←/→", "left / right 4 columns (when not wrapping)"),
        ("H/L", "left / right half a page (when not wrapping)"),
        ("home/end", "line start / end (when not wrapping)"),
        ("Search", ""),
        ("/", "search (regex, smart case)"),
        ("n / N", "next / previous match"),
        ("esc", "hide search highlights, then back to the live view"),
        ("Time machine", ""),
        ("space", "toggle time machine"),
        ("J / K", "older / newer result"),
        ("< / >", "oldest / newest result"),
        ("esc", "back to the live view"),
        ("Input box", ""),
        ("←/→ home/end ctrl-a/e", "move the cursor"),
        ("ctrl-u / ctrl-w", "delete to start / previous word"),
        ("↑/↓", "history"),
        ("enter", "submit"),
        ("esc, ctrl-c", "cancel"),
        ("Mouse", ""),
        (
            "wheel",
            "scroll (ctrl: faster; over the history: older / newer result)",
        ),
        ("click", "select a result in the history"),
        ("drag", "select text; copied on release"),
        ("shift-drag", "the terminal's own selection"),
        ("General", ""),
        ("s", "suspend / resume running the command"),
        ("d", "toggle diff"),
        ("b", "toggle bell"),
        ("w", "toggle wrap"),
        ("t", "toggle title"),
        ("r", "refresh (run the command now)"),
        ("?", "help"),
        ("q, ctrl-c", "quit"),
    ];
    let lines: Vec<Line> = rows
        .iter()
        .map(|(k, v)| {
            if v.is_empty() {
                Line::styled(format!(" {k}"), Style::new().fg(ACCENT))
            } else {
                Line::from(vec![
                    Span::styled(format!("   {k:<24}"), Style::new().fg(YELLOW)),
                    Span::raw(*v),
                ])
            }
        })
        .collect();
    // The same help window as track and git-graph.
    let area = centered(f.area(), 88, lines.len() as u16 + 2);
    let max = lines
        .len()
        .saturating_sub(area.height.saturating_sub(2) as usize);
    app.help_max = max;
    app.help_scroll = app.help_scroll.min(max);
    let title = if max > 0 {
        " Help — j/k scroll, any other key closes "
    } else {
        " Help — any key to close "
    };
    f.render_widget(Clear, area);
    f.render_widget(
        Paragraph::new(lines)
            .scroll((app.help_scroll.min(u16::MAX as usize) as u16, 0))
            .block(panel(Span::styled(title, Style::new().fg(ACCENT)), true)),
        area,
    );
}

fn format_duration(d: std::time::Duration) -> String {
    let ms = d.as_secs_f64() * 1000.0;
    if ms < 1000.0 {
        format!("{ms:.0}ms")
    } else {
        format!("{:.2}s", ms / 1000.0)
    }
}
