<img src="assets/icon.svg" width="64" height="64" alt="">

# hiraishin

A fuzzy finder for [Zellij](https://zellij.dev) that searches **every session, tab and pane** on your machine and jumps straight to the one you pick, including panes in other sessions.

Think of it as tmux's `choose-tree` (or herdr's workspace switcher on `Ctrl b g`), extended down to individual panes.

hiraishin shows everything as one tree, sessions → tabs → panes. Your current session comes first, then the other sessions alphabetically.

```
┌ hiraishin ──────────────────────────────────────────────── PIN [ ] ┐
│ ❯ ▏                                     all  sessions  tabs  panes │
│────────────────────────────────────────────────────────────────────│
│▌ alpha                                             current · 2 tabs│
│  ├─ editor                                         active · 2 panes│
│  │  ├─ nvim                                                 focused│
│  │  └─ build-watch                                                 │
│  └─ notes                                                    1 pane│
│     └─ todo                                                        │
│  beta                                                        2 tabs│
│  ├─ api                                                     2 panes│
│  │  ├─ server                                                      │
│  │  └─ tests                                                       │
│  └─ database                                                 1 pane│
│     └─ psql                                                        │
│ ↑↓ move · ⏎ jump · ⇥ scope · ^u clear · esc close            17/17 │
└────────────────────────────────────────────────────────────────────┘
```

Typing filters the tree instead of flattening it. Each match keeps its parent tab and session above it as dimmed context rows, which can't be selected. Sessions with the best matches move to the top, and the best match is selected for you.

```
│ ❯ api▏                                  all  sessions  tabs  panes │
│────────────────────────────────────────────────────────────────────│
│  beta                                                        2 tabs│
│▌ └─ api                                                     2 panes│
│     ├─ server                                                      │
│     └─ tests                                                       │
│  alpha                                             current · 2 tabs│
│  └─ editor                                         active · 2 panes│
│     ├─ nvim                                                 focused│
│     └─ build-watch                                                 │
```

## Features

- **One tree for everything.** Sessions, tabs and panes from every running Zellij session, not just the current one, drawn with tree guides. Session names are colored differently from tab names, so rows don't need type labels.
- **fzf-style matching.** Each entry is matched on its full path (`session › tab › pane`), so `beta tests` finds the `tests` pane in session `beta`. Space-separated terms must all match, in any order. Matching is smart-case: all-lowercase queries ignore case.
- **Precise jumps.**
  - Picking a *session* switches to it.
  - Picking a *tab* switches session if needed and focuses that tab.
  - Picking a *pane* switches session if needed, moves to the pane's tab and focuses the pane. Hidden panes are brought back, and floating panes are shown.
- **Scopes.** Press `Tab` to cycle the scope between all, sessions, tabs and panes. In the tabs and panes scopes, parent rows stay visible as dimmed context, and the selection skips over them.
- **Status at a glance.** Each row shows whether it's the current session, the active tab or the focused pane, along with tab and pane counts and floating, hidden or exited state.
- **Mouse.** Scroll to move the selection, click a row to jump to it.

## Requirements

- Zellij **0.45.x**. The plugin is built against `zellij-tile 0.45`. Zellij plugins are tied to the plugin API version, so rebuild against the matching `zellij-tile` if you're on another version.
- To build from source: Rust (via [rustup](https://rustup.rs)) and the `wasm32-wasip1` target.

## Install

### 1. Build

```sh
# one-time toolchain setup
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh   # skip if you already have rustup
rustup target add wasm32-wasip1

# build and copy to ~/.config/zellij/plugins/hiraishin.wasm
make install
```

`make install` runs `cargo build --release` (the repo's `.cargo/config.toml` already targets `wasm32-wasip1`) and copies `target/wasm32-wasip1/release/hiraishin.wasm` into `~/.config/zellij/plugins/`. Set `PLUGIN_DIR=/some/dir` to install somewhere else.

You can also do it by hand:

```sh
cargo build --release
mkdir -p ~/.config/zellij/plugins
cp target/wasm32-wasip1/release/hiraishin.wasm ~/.config/zellij/plugins/
```

### 2. Add a key binding

Edit `~/.config/zellij/config.kdl` and add a binding inside the `keybinds { … }` block.

**`Ctrl b` then `g`** (tmux mode, like herdr's `Ctrl b g`):

```kdl
keybinds {
    tmux {
        bind "g" {
            LaunchOrFocusPlugin "file:~/.config/zellij/plugins/hiraishin.wasm" {
                floating true
                move_to_focused_tab true
            }
            SwitchToMode "normal"
        }
    }
}
```

**Or a single chord from any mode** (e.g. `Alt g`):

```kdl
keybinds {
    shared_except "locked" {
        bind "Alt g" {
            LaunchOrFocusPlugin "file:~/.config/zellij/plugins/hiraishin.wasm" {
                floating true
                move_to_focused_tab true
            }
        }
    }
}
```

If you already have a `tmux { … }` or `shared_except "locked" { … }` block, add the `bind` to that block instead of creating a second one. Zellij reloads `config.kdl` automatically. If the binding doesn't work, restart the session.

### 3. Grant permissions (first run only)

The first time you open hiraishin, Zellij asks you to allow it to:

1. **Access Zellij state**, to list sessions, tabs and panes.
2. **Change Zellij state**, to switch sessions and focus tabs and panes.

Press `y`. Zellij remembers the answer, so you won't be asked again unless the plugin path changes.

### Try it without a key binding

```sh
zellij action launch-or-focus-plugin --floating "file:$HOME/.config/zellij/plugins/hiraishin.wasm"
```

## Usage

Open hiraishin with your key binding, type to filter, then press `Enter` to jump. hiraishin closes itself after a jump.

| Key | Action |
| --- | --- |
| *type* | Filter (fuzzy, smart-case, space-separated terms) |
| `Enter` | Jump to the selected session, tab or pane |
| `↑` / `↓`, `Ctrl p` / `Ctrl n`, `Ctrl k` / `Ctrl j` | Move the selection (skips dimmed context rows, wraps around) |
| `PgUp` / `PgDn`, `Ctrl b` / `Ctrl f` | Move one page |
| `Home` / `End` | Jump to the first or last result |
| `Tab` / `Shift Tab` | Cycle the scope: all → sessions → tabs → panes |
| `Backspace` | Delete one character |
| `Ctrl w` | Delete the last word |
| `Ctrl u` | Clear the query |
| `Esc`, `Ctrl c`, `Ctrl g` | Close hiraishin |
| Mouse wheel / click | Move the selection / jump to the clicked row |

### Search tips

- `work` shows session `work` with its tabs and panes, and selects the session itself, since a parent wins ties with its children.
- `work nvim` narrows to `nvim` panes inside session `work`.
- `Tab` to the **panes** scope and type part of a command (`cargo`, `psql`, `npm`) to find the pane that's running it. Pane names are the pane titles Zellij shows in the frame, which usually reflect the running command or cwd, or the name you gave the pane.
- `Tab` to the **sessions** scope for a plain session switcher.

## Configuration

Pass options inside the `LaunchOrFocusPlugin` block:

```kdl
LaunchOrFocusPlugin "file:~/.config/zellij/plugins/hiraishin.wasm" {
    floating true
    move_to_focused_tab true

    show_plugins "true"        // also list plugin panes (e.g. strider, filepicker). Default: false
    show_resurrectable "true"  // also list exited sessions; picking one resurrects it. Default: false
}
```

UI panes such as the tab bar and status bar are never listed, and neither is hiraishin itself.

## How it works

- hiraishin subscribes to Zellij's `SessionUpdate` event, which carries the tabs and panes of every session on the machine. Live `TabUpdate` and `PaneUpdate` events override that data for the current session, so the current session is always up to date. On first run it also calls `get_session_list()` right after permission is granted.
- Jumps within the current session use `switch_tab_to` and `focus_pane_with_id`. Jumps to another session use `switch_session_with_focus(session, tab, pane)`, which attaches your client to that session and focuses the target in one step.
- The list refreshes as sessions change while hiraishin is open, and the selection stays on the same item.

## Development

```sh
make build   # cargo build --release → target/wasm32-wasip1/release/hiraishin.wasm
make test    # unit tests for entry building and ranking, run on the host target
```

The code is split into:

- `src/model.rs`: pure logic that turns `SessionInfo` into tree entries, then filters them by scope and query into a tree view with guides. It makes no host calls, so it can be unit-tested natively.
- `src/main.rs`: the Zellij plugin itself (events, key handling, rendering and navigation).

For a fast edit loop, run hiraishin in a floating pane and reload it after each build:

```sh
cargo build --release && zellij action start-or-reload-plugin "file:$PWD/target/wasm32-wasip1/release/hiraishin.wasm"
```

## Troubleshooting

- **Only the current session shows up.** Other sessions come from Zellij's session metadata, which is refreshed about once a second. Wait a moment. If they still don't appear, make sure `disable_session_metadata` isn't set to `true` in your config.
- **"hiraishin needs permission…".** You denied the permission prompt. Remove the hiraishin entry from Zellij's `permissions.kdl` in the Zellij cache directory (on macOS: `~/Library/Caches/org.Zellij-Contributors.Zellij/permissions.kdl`) and open hiraishin again.
- **The plugin fails to load after a Zellij upgrade.** Bump `zellij-tile` in `Cargo.toml` to match your Zellij version, then run `make install` again.
