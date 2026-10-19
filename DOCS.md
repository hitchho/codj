# Keybinds

|Region|Mode|Keybind|Desc|
|------|----|-----|------|
|Any|Normal|`m`|Toggle mute|
|Any|Normal|`p`|Toggle `play` for the currently playing song|
|Any|Normal|`Alt + j`|Select the region `bellow`|
|Any|Normal|`Alt + k`|Select the region `above`|
|List|Normal|`j`|Scroll down|
|List|Normal|`k`|Scroll up|
|List|Normal|`Space` or `Enter`|Play the music under selection|
|List|Normal|`s`|Scroll to the currently playing music|
|List|Normal|`q`|Add the music under selection to the play queue (the list title shows the queue length)|
|List|Normal|`m`|Queue 5 tracks that sound like the one under selection (needs musad's audio analysis; the list title shows the highlighted track's estimated BPM and loudness)|
|List|Normal|`Shift + q`|Open / close the queue panel|
|List|Normal|`h` / `l`|Seek 5 seconds back / forward|
|List|Normal|`Shift + h` / `Shift + l`|Volume down / up|
|List|Normal|`ZZ`|Quit (also `:q`, `:q!`, `:wq`, `:x`)|
|Queue|Any|`j` / `k`|Move in the queue|
|Queue|Any|`Shift + j` / `Shift + k`|Move the selected track later / earlier in the queue|
|Queue|Any|`x`, `dd` or `Delete`|Remove from the queue|
|Queue|Any|`Enter`|Play it now|
|Queue|Any|`z`, `q`, `Esc` or `Shift + q`|Close the queue panel|
|List|Normal|`gg`|Scroll to the start of the list|
|List|Normal|`G`|Scroll to the end of the list|
|List|Normal|`Ctrl + d`|Scroll half page down|
|List|Normal|`Ctrl + u`|Sroll half page up|
|List|Normal|`gg`|Go to the top of the list|
|List|Normal|`G`|Go to the bottom of the list|
|List|Normal|`/` or `f`|Enable `Search` mode: case-insensitive regex over title, artist and file name (`artist:`, `genre:`, `title:` to pick one field)|
|List|Search|`Enter`|Play the selected match, or search soulseek for the query when nothing matches|
|List|Search|`Ctrl + Enter`|Search soulseek for the query even when the library has matches (`Ctrl + j` or `Alt + Enter` in terminals that send Ctrl+Enter as plain Enter)|
|List|Search|`Esc`|Close the search, back to the full list with the cursor on the highlighted track|
|List|Search|`Up` / `Down`|Move through the matches while typing|
|List|Normal|`:`|Enable `Command` mode|
|List|Search|`Character`|Register the character to the search querry, check [this](#Search_filters) for search tricks|
|List|Command|`Character`|Register the character to the command querry, check [this](#commands) for available commands|
|List|Command|`Enter`|Execute command|
|List|After Search|`j`|Scroll down|
|List|After Search|`k`|Scroll up|
|List|After Search|`Space` or `Enter`|Play the music under selection|
|List|After Search|`p`|Toggle `play` for the currently playing song|
|Playlist|Normal|`j`|Move down|
|Playlist|Normal|`k`|Move up|
|Playlist|Normal|`Enter`|Select current playlist|
|Lyrics|Normal|`j`|Move down|
|Lyrics|Normal|`k`|Move up|
|Lyrics|Normal|`Enter`|Play music from line if possible|
|Actions|Normal|`l`|Move right|
|Actions|Normal|`h`|Move left|
|Actions|Normal|`Space` or `Enter`|Apply selected action (available only for `next, previous, play and stop` actions)|
|Actions|Normal|`Tab`|Cycle through next option (available only for repeat and list fields)|
|Actions|Normal|`Shift + Tab`|Cycle through previous option (available only for repeat and list fields)|
|Seeker|Any|`k`|Toggle between `continue` playing and `pause`|
|Seeker|Any|`l`|Seek to the next 5 seconds or play the next music if remaning duration less than 5|
|Seeker|Any|`h`|Seek to the previous 5 seconds or play the previous music if remaning duration less than 5|
|Seeker|Any|`Alt + l` or `Alt + h`|Select the `Volume` region|
|Volume|Normal|`l` or `k`|Increase volume|
|Volume|Normal|`h` or `j`|Decrease Volume|
|Volume|Normal|`Alt + l` or `Alt + h`|Select the `Seeker` region|

`Actions`, `Seeker` and `Volume` are the parts of the now-playing bar at the bottom: the controls on the left, the seek bar, and the volume on the right. Repeat and sort are shown in the bar's top-right corner; `h`/`l` in the controls reach them.

# Mouse

|where|mouse|action|
|---|---|---|
|Music list|click|Select the track|
|Music list|click the same track again|Play it|
|Music list|wheel|Scroll|
|Bar controls|click `⏮` `⏵`/`⏸` `⏭` `⏹`|Previous, play/pause, next, stop|
|Seek bar|click or drag|Jump to that point|
|Seek bar|wheel|Seek 5 s|
|Volume|click, drag or wheel|Set the volume|
|`repeat …` / `sort …` in the bar title|click|Switch to the next mode|

The client captures the mouse, so selecting text in the terminal needs `Shift` held (in most terminals).

# Search filters

|filter|description|
|------|-----------|
|`:title <title>`| Serch by title |
|`:artist <artist>`| Search by artist |
|`:duration <mm:ss>`| Search by duration |
|`:genre <genre>`| Search by genre|

# Commands

|filter|description|
|------|-----------|
|`:move <up/down>`|Moves the selection cursor up/down|
|`:play`|Plays the selected playlist|
|`:resume`|Resumes playing music|
|`:add_to_playlist <playlist-name>`|Adds selected song to playlist|
|`:create_playlist <playlist-name>`|Creates a new playlist|
|`:rename_playlist <old-name> <new-name>`|Renames playlist with old name to new one (can't rename `default`) |
|`:remove_playlist <playlist-name>`|Remove playlis by the name (can't remove `default` and requires reload to take effect)|
|`:toggle_mute`|Toggle mute|
|`:q` | `:quit`|Quit client|
|`:slsk_connect`|Connect to soulseek network (requires config)|
|`:slsk_disconnect`|Disconnect from soulseek network|
|`:slsk <query>` | `:soulseek <query>`|Search for music on soulseek network (connects on first use)|
|`:slsk`|Open the soulseek panel|
|`:settings` | `:set`|Open the soulseek settings panel: `j`/`k` move, `Enter` edits, `Space` toggles, `Esc` closes. Changes are saved to your config file right away (comments kept); ranking settings apply to the current results immediately|

# Soulseek

Soulseek integration allows searching and downloading music from the soulseek p2p network. This feature is completely optional and only activates when configured.

## Registration

Soulseek uses optimistic registration - if you connect with a username that doesn't exist, the server automatically creates an account with that username and password. No separate registration step is needed.

## Soulseek Keybinds

|Region|Keybind|Desc|
|------|-------|-----|
|Any|`Ctrl + s`|Toggle soulseek panel|
|Soulseek|`Tab` or `Shift + Tab`|Switch focus between the results table and the downloads list (the focused one has a yellow border and highlighted row)|
|Soulseek|`f` or `/`|Filter the results by file name (case-insensitive regex). `Enter` keeps the filter, `Esc` clears it, arrows move while typing|
|Soulseek|`j`|Move down in the focused list|
|Soulseek|`k`|Move up in the focused list|
|Soulseek|`Enter` or `Space`|Queue selected result for download (results focused)|
|Soulseek|`x`, `dd` or `Delete`|Remove the selected download, cancelling it if still running (downloads focused)|
|Soulseek|`Shift + j` / `Shift + k`|Move the selected download down / up; waiting downloads start top first (`max_concurrent_downloads` run at once)|
|Soulseek|`:`|Open the command input, returns to the soulseek panel afterwards|
|Soulseek|`c`|Clear finished downloads|
|Soulseek|`G`|Go to bottom of the focused list|
|Soulseek|`Ctrl + d`|Scroll down 10 items|
|Soulseek|`Ctrl + u`|Scroll up 10 items|
|Soulseek|`z`, `q` or `Esc`|Close soulseek panel (`Esc` clears an active filter first)|

## Soulseek Results Display

Results are automatically sorted by quality score which considers:
- **Format**: WAV > FLAC > ALAC > MP3 > OGG > AAC > WMA, or your own order with `formats` in `[soulseek]` (which also hides every other format)
- **Bitrate**: Higher bitrate = better score
- **Sample Rate**: Higher than 44.1kHz gets bonus points
- **VBR**: VBR gets slight bonus for lossy formats
- **Availability**: Free slots and fast uploads get bonus

## Download States

Downloads are shown with colored status:
- **Gray**: Pending (queued)
- **Yellow**: Connecting to peer
- **Cyan**: Downloading (with progress bar)
- **Green**: Complete
- **Red**: Failed or Cancelled

# Configuration

Config file is optional, but if you want to have it, you can to either pass it as the first argument to `musa` or put it on `$HOME/.config/musa/config.toml` (the old `$HOME/.config/mplayer-client/config.toml` is still read if that doesn't exist)
```Toml
# EVERY OPTION CAN BE EMITTED
[config]
fps = 30 # FPS, default to 30
# puting `false` on any of these does not disable it, it just hide it when
# the client starts, you can toggle some of them (lyrics) using a keybind
lyrics = true # display lyrics, defaults to false
genre = false # display genre, default to false

[scripts]
# region_name = path/to/script.lua
# only full path work for now
all = "/home/user/.config/musa/lua/all.lua"
list = "/home/user/.config/musa/lua/list.lua"
# lyrics = "/home/user/.config/musa/list.lua"
# actions = "/home/user/.config/musa/actions.lua"
# seeker = "/home/user/.config/musa/seeker.lua"
# volume = "/home/user/.config/musa/volume.lua"

[soulseek]
# soulseek network search for music
enabled = true
username = "your_username"
password = "your_password"
# server_host = "server.slsknet.org"
# server_port = 2242
# listen_port = 2234
# max_results = 50
# result ranking/filtering:
# formats = ["flac", "mp3", "ogg"]  # only show these, earlier = preferred (unset: all audio)
# min_bitrate = 256                 # hide lossy files below this kbps (lossless always kept)
# max_size_mb = 0                   # hide files bigger than this, 0 = no limit
# prefer_free_slots = true          # rank peers with free upload slots higher
# download_dir = "/home/user/Music/soulseek"
# share_dir = "/home/user/Music/shared"  # optional
```

## Soulseek Sharing

Soulseek requires users to share files to participate in the network. By default, musa shares a virtual dummy.mp3 file (100KB, 128kbps, 3 minutes) to satisfy this requirement without exposing any real files.

If you want to share a real directory, set `share_dir` in your config. Otherwise, the dummy file is used automatically.

## Config

|field|description|
|------|-----------|
|`fps`| FPS lock for this client |
|`lyrics`| Display lyrics window be display whenever a client is opened (this does not disable it) |
|`genre`| Display genre row in the music list |

## Scripts

Each script will run **after each key press** whenever it's corresponding **region is sellected**, and the `all` will run when any region is selected. 
Each script should be a path to a lua script with the following format
```Lua
return function(args)
-- do stuff here
end
```
`args` is a lua table with some usefull methods 
|function name|descriptoin|
|-------------|-----------|
|`args:modifier()`|key modifier|
|`args:region()`  |selected region|
|`args:code()`    |pressed key code|

>[!NOTE]
> For now, you cannot override an existing keybind, this will change in the future.

- Modifiers: None, CONTROL, ALT, SHIFT.

- Regions: List, Action, Seeker, Volume, Lyrics.

- Code: check `enum KCode` in `./src/ui.rs` cuz it's kinda too big.

# Extra

Check the example(s) in [`./examples/`](./examples) for inspiration
>[!NOTE]
> More funcitonal API will be provided in the future, like functions/methods that allows you to iterract with client programatically. if you think this deserves better, make a PR and I'll happily review it :3
