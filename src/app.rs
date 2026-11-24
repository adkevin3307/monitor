use std::collections::BTreeSet;
use std::io::Write;
use std::sync::Arc;
use std::time::Duration;

use ansi_to_tui::IntoText;
use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::{Margin, Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use unicode_width::UnicodeWidthChar;

use crate::cli::Cli;
use crate::runner::{Msg, Runner, Snapshot};

const TAB_WIDTH: usize = 8;
const MOUSE_SCROLL_LINES: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitReason {
    User,
    Error(i32),
    Changed,
}

#[derive(PartialEq, Eq, Clone)]
struct RowsKey {
    id: usize,
    prev: Option<usize>,
    diff: bool,
    query: Option<String>,
    wrap: bool,
    width: u16,
}

pub struct Rows {
    pub lines: Vec<Line<'static>>,
    /// Visual row index of every search match, sorted.
    pub matches: Vec<usize>,
    /// Width of the widest row (for horizontal scrolling).
    pub max_width: usize,
}

pub struct App {
    pub runner: Runner,
    pub command: String,
    pub interval: Duration,
    pub precise: bool,
    pub max_history: usize,
    /// Merge runs whose result equals the previous snapshot instead of storing them.
    pub skip_unchanged: bool,
    pub max_history_bytes: usize,
    /// Bytes held by stored outputs (shared outputs counted once).
    pub history_bytes: usize,
    pub errexit: bool,
    pub chgexit: bool,

    pub history: Vec<Snapshot>,
    pub running: BTreeSet<usize>,
    /// Index into `history` while in time-machine mode.
    pub selected: Option<usize>,

    pub suspended: bool,
    pub diff: bool,
    pub bell: bool,
    pub show_title: bool,
    pub wrap: bool,
    pub show_help: bool,

    pub scroll_y: usize,
    pub scroll_x: usize,
    pub view_height: usize,
    pub view_width: usize,
    /// Where the history panel was drawn last frame (empty when hidden).
    pub history_area: Rect,
    /// First visible row of the history list, kept between frames.
    pub history_offset: usize,

    pub query: Option<String>,
    pub search_input: Option<String>,
    pub message: Option<String>,
    pending_jump: bool,

    pub exit: Option<ExitReason>,
    cache: Option<(RowsKey, Rows)>,
}

impl App {
    pub fn new(cli: &Cli, runner: Runner) -> Self {
        App {
            runner,
            command: cli.command_display(),
            interval: cli.interval,
            precise: cli.precise,
            max_history: cli.max_history as usize,
            skip_unchanged: !cli.keep_unchanged,
            max_history_bytes: cli.max_history_size,
            history_bytes: 0,
            errexit: cli.errexit,
            chgexit: cli.chgexit,
            history: Vec::new(),
            running: BTreeSet::new(),
            selected: None,
            suspended: false,
            diff: cli.differences,
            bell: cli.bell,
            show_title: !cli.no_title,
            wrap: !cli.unfold,
            show_help: false,
            scroll_y: 0,
            scroll_x: 0,
            view_height: 0,
            view_width: 0,
            history_area: Rect::default(),
            history_offset: 0,
            query: None,
            search_input: None,
            message: None,
            pending_jump: false,
            exit: None,
            cache: None,
        }
    }

    pub fn time_machine(&self) -> bool {
        self.selected.is_some()
    }

    pub fn current_index(&self) -> Option<usize> {
        match self.selected {
            Some(i) => Some(i),
            None => self.history.len().checked_sub(1),
        }
    }

    pub fn current(&self) -> Option<&Snapshot> {
        self.current_index().map(|i| &self.history[i])
    }

    // ---------------------------------------------------------------- runner

    pub fn on_msg(&mut self, msg: Msg) {
        match msg {
            Msg::Started(id) => {
                self.running.insert(id);
            }
            Msg::Finished(mut snap) => {
                self.running.remove(&snap.id);
                let pos = self.history.partition_point(|s| s.id < snap.id);
                let is_latest = pos == self.history.len();
                let failed = snap.code != Some(0);
                let code = snap.code.unwrap_or(-1);
                if is_latest && failed && self.errexit {
                    self.exit = Some(ExitReason::Error(code));
                }

                if self.skip_unchanged {
                    if let Some(prev) = pos.checked_sub(1).map(|p| &mut self.history[p]) {
                        if prev.same_result(&snap) {
                            prev.absorb(&snap);
                            return;
                        }
                    }
                }

                snap.changed = match pos.checked_sub(1) {
                    Some(p) => {
                        let prev = &self.history[p].output;
                        if **prev == *snap.output {
                            // Unchanged: share the previous buffer instead of keeping a copy.
                            snap.output = prev.clone();
                            false
                        } else {
                            true
                        }
                    }
                    None => false,
                };
                if let Some(next) = self.history.get_mut(pos) {
                    if *next.output == *snap.output {
                        next.output = snap.output.clone();
                        next.changed = false;
                    } else {
                        next.changed = true;
                    }
                }
                let changed = snap.changed;
                self.history.insert(pos, snap);
                if let Some(sel) = self.selected.as_mut() {
                    if pos <= *sel {
                        *sel += 1;
                    }
                }
                // With precise mode, runs can finish out of order: the newer
                // neighbour may now turn out to be an unchanged repeat.
                if self.skip_unchanged
                    && self.history.get(pos + 1).is_some_and(|n| n.same_result(&self.history[pos]))
                {
                    let next = self.history.remove(pos + 1);
                    self.history[pos].absorb(&next);
                    if let Some(sel) = self.selected.as_mut() {
                        if *sel > pos {
                            *sel -= 1;
                        }
                    }
                    if pos + 1 < self.history.len() {
                        let differs = *self.history[pos + 1].output != *self.history[pos].output;
                        self.history[pos + 1].changed = differs;
                    }
                }
                self.trim_history();

                if is_latest && changed {
                    if self.bell {
                        let mut out = std::io::stdout();
                        let _ = out.write_all(b"\x07");
                        let _ = out.flush();
                    }
                    if self.chgexit {
                        self.exit = Some(ExitReason::Changed);
                    }
                }
            }
        }
    }

    /// Bytes owned by `history[i]`, i.e. not shared with the snapshot before it.
    fn owned_bytes(&self, i: usize) -> usize {
        let out = &self.history[i].output;
        match i.checked_sub(1) {
            Some(p) if Arc::ptr_eq(&self.history[p].output, out) => 0,
            _ => out.len(),
        }
    }

    /// Drops the oldest snapshots until both the count and the byte budget
    /// are respected (the newest snapshot is always kept).
    fn trim_history(&mut self) {
        let mut total: usize = (0..self.history.len()).map(|i| self.owned_bytes(i)).sum();
        let mut excess = 0;
        while self.history.len() - excess > 1
            && (self.history.len() - excess > self.max_history || total > self.max_history_bytes)
        {
            let first = &self.history[excess].output;
            let shared = self.history.get(excess + 1).is_some_and(|n| Arc::ptr_eq(&n.output, first));
            if !shared {
                total -= first.len();
            }
            excess += 1;
        }
        self.history_bytes = total;
        if excess == 0 {
            return;
        }
        self.history.drain(..excess);
        if let Some(sel) = self.selected.as_mut() {
            *sel = sel.saturating_sub(excess);
        }
    }

    // ------------------------------------------------------------- rendering

    /// Rows of the currently displayed snapshot, laid out for `width` columns.
    pub fn rows(&mut self, width: u16) -> Option<&Rows> {
        let idx = self.current_index()?;
        let key = RowsKey {
            id: self.history[idx].id,
            prev: idx.checked_sub(1).map(|p| self.history[p].id),
            diff: self.diff,
            query: self.query.clone(),
            wrap: self.wrap,
            width,
        };
        if self.cache.as_ref().map(|(k, _)| k) != Some(&key) {
            let prev = if self.diff { idx.checked_sub(1).map(|p| &self.history[p]) } else { None };
            let rows = build_rows(
                &self.history[idx],
                prev,
                self.query.as_deref(),
                self.wrap,
                width as usize,
            );
            self.cache = Some((key, rows));
        }
        self.cache.as_ref().map(|(_, r)| r)
    }

    /// Rows built by the last call to [`App::rows`].
    pub fn cached_rows(&self) -> Option<&Rows> {
        self.cache.as_ref().map(|(_, r)| r)
    }

    fn row_count(&self) -> usize {
        self.cache.as_ref().map(|(_, r)| r.lines.len()).unwrap_or(0)
    }

    pub fn clamp_scroll(&mut self) {
        let max_y = self.row_count().saturating_sub(self.view_height);
        self.scroll_y = self.scroll_y.min(max_y);
        if self.wrap {
            self.scroll_x = 0;
        } else {
            let w = self.cache.as_ref().map(|(_, r)| r.max_width).unwrap_or(0);
            self.scroll_x = self.scroll_x.min(w.saturating_sub(self.view_width));
        }
    }

    // ----------------------------------------------------------------- input

    pub fn on_key(&mut self, key: KeyEvent) {
        if key.kind == KeyEventKind::Release {
            return;
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);

        if ctrl && key.code == KeyCode::Char('c') {
            self.exit = Some(ExitReason::User);
            return;
        }

        if let Some(input) = self.search_input.as_mut() {
            match key.code {
                KeyCode::Esc => self.search_input = None,
                KeyCode::Enter => {
                    let q = self.search_input.take().unwrap_or_default();
                    self.query = if q.is_empty() { None } else { Some(q) };
                    // Search from the top of the current view on the next frame.
                    self.scroll_y = self.scroll_y.saturating_sub(1);
                    self.pending_jump = true;
                }
                KeyCode::Backspace => {
                    if input.pop().is_none() {
                        self.search_input = None;
                    }
                }
                KeyCode::Char('u') if ctrl => input.clear(),
                KeyCode::Char(c) if !ctrl => input.push(c),
                _ => {}
            }
            return;
        }

        if self.show_help {
            self.show_help = false;
            return;
        }

        self.message = None;
        let half = (self.view_height / 2).max(1);
        let page = self.view_height.max(1);

        match key.code {
            // Ctrl scrolling first so the plain-letter toggles below don't shadow it.
            KeyCode::Char('d') if ctrl => self.scroll_y += half,
            KeyCode::Char('u') if ctrl => {
                self.scroll_y = self.scroll_y.saturating_sub(half)
            }
            KeyCode::Char('f') if ctrl => self.scroll_y += page,
            KeyCode::Char('b') if ctrl => self.scroll_y = self.scroll_y.saturating_sub(page),

            KeyCode::Char('q') => self.exit = Some(ExitReason::User),
            KeyCode::Esc => {
                if self.query.is_some() {
                    self.query = None;
                } else if self.time_machine() {
                    self.selected = None;
                }
            }
            KeyCode::Char('?') => self.show_help = true,

            // time machine
            KeyCode::Char(' ') => {
                self.selected = match self.selected {
                    Some(_) => None,
                    None => self.history.len().checked_sub(1),
                };
            }
            KeyCode::Char('J') => self.move_history(-1),
            KeyCode::Char('K') => self.move_history(1),
            KeyCode::Down if shift => self.move_history(-1),
            KeyCode::Up if shift => self.move_history(1),
            KeyCode::Char('<') => self.jump_history(0),
            KeyCode::Char('>') => self.jump_history(usize::MAX),

            // toggles
            KeyCode::Char('s') => {
                self.suspended = !self.suspended;
                self.runner.set_suspended(self.suspended);
            }
            KeyCode::Char('r') => self.runner.run_now(),
            KeyCode::Char('d') => self.diff = !self.diff,
            KeyCode::Char('b') => {
                self.bell = !self.bell;
                self.message = Some(format!("bell {}", if self.bell { "on" } else { "off" }));
            }
            KeyCode::Char('t') => self.show_title = !self.show_title,
            KeyCode::Char('w') | KeyCode::Char('u') => self.wrap = !self.wrap,

            // scrolling
            KeyCode::Char('j') | KeyCode::Down => self.scroll_y += 1,
            KeyCode::Char('k') | KeyCode::Up => self.scroll_y = self.scroll_y.saturating_sub(1),
            KeyCode::PageDown => self.scroll_y += page,
            KeyCode::PageUp => self.scroll_y = self.scroll_y.saturating_sub(page),
            KeyCode::Char('g') | KeyCode::Home => self.scroll_y = 0,
            KeyCode::Char('G') | KeyCode::End => self.scroll_y = usize::MAX,
            KeyCode::Char('h') | KeyCode::Left => self.scroll_x = self.scroll_x.saturating_sub(4),
            KeyCode::Char('l') | KeyCode::Right => self.scroll_x += 4,
            KeyCode::Char('0') => self.scroll_x = 0,

            // search
            KeyCode::Char('/') => self.search_input = Some(String::new()),
            KeyCode::Char('n') => self.jump_match(true),
            KeyCode::Char('N') => self.jump_match(false),
            _ => {}
        }
        self.clamp_scroll();
    }

    pub fn on_mouse(&mut self, mouse: MouseEvent) {
        if self.show_help || self.search_input.is_some() {
            return;
        }
        let over_history = self.history_area.contains(Position::new(mouse.column, mouse.row));
        match mouse.kind {
            // The history lists newest first, so scrolling down goes to older results.
            MouseEventKind::ScrollDown if over_history => self.move_history(-1),
            MouseEventKind::ScrollUp if over_history => self.move_history(1),
            MouseEventKind::ScrollDown => self.scroll_y += MOUSE_SCROLL_LINES,
            MouseEventKind::ScrollUp => self.scroll_y = self.scroll_y.saturating_sub(MOUSE_SCROLL_LINES),
            MouseEventKind::Down(MouseButton::Left) if over_history => {
                self.click_history(mouse.row);
                return;
            }
            MouseEventKind::ScrollRight => self.scroll_x += 4,
            MouseEventKind::ScrollLeft => self.scroll_x = self.scroll_x.saturating_sub(4),
            _ => return,
        }
        self.clamp_scroll();
    }

    /// Selects the history row at terminal row `y` (clicks on the border are ignored).
    fn click_history(&mut self, y: u16) {
        let inner = self.history_area.inner(Margin { vertical: 1, horizontal: 1 });
        if y < inner.y || y >= inner.bottom() {
            return;
        }
        // The list shows the newest result first.
        let row = self.history_offset + (y - inner.y) as usize;
        if let Some(idx) = self.history.len().checked_sub(row + 1) {
            self.selected = Some(idx);
        }
    }

    /// `delta` > 0 moves to newer snapshots, < 0 to older ones.
    fn move_history(&mut self, delta: isize) {
        let Some(cur) = self.current_index() else { return };
        let last = self.history.len() - 1;
        let next = cur.saturating_add_signed(delta).min(last);
        self.selected = Some(next);
    }

    fn jump_history(&mut self, idx: usize) {
        if let Some(last) = self.history.len().checked_sub(1) {
            self.selected = Some(idx.min(last));
        }
    }

    fn jump_match(&mut self, forward: bool) {
        let Some((_, rows)) = self.cache.as_ref() else { return };
        if self.query.is_none() {
            return;
        }
        if rows.matches.is_empty() {
            self.message = Some("pattern not found".into());
            return;
        }
        let cur = self.scroll_y;
        let target = if forward {
            rows.matches.iter().copied().find(|&r| r > cur).unwrap_or(rows.matches[0])
        } else {
            rows.matches.iter().rev().copied().find(|&r| r < cur).unwrap_or(*rows.matches.last().unwrap())
        };
        self.scroll_y = target;
    }

    /// Called after rows are (re)built for the frame: performs a deferred jump
    /// to the first search match once the match positions are known.
    pub fn after_layout(&mut self) {
        if std::mem::take(&mut self.pending_jump) {
            self.jump_match(true);
        }
        self.clamp_scroll();
    }
}

/// Turns a snapshot into display rows: parses ANSI colors, expands tabs,
/// highlights differences from `prev` and matches of `query`, and wraps.
pub fn build_rows(
    snap: &Snapshot,
    prev: Option<&Snapshot>,
    query: Option<&str>,
    wrap: bool,
    width: usize,
) -> Rows {
    let lines = styled_chars(&snap.output);
    let prev_lines: Option<Vec<Vec<char>>> = prev.map(|p| {
        styled_chars(&p.output).into_iter().map(|l| l.into_iter().map(|(c, _)| c).collect()).collect()
    });

    let diff_style = Style::default().add_modifier(Modifier::REVERSED);
    let match_style = Style::default().bg(Color::Yellow).fg(Color::Black);

    // Smart case: case-insensitive unless the query contains uppercase.
    let query: Option<(Vec<char>, bool)> = query.filter(|q| !q.is_empty()).map(|q| {
        let sensitive = q.chars().any(char::is_uppercase);
        let chars = if sensitive { q.chars().collect() } else { q.to_lowercase().chars().collect() };
        (chars, sensitive)
    });

    let mut rows = Vec::new();
    let mut matches = Vec::new();
    let mut max_width = 0;
    let width = width.max(1);

    for (i, mut line) in lines.into_iter().enumerate() {
        if let Some(prev_lines) = &prev_lines {
            let empty = Vec::new();
            let old = prev_lines.get(i).unwrap_or(&empty);
            for (j, (c, style)) in line.iter_mut().enumerate() {
                if old.get(j) != Some(c) {
                    *style = style.patch(diff_style);
                }
            }
        }

        let mut match_starts = Vec::new();
        if let Some((q, sensitive)) = &query {
            let hay: Vec<char> = if *sensitive {
                line.iter().map(|(c, _)| *c).collect()
            } else {
                line.iter().map(|(c, _)| c.to_lowercase().next().unwrap_or(*c)).collect()
            };
            let mut j = 0;
            while j + q.len() <= hay.len() {
                if hay[j..j + q.len()] == q[..] {
                    match_starts.push(j);
                    for k in j..j + q.len() {
                        line[k].1 = line[k].1.patch(match_style);
                    }
                    j += q.len();
                } else {
                    j += 1;
                }
            }
        }

        // Lay out into visual rows.
        let mut row: Vec<(char, Style)> = Vec::new();
        let mut row_w = 0;
        let mut starts = match_starts.into_iter().peekable();
        for (j, (c, style)) in line.into_iter().enumerate() {
            let cw = c.width().unwrap_or(0);
            if wrap && row_w + cw > width && !row.is_empty() {
                max_width = max_width.max(row_w);
                rows.push(to_line(std::mem::take(&mut row)));
                row_w = 0;
            }
            while starts.peek() == Some(&j) {
                starts.next();
                matches.push(rows.len());
            }
            row.push((c, style));
            row_w += cw;
        }
        max_width = max_width.max(row_w);
        rows.push(to_line(row));
    }
    matches.dedup();

    Rows { lines: rows, matches, max_width }
}

/// Parses ANSI output into lines of (char, style), expanding tabs and
/// dropping other control characters.
fn styled_chars(output: &str) -> Vec<Vec<(char, Style)>> {
    let output = output.strip_suffix('\n').unwrap_or(output);
    let text: Text = output.as_bytes().into_text().unwrap_or_else(|_| Text::raw(output.to_owned()));
    text.lines
        .into_iter()
        .map(|line| {
            let base = line.style;
            let mut out = Vec::new();
            let mut col = 0;
            for span in line.spans {
                let style = base.patch(span.style);
                for c in span.content.chars() {
                    match c {
                        '\t' => {
                            let n = TAB_WIDTH - col % TAB_WIDTH;
                            out.extend(std::iter::repeat_n((' ', style), n));
                            col += n;
                        }
                        c if c.is_control() => {}
                        c => {
                            out.push((c, style));
                            col += c.width().unwrap_or(0);
                        }
                    }
                }
            }
            out
        })
        .collect()
}

fn to_line(chars: Vec<(char, Style)>) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut buf = String::new();
    let mut cur: Option<Style> = None;
    for (c, style) in chars {
        if cur != Some(style) {
            if let Some(s) = cur {
                spans.push(Span::styled(std::mem::take(&mut buf), s));
            }
            cur = Some(style);
        }
        buf.push(c);
    }
    if let Some(s) = cur {
        spans.push(Span::styled(buf, s));
    }
    Line::from(spans)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Local;

    fn snap(id: usize, output: &str) -> Snapshot {
        Snapshot { id, start: Local::now(), duration: Duration::ZERO, output: output.into(), code: Some(0), changed: false, runs: 1, last_seen: Local::now() }
    }

    fn text(line: &Line) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn wraps_and_expands_tabs() {
        let rows = build_rows(&snap(1, "a\tb\n0123456789\n"), None, None, true, 4);
        let t: Vec<String> = rows.lines.iter().map(text).collect();
        assert_eq!(t, vec!["a   ", "    ", "b", "0123", "4567", "89"]);
        let rows = build_rows(&snap(1, "0123456789\nx"), None, None, false, 4);
        assert_eq!(rows.lines.len(), 2);
        assert_eq!(rows.max_width, 10);
    }

    #[test]
    fn highlights_differences() {
        let rows = build_rows(&snap(2, "abc\nxyz"), Some(&snap(1, "abd\nxyz")), None, true, 80);
        let first = &rows.lines[0];
        assert_eq!(text(first), "abc");
        assert!(!first.spans[0].style.add_modifier.contains(Modifier::REVERSED));
        assert!(first.spans.last().unwrap().style.add_modifier.contains(Modifier::REVERSED));
        assert_eq!(rows.lines[1].spans.len(), 1);
    }

    #[test]
    fn search_smart_case_and_ansi() {
        let out = "\x1b[31mError\x1b[0m one\nfine\nerror two";
        let rows = build_rows(&snap(1, out), None, Some("error"), true, 80);
        assert_eq!(text(&rows.lines[0]), "Error one");
        assert_eq!(rows.lines[0].spans[0].style.fg, Some(Color::Black)); // match overrides red
        assert_eq!(rows.matches, vec![0, 2]);
        let rows = build_rows(&snap(1, out), None, Some("Error"), true, 80);
        assert_eq!(rows.matches, vec![0]);
    }
}

#[cfg(test)]
mod history_tests {
    use super::*;
    use clap::Parser;
    use chrono::Local;

    fn app(extra: &[&str]) -> App {
        // A huge interval: the runner's single run finishes into a dropped channel.
        let mut args = vec!["monitor", "-n", "1h"];
        args.extend_from_slice(extra);
        args.push("true");
        let cli = Cli::try_parse_from(args).unwrap();
        let (runner, _rx) = Runner::start(&cli);
        App::new(&cli, runner)
    }

    fn done(app: &mut App, id: usize, output: &str, code: i32) {
        let t = Local::now();
        app.on_msg(Msg::Finished(Snapshot {
            id,
            start: t,
            duration: Duration::ZERO,
            output: output.into(),
            code: Some(code),
            changed: false,
            runs: 1,
            last_seen: t,
        }));
    }

    fn ids(app: &App) -> Vec<(usize, usize)> {
        app.history.iter().map(|s| (s.id, s.runs)).collect()
    }

    #[test]
    fn merges_unchanged_runs() {
        let mut a = app(&[]);
        for (id, out, code) in [(1, "a", 0), (2, "a", 0), (3, "b", 0), (4, "b", 0), (5, "b", 1), (6, "a", 0)] {
            done(&mut a, id, out, code);
        }
        // Same output but a different exit status still counts as a change.
        assert_eq!(ids(&a), vec![(1, 2), (3, 2), (5, 1), (6, 1)]);
    }

    #[test]
    fn merges_out_of_order_runs() {
        let mut a = app(&[]);
        done(&mut a, 1, "a", 0);
        done(&mut a, 3, "b", 0);
        done(&mut a, 2, "b", 0); // finishes late, same as its newer neighbour
        assert_eq!(ids(&a), vec![(1, 1), (2, 2)]);
        assert!(a.history[1].changed);
    }

    #[test]
    fn keep_unchanged_records_every_run() {
        let mut a = app(&["--keep-unchanged"]);
        for id in 1..=3 {
            done(&mut a, id, "same", 0);
        }
        assert_eq!(a.history.len(), 3);
        // Unchanged results share one buffer.
        assert!(Arc::ptr_eq(&a.history[0].output, &a.history[2].output));
        assert_eq!(a.history_bytes, 4);
    }
}
