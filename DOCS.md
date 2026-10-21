# Keybinds

|Region|Mode|Keybind|Desc|
|------|----|-----|------|
|Any|Normal|`m`|Toggle mute|
|Any|Normal|`p`|Toggle `play` for the currently playing song|
|Any|Normal|`Alt + j`|Select the region `bellow`|
|Any|Normal|`Alt + k`|Select the region `above`|
|List|Normal|`j`|Scroll down|
|List|Normal|`k`|Scroll up|
|Any|Any|`Space`|Pause / continue (except while typing in search, the command line or a filter)|
|List|Normal|`Enter`|Play the music under selection|
|List|Normal|`s`|Scroll to the currently playing music|
|List|Normal|`q`|Add the music under selection to the play queue (the list title shows the queue length)|
|List|Normal|`m`|Queue 5 tracks that sound like the one under selection (needs codjd's audio analysis; the list title shows the highlighted track's estimated BPM and loudness)|
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
|List|After Search|`Enter`|Play the music under selection|
|List|After Search|`p`|Toggle `play` for the currently playing song|
|Playlist|Normal|`j`|Move down|
|Playlist|Normal|`k`|Move up|
|Playlist|Normal|`Enter`|Select current playlist|
|Lyrics|Normal|`j`|Move down|
|Lyrics|Normal|`k`|Move up|
|Lyrics|Normal|`Enter`|Play music from line if possible|
|Actions|Normal|`l`|Move right|
|Actions|Normal|`h`|Move left|
|Actions|Normal|`Enter`|Apply selected action (available only for `next, previous, play and stop` actions)|
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
|`:dupes` | `:duplicates`|Show every copy of a song, or (default) only the best one: lossless over lossy, then the bigger file. Copies are the same title and artist with about the same length; untagged files are never merged. The list title says how many are hidden|
|`:dj`|Open the live [DJ mode](#dj-mode) (also `Shift + m` in the list)|
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
|Soulseek|`Enter`|Queue selected result for download (results focused)|
|Soulseek|`a`|Download the whole folder (album) of the selected result: every file that user has in the same folder among the results, in track order, filtered by your `formats`/bitrate/size settings, saved into a subfolder named after the remote folder (results focused)|
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
- **Gray**: Pending (waiting for a free download slot)
- **Blue**: Queued at the uploader (waiting for them to start sending)
- **Yellow**: Connecting to peer
- **Cyan**: Downloading (with progress bar)
- **Green**: Complete
- **Red**: Failed or Cancelled

Files are saved as `<name>.part` until they are complete.

## Retrying From Another User

When a download is queued, up to 3 other users who had the same file in the search results (same file name, size within 1%) are remembered, best ranked first. If the download fails (peer unreachable, denied, upload failed, timeout) or stays queued at the uploader for 5 minutes without the transfer starting, the next of those users is tried automatically, in the same list position. The `source` column shows `retry 2/3 from <user>` while that happens, and the reason when a download failed for good.

# DJ mode

Two decks, a mixer and a library on one screen, played from the keyboard. Open it with `:dj` or `Shift + m` in the list (`Shift + d` already downloads lyrics). It needs a codjd with the deck API (`docs/dj-api.md`); against an older codjd the screen opens and says so in the status line.

```
 DJ   Esc back · Tab search · < > load A/B · ...                         ● live
┌ DECK A  ▶ playing ────────────────┐┌ DECK B  ⏸ paused ─────────────────┐
│ title — artist                    ││                                   │
│ 129.55 BPM (trk 127.98) key 8A +1.23% SYNC                            │
│ 01:23.4 -5:53  LOOP 4  HP ●       ││ time, loop, headphone-cue         │
│ 1 ███ 2 ██░ 3 ░░░ 4 ░░░          ││ phase meter: the 4 beats, current lit
│  1 0:12  2 --  3 1:35  4 --  more ○○○○   hot cues                     │
│ ▾▆▆█▆▆█▆▆ (overview + playhead)   ││                                   │
│ ╎█ ╎██┃██╎██│██ (zoomed strip)    ││ 8 s around the playhead, beat grid│
├ Mixer  quantize ON · internal · xruns 0 · phase Δ +0.03 ────────────────┤
│ A  LOW ──│██ MID … HI … FLT … FAD ████░                                  │
│ B  …                                                                    │
│ XF A ━━━━━━◆━━━━━━ B                                                    │
┌ Library ────────────────────────────────────────────────────────────────┐
```

The waveform is fetched once per loaded track (`GetWaveform`) and drawn with block characters coloured by band (low red, mid green, high blue); bright up to the rms level, dim up to the peak. The deck state is polled at 30 Hz only while DJ mode is open; a failing poll shows in the status line and header (`○ no data`), never a crash. Leaving DJ mode (or quitting) never stops the music.

## Keys

Letters are the same in every terminal; `Alt` combinations are the mixer. Digits for hot cues and `Shift + digit` use the US layout.

|Control|Deck A (left hand)|Deck B (right hand)|
|-------|------------------|-------------------|
|Cue (hold = preview)|`z`|`/`|
|Play / pause|`x`|`.`|
|Sync to the other deck|`c`|`,`|
|Loop on / off (4 beats unless you [changed the loop length](#control-hints-and-editing))|`v`|`m`|
|Pitch − / +|`a` / `s`|`k` / `l`|
|Nudge − / + (hold)|`q` / `w`|`o` / `p`|
|Hot cue 1–4 (empty: set, set: jump)|`1` `2` `3` `4`|`7` `8` `9` `0`|
|Clear hot cue|`Shift + 1`…`4`|`Shift + 7`…`0`|
|Beat jump back / forward (4 beats unless you changed the jump size)|`e` / `r`|`u` / `i`|
|EQ low + / −|`Alt + q` / `Alt + a`|`Alt + u` / `Alt + j`|
|EQ mid + / −|`Alt + w` / `Alt + s`|`Alt + i` / `Alt + k`|
|EQ high + / −|`Alt + e` / `Alt + d`|`Alt + o` / `Alt + l`|
|Filter + / − (+ high-pass, − low-pass)|`Alt + r` / `Alt + f`|`Alt + p` / `Alt + ;`|
|Channel fader up / down|`g` / `b`|`h` / `n`|

|Key|Does|
|---|----|
|`t` / `y`|Crossfader toward A / toward B|
|`f`|[Control hints](#control-hints-and-editing): label every control, type a label to select it|
|`]` / `[` / `\`|Step the selected control up / down / reset it|
|`;`|Toggle between the last two selected controls|
|Mouse wheel|Edits the selected control (up = +); with nothing selected it scrolls the library. Clicks do nothing in DJ mode|
|`F1` / `F2`|Headphone cue on deck A / B|
|`F5`|Quantize on / off (mixer-wide)|
|`F6` / `F7` / `F8`|[Set](#sets-in-dj-mode): arm the next mix / auto mode on-off / disarm|
|`F9`|[Record](#capture-and-recording) a mix into the active set on / off (same as `:rec`)|
|`F10` / `F11`|[Editor pane / AI prompt box](#editor-pane-and-ai-prompt-box)|
|`Up` / `Down`, `PageUp` / `PageDown`|Move in the library|
|`<` or `Shift + a`|Load the highlighted track onto deck A (`Shift + ,`)|
|`>` or `Shift + b`|Load it onto deck B (`Shift + .`)|
|`Enter`|Load it onto the deck that is not playing (A when both are idle)|
|`Tab`|Type in the library search (same regex and `artist:`/`title:`/`genre:` filters as the list); `Tab` or `Enter` hands the keys back to the decks, `Esc` clears the search|
|`Shift + ;` (`:`)|DJ command line: `:q` back to the normal view, `:load a`, `:load b`, `:grid a x2` / `/2` / `x1.5` / `/1.5` / `reset` to correct a deck's detected tempo (saved per track). A yellow `?` after the BPM means the tempo detectors disagree, so the grid is likely half, double or 1.5× off. Sync refuses changes over ±16 % and says which tempos clash.|
|`Esc`|Closes the set error overlay if open; cancels the hints if they are up; clears the selected control; else back to the normal view|

Tap vs hold: tapping EQ, filter, faders and crossfader steps 0.05, pitch steps 0.1 %. Holding ramps after 0.2 s with an ease-in to a full sweep in about 1.5 s (pitch ramps slowly, 1 to 4 %/s). Values are sent at most about 30 times a second per control and coalesced, so a slow bus never builds a backlog; the screen always shows what codjd reports.

## Control hints and editing

Every adjustable control on the screen can be picked with a short label and then edited with one hand on a key, the wheel or a knob, the way Vimium labels links.

1. `f` draws a label (black on yellow) over or next to every control. While the labels are up the keys only match labels: nothing plays, `Esc` or `f` cancels.
2. Type the label. The controls and their labels use the left hand only (`a s d q w e r t z x c v g b`); the seven most used ones have a single letter:

|Label|Control|Label|Control|
|-----|-------|-----|-------|
|`a` / `s`|filter A / B|`e` / `r`|channel fader A / B|
|`d` / `q`|EQ low A / B|`w`|crossfader|
|`ta` / `ts`|EQ mid A / B|`ga` / `gs`|EQ high A / B|
|`xa` / `xs`|pitch A / B|`za` / `zs`|gain (trim, dB) A / B|
|`ca` / `cs`|loop length A / B (for the next `v` / `m`)|`va` / `vs`|beat jump size A / B (for `e r u i`)|
|`ba`|master|`bs`|cue mix|
|`bd`|sync range (the largest tempo change a synced play may need)||

   The table is fixed, so a label never moves with the window size. A control the screen has no room for at the moment (small terminals drop master / cue mix / sync range first, then jump size and gain) gets no label and cannot be picked.
3. The control is now selected: it is drawn inverted, the right end of the status line shows a chip like `◆ A filter -0.30`, and `f` brings the labels back for another pick. `;` swaps with the control selected before it (two controls you play together: filter and crossfader). `Esc` clears the selection.

Editing the selected control:

|Control|`]` / `[` step|`\` or knob press|
|-------|---------------|-------------------|
|EQ, filter, crossfader, channel fader, cue mix, master|0.05|EQ / filter 0, crossfader 0, fader 1, cue mix 0, master 1|
|Pitch|0.1 %|0 %|
|Gain|0.5 dB|the track's auto-gain (what the deck loaded with)|
|Loop length, jump size|×2 / ÷2 along 1/4 1/2 1 2 4 8 16 32 beats|4 beats|
|Sync range|0.01 (1 %)|0.16 (codjd's default)|

Everything goes through the normal `DjSet` path (coalesced, at most ~30 a second per control), so codjd sees a manual move: it cancels the automation of that control and records it into a mix recording. Loop length and jump size are only used by the next `v` / `m` and `e r u i`; they live in codj, not in codjd. The decks show them as `loop 4` (idle loops), `jmp 4` and `gain +0.0` in the time line when it has room; the mixer line shows `MST`, `CUE` and `RNG` on wide terminals (72 columns and up).

Codjd does not report a manual gain trim or the sync range, so codj shows the last value it sent itself (a track loaded afterwards starts from its own auto-gain again). The sync range is assumed to be codjd's default 0.16 whenever DJ mode opens, since codjd cannot be asked for it.

**Acceleration** (keys, wheel and knob alike): detents less than 60 ms apart multiply the step, x2 for the second, x4 from the third on, until you pause or reverse. A slow turn is always one step per detent; a held `]` accelerates through the keyboard's auto-repeat. For loop length and jump size a x4 detent moves up to four notches.

**Mouse wheel**: enabled in DJ mode for the wheel only. With a selected control the wheel edits it (up = +); with none it scrolls the library list. It is ignored while the set editor or the AI prompt box is open.

## Hardware knob

A rotary encoder that sends volume keys (the knob on many keyboards' media cluster) can drive the same editing. It is off until the config names the device:

```toml
[knob]
device = "AK820 Consumer Control"   # part of the input device's name (see `evtest`); empty = off
up = 115                            # Linux input key codes, these are the defaults:
down = 114                          #   KEY_VOLUMEUP, KEY_VOLUMEDOWN,
press = 113                         #   KEY_MUTE
grab = true                         # EVIOCGRAB the device only while DJ mode is open
```

- Turning edits the selected control with the same acceleration (up = +); press resets it. With nothing selected, turning scrolls the library and a press loads the highlighted track onto the deck that is not playing (like `Enter`).
- Only the three configured codes are acted on; everything else the device sends is dropped unseen (never logged or stored), because the device may also be your keyboard.
- With `grab = true` the device is taken exclusively while DJ mode is open, so the system volume does not move; leaving DJ mode releases it again (and so does the kernel if codj dies or panics, because the grab belongs to the open file). A device that also has letter keys is never grabbed, it would swallow your typing; use the consumer-control node (`device` matches the first node whose name contains it and that has one of the configured keys).
- It needs read access to `/dev/input/event*` (the `input` group). A missing device, no permission or an unplugged knob shows one note in the status line; DJ mode works without it.
- The knob is read by the `evdev` crate on its own thread and handed to the screen over a channel; `cargo test` includes a test with a virtual device that is skipped when `/dev/uinput` is not writable.

## Sets in DJ mode

A set is a text file (`docs/set-language.md` in the codjd repo): tracks, cues and the transitions between them in bars and beats. codjd runs the mixes; DJ mode shows the set and arms mixes. All of it needs a codjd with the "Set runtime (D-Bus)" methods (`docs/dj-api.md`); against an older codjd the strip never appears and a set command says `codjd has no set runtime: update codjd`.

Mixes and tracks are numbered from 1 on screen (the bus counts from 0).

|Command|Does|
|-------|----|
|`:set`|List the sets with their error counts in the status line|
|`:set <name>`|Make a set active (name, file name or stem); `:set none` deactivates|
|`:arm <n>` / `:arm next`|Arm mix `n`: its outgoing track must be on a deck, the incoming one is loaded on the other deck, synced; the mix runs by itself when the outgoing deck reaches the mix start. `next` picks the mix after a running one, else the first whose outgoing track is on a deck (a playing one first)|
|`:disarm`|Cancel the armed or running mix's remaining automation; controls stay where they are|
|`:auto on` / `:auto off`|Auto mode: after a mix ends, the next one whose outgoing track is on air is armed (`:auto` alone toggles)|
|`:preview <n> [bars]`|Audition mix `n`: both tracks loaded, the outgoing deck `bars` (default 8) before the mix start, armed and playing|
|`:load <n>`|Load track `n` of the set onto the free deck with the set's grid/gain overrides (`:load a` / `:load b` still load the library selection)|
|`:e`|The in-TUI editor pane on the active set (also `F10`), see below|
|`:ai <text>`|Run the AI agent on a request (also `F11`), see below|
|`:errors`|The active set's errors as `file:line:col: message` in an overlay; `Esc` closes it|
|`:edit`|Open the set file in `$EDITOR` (else `$VISUAL`, else `vi`). The terminal is handed over (raw mode, alternate screen, mouse capture and keyboard-protocol flags are given back), the audio keeps playing, and everything is restored when the editor exits; codjd reloads the file when you save|

|Key|Does|
|---|----|
|`F6`|Arm the next mix (same as `:arm next`); a mix that is armed but not yet started is not re-armed|
|`F7`|Auto mode on / off|
|`F8`|Disarm|
|`F10`|Editor pane (same as `:e`)|
|`F11`|AI prompt box (same as `:ai`); brings back a hidden run|

`Enter` is not used for this: it loads the library selection onto the free deck.

While a set is active a strip sits between the decks and the mixer: the set name, `✓ no errors` or `✗ N errors` in red, `AUTO` when auto mode is on, the track list with the track on air (`▶`, green) and the incoming one (`»`, yellow) and the mixes as segments between them (`───16b▶` is a 16-bar mix, bold when armed). For the armed mix there is a progress bar over its length in bars with the current bar and a `◆` at each upcoming automation item, and below it the next items (`bar 16 · swap low`). Without an armed mix the strip lists the mixes and which one F6 would arm. It is 6 rows on a tall screen, 4 on a medium one and one line on a small one. `SetState` is polled at about 7 Hz while a set is active, once a second otherwise.

## Capture and recording

Playing by hand turns into set code: codjd writes the active set's file itself (line-level edits, comments and formatting stay, git commit or `.bak` per edit, see "Editing from codj" in `docs/set-language.md`). All of it needs an active set (`:set <name>`) and a codjd with the "Set editing" methods (`docs/dj-api.md`); the commands answer in the status line with codjd's reply.

|Command|Does|
|-------|----|
|`:cue <name> [a\|b]`|Capture the deck's position as cue `name` (`cue drop bar 33` in that track's block; a track not yet in the set is added). Status line: `cue drop → aft bar 33`|
|`:loopsave <name> [a\|b]`|Capture the deck's active loop as loop `name`|
|`:gridsave [a\|b]`|Write the deck's current grid (after `:grid` corrections) into the set as a `grid` line|
|`:rec` / `:rec on` / `:rec off`, `F9`|Start / stop recording a mix. While recording every control change on either deck (keys, MCP, D-Bus) is logged; stopping writes the `mix` block and says e.g. `recorded mix 2: aft → smk, 16 bars, 9 items`|

Names are set-language identifiers (letters, digits, `_`, `-`, starting with a letter or `_`). Without `a` or `b` the deck is chosen by this rule: the deck that is playing; when both play, the louder one (channel fader times its share of the crossfader), on a tie the one on the crossfader's side (A when centered); when neither plays, the only loaded deck; otherwise the command asks for `a` or `b`.

While recording the set strip shows a red `● REC 12.5 bars · 34 events` (bars of the outgoing deck's clock, events logged so far); `SetRecording` is polled at about 4 Hz while recording and once a second otherwise, so a recording started over MCP shows too.

## Editor pane and AI prompt box

Both need a codjd with `SetSource` / `SetCheck` / `SetWrite` ("Set sources" in `docs/dj-api.md`); against an older codjd they say `codjd has no SetSource: update codjd`. Both take the rows of the library and the set strip; the decks and the mixer stay visible. **While one is open the DJ keys are off**: the header turns magenta (`EDIT` / `AI`, "DJ keys are OFF while typing here") and every key goes to the pane. Key-release reporting is switched off meanwhile, like in the search box, so typed text arrives as text.

### Editor (`:e` or `F10`)

The active set's source in a vim-style editor ([edtui](https://crates.io/crates/edtui)): normal / insert / visual, motions, `dd`, `yy` / `p`, `u` / `Ctrl-r`, `/` search with `n` / `N`.

|Key|Does|
|---|----|
|`Esc` in normal mode, `:q`|Close the pane (the deck keys are live again). With unsaved text `:q` refuses, `Esc` warns once and a second `Esc` (or `:q!`) discards|
|`:w` or `Ctrl-s`|`SetWrite` with author `human` and the hash the text was read at. Refused texts (errors) show the reason in the last line|
|`:wq` / `:x`|Write, then close once the write worked|
|`:e!`|Read the source again, dropping your edits|

Errors: the text is sent to `SetCheck` 400 ms after the last edit. Every `file:line:col: message` becomes a red `✗` in the gutter of its line, the title shows `✗ N`, and the message of the cursor's line is shown under the text. A message without a position is shown there too.

If somebody else saved first (vim, an agent, a capture) `:w` is refused with "set changed since you read it" and the pane asks: `r` reload theirs (your edits are discarded), `d` view the diff (theirs to yours, `j` / `k` scroll), `o` overwrite (reads the current hash, then writes yours), `Esc` keep editing. `:edit` still hands the file to `$EDITOR`.

### AI prompt box (`:ai <text>` or `F11`)

`:ai make the second mix longer` (or `F11`, type, `Enter`) runs the agent from `[agent]` in the config as a child process. codj holds no AI keys: the command brings its own login.

What it gets: a preamble (the active set's name and file, its tracks and mixes, both decks' track / BPM / key / position, and the rules: edit sets only through the codj MCP set tools, `set_check` before `set_write`, never touch audio unless live) followed by your text, on stdin or as the last argument (`prompt_via`). codj's MCP server is attached:

- `claude` (the command's program name): `claude -p --mcp-config <tmp>.json --strict-mcp-config --tools "" --allowedTools mcp__codj__sets_list,mcp__codj__set_source,... ` and the prompt on stdin. The temp JSON is `{"mcpServers":{"codj":{"command":"<this codj binary>","args":["mcp"]}}}` (`"args":["mcp","--live"]` when `live = true`) and is deleted when the run ends. `--tools ""` turns off Claude's own file and shell tools, so the set tools are all it can use.
- any other command: the same JSON file is written and named in the environment: `CODJ_MCP_CONFIG=<path>`, `CODJ_MCP_LIVE=0|1`, `CODJ_MCP_TOOLS=mcp__codj__set_check,...`. A wrapper script passes them on to its agent.

The output (stdout and stderr) streams into the pane; the UI and the audio-facing polling never wait for it. `Ctrl-C` kills the child. `Esc` hides the pane (the agent keeps running; the status line says so and `F11` brings it back) so the deck keys work during a run. When the child exits, codj reads every set again and compares hashes: for each set the agent changed (or created) it shows a unified diff. `y` keeps; `n` undoes by writing the old text back with `SetWrite` (author `human`, on the hash the agent left, so a save made since refuses the undo; the pane notes "undo agent change"). A set the agent created is left in place.

`[agent]` in the config:

|field|default|description|
|-----|-------|-----------|
|`command`|`claude -p`|The agent command line; spaces split, `"double quotes"` group|
|`live`|`false`|`true` passes `--live` to `codj mcp`, so the agent may arm / preview / load (it changes what the audience hears)|
|`prompt_via`|`"stdin"`|`"stdin"` or `"arg"` (last argument; for `claude` after `--`)|
|`mcp_tools`|all set tools (+ live ones when `live`)|Allow-list of MCP tool names, without the `mcp__codj__` prefix|

## Hold and release

Hold-to-preview cue, hold-to-nudge and the ramps need to know when a key is released. In terminals with the kitty keyboard protocol (kitty, foot, WezTerm, Ghostty, Alacritty) DJ mode asks for key press/repeat/release events while it is open (`DISAMBIGUATE_ESCAPE_CODES | REPORT_EVENT_TYPES | REPORT_ALL_KEYS_AS_ESCAPE_CODES`; letters only report releases with the last flag) and gives the extra flags back when you leave DJ mode, on exit and in the panic hook. While you type in the search box or command line the extra flags are off, so typed text arrives as text.

Without release events (other terminals, tmux) DJ mode says so in the status line and falls back:

- cue: press starts the preview, press again ends it (on a playing deck a press just stops at the cue point)
- nudge: a tap bends the tempo for 0.3 s; holding the key keeps it alive through the keyboard's auto-repeat
- EQ, filter, faders, crossfader, pitch: one step per key event, so holding sweeps at your key repeat rate

Auto-repeat of a tap key (play, sync, loop, hot cue, beat jump) is ignored.

## Testing without codjd's engine

`examples/fake_codjd.rs` serves the deck API (and a fake set runtime: sets `demo` and `broken`, 3 tracks, 2 mixes, an armed mix that advances) with plausible changing state and logs every call, `examples/dj_e2e.sh` runs codj against it in tmux (sets, capture and recording included, and phase 5: control hints, label selection, `] [ \ ;` edits, the mouse wheel as SGR mouse reports, the jump keys; it also serves a set's text and hash, with conflict detection on stale writes, for the editing methods), `examples/mcp_e2e.sh` drives `codj mcp` over JSON-RPC against it (set tools, stale/fresh `base_hash`, the `--live` guardrail), all on an isolated bus (`dbus-run-session`); neither touches the real session bus.

# Configuration

Config file is optional, but if you want to have it, you can to either pass it as the first argument to `codj` or put it on `$HOME/.config/codj/config.toml` (the old `$HOME/.config/mplayer-client/config.toml` is still read if that doesn't exist)
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
all = "/home/user/.config/codj/lua/all.lua"
list = "/home/user/.config/codj/lua/list.lua"
# lyrics = "/home/user/.config/codj/list.lua"
# actions = "/home/user/.config/codj/actions.lua"
# seeker = "/home/user/.config/codj/seeker.lua"
# volume = "/home/user/.config/codj/volume.lua"

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

Soulseek requires users to share files to participate in the network. By default, codj shares a virtual dummy.mp3 file (100KB, 128kbps, 3 minutes) to satisfy this requirement without exposing any real files.

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
