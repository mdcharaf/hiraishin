mod model;

use std::collections::BTreeMap;
use std::time::Duration;

use fuzzy_matcher::skim::SkimMatcherV2;
use model::{Entry, Kind, Options, Row, Scope, Target, TreeView, build_entries, tree_view};
use zellij_tile::prelude::*;

/// Rows used by the prompt line and the separator under it.
const HEADER_ROWS: usize = 2;
/// Width of the right-aligned status column.
const META_WIDTH: usize = 26;

#[derive(Default)]
struct State {
    sessions: Vec<SessionInfo>,
    resurrectable: Vec<String>,
    current_session: Option<String>,
    // Fresher than the session metadata Zellij writes to disk once a second.
    current_tabs: Option<Vec<TabInfo>>,
    current_panes: Option<PaneManifest>,

    opts: Options,
    permission_denied: bool,

    entries: Vec<Entry>,
    view: TreeView,
    matcher: SkimMatcherV2,
    query: String,
    scope: Scope,
    selected: usize,
    offset: usize,
    list_rows: usize,
}

register_plugin!(State);

impl ZellijPlugin for State {
    fn load(&mut self, configuration: BTreeMap<String, String>) {
        let flag = |key: &str| configuration.get(key).is_some_and(|v| v.trim() == "true");
        self.opts = Options {
            show_plugins: flag("show_plugins"),
            show_resurrectable: flag("show_resurrectable"),
            self_plugin_id: Some(get_plugin_ids().plugin_id),
        };
        self.matcher = SkimMatcherV2::default().smart_case();

        request_permission(&[
            PermissionType::ReadApplicationState,
            PermissionType::ChangeApplicationState,
        ]);
        subscribe(&[
            EventType::PermissionRequestResult,
            EventType::ModeUpdate,
            EventType::TabUpdate,
            EventType::PaneUpdate,
            EventType::SessionUpdate,
            EventType::Key,
            EventType::Mouse,
        ]);
    }

    fn update(&mut self, event: Event) -> bool {
        match event {
            Event::PermissionRequestResult(status) => {
                self.permission_denied = status == PermissionStatus::Denied;
                if !self.permission_denied {
                    rename_plugin_pane(get_plugin_ids().plugin_id, "hiraishin");
                    // On first run the initial SessionUpdate arrives before the grant and is
                    // dropped; Zellij only re-sends it on change, so fetch a snapshot now.
                    if let Ok(snapshot) = get_session_list() {
                        self.set_sessions(snapshot.live_sessions, snapshot.resurrectable_sessions);
                    }
                }
                true
            }
            Event::ModeUpdate(mode) => {
                if mode.session_name.is_some() && mode.session_name != self.current_session {
                    self.current_session = mode.session_name;
                    self.refresh();
                    return true;
                }
                false
            }
            Event::TabUpdate(tabs) => {
                self.current_tabs = Some(tabs);
                self.refresh();
                true
            }
            Event::PaneUpdate(panes) => {
                self.current_panes = Some(panes);
                self.refresh();
                true
            }
            Event::SessionUpdate(sessions, resurrectable) => {
                self.set_sessions(sessions, resurrectable);
                true
            }
            Event::Key(key) => self.handle_key(key),
            Event::Mouse(mouse) => self.handle_mouse(mouse),
            _ => false,
        }
    }

    fn render(&mut self, rows: usize, cols: usize) {
        self.list_rows = rows.saturating_sub(HEADER_ROWS + 1);
        self.scroll_into_view();

        self.render_prompt(cols);
        print_text_with_coordinates(Text::new("─".repeat(cols)).dim_all(), 0, 1, Some(cols), None);

        let notice = if self.permission_denied {
            Some(("hiraishin needs permission to read and change application state.".to_owned(), true))
        } else if self.entries.is_empty() {
            Some(("Waiting for session data…".to_owned(), false))
        } else if self.view.rows.is_empty() {
            Some((format!("No {} match “{}”", self.scope_noun(), self.query), false))
        } else {
            None
        };
        if let Some((msg, is_error)) = notice {
            let width = cols.saturating_sub(2);
            let text = Text::new(truncate_right(&msg, width));
            let text = if is_error { text.error_color_all() } else { text.dim_all() };
            print_text_with_coordinates(text, 1, HEADER_ROWS, Some(width), None);
        }

        let visible = self.view.rows.iter().enumerate().skip(self.offset).take(self.list_rows);
        for (row, (i, r)) in visible.enumerate() {
            let text = self.render_row(r, i == self.selected, cols);
            print_text_with_coordinates(text, 0, HEADER_ROWS + row, Some(cols), None);
        }

        if rows > HEADER_ROWS + 1 {
            self.render_footer(rows - 1, cols);
        }
    }
}

impl State {
    fn set_sessions(&mut self, sessions: Vec<SessionInfo>, resurrectable: Vec<(String, Duration)>) {
        self.sessions = sessions;
        self.resurrectable = resurrectable.into_iter().map(|(name, _)| name).collect();
        self.refresh();
    }

    /// Rebuilds the entry list from the latest state, keeping the selection on the
    /// same item when it still exists (session updates arrive every second).
    fn refresh(&mut self) {
        let previous = self.selected_entry().map(|e| e.target.clone());

        let mut sessions = self.sessions.clone();
        let current_name = self
            .current_session
            .clone()
            .or_else(|| sessions.iter().find(|s| s.is_current_session).map(|s| s.name.clone()));
        if let Some(name) = current_name {
            let idx = match sessions.iter().position(|s| s.name == name) {
                Some(idx) => idx,
                None => {
                    sessions.push(SessionInfo {
                        name,
                        connected_clients: 1,
                        ..Default::default()
                    });
                    sessions.len() - 1
                }
            };
            for (i, s) in sessions.iter_mut().enumerate() {
                s.is_current_session = i == idx;
            }
            if let Some(tabs) = &self.current_tabs {
                sessions[idx].tabs = tabs.clone();
            }
            if let Some(panes) = &self.current_panes {
                sessions[idx].panes = panes.clone();
            }
            if sessions[idx].tabs.is_empty() {
                // Nothing known about the current session yet; don't show it half-built.
                sessions.remove(idx);
            }
        }

        self.entries = build_entries(&sessions, &self.resurrectable, &self.opts);
        self.rebuild_view();

        let previous = previous.and_then(|target| {
            self.view
                .rows
                .iter()
                .position(|r| r.selectable() && self.entries[r.index].target == target)
        });
        match previous {
            Some(pos) => self.selected = pos,
            None => self.snap_selection(),
        }
    }

    fn rebuild_view(&mut self) {
        self.view = tree_view(&self.entries, &self.query, self.scope, &self.matcher);
    }

    /// Re-filter after the query or scope changed; the best match gets selected.
    fn requery(&mut self) {
        self.rebuild_view();
        self.selected = self.view.best.unwrap_or(0);
        self.offset = 0;
    }

    fn selected_entry(&self) -> Option<&Entry> {
        self.view
            .rows
            .get(self.selected)
            .filter(|r| r.selectable())
            .map(|r| &self.entries[r.index])
    }

    fn is_selectable(&self, i: usize) -> bool {
        self.view.rows.get(i).is_some_and(Row::selectable)
    }

    /// Moves to the next selectable row in `delta`'s direction, skipping context rows
    /// and wrapping around at the ends like fzf's cycle mode.
    fn move_selection(&mut self, delta: isize) {
        let len = self.view.rows.len() as isize;
        if self.view.matched == 0 {
            return;
        }
        let step = delta.signum();
        let mut i = self.selected as isize;
        for _ in 0..delta.abs() {
            loop {
                i = (i + step).rem_euclid(len);
                if self.is_selectable(i as usize) {
                    break;
                }
            }
        }
        self.selected = i as usize;
    }

    fn page_selection(&mut self, delta: isize) {
        let len = self.view.rows.len();
        if len == 0 {
            return;
        }
        let page = self.list_rows.max(1) as isize * delta;
        self.selected = (self.selected as isize + page).clamp(0, len as isize - 1) as usize;
        self.snap_selection();
    }

    /// Ensures the selection is on a selectable row, preferring the nearest one below.
    fn snap_selection(&mut self) {
        let len = self.view.rows.len();
        self.selected = self.selected.min(len.saturating_sub(1));
        if len == 0 || self.is_selectable(self.selected) {
            return;
        }
        let below = (self.selected..len).find(|&i| self.is_selectable(i));
        let above = (0..self.selected).rev().find(|&i| self.is_selectable(i));
        if let Some(i) = below.or(above) {
            self.selected = i;
        }
    }

    fn scroll_into_view(&mut self) {
        let rows = self.list_rows.max(1);
        if self.selected < self.offset {
            self.offset = self.selected;
        } else if self.selected >= self.offset + rows {
            self.offset = self.selected + 1 - rows;
        }
        let max_offset = self.view.rows.len().saturating_sub(rows);
        self.offset = self.offset.min(max_offset);
    }

    fn handle_key(&mut self, key: KeyWithModifier) -> bool {
        let ctrl = |c: char| key.is_key_with_ctrl_modifier(BareKey::Char(c));
        match key.bare_key {
            BareKey::Esc => close_self(),
            _ if ctrl('c') || ctrl('g') => close_self(),
            BareKey::Enter => self.jump(),
            BareKey::Up => self.move_selection(-1),
            BareKey::Down => self.move_selection(1),
            _ if ctrl('p') || ctrl('k') => self.move_selection(-1),
            _ if ctrl('n') || ctrl('j') => self.move_selection(1),
            BareKey::PageUp => self.page_selection(-1),
            BareKey::PageDown => self.page_selection(1),
            _ if ctrl('b') => self.page_selection(-1),
            _ if ctrl('f') => self.page_selection(1),
            BareKey::Home => {
                self.selected = 0;
                self.snap_selection();
            }
            BareKey::End => {
                self.selected = self.view.rows.len().saturating_sub(1);
                self.snap_selection();
            }
            BareKey::Tab if key.has_modifiers(&[KeyModifier::Shift]) => {
                self.scope = self.scope.prev();
                self.requery();
            }
            BareKey::Tab => {
                self.scope = self.scope.next();
                self.requery();
            }
            BareKey::Backspace => {
                if self.query.pop().is_none() {
                    return false;
                }
                self.requery();
            }
            _ if ctrl('u') => {
                self.query.clear();
                self.requery();
            }
            _ if ctrl('w') => {
                let trimmed = self.query.trim_end();
                let cut = trimmed.rfind(char::is_whitespace).map_or(0, |i| i + 1);
                self.query.truncate(cut);
                self.requery();
            }
            BareKey::Char(c)
                if key.has_no_modifiers() || key.has_only_modifiers(&[KeyModifier::Shift]) =>
            {
                self.query.push(c);
                self.requery();
            }
            _ => return false,
        }
        true
    }

    fn handle_mouse(&mut self, mouse: Mouse) -> bool {
        match mouse {
            Mouse::ScrollUp(_) => self.move_selection(-1),
            Mouse::ScrollDown(_) => self.move_selection(1),
            Mouse::LeftClick(line, _) => {
                let Some(row) = usize::try_from(line).ok().and_then(|l| l.checked_sub(HEADER_ROWS)) else {
                    return false;
                };
                if row >= self.list_rows || !self.is_selectable(self.offset + row) {
                    return false;
                }
                self.selected = self.offset + row;
                self.jump();
            }
            _ => return false,
        }
        true
    }

    fn jump(&mut self) {
        let Some(entry) = self.selected_entry() else {
            return;
        };
        let here = entry.in_current_session;
        match entry.target.clone() {
            Target::Session { name, .. } => {
                if !here {
                    // Also resurrects exited sessions.
                    switch_session(Some(&name));
                }
            }
            Target::Tab { session, position } => {
                if here {
                    switch_tab_to(position as u32 + 1);
                } else {
                    switch_session_with_focus(&session, Some(position), None);
                }
            }
            Target::Pane {
                session,
                tab_position,
                id,
                is_plugin,
            } => {
                if here {
                    let pane = if is_plugin { PaneId::Plugin(id) } else { PaneId::Terminal(id) };
                    focus_pane_with_id(pane, true, false);
                } else {
                    switch_session_with_focus(&session, Some(tab_position), Some((id, is_plugin)));
                }
            }
        }
        close_self();
    }

    fn scope_noun(&self) -> &'static str {
        match self.scope {
            Scope::All => "items",
            other => other.label(),
        }
    }

    fn render_prompt(&self, cols: usize) {
        let query_width = cols.saturating_sub(5);
        let query = truncate_left(&self.query, query_width);
        let prompt = format!(" ❯ {query}▏");
        print_text_with_coordinates(
            Text::new(&prompt).color_range(2, 0..3),
            0,
            0,
            Some(cols),
            None,
        );

        // Scope pills on the right: "all  sessions  tabs  panes".
        let mut pills = String::new();
        let mut active = 0..0;
        for (i, scope) in Scope::ALL.iter().enumerate() {
            if i > 0 {
                pills.push_str("  ");
            }
            let start = pills.chars().count();
            pills.push_str(scope.label());
            if *scope == self.scope {
                active = start..pills.chars().count();
            }
        }
        let pills_width = pills.chars().count();
        if cols > prompt.chars().count() + pills_width + 2 {
            let text = Text::new(&pills).dim_all().color_range(0, active);
            print_text_with_coordinates(text, cols - pills_width - 1, 0, None, None);
        }
    }

    fn render_row(&self, row: &Row, selected: bool, cols: usize) -> Text {
        let entry = &self.entries[row.index];

        // "▌ ├─ name ……………… meta "
        let gutter = if selected { "▌ " } else { "  " };
        let gutter_len = gutter.chars().count();
        let guide_len = row.guide.chars().count();

        let meta_width = META_WIDTH.min(cols / 3);
        let meta = truncate_right(&entry.meta, meta_width);
        let label_width = cols.saturating_sub(gutter_len + meta_width + 2).max(1);
        let label = truncate_right(&format!("{}{}", row.guide, entry.name), label_width);

        let pad = label_width.saturating_sub(label.chars().count());
        let meta_pad = meta_width.saturating_sub(meta.chars().count());
        let line = format!("{gutter}{label}{}  {}{meta}", " ".repeat(pad), " ".repeat(meta_pad));
        let total = line.chars().count();
        let line = format!("{line}{}", " ".repeat(cols.saturating_sub(total)));

        let name_start = gutter_len + guide_len;
        let name_end = gutter_len + label.chars().count();
        let meta_start = gutter_len + label_width + 2 + meta_pad;
        let meta_range = meta_start..meta_start + meta.chars().count();
        let highlights: Vec<usize> = row
            .indices
            .iter()
            .map(|i| i + name_start)
            .filter(|&i| i < name_end)
            .collect();

        let mut text = Text::new(line).dim_range(gutter_len..name_start);
        if !row.selectable() {
            // Context row: an ancestor shown only to keep a match in its place.
            text = text.dim_range(name_start..name_end);
        } else {
            let level = match entry.kind {
                Kind::Session => Some(2),
                Kind::Tab => Some(1),
                Kind::Pane => None,
            };
            if let Some(level) = level {
                let name: Vec<usize> = (name_start..name_end).filter(|i| !highlights.contains(i)).collect();
                text = text.color_indices(level, name);
            }
        }
        if !highlights.is_empty() {
            text = text.color_indices(0, highlights);
        }
        text = if entry.here {
            text.success_color_range(meta_range)
        } else {
            text.dim_range(meta_range)
        };
        if selected {
            text = text.selected().color_range(0, 0..1);
        }
        text
    }

    fn render_footer(&self, row: usize, cols: usize) {
        let count = format!("{}/{} ", self.view.matched, self.entries.len());
        let help = " ↑↓ move · ⏎ jump · ⇥ scope · ^u clear · esc close";
        let help = truncate_right(help, cols.saturating_sub(count.chars().count() + 1));
        let gap = cols.saturating_sub(help.chars().count() + count.chars().count());
        let line = format!("{help}{}{count}", " ".repeat(gap));
        print_text_with_coordinates(Text::new(line).dim_all(), 0, row, Some(cols), None);
    }
}

fn truncate_right(s: &str, width: usize) -> String {
    let len = s.chars().count();
    if len <= width {
        return s.to_owned();
    }
    if width == 0 {
        return String::new();
    }
    let mut out: String = s.chars().take(width - 1).collect();
    out.push('…');
    out
}

/// Keeps the tail of `s`, so the end of a long query stays visible.
fn truncate_left(s: &str, width: usize) -> String {
    let len = s.chars().count();
    if len <= width {
        return s.to_owned();
    }
    if width == 0 {
        return String::new();
    }
    let mut out = String::from("…");
    out.extend(s.chars().skip(len - (width - 1)));
    out
}
