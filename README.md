<h1 align="center">codj</h1>

**codj** is a terminal music player with built-in [Soulseek](https://www.slsknet.org) search and downloads, written in [Rust](https://www.rust-lang.org). It is the TUI for [**codjd**](https://github.com/yassinebenarbia/mplayer-server), the player daemon, and talks to it over [D-Bus](https://www.freedesktop.org/wiki/Software/dbus/#whatisd-bus). Music keeps playing, and downloads keep going, when codj is closed.

*codj* reads two ways: **co-DJ** (agents can plan and mix with you over MCP) and **code + DJ** (sets are written as code, in your own vim). It is growing into a keyboard-driven DJ system; see the [DJ mode design](https://github.com/yassinebenarbia/mplayer-server/blob/main/docs/dj-mode.md). It was called *musa* before.

Configs in `~/.config/musa` and `~/.config/mplayer-client` from before the renames are still read.

# Showcase
||||
|-------|-------|-------|
|![ShowCase](./assets/mplayer_smoll_with_lyrics_showcase.png "a title")|![Show Case](./assets/mplayer_large_showcase.png "show case 2")|![Show Case](./assets/mplayer_mid_showcase.png "show case 3")|
___

![Show Case](./assets/mplayer_wide_showcase.png "show case 4")

# Features
- vim-style keys: `j`/`k`, `gg`/`G`, `h`/`l` seek, `H`/`L` volume, `ZZ` / `:q` to quit
- regex search over title, artist and file name (`f` or `/`); no match? `Enter` searches Soulseek
- play queue (`q` to queue, `Shift+Q` to open it)
- live DJ mode (`:dj`, `Shift + m`): two decks with waveforms, beat phase meter, sync, loops, hot cues, EQ, filter, faders and crossfader, all from the keyboard (hold/release with the kitty keyboard protocol); needs a codjd with the deck API, see [DOCS.md](./DOCS.md#dj-mode)
- "more like this" (`m`): codjd analyzes your library locally and queues similar-sounding tracks
- Soulseek search, filtering, downloads with a concurrency limit and reorderable queue; finished downloads show up in the library
- `:settings` panel, saved straight to your config file
- lyrics, playlists and Lua scripting

# Usage
- Install and run `codjd`.
- Install codj from this repo: `cargo install --path .` (the binary is called `codj`).
- Copy the [config file](./config.example.toml) to `$HOME/.config/codj/config.toml` and adjust it (optional).
- Run `codj`.
>[!NOTE]
> You can also pass the config file as the first argument. A config at the old `$HOME/.config/mplayer-client/config.toml` is still read if the new one doesn't exist.

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

Workflow for an agent editing a set: `sets_list` -> `set_source` (keep the `hash`) -> edit the text, preferably one track or mix block, leaving the rest as it is -> `set_check` until `valid` -> `set_write` with `base_hash` = that hash. If it answers "set changed since you read it" (vim, codj or another agent saved meanwhile) call `set_source` again, redo the edit on the new text and write with the new hash. Writing a set never changes what is playing; a running mix keeps its plan and the edit applies to the next arm. Needs a codjd with the set editing methods (`docs/dj-api.md`, "Set editing" and "Set sources").

Live guardrail: by default anything that changes what the audience hears is refused: `play_index`, `toggle`, `next`, `previous`, `seek`, `set_volume`, every `deck_*` call on deck 0 and `deck_set` of `master` or `crossfader`. The set runtime tools (`set_arm`, `set_disarm`, `set_auto`, `set_preview`, `set_load`) are refused the same way. Reads, queue edits, deck 1 (the cue deck) and set editing (sources, writes, captures, recording) always work. Use `"args": ["mcp", "--live"]` to allow everything. State-changing calls are logged to stderr with a timestamp.

# Thanks to:
- [Ratatui](https://github.com/ratatui-org/ratatui)
- [Lofty-rs](https://github.com/Serial-ATA/lofty-rs)
- [Zbus](https://github.com/dbus2/zbus)
- [Serde](https://github.com/serde-rs/serde)
> and many others

# License and origin
codj is a fork of [mplayer-client](https://github.com/yassinebenarbia/mplayer-client) by yassinebenarbia, renamed and extended (Soulseek, play queue, search, settings and more). It is licensed under the [GPL-3.0-or-later](./LICENCE), like the original.
