<h1 align="center">musa</h1>

**musa** is a terminal music player with built-in [Soulseek](https://www.slsknet.org) search and downloads, written in [Rust](https://www.rust-lang.org). It is the TUI for [**musad**](https://github.com/yassinebenarbia/mplayer-server), the player daemon, and talks to it over [D-Bus](https://www.freedesktop.org/wiki/Software/dbus/#whatisd-bus). Music keeps playing, and downloads keep going, when musa is closed.

*musa* is Finnish for "music".

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
- "more like this" (`m`): musad analyzes your library locally and queues similar-sounding tracks
- Soulseek search, filtering, downloads with a concurrency limit and reorderable queue; finished downloads show up in the library
- `:settings` panel, saved straight to your config file
- lyrics, playlists and Lua scripting

# Usage
- Install and run `musad`.
- Install musa from this repo: `cargo install --path .` (the binary is called `musa`).
- Copy the [config file](./config.example.toml) to `$HOME/.config/musa/config.toml` and adjust it (optional).
- Run `musa`.
>[!NOTE]
> You can also pass the config file as the first argument. A config at the old `$HOME/.config/mplayer-client/config.toml` is still read if the new one doesn't exist.

# Keybinds and Configurations
[docs](./DOCS.md)

# Status bar widget

`musa status` prints one line (`▶ Aftertekno — Pneumatix 02:18/07:17`, `⏸` when paused, `■` when stopped) and exits; with `--follow` it keeps printing whenever the track, pause state or the second changes, and survives musad restarts (prints an empty line, or `--offline TEXT`, while musad is away).

```
musa status [--follow] [--format "{icon} {title} — {artist} {elapsed}/{total}"] [--max-width N] [--offline TEXT]
musa toggle | next | prev | vol+ | vol-     # one-shot controls, exit 1 if musad isn't running
```

`--max-width` shortens the title with `…`; `{artist}` and its ` — ` vanish when a track has no artist. Volume moves in steps of 5%.

Polybar:

```
[module/musa]
type = custom/script
exec = musa status --follow --max-width 40
tail = true
click-left = musa toggle
click-right = musa next
scroll-up = musa vol+
scroll-down = musa vol-
```

Waybar:

```
"custom/musa": {
    "exec": "musa status --follow --max-width 40",
    "on-click": "musa toggle",
    "on-click-right": "musa next",
    "on-scroll-up": "musa vol+",
    "on-scroll-down": "musa vol-"
}
```

# Agents (MCP)

`musa mcp` is an [MCP](https://modelcontextprotocol.io) server on stdio, so agents can browse the library, queue tracks and drive musad. It needs a running `musad` (it never starts one; tools return an error if it is away). Claude Code config (`.mcp.json` or `claude mcp add`):

```
{ "mcpServers": { "musa": { "command": "musa", "args": ["mcp"] } } }
```

Tools: `library`, `now_playing`, `queue`, `track_features`, `similar`, `analysis_status`, `play_index`, `toggle`, `next`, `previous`, `seek`, `set_volume`, `queue_add`, `queue_remove`, `queue_similar`, `deck_state`, `deck_load`, `deck_play`, `deck_pause`, `deck_seek`, `deck_set`, `audio_health`, `audio_devices`.

Live guardrail: by default anything that changes what the audience hears is refused: `play_index`, `toggle`, `next`, `previous`, `seek`, `set_volume`, every `deck_*` call on deck 0 and `deck_set` of `master` or `crossfader`. Reads, queue edits and deck 1 (the cue deck) always work. Use `"args": ["mcp", "--live"]` to allow everything. State-changing calls are logged to stderr with a timestamp.

# Thanks to:
- [Ratatui](https://github.com/ratatui-org/ratatui)
- [Lofty-rs](https://github.com/Serial-ATA/lofty-rs)
- [Zbus](https://github.com/dbus2/zbus)
- [Serde](https://github.com/serde-rs/serde)
> and many others

# License and origin
musa is a fork of [mplayer-client](https://github.com/yassinebenarbia/mplayer-client) by yassinebenarbia, renamed and extended (Soulseek, play queue, search, settings and more). It is licensed under the [GPL-3.0-or-later](./LICENCE), like the original.
