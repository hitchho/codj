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

# Thanks to:
- [Ratatui](https://github.com/ratatui-org/ratatui)
- [Lofty-rs](https://github.com/Serial-ATA/lofty-rs)
- [Zbus](https://github.com/dbus2/zbus)
- [Serde](https://github.com/serde-rs/serde)
> and many others

# License and origin
musa is a fork of [mplayer-client](https://github.com/yassinebenarbia/mplayer-client) by yassinebenarbia, renamed and extended (Soulseek, play queue, search, settings and more). It is licensed under the [GPL-3.0-or-later](./LICENCE), like the original.
