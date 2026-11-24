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
use crate::input::{LineEditor, push_history};
use crate::matcher::Matcher;
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
    /// Whether each row continues the previous one (a wrapped long line).
    pub continued: Vec<bool>,
}

/// A mouse selection in the output, in content coordinates (row, column).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    pub anchor: (usize, usize),
    pub cursor: (usize, usize),
    pub dragged: bool,
}

impl Selection {
    /// Start and end, in reading order; both ends are inclusive.
    pub fn range(&self) -> ((usize, usize), (usize, usize)) {
        if self.anchor <= self.cursor { (self.anchor, self.cursor) } else { (self.cursor, self.anchor) }
    }

    /// Columns `[from, to)` selected on `row`, if any.
    pub fn columns(&self, row: usize) -> Option<(usize, usize)> {
        let (start, end) = self.range();
        if row < start.0 || row > end.0 {
            return None;
        }
        let from = if row == start.0 { start.1 } else { 0 };
        let to = if row == end.0 { end.1 + 1 } else { usize::MAX };
        Some((from, to))
    }
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
    /// Help scroll position; `help_max` is set when it is drawn.
    pub help_scroll: usize,
    pub help_max: usize,

    pub scroll_y: usize,
    pub scroll_x: usize,
    pub view_height: usize,
    pub view_width: usize,
    /// Where the history panel was drawn last frame (empty when hidden).
    pub history_area: Rect,
    /// First visible row of the history list, kept between frames.
    pub history_offset: usize,
    /// Inner area of the output panel last frame.
    pub output_area: Rect,
    pub selection: Option<Selection>,
    /// Text being selected in the history list: (list row, column after the marker).
    pub history_selection: Option<Selection>,
    /// Snapshot (by id) held on screen while a selection is being made,
    /// so new output can't shift the text under the mouse.
    pinned: Option<usize>,

    /// The active search; `search_hidden` hides its highlights (esc, like vim's
    /// :noh) until n / N.
    pub search: Option<Matcher>,
    pub search_hidden: bool,
    /// The search box is open (`/`); `editor` holds what is typed.
    pub search_open: bool,
    pub editor: LineEditor,
    search_history: Vec<String>,
    /// Scroll position when the search box opened, restored on cancel.
    search_origin: usize,
    pub message: Option<String>,
    /// A jump to a search match, done once the rows for the frame are built.
    pending_jump: Option<Jump>,

    pub exit: Option<ExitReason>,
    cache: Option<(RowsKey, Rows)>,
    /// What changed from the previous snapshot, keyed by (snapshot id, previous id).
    change_cache: Option<((usize, usize), Option<Change>)>,
}

/// Lines that differ from the previous snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Change {
    /// First differing line (1-based).
    pub first: usize,
    pub lines: usize,
}

/// Compares two outputs line by line.
pub fn line_change(prev: &str, cur: &str) -> Option<Change> {
    let (mut a, mut b) = (prev.lines(), cur.lines());
    let mut first = None;
    let mut lines = 0;
    for i in 1.. {
        match (a.next(), b.next()) {
            (None, None) => break,
            (x, y) if x == y => {}
            _ => {
                first.get_or_insert(i);
                lines += 1;
            }
        }
    }
    first.map(|first| Change { first, lines })
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
            help_scroll: 0,
            help_max: 0,
            scroll_y: 0,
            scroll_x: 0,
            view_height: 0,
            view_width: 0,
            history_area: Rect::default(),
            history_offset: 0,
            output_area: Rect::default(),
            selection: None,
            history_selection: None,
            pinned: None,
            search: None,
            search_hidden: false,
            search_open: false,
            editor: LineEditor::default(),
            search_history: Vec::new(),
            search_origin: 0,
            message: None,
            pending_jump: None,
            exit: None,
            cache: None,
            change_cache: None,
        }
    }

    pub fn time_machine(&self) -> bool {
        self.selected.is_some()
    }

    pub fn current_index(&self) -> Option<usize> {
        if let Some(i) = self.selected {
            return Some(i);
        }
        if let Some(id) = self.pinned {
            if let Ok(i) = self.history.binary_search_by_key(&id, |s| s.id) {
                return Some(i);
            }
        }
        self.history.len().checked_sub(1)
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
            query: self.highlight_query(),
            wrap: self.wrap,
            width,
        };
        if self.cache.as_ref().map(|(k, _)| k) != Some(&key) {
            let prev = if self.diff { idx.checked_sub(1).map(|p| &self.history[p]) } else { None };
            let matcher = key.query.as_deref().and_then(|q| Matcher::new(q, false));
            let rows = build_rows(&self.history[idx], prev, matcher.as_ref(), self.wrap, width as usize);
            self.cache = Some((key, rows));
        }
        self.cache.as_ref().map(|(_, r)| r)
    }

    /// What the shown snapshot changed from the one before it (cached).
    pub fn change(&mut self) -> Option<Change> {
        let idx = self.current_index()?;
        let prev = idx.checked_sub(1)?;
        let key = (self.history[idx].id, self.history[prev].id);
        if self.change_cache.as_ref().map(|(k, _)| k) != Some(&key) {
            let change = line_change(&self.history[prev].output, &self.history[idx].output);
            self.change_cache = Some((key, change));
        }
        self.change_cache.as_ref().and_then(|(_, c)| *c)
    }

    /// Text of history list row `row` (newest first), after the selection marker.
    pub fn history_row_text(&self, row: usize) -> Option<String> {
        let s = &self.history[self.history.len().checked_sub(row + 1)?];
        let runs = if s.runs > 1 { format!(" ×{}", s.runs) } else { String::new() };
        Some(format!("{}{runs}", s.start.format("%H:%M:%S")))
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

        if self.search_open {
            self.on_search_key(key, ctrl);
            return;
        }

        if self.show_help {
            // Scroll when the help doesn't fit; any other key closes it.
            match key.code {
                KeyCode::Char('j') | KeyCode::Down => self.help_scroll += 1,
                KeyCode::Char('k') | KeyCode::Up => self.help_scroll = self.help_scroll.saturating_sub(1),
                KeyCode::PageDown => self.help_scroll += 10,
                KeyCode::PageUp => self.help_scroll = self.help_scroll.saturating_sub(10),
                KeyCode::Char('g') => self.help_scroll = 0,
                KeyCode::Char('G') => self.help_scroll = usize::MAX,
                _ => self.show_help = false,
            }
            self.help_scroll = self.help_scroll.min(self.help_max);
            return;
        }
        self.selection = None;
        self.history_selection = None;
        self.pinned = None;

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
                // First hide the search highlights (n / N show them again).
                if self.search.is_some() && !self.search_hidden {
                    self.search_hidden = true;
                } else if self.time_machine() {
                    self.selected = None;
                }
            }
            KeyCode::Char('?') => {
                self.show_help = true;
                self.help_scroll = 0;
            }

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
                self.message = Some(format!("Bell {}", if self.bell { "on" } else { "off" }));
            }
            KeyCode::Char('t') => self.show_title = !self.show_title,
            KeyCode::Char('w') | KeyCode::Char('u') => self.wrap = !self.wrap,

            // scrolling
            KeyCode::Char('j') | KeyCode::Down => self.scroll_y += 1,
            KeyCode::Char('k') | KeyCode::Up => self.scroll_y = self.scroll_y.saturating_sub(1),
            KeyCode::PageDown => self.scroll_y += page,
            KeyCode::PageUp => self.scroll_y = self.scroll_y.saturating_sub(page),
            KeyCode::Char('g') => self.scroll_y = 0,
            KeyCode::Char('G') => self.scroll_y = usize::MAX,
            KeyCode::Char('h') | KeyCode::Left => self.scroll_x = self.scroll_x.saturating_sub(4),
            KeyCode::Char('l') | KeyCode::Right => self.scroll_x += 4,
            // Line start / end (clamp_scroll stops at the end of the widest line)
            KeyCode::Char('H') => self.scroll_x = self.scroll_x.saturating_sub((self.view_width / 2).max(1)),
            KeyCode::Char('L') => self.scroll_x += (self.view_width / 2).max(1),
            KeyCode::Home => self.scroll_x = 0,
            KeyCode::End => self.scroll_x = usize::MAX,

            // search
            KeyCode::Char('/') => {
                self.search_open = true;
                self.editor.clear();
                self.search_origin = self.scroll_y;
            }
            KeyCode::Char('n') => self.request_jump(Jump::Next),
            KeyCode::Char('N') => self.request_jump(Jump::Prev),
            _ => {}
        }
        self.clamp_scroll();
    }

    pub fn on_mouse(&mut self, mouse: MouseEvent) {
        // ctrl + wheel scrolls faster, as in track and git-graph.
        let lines = if mouse.modifiers.contains(KeyModifiers::CONTROL) { 15 } else { MOUSE_SCROLL_LINES };
        if self.show_help {
            match mouse.kind {
                MouseEventKind::ScrollDown => self.help_scroll = (self.help_scroll + 3).min(self.help_max),
                MouseEventKind::ScrollUp => self.help_scroll = self.help_scroll.saturating_sub(3),
                _ => {}
            }
            return;
        }
        if self.search_open {
            return;
        }
        let pos = Position::new(mouse.column, mouse.row);
        let over_history = self.history_area.contains(pos);
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) if self.output_area.contains(pos) => {
                let at = self.content_pos(mouse.column, mouse.row);
                self.selection = Some(Selection { anchor: at, cursor: at, dragged: false });
                self.pinned = self.current().map(|s| s.id);
                return;
            }
            MouseEventKind::Drag(MouseButton::Left) if self.selection.is_some() => {
                // Dragging past the top or bottom edge scrolls the output.
                if mouse.row < self.output_area.y {
                    self.scroll_y = self.scroll_y.saturating_sub(1);
                } else if mouse.row >= self.output_area.bottom() {
                    self.scroll_y += 1;
                }
                self.clamp_scroll();
                let at = self.content_pos(mouse.column, mouse.row);
                if let Some(sel) = self.selection.as_mut() {
                    sel.cursor = at;
                    sel.dragged = true;
                }
                return;
            }
            MouseEventKind::Up(MouseButton::Left) if self.selection.is_some() => {
                self.finish_selection();
                return;
            }
            // In the history, a click picks a result and a drag selects text.
            MouseEventKind::Down(MouseButton::Left) if over_history => {
                self.click_history(mouse.row);
                self.message = None;
                if let Some(at) = self.history_pos(mouse.column, mouse.row) {
                    self.history_selection = Some(Selection { anchor: at, cursor: at, dragged: false });
                }
                return;
            }
            MouseEventKind::Drag(MouseButton::Left) if self.history_selection.is_some() => {
                let at = self.history_pos(mouse.column, mouse.row);
                if let (Some(sel), Some(at)) = (self.history_selection.as_mut(), at) {
                    sel.cursor = at;
                    sel.dragged = true;
                }
                return;
            }
            MouseEventKind::Up(MouseButton::Left) if self.history_selection.is_some() => {
                if let Some(sel) = self.history_selection.take().filter(|s| s.dragged) {
                    let text = self.history_text(&sel);
                    self.copy_text(&text);
                }
                return;
            }
            // The history lists newest first, so scrolling down goes to older results.
            MouseEventKind::ScrollDown if over_history => self.move_history(-1),
            MouseEventKind::ScrollUp if over_history => self.move_history(1),
            MouseEventKind::ScrollDown => self.scroll_y += lines,
            MouseEventKind::ScrollUp => self.scroll_y = self.scroll_y.saturating_sub(lines),
            MouseEventKind::ScrollRight => self.scroll_x += 4,
            MouseEventKind::ScrollLeft => self.scroll_x = self.scroll_x.saturating_sub(4),
            _ => return,
        }
        self.clamp_scroll();
    }

    /// Maps a terminal cell to (row, column) in the output content,
    /// clamped to the visible output area.
    fn content_pos(&self, x: u16, y: u16) -> (usize, usize) {
        let a = self.output_area;
        let x = x.clamp(a.x, a.right().saturating_sub(1));
        let y = y.clamp(a.y, a.bottom().saturating_sub(1));
        let rows = self.cached_rows().map(|r| r.lines.len()).unwrap_or(0);
        let row = (self.scroll_y + (y - a.y) as usize).min(rows.saturating_sub(1));
        (row, self.scroll_x + (x - a.x) as usize)
    }

    /// Copies the selected text to the clipboard and releases the view.
    fn finish_selection(&mut self) {
        let sel = self.selection.take();
        self.pinned = None;
        let Some(sel) = sel.filter(|s| s.dragged) else { return };
        let Some(rows) = self.cached_rows() else { return };
        let text = selected_text(rows, &sel);
        self.copy_text(&text);
    }

    /// Copies `text` (if any) to the clipboard and says so in the footer.
    fn copy_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        copy_to_clipboard(text);
        let n = text.chars().count();
        self.message = Some(format!("Copied {n} character{} to the clipboard", if n == 1 { "" } else { "s" }));
    }

    /// The history text covered by `sel`, one list row per line.
    pub fn history_text(&self, sel: &Selection) -> String {
        let (start, end) = sel.range();
        let lines: Vec<String> = (start.0..=end.0)
            .filter_map(|r| {
                let (from, to) = sel.columns(r)?;
                Some(slice_columns(&self.history_row_text(r)?, from, to))
            })
            .collect();
        lines.join("\n")
    }

    /// Maps a terminal cell to (row, column) in the history list text, clamped to
    /// the list; the column skips the 2-cell selection marker.
    fn history_pos(&self, x: u16, y: u16) -> Option<(usize, usize)> {
        let inner = self.history_area.inner(Margin { vertical: 1, horizontal: 1 });
        if inner.width == 0 || inner.height == 0 || self.history.is_empty() {
            return None;
        }
        let x = x.clamp(inner.x, inner.right() - 1);
        let y = y.clamp(inner.y, inner.bottom() - 1);
        let row = (self.history_offset + (y - inner.y) as usize).min(self.history.len() - 1);
        Some((row, ((x - inner.x) as usize).saturating_sub(2)))
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

    /// The pattern to highlight: what is being typed in the search box, or the
    /// active search unless its highlights are hidden.
    fn highlight_query(&self) -> Option<String> {
        if self.search_open {
            Some(self.editor.text().to_string()).filter(|q| !q.is_empty())
        } else {
            self.search.as_ref().filter(|_| !self.search_hidden).map(|m| m.pattern.clone())
        }
    }

    /// Keys while the search box is open.
    fn on_search_key(&mut self, key: KeyEvent, ctrl: bool) {
        let before = self.editor.text().to_string();
        match key.code {
            KeyCode::Esc => {
                // Cancel: back to where the search started, with the old search.
                self.search_open = false;
                self.scroll_y = self.search_origin;
                return;
            }
            KeyCode::Enter => {
                let text = self.editor.text().to_string();
                let query = if text.trim().is_empty() { String::new() } else { text };
                push_history(&mut self.search_history, &query);
                self.search = Matcher::new(&query, false);
                self.search_hidden = false;
                self.search_open = false;
                self.scroll_y = self.search_origin;
                if self.search.is_some() {
                    self.pending_jump = Some(Jump::From(self.search_origin, true));
                }
                return;
            }
            KeyCode::Backspace if self.editor.text().is_empty() => {
                self.search_open = false;
                self.scroll_y = self.search_origin;
                return;
            }
            KeyCode::Backspace => self.editor.backspace(),
            KeyCode::Delete => self.editor.delete(),
            KeyCode::Left => self.editor.left(),
            KeyCode::Right => self.editor.right(),
            KeyCode::Home => self.editor.home(),
            KeyCode::End => self.editor.end(),
            KeyCode::Char('a') if ctrl => self.editor.home(),
            KeyCode::Char('e') if ctrl => self.editor.end(),
            KeyCode::Char('u') if ctrl => self.editor.kill_to_start(),
            KeyCode::Char('w') if ctrl => self.editor.delete_word(),
            KeyCode::Up => self.editor.history_prev(&self.search_history),
            KeyCode::Down => self.editor.history_next(&self.search_history),
            KeyCode::Char(c) if !ctrl => self.editor.insert(c),
            _ => {}
        }
        // Search while typing: jump to the first match from where it started.
        if self.editor.text() != before {
            self.scroll_y = self.search_origin;
            self.pending_jump = Some(Jump::From(self.search_origin, false));
        }
    }

    /// n / N: shows hidden highlights and jumps once the rows are rebuilt.
    fn request_jump(&mut self, jump: Jump) {
        if self.search.is_none() {
            self.message = Some("No active search (press / to search)".into());
            return;
        }
        self.search_hidden = false;
        self.pending_jump = Some(jump);
    }

    fn do_jump(&mut self, jump: Jump) {
        let Some((_, rows)) = self.cache.as_ref() else { return };
        let matches = &rows.matches;
        let (Some(&first), Some(&last)) = (matches.first(), matches.last()) else {
            if matches!(jump, Jump::From(_, false)) {
                return;
            }
            let label = self.search.as_ref().map(Matcher::label).unwrap_or_default();
            self.message = Some(format!("Pattern not found: {label}"));
            return;
        };
        let cur = self.scroll_y;
        let target = match jump {
            Jump::From(from, _) => matches.iter().copied().find(|&r| r >= from).unwrap_or(first),
            Jump::Next => matches.iter().copied().find(|&r| r > cur).unwrap_or_else(|| {
                self.message = Some("Search hit BOTTOM, continuing at TOP".into());
                first
            }),
            Jump::Prev => matches.iter().rev().copied().find(|&r| r < cur).unwrap_or_else(|| {
                self.message = Some("Search hit TOP, continuing at BOTTOM".into());
                last
            }),
        };
        self.scroll_y = target;
    }

    /// Position of the match at the top of the view, and how many there are.
    pub fn match_position(&self) -> (Option<usize>, usize) {
        let Some(rows) = self.cached_rows() else { return (None, 0) };
        (rows.matches.iter().position(|&r| r == self.scroll_y), rows.matches.len())
    }

    /// Called after rows are (re)built for the frame: performs a deferred jump
    /// to a search match once the match positions are known.
    pub fn after_layout(&mut self) {
        if let Some(jump) = self.pending_jump.take() {
            self.do_jump(jump);
        }
        self.clamp_scroll();
    }
}

#[derive(Debug, Clone, Copy)]
enum Jump {
    /// The first match at or after a row (wrapping); `true` reports "not found".
    From(usize, bool),
    Next,
    Prev,
}

/// Turns a snapshot into display rows: parses ANSI colors, expands tabs,
/// highlights differences from `prev` and matches of `query`, and wraps.
pub fn build_rows(
    snap: &Snapshot,
    prev: Option<&Snapshot>,
    search: Option<&Matcher>,
    wrap: bool,
    width: usize,
) -> Rows {
    let lines = styled_chars(&snap.output);
    let prev_lines: Option<Vec<Vec<char>>> = prev.map(|p| {
        styled_chars(&p.output).into_iter().map(|l| l.into_iter().map(|(c, _)| c).collect()).collect()
    });

    let diff_style = Style::default().add_modifier(Modifier::REVERSED);
    let match_style = Style::default().bg(Color::Yellow).fg(Color::Black);


    let mut rows = Vec::new();
    let mut continued = Vec::new();
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
        if let Some(search) = search {
            // Regex byte ranges mapped back to character indices.
            let hay: String = line.iter().map(|(c, _)| *c).collect();
            let char_at: Vec<usize> = hay.char_indices().map(|(b, _)| b).collect();
            let index = |byte: usize| char_at.partition_point(|&b| b < byte);
            for range in search.find_ranges(&hay) {
                let (start, end) = (index(range.start), index(range.end));
                match_starts.push(start);
                for cell in &mut line[start..end] {
                    cell.1 = cell.1.patch(match_style);
                }
            }
        }

        // Lay out into visual rows.
        let mut row: Vec<(char, Style)> = Vec::new();
        let mut row_w = 0;
        let mut cont = false;
        let mut starts = match_starts.into_iter().peekable();
        for (j, (c, style)) in line.into_iter().enumerate() {
            let cw = c.width().unwrap_or(0);
            if wrap && row_w + cw > width && !row.is_empty() {
                max_width = max_width.max(row_w);
                rows.push(to_line(std::mem::take(&mut row)));
                continued.push(cont);
                cont = true;
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
        continued.push(cont);
    }
    matches.dedup();

    Rows { lines: rows, matches, max_width, continued }
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

/// The text covered by `sel`. Wrapped rows are joined back into one line.
pub fn selected_text(rows: &Rows, sel: &Selection) -> String {
    let (start, end) = sel.range();
    let mut out = String::new();
    for r in start.0..=end.0.min(rows.lines.len().saturating_sub(1)) {
        let Some((from, to)) = sel.columns(r) else { continue };
        if r > start.0 && !rows.continued[r] {
            out.push('\n');
        }
        let mut col = 0;
        for span in &rows.lines[r].spans {
            for c in span.content.chars() {
                let w = c.width().unwrap_or(0);
                if col < to && col + w.max(1) > from {
                    out.push(c);
                }
                col += w;
            }
        }
    }
    out
}

/// The characters of `text` within display columns `[from, to)`.
pub fn slice_columns(text: &str, from: usize, to: usize) -> String {
    let mut out = String::new();
    let mut col = 0;
    for c in text.chars() {
        let w = c.width().unwrap_or(0);
        if col < to && col + w.max(1) > from {
            out.push(c);
        }
        col += w;
    }
    out
}

/// Puts `text` on the system clipboard with an OSC 52 escape sequence,
/// which most modern terminals (and tmux with `set-clipboard on`) support.
fn copy_to_clipboard(text: &str) {
    let seq = format!("\x1b]52;c;{}\x07", base64(text.as_bytes()));
    let mut out = std::io::stdout();
    let _ = out.write_all(seq.as_bytes());
    let _ = out.flush();
}

fn base64(data: &[u8]) -> String {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ABC[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
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
    fn selection_text_joins_wrapped_rows() {
        // "0123456789" wraps into 3 rows at width 4, then "abc".
        let rows = build_rows(&snap(1, "0123456789\nabc\nxyz"), None, None, true, 4);
        assert_eq!(rows.continued, vec![false, true, true, false, false]);
        let sel = |a, c| Selection { anchor: a, cursor: c, dragged: true };
        // Both ends are inclusive.
        assert_eq!(selected_text(&rows, &sel((0, 2), (2, 0))), "2345678");
        assert_eq!(selected_text(&rows, &sel((3, 1), (1, 3))), "789\nab");
        assert_eq!(selected_text(&rows, &sel((2, 1), (4, 0))), "9\nabc\nx");
        // Dragging backwards gives the same text.
        assert_eq!(selected_text(&rows, &sel((4, 0), (2, 1))), "9\nabc\nx");
    }

    #[test]
    fn line_change_finds_first_and_count() {
        assert_eq!(line_change("a\nb\nc", "a\nb\nc"), None);
        assert_eq!(line_change("a\nb\nc", "a\nX\nc"), Some(Change { first: 2, lines: 1 }));
        assert_eq!(line_change("a\nb", "a\nX\nY\nZ"), Some(Change { first: 2, lines: 3 }));
        assert_eq!(line_change("a\nb\nc", "a"), Some(Change { first: 2, lines: 2 }));
    }

    #[test]
    fn base64_encoding() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64("héllo\n".as_bytes()), "aMOpbGxvCg==");
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
        let rows = build_rows(&snap(1, out), None, Matcher::new("error", false).as_ref(), true, 80);
        assert_eq!(text(&rows.lines[0]), "Error one");
        assert_eq!(rows.lines[0].spans[0].style.fg, Some(Color::Black)); // match overrides red
        assert_eq!(rows.matches, vec![0, 2]);
        let rows = build_rows(&snap(1, out), None, Matcher::new("Error", false).as_ref(), true, 80);
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
