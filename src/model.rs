//! Pure data model: turns Zellij session state into a flat list of searchable
//! entries and ranks them against a query. Kept free of host calls so it can be
//! unit-tested natively.

use fuzzy_matcher::FuzzyMatcher;
use fuzzy_matcher::skim::SkimMatcherV2;
use zellij_tile::prelude::{PaneInfo, SessionInfo};

pub const SEP: &str = " › ";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Session,
    Tab,
    Pane,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Scope {
    #[default]
    All,
    Sessions,
    Tabs,
    Panes,
}

impl Scope {
    pub const ALL: [Scope; 4] = [Scope::All, Scope::Sessions, Scope::Tabs, Scope::Panes];

    pub fn label(self) -> &'static str {
        match self {
            Scope::All => "all",
            Scope::Sessions => "sessions",
            Scope::Tabs => "tabs",
            Scope::Panes => "panes",
        }
    }

    pub fn next(self) -> Scope {
        let i = Self::ALL.iter().position(|s| *s == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }

    pub fn prev(self) -> Scope {
        let i = Self::ALL.iter().position(|s| *s == self).unwrap_or(0);
        Self::ALL[(i + Self::ALL.len() - 1) % Self::ALL.len()]
    }

    pub fn admits(self, kind: Kind) -> bool {
        match self {
            Scope::All => true,
            Scope::Sessions => kind == Kind::Session,
            Scope::Tabs => kind == Kind::Tab,
            Scope::Panes => kind == Kind::Pane,
        }
    }
}

/// Where selecting an entry should take the user.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Session {
        name: String,
        resurrect: bool,
    },
    Tab {
        session: String,
        position: usize,
    },
    Pane {
        session: String,
        tab_position: usize,
        id: u32,
        is_plugin: bool,
    },
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub kind: Kind,
    pub target: Target,
    /// The entry's own name (session name, tab name or pane title).
    pub name: String,
    /// Breadcrumb `session › tab › pane`; this is what the query is matched against.
    pub path: String,
    /// Nesting level in the tree (0 = session, 1 = tab, 2 = pane).
    pub depth: usize,
    /// Index of the parent entry (a pane's tab, a tab's session).
    pub parent: Option<usize>,
    /// Short status shown on the right (e.g. "current · 3 tabs").
    pub meta: String,
    /// Where the user is right now: the current session, its active tab, or its focused pane.
    pub here: bool,
    pub in_current_session: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Options {
    pub show_plugins: bool,
    pub show_resurrectable: bool,
    /// The picker's own plugin id, so it never lists itself.
    pub self_plugin_id: Option<u32>,
}

/// Flattens sessions into entries in tree order: the current session first, then
/// the rest alphabetically; each session followed by its tabs, each tab by its panes.
pub fn build_entries(sessions: &[SessionInfo], resurrectable: &[String], opts: &Options) -> Vec<Entry> {
    let mut ordered: Vec<&SessionInfo> = sessions.iter().collect();
    ordered.sort_by(|a, b| {
        b.is_current_session
            .cmp(&a.is_current_session)
            .then_with(|| a.name.cmp(&b.name))
    });

    let mut entries = Vec::new();
    for session in ordered {
        push_session(&mut entries, session, opts);
    }

    if opts.show_resurrectable {
        let mut names: Vec<&String> = resurrectable
            .iter()
            .filter(|name| !sessions.iter().any(|s| &s.name == *name))
            .collect();
        names.sort();
        for name in names {
            entries.push(Entry {
                kind: Kind::Session,
                target: Target::Session {
                    name: name.clone(),
                    resurrect: true,
                },
                name: name.clone(),
                path: name.clone(),
                depth: 0,
                parent: None,
                meta: "exited · resurrect".to_owned(),
                here: false,
                in_current_session: false,
            });
        }
    }

    entries
}

fn push_session(entries: &mut Vec<Entry>, session: &SessionInfo, opts: &Options) {
    let current = session.is_current_session;
    let mut tabs: Vec<_> = session.tabs.iter().collect();
    tabs.sort_by_key(|t| t.position);

    let mut meta = Vec::new();
    if current {
        meta.push("current".to_owned());
    }
    meta.push(plural(tabs.len(), "tab"));
    if session.connected_clients > 1 {
        meta.push(plural(session.connected_clients, "client"));
    }

    let session_index = entries.len();
    entries.push(Entry {
        kind: Kind::Session,
        target: Target::Session {
            name: session.name.clone(),
            resurrect: false,
        },
        name: session.name.clone(),
        path: session.name.clone(),
        depth: 0,
        parent: None,
        meta: meta.join(" · "),
        here: current,
        in_current_session: current,
    });

    for tab in tabs {
        let tab_path = format!("{}{SEP}{}", session.name, tab.name);
        let panes: Vec<&PaneInfo> = session
            .panes
            .panes
            .get(&tab.position)
            .map(|panes| {
                let mut panes: Vec<&PaneInfo> = panes
                    .iter()
                    .filter(|p| is_listed(p, current, opts))
                    .collect();
                // Tiled panes before floating ones; otherwise keep Zellij's order.
                panes.sort_by_key(|p| p.is_floating);
                panes
            })
            .unwrap_or_default();

        let mut meta = Vec::new();
        if tab.active {
            meta.push("active".to_owned());
        }
        meta.push(plural(panes.len(), "pane"));

        let tab_index = entries.len();
        entries.push(Entry {
            kind: Kind::Tab,
            target: Target::Tab {
                session: session.name.clone(),
                position: tab.position,
            },
            name: tab.name.clone(),
            path: tab_path.clone(),
            depth: 1,
            parent: Some(session_index),
            meta: meta.join(" · "),
            here: current && tab.active,
            in_current_session: current,
        });

        for pane in panes {
            let name = pane_name(pane);
            let mut meta = Vec::new();
            if pane.is_focused && tab.active {
                meta.push("focused");
            }
            if pane.is_floating {
                meta.push("floating");
            }
            if pane.is_suppressed {
                meta.push("hidden");
            }
            if pane.exited {
                meta.push("exited");
            }
            if pane.is_plugin {
                meta.push("plugin");
            }
            entries.push(Entry {
                kind: Kind::Pane,
                target: Target::Pane {
                    session: session.name.clone(),
                    tab_position: tab.position,
                    id: pane.id,
                    is_plugin: pane.is_plugin,
                },
                path: format!("{tab_path}{SEP}{name}"),
                name,
                depth: 2,
                parent: Some(tab_index),
                meta: meta.join(" · "),
                here: current && tab.active && pane.is_focused,
                in_current_session: current,
            });
        }
    }
}

fn is_listed(pane: &PaneInfo, in_current_session: bool, opts: &Options) -> bool {
    if !pane.is_selectable {
        return false;
    }
    if pane.is_plugin {
        if in_current_session && Some(pane.id) == opts.self_plugin_id {
            return false;
        }
        return opts.show_plugins;
    }
    true
}

fn pane_name(pane: &PaneInfo) -> String {
    let title = pane.title.trim();
    if !title.is_empty() {
        return title.to_owned();
    }
    if let Some(command) = pane.terminal_command.as_deref().filter(|c| !c.is_empty()) {
        return command.to_owned();
    }
    format!("Pane #{}", pane.id)
}

fn plural(n: usize, word: &str) -> String {
    if n == 1 {
        format!("1 {word}")
    } else {
        format!("{n} {word}s")
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    /// Index into the entries vector.
    pub index: usize,
    /// Tree guide drawn before the name, e.g. `│  ├─ `.
    pub guide: String,
    /// Match score; `None` for context rows, which are only shown so that a match
    /// keeps its place in the tree and can't be selected.
    pub score: Option<i64>,
    /// Char indices into `Entry::name` that matched the query.
    pub indices: Vec<usize>,
}

impl Row {
    pub fn selectable(&self) -> bool {
        self.score.is_some()
    }
}

#[derive(Clone, Debug, Default)]
pub struct TreeView {
    pub rows: Vec<Row>,
    /// Row index of the best match (the first selectable row when not searching).
    pub best: Option<usize>,
    /// Number of selectable rows.
    pub matched: usize,
}

/// Filters the session › tab › pane tree by scope and query. An entry is kept when it
/// matches, and its ancestors are kept as context so the hierarchy stays intact.
/// Whitespace splits the query into terms that must all match (fzf-style) against
/// the entry's full path. While searching, sessions are ordered by their best match;
/// within a session the tree order is kept.
pub fn tree_view(entries: &[Entry], query: &str, scope: Scope, matcher: &SkimMatcherV2) -> TreeView {
    let terms: Vec<&str> = query.split_whitespace().collect();
    let hits: Vec<Option<(i64, Vec<usize>)>> = entries
        .iter()
        .map(|entry| {
            if !scope.admits(entry.kind) {
                return None;
            }
            let mut score = 0;
            let mut indices = Vec::new();
            for term in &terms {
                let (s, mut idx) = matcher.fuzzy_indices(&entry.path, term)?;
                score += s;
                indices.append(&mut idx);
            }
            // The path ends with the entry's own name; keep only hits inside it.
            let offset = entry.path.chars().count() - entry.name.chars().count();
            let mut indices: Vec<usize> = indices
                .into_iter()
                .filter(|&i| i >= offset)
                .map(|i| i - offset)
                .collect();
            indices.sort_unstable();
            indices.dedup();
            Some((score, indices))
        })
        .collect();

    let mut include = vec![false; entries.len()];
    for (i, hit) in hits.iter().enumerate() {
        if hit.is_some() {
            let mut node = Some(i);
            while let Some(k) = node {
                include[k] = true;
                node = entries[k].parent;
            }
        }
    }

    // Each session and everything below it forms a contiguous block of entries.
    let mut blocks: Vec<std::ops::Range<usize>> = Vec::new();
    for (i, entry) in entries.iter().enumerate() {
        if entry.depth == 0 || blocks.is_empty() {
            blocks.push(i..i + 1);
        } else if let Some(last) = blocks.last_mut() {
            last.end = i + 1;
        }
    }
    if !terms.is_empty() {
        let best = |range: &std::ops::Range<usize>| {
            range.clone().filter_map(|i| hits[i].as_ref().map(|h| h.0)).max()
        };
        // Stable: ties keep the original order (current session first).
        blocks.sort_by(|a, b| best(b).cmp(&best(a)));
    }

    let mut rows: Vec<Row> = blocks
        .into_iter()
        .flatten()
        .filter(|&i| include[i])
        .map(|i| Row {
            index: i,
            guide: String::new(),
            score: hits[i].as_ref().map(|h| h.0),
            indices: hits[i].as_ref().map(|h| h.1.clone()).unwrap_or_default(),
        })
        .collect();

    draw_guides(entries, &mut rows);

    let mut best: Option<usize> = None;
    for (i, row) in rows.iter().enumerate() {
        let better = match (row.score, best.and_then(|b| rows[b].score)) {
            (Some(_), None) => true,
            (Some(s), Some(b)) => !terms.is_empty() && s > b,
            _ => false,
        };
        if better {
            best = Some(i);
        }
    }
    let matched = rows.iter().filter(|r| r.selectable()).count();
    TreeView { rows, best, matched }
}

/// Fills in `├─ ` / `└─ ` / `│  ` guides based on which rows are visible.
fn draw_guides(entries: &[Entry], rows: &mut [Row]) {
    let depths: Vec<usize> = rows.iter().map(|r| entries[r.index].depth).collect();
    // continues[d]: the most recent row at depth d has a later sibling.
    let mut continues = vec![false; depths.iter().max().map_or(0, |d| d + 1)];
    for i in 0..rows.len() {
        let depth = depths[i];
        let has_next_sibling = depths[i + 1..]
            .iter()
            .find(|&&d| d <= depth)
            .is_some_and(|&d| d == depth);
        let mut guide = String::new();
        for &cont in continues.iter().take(depth).skip(1) {
            guide.push_str(if cont { "│  " } else { "   " });
        }
        if depth > 0 {
            guide.push_str(if has_next_sibling { "├─ " } else { "└─ " });
        }
        continues[depth] = has_next_sibling;
        rows[i].guide = guide;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use zellij_tile::prelude::{PaneManifest, TabInfo};

    fn pane(id: u32, title: &str, focused: bool) -> PaneInfo {
        PaneInfo {
            id,
            title: title.to_owned(),
            is_focused: focused,
            is_selectable: true,
            ..Default::default()
        }
    }

    fn session(name: &str, current: bool, tabs: &[(&str, Vec<PaneInfo>)]) -> SessionInfo {
        let mut panes = HashMap::new();
        let tabs = tabs
            .iter()
            .enumerate()
            .map(|(position, (tab_name, tab_panes))| {
                panes.insert(position, tab_panes.clone());
                TabInfo {
                    position,
                    name: (*tab_name).to_owned(),
                    active: position == 0,
                    ..Default::default()
                }
            })
            .collect();
        SessionInfo {
            name: name.to_owned(),
            tabs,
            panes: PaneManifest { panes },
            is_current_session: current,
            connected_clients: 1,
            ..Default::default()
        }
    }

    fn fixture() -> Vec<SessionInfo> {
        vec![
            session(
                "zeta",
                false,
                &[("logs", vec![pane(1, "tail -f app.log", true)])],
            ),
            session(
                "work",
                true,
                &[
                    (
                        "editor",
                        vec![pane(1, "nvim src/main.rs", true), pane(2, "cargo watch", false)],
                    ),
                    ("server", vec![pane(3, "npm run dev", true)]),
                ],
            ),
        ]
    }

    fn matcher() -> SkimMatcherV2 {
        SkimMatcherV2::default().smart_case()
    }

    /// Renders a view as `guide + name`, with context rows wrapped in parentheses.
    fn lines(entries: &[Entry], view: &TreeView) -> Vec<String> {
        view.rows
            .iter()
            .map(|r| {
                let name = &entries[r.index].name;
                if r.selectable() {
                    format!("{}{name}", r.guide)
                } else {
                    format!("{}({name})", r.guide)
                }
            })
            .collect()
    }

    #[test]
    fn builds_tree_with_current_session_first() {
        let entries = build_entries(&fixture(), &[], &Options::default());
        let all: Vec<&str> = entries.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(
            all,
            vec![
                "work",
                "work › editor",
                "work › editor › nvim src/main.rs",
                "work › editor › cargo watch",
                "work › server",
                "work › server › npm run dev",
                "zeta",
                "zeta › logs",
                "zeta › logs › tail -f app.log",
            ]
        );
        assert!(entries[0].here);
        assert!(entries[1].here);
        assert!(entries[2].here);
        assert!(!entries[3].here);
        assert!(!entries[6].here);
        assert_eq!(entries[0].meta, "current · 2 tabs");
        assert_eq!(entries[1].meta, "active · 2 panes");
    }

    #[test]
    fn hides_unselectable_self_and_plugin_panes() {
        let mut sessions = fixture();
        let panes = sessions[1].panes.panes.get_mut(&0).unwrap();
        panes.push(PaneInfo {
            id: 7,
            is_plugin: true,
            is_selectable: true,
            title: "hiraishin".into(),
            ..Default::default()
        });
        panes.push(PaneInfo {
            id: 8,
            is_plugin: true,
            is_selectable: true,
            title: "strider".into(),
            ..Default::default()
        });
        panes.push(PaneInfo {
            id: 9,
            is_plugin: true,
            is_selectable: false,
            title: "tab-bar".into(),
            ..Default::default()
        });
        let mut opts = Options {
            self_plugin_id: Some(7),
            ..Default::default()
        };
        let names = |opts: &Options| -> Vec<String> {
            build_entries(&sessions, &[], opts)
                .into_iter()
                .filter(|e| e.kind == Kind::Pane)
                .map(|e| e.name)
                .collect()
        };
        assert!(!names(&opts).contains(&"strider".to_string()));
        opts.show_plugins = true;
        let shown = names(&opts);
        assert!(shown.contains(&"strider".to_string()));
        assert!(!shown.contains(&"hiraishin".to_string()));
        assert!(!shown.contains(&"tab-bar".to_string()));
    }

    #[test]
    fn resurrectable_sessions_are_opt_in() {
        let res = vec!["old".to_string(), "work".to_string()];
        let without = build_entries(&fixture(), &res, &Options::default());
        assert!(!without.iter().any(|e| e.name == "old"));
        let opts = Options {
            show_resurrectable: true,
            ..Default::default()
        };
        let with = build_entries(&fixture(), &res, &opts);
        let old: Vec<&Entry> = with.iter().filter(|e| e.name == "old").collect();
        assert_eq!(old.len(), 1);
        assert_eq!(
            old[0].target,
            Target::Session {
                name: "old".into(),
                resurrect: true
            }
        );
        // A live session with the same name is not duplicated.
        assert_eq!(with.iter().filter(|e| e.path == "work").count(), 1);
    }

    #[test]
    fn parents_point_at_the_enclosing_tab_and_session() {
        let entries = build_entries(&fixture(), &[], &Options::default());
        assert_eq!(entries[0].parent, None);
        assert_eq!(entries[1].parent, Some(0));
        assert_eq!(entries[2].parent, Some(1));
        assert_eq!(entries[5].parent, Some(4));
        assert_eq!(entries[7].parent, Some(6));
    }

    #[test]
    fn empty_query_shows_full_tree_with_guides() {
        let entries = build_entries(&fixture(), &[], &Options::default());
        let view = tree_view(&entries, "", Scope::All, &matcher());
        assert_eq!(
            lines(&entries, &view),
            vec![
                "work",
                "├─ editor",
                "│  ├─ nvim src/main.rs",
                "│  └─ cargo watch",
                "└─ server",
                "   └─ npm run dev",
                "zeta",
                "└─ logs",
                "   └─ tail -f app.log",
            ]
        );
        assert_eq!(view.best, Some(0));
        assert_eq!(view.matched, entries.len());
    }

    #[test]
    fn scope_keeps_ancestors_as_unselectable_context() {
        let entries = build_entries(&fixture(), &[], &Options::default());
        let view = tree_view(&entries, " ", Scope::Tabs, &matcher());
        assert_eq!(
            lines(&entries, &view),
            vec!["(work)", "├─ editor", "└─ server", "(zeta)", "└─ logs"]
        );
        assert_eq!(view.best, Some(1));
        assert_eq!(view.matched, 3);

        let view = tree_view(&entries, "", Scope::Sessions, &matcher());
        assert_eq!(lines(&entries, &view), vec!["work", "zeta"]);
    }

    #[test]
    fn query_filters_tree_and_selects_best_match() {
        let entries = build_entries(&fixture(), &[], &Options::default());

        let view = tree_view(&entries, "nvim", Scope::All, &matcher());
        assert_eq!(
            lines(&entries, &view),
            vec!["(work)", "└─ (editor)", "   └─ nvim src/main.rs"]
        );
        assert_eq!(view.best, Some(2));
        assert_eq!(view.matched, 1);

        // Multiple terms must all match, in any position.
        let view = tree_view(&entries, "zeta tail", Scope::All, &matcher());
        assert_eq!(
            lines(&entries, &view),
            vec!["(zeta)", "└─ (logs)", "   └─ tail -f app.log"]
        );

        assert!(tree_view(&entries, "qqqq", Scope::All, &matcher()).rows.is_empty());
    }

    #[test]
    fn query_orders_sessions_by_best_match() {
        let entries = build_entries(&fixture(), &[], &Options::default());
        // Only zeta's tab matches well, so zeta moves above the current session.
        let view = tree_view(&entries, "logs", Scope::All, &matcher());
        assert_eq!(entries[view.rows[0].index].name, "zeta");
        let best = &view.rows[view.best.unwrap()];
        assert_eq!(entries[best.index].name, "logs");
    }

    #[test]
    fn match_indices_point_into_the_entry_name() {
        let entries = build_entries(&fixture(), &[], &Options::default());
        let view = tree_view(&entries, "work cargo", Scope::Panes, &matcher());
        let row = &view.rows[view.best.unwrap()];
        let name: Vec<char> = entries[row.index].name.chars().collect();
        let hit: String = row.indices.iter().map(|&i| name[i]).collect();
        // "work" matched the session part of the path, so only "cargo" is highlighted.
        assert_eq!(hit, "cargo");
    }

    #[test]
    fn scope_cycles() {
        assert_eq!(Scope::All.next(), Scope::Sessions);
        assert_eq!(Scope::Panes.next(), Scope::All);
        assert_eq!(Scope::All.prev(), Scope::Panes);
    }
}
