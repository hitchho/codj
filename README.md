<h1 align="center">codj</h1>

**codj** is a terminal music player with built-in [Soulseek](https://www.slsknet.org) search and downloads, written in [Rust](https://www.rust-lang.org). It is the TUI for [**codjd**](https://github.com/yassinebenarbia/mplayer-server), the player daemon, and talks to it over [D-Bus](https://www.freedesktop.org/wiki/Software/dbus/#whatisd-bus). Music keeps playing, and downloads keep going, when codj is closed.

*codj* reads two ways: **co-DJ** (agents can plan and mix with you over MCP) and **code + DJ** (sets are written as code, in your own vim). It is growing into a keyboard-driven DJ system; see the [DJ mode design](https://github.com/yassinebenarbia/mplayer-server/blob/main/docs/dj-mode.md). It was called *musa* before.

# Quick start

```sh
cargo install --path .                    # codj, this repo
cargo install --path ../mplayer-server    # codjd, the daemon (github.com/yassinebenarbia/mplayer-server)
codjd fetch-models                        # once: the beat-detection model (83 MB)
codj                                      # starts codjd if it isn't running; plays ~/Music
```

No config is needed. To use another folder, put `path = "~/Music/dj"` in `~/.config/codjd/config.toml`.

The first five minutes, all vim keys:

| | |
|---|---|
| `?` | every key on the screen, `:help` the same |
| `j` `k` `5j` `gg` `G` | move |
| `/` then `Enter`, `n` `N` | search (regex), next / previous match |
| `Enter`, `Space` | play, pause |
| `:slsk <artist>` | search Soulseek and download (first time: `:set`, fill in a username) |
| `:dj` | DJ mode: two decks; `Tab` then `j`/`k` and `a`/`b` loads a track onto deck A/B |
| `:tutor` | a guided first mix with your own tracks (opens DJ mode) |
| `:set new friday`, `F9` | in DJ mode: start a set, then record your mixing into it as code (`F10` edits it) |
| `:q` | quit (the music keeps playing in codjd) |

`:` commands complete with `Tab`. A config from before the renames (`~/.config/musa`, `~/.config/mplayer-client`) is still read.

# Showcase
||||
|-------|-------|-------|
|![ShowCase](./assets/mplayer_smoll_with_lyrics_showcase.png "a title")|![Show Case](./assets/mplayer_large_showcase.png "show case 2")|![Show Case](./assets/mplayer_mid_showcase.png "show case 3")|
___

![Show Case](./assets/mplayer_wide_showcase.png "show case 4")

# Features
- vim-style keys: `j`/`k`, `gg`/`G`, `h`/`l` seek, `H`/`L` volume, `ZZ` / `:q` to quit
- regex search over title, artist and file name (`/`, then `n`/`N`); no match? `Enter` searches Soulseek
- play queue (`q` to queue, `Shift+Q` to open it)
- live DJ mode (`:dj`, `Shift + m`): two decks with waveforms, beat phase meter, sync, loops, hot cues, EQ, filter, faders and crossfader, all from the keyboard and vim-first: `h` / `l` focus a deck, `Space` plays, `c` cues, `s` syncs, `4w` `2W` `}` jump beats, bars and phrases, `ma` / `'a` are marks (hot cues), `v` loops, `+` `-` `=` pitch, `.` repeats, `u` / `Ctrl-r` undo, `f` picks any control for `]` `[`, `:fader` `:low kill` `:xfade` set the mixer, `:q` leaves (hold/release with the kitty keyboard protocol); needs a codjd with the deck API, see [DOCS.md](./DOCS.md#dj-mode)
- learn it as you go: `?` draws the key map on the screen with a one-line musical explanation for every key, `:help bass swap` explains the words, `:tutor` walks you through a first mix with your own library (it notices each step as you do it), `:mix new blend` writes the transition as set code; see [Learning](./DOCS.md#learning)
- patch decks (`F12`, `:patch new d1`): four slots of synths and drum machines built from node graphs that play in time with the decks, with a vim-key step sequencer grid (`hjkl`, `x`, `A-H` patterns), knob hints, LIVE / CUE per slot, and MCP tools so an agent can write patches; needs a codjd with the patch methods, see [Patch decks](./DOCS.md#patch-decks)
- "more like this" (`m`): codjd analyzes your library locally and queues similar-sounding tracks
- Soulseek search, filtering, downloads with a concurrency limit and reorderable queue; finished downloads show up in the library
- `:settings` panel, saved straight to your config file
- lyrics, playlists and Lua scripting

# Keybinds and Configurations
[docs](./DOCS.md)

# Status bar widget

`codj status` prints one line (`▶ Aftertekno — Pneumatix 02:18/07:17`, `⏸` when paused, `■` when stopped) and exits; with `--follow` it keeps printing whenever the track, pause state or the second changes, and survives codjd restarts (prints an empty line, or `--offline TEXT`, while codjd is away).

```
codj status [--follow] [--format "{icon} {title} — {artist} {elapsed}/{total}"] [--max-width N] [--offline TEXT]
codj toggle | next | prev | vol+ | vol-     # one-shot controls, exit 1 if codjd isn't running
```

`--max-width` shortens the title with `…`; `{artist}` and its ` — ` vanish when a track has no artist. Volume moves in steps of 5%.

Polybar:

```
[module/codj]
type = custom/script
exec = codj status --follow --max-width 40
tail = true
click-left = codj toggle
click-right = codj next
scroll-up = codj vol+
scroll-down = codj vol-
```

Waybar:

```
"custom/codj": {
    "exec": "codj status --follow --max-width 40",
    "on-click": "codj toggle",
    "on-click-right": "codj next",
    "on-scroll-up": "codj vol+",
    "on-scroll-down": "codj vol-"
}
```

# Agents (MCP)

`codj mcp` is an [MCP](https://modelcontextprotocol.io) server on stdio, so agents can browse the library, queue tracks and drive codjd. It needs a running `codjd` (it never starts one; tools return an error if it is away). Claude Code config (`.mcp.json` or `claude mcp add`):

```
{ "mcpServers": { "codj": { "command": "codj", "args": ["mcp"] } } }
```

Tools: `library`, `now_playing`, `queue`, `track_features`, `similar`, `analysis_status`, `play_index`, `toggle`, `next`, `previous`, `seek`, `set_volume`, `queue_add`, `queue_remove`, `queue_similar`, `deck_state`, `deck_load`, `deck_play`, `deck_pause`, `deck_seek`, `deck_set`, `audio_health`, `audio_devices`.

Set tools (sets are `.codj` text files, see `docs/set-language.md` in the codjd repo; `set_language_help` returns a cheat sheet of the syntax):

|Tool|Does|Class|
|----|----|-----|
|`sets_list`, `set_state`|Set files with error counts; the active set's tracks, mixes (0-based, file order) and runtime state|always|
|`set_source`|`{text, hash}` of a set file as on disk|always|
|`set_check`|Errors (`file:line:col: message`) of a text as if it were saved, nothing is written|always|
|`set_write`|Save after checking; needs `base_hash` (the hash of the version you edited); the history names the author `agent:<MCP client name>`|always|
|`set_use`|Make a set active|always|
|`set_cue`, `set_loop_capture`, `set_grid_capture`|Capture a deck's position / loop / grid into the active set|always|
|`set_record`|Start / stop recording a mix into the active set|always|
|`set_arm`, `set_disarm`, `set_auto`, `set_preview`, `set_load`|Run the set: they start, stop or load audio|`--live` only|

Patch tools (patch decks: synths and drum machines as JSON node graphs, see `docs/patches.md` in the codjd repo):

|Tool|Does|Class|
|----|----|-----|
|`patch_node_types`|The node catalog: inputs with defaults, outputs, params, and the file format rules|always|
|`patch_list`, `patch_state`|Patch files with titles; the four slots (loaded patch, live or cue-only, cpu, muted reason, knobs, sequencers with patterns)|always|
|`patch_source`, `patch_check`|`{json, hash}` of a patch file; errors (`<node>: message`) of JSON as if saved|always|
|`patch_write`, `patch_import`|Save after checking, `base_hash` required (author `agent:<client>`); import a NoiseCraft `.ncft`|always|
|`patch_action`|Edit the loaded patch live: knob, cell, pattern, structure (audible at once on a LIVE slot)|always|
|`patch_load`, `patch_unload`, `patch_live`|Load into a slot (cue-only), unload, send to the master|`--live` only|

Workflow for patches: `patch_node_types` -> write the JSON -> `patch_check` -> `patch_write` -> (live) `patch_load` -> `patch_action` for knobs and cells -> `patch_live`.

Workflow for an agent editing a set: `sets_list` -> `set_source` (keep the `hash`) -> edit the text, preferably one track or mix block, leaving the rest as it is -> `set_check` until `valid` -> `set_write` with `base_hash` = that hash. If it answers "set changed since you read it" (vim, codj or another agent saved meanwhile) call `set_source` again, redo the edit on the new text and write with the new hash. Writing a set never changes what is playing; a running mix keeps its plan and the edit applies to the next arm. Needs a codjd with the set editing methods (`docs/dj-api.md`, "Set editing" and "Set sources").

Live guardrail: by default anything that changes what the audience hears is refused: `play_index`, `toggle`, `next`, `previous`, `seek`, `set_volume`, every `deck_*` call on deck 0 and `deck_set` of `master` or `crossfader`. The set runtime tools (`set_arm`, `set_disarm`, `set_auto`, `set_preview`, `set_load`) are refused the same way. The patch tools `patch_load`, `patch_unload` and `patch_live` are too. Reads, queue edits, deck 1 (the cue deck), set editing (sources, writes, captures, recording) and the other patch tools (files, state, `patch_action`) always work. Use `"args": ["mcp", "--live"]` to allow everything. State-changing calls are logged to stderr with a timestamp.

# Thanks to:
- [Ratatui](https://github.com/ratatui-org/ratatui)
- [Lofty-rs](https://github.com/Serial-ATA/lofty-rs)
- [Zbus](https://github.com/dbus2/zbus)
- [Serde](https://github.com/serde-rs/serde)
> and many others

# License and origin
codj is a fork of [mplayer-client](https://github.com/yassinebenarbia/mplayer-client) by yassinebenarbia, renamed and extended (Soulseek, play queue, search, settings and more). It is licensed under the [GPL-3.0-or-later](./LICENCE), like the original.
