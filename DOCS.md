# Keybinds

|Region|Mode|Keybind|Desc|
|------|----|-----|------|
|Any|Normal|`?`, `:help`|Every key of the normal view on the screen|
|Any|Normal|`Space`|Pause / continue|
|Any|Normal|`Ctrl + n` / `Ctrl + p`|Next / previous track|
|Any|Normal|`Ctrl + w` then `j` / `k`|Select the panel below / above (also `Alt + j` / `Alt + k`)|
|Any|Normal|`:mute`, `:lyrics`, `:playlists`|Mute, the lyrics panel, the playlists panel|
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
|List|Search|`Enter`|Put the cursor on the match in the full list, like vim's `/` (a second `Enter` plays it); with no match it searches Soulseek|
|List|Normal|`n` / `N`|Next / previous match of the last search|
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
|Playlist|Normal|`j` `k` `gg` `G`|Move (`:pl` opens the panel and puts the cursor here)|
|Playlist|Normal|`Enter` or `l`|Open the playlist and go back to its tracks|
|Playlist|Normal|`Tab` or `z`|Back to the tracks, panel stays open|
|Playlist|Normal|`Esc` or `q`|Close the panel|
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
|`:pl <name>`|Opens the playlist|
|`:pl new <name>` `:pl add <name>` `:pl mv <old> <new>` `:pl rm <name>`|Short forms of the four commands below|
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
|`:dj`|Open the live [DJ mode](#dj-mode) (`?` explains every key there, `:tutor` teaches a first mix) (also `Shift + m` in the list)|
|`:automix` / `:automix on\|off`|[Automix](#automix), the beat-matched transitions between tracks, off / on (toggles without an argument)|
|`:autoplay` / `:autoplay on\|off`|Never run out of music: while it plays and fewer than two tracks are queued, codjd queues the library track that blends best after the last one (tempo within 4 %, a compatible key, a steady beat grid, not played lately; then within 8 % in any key). The picks are ordinary queue entries (`Shift+Q` shows them; remove or move them freely). Off by default (`[player] autoplay = true` in codjd's config)|
|`:settings` | `:set`|Open the soulseek settings panel: `j`/`k` move, `Enter` edits, `Space` toggles, `Esc` closes. Changes are saved to your config file right away (comments kept); ranking settings apply to the current results immediately|

# Soulseek

Soulseek integration allows searching and downloading music from the soulseek p2p network. This feature is completely optional and only activates when configured.


### Library (vim keys)

`Tab` focuses the library; the deck keys are off while it has focus (the box turns yellow and says `LIBRARY`). `/` in the decks goes straight into its search.

|Key|Does|
|---|----|
|`j` / `k`, `Ctrl-d` / `Ctrl-u`, `gg` / `G`|Move|
|`/`|Search (regex, `artist:` `title:` `genre:`); `Enter` keeps the filter and goes back to the list, `Esc` clears it|
|`a` / `b`|Load the highlighted track onto deck A / B and focus that deck|
|`Enter`|Load it onto the focused deck (`h` / `l`); onto the other one if the focused deck is playing|
|`:`|Command line|
|`Tab` / `Esc`|Back to the deck keys (`Esc` does not leave DJ mode; `:q` does)|

## Automix

Press play and leave it: near the end of each track codjd loads the next one (what `Ctrl-n` would play: the queue first, then the list) on its second deck and mixes into it on the beat, like a DJ. You never type or set anything; the play bar's bottom-left corner says what is coming and when, `next: CRIA · blend in 1:12`, then `mixing into CRIA · blend` while it runs, and the playing track, the list cursor and MPRIS move to the new track the moment it starts.

| the two tracks | transition |
|---|---|
| tempos within 6 % | `blend`: 32 bars, beatmatched, the bass swaps halfway |
| within 12 % | `filter-blend`: 16 bars, beatmatched, the old track muffled while the new one opens up |
| further apart | an 8 bar crossfade on the beat, not beatmatched |
| a track without a beat grid (`codjd fetch-models` gives grids) | an 8 second crossfade |

The mix starts about 32 bars before the end, on a phrase line (where music changes: bar 1, 17, 33 ...). Repeat one and no-repeat never mix. Picking a track, `Ctrl-n`, a seek or stop drop a transition (a running one lands on the new track first); a pause during one lands it too. It never touches a deck you loaded in DJ mode and stays out of the way while a set is in use. With `:autoplay` on as well, the queue never runs dry: codjd keeps queuing tracks that blend, so automix always has a next track. `:automix` turns it off and on (`[player] automix = false` in codjd's config makes off the default). In DJ mode `gt` runs the same thing between the decks.

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
 DJ   h l focus deck · Tab library · : command (:q leaves) · ? keys     ● live
┏ DECK A  ▶ FOCUS  ▶ playing ━━━━━━━┓┌ DECK B  ⏸ paused ─────────────────┐
│ title — artist                    ││                                   │
│ 129.55 BPM (trk 127.98) key 8A +1.23% SYNC                            │
│ 01:23.4 -5:53  LOOP 4  HP ●       ││ time, loop, headphone-cue         │
│ 1 ███ 2 ██░ 3 ░░░ 4 ░░░          ││ phase meter: the 4 beats, current lit
│  a 0:12  b --  c 1:35  d --  e○ f○ g○ h○   marks (hot cues)          │
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

DJ mode is vim for decks. **One deck has the focus** (A at the start) and every deck key acts on it; `h` focuses deck A, `l` deck B (the arrow keys do too). The focused deck has a thick, bright border and a `▶ FOCUS` badge. Motions take a **count** typed in front, like `4w` or `2}`; the half-typed count or command shows at the right end of the status line (like vim's showcmd: `4`, `42m`), `Esc` drops it. Letters are the same in every terminal. **`Esc` never leaves DJ mode, `:q` does.**

|Key|Does|
|---|----|
|`h` / `l`, `Left` / `Right`|Focus deck A / B|
|`Space`|Play / pause the focused deck|
|`c`|Cue (CDJ cue): hold to preview where the terminal reports key releases, else press to start and press again to end|
|`s`|Sync the focused deck to the other|
|`a` / `b`|Load the highlighted library track onto deck A / B, from the decks (no `Tab`)|
|`w` / `ge`|Jump 1 beat forward / back, `4w` 4 beats, `12ge` 12 beats back (beat phase is kept)|
|`[` / `]` on a paused deck|**Find a spot**: scrub back / forward by the jog step (hold to spin faster); `g[` / `g]` make the step finer / coarser (0.02 s to 5 s, shown as JOG). The status line says all this while the focused deck is paused. On a playing deck `[` / `]` bend the tempo instead|
|`W` / `B`|Jump a bar (4 beats) forward / back, `2W` two bars. A bar is taken as 4 beats, codjd reports no time signature|
|`}` / `{`|Jump a phrase (16 bars, 64 beats) forward / back, `2}` two phrases|
|`t` + letter|**Your transition**, from the playing deck into the other one, bringing in the **highlighted library track** (loaded onto the free deck first, unless that deck holds it already: then it comes in from where you cued it), executed by codj on the beat: `tb` blend, `ts` bass swap, `te` EQ swap, `tf` filter blend, `to` echo-out, `tr` loop roll, `tc` cut, `tk` brake, `tt` codj picks. A count sets the length in bars (`16tb`, `32te`). It starts on the playing deck's next 8 bar line; `T` instead of `t` starts it at the outro (`Tb`). The new track starts where you cued its deck (jog wheel, a mark, `c`), or at its hot spot when it sits at the start. The status line says what was planned; `u` or `F8` cancels it, and once it is running **rolls it back**: the new track stops and the old one plays on alone at full level. With a keyboard you turn one control at a time; this is how a whole transition (faders, bass swap, filter) happens at once|
|`j` / `k`, `Enter`, `q`|The library from the decks, no `Tab` needed: move the highlight, load it onto the free deck (the one not playing; it gets the focus), queue it. `Tab` still opens the full browse (`Ctrl-d`, `gg`, `G`, `a` / `b` for a given deck)|
|`F9` … `F12`|From any pane: `F10` set editor, `F11` AI, `F12` patch decks jump straight to each other (the editor keeps its unsaved text, a running agent keeps running); the open pane's own key closes it. `F9` records from inside a pane too|
|`gt`|[Automix](#automix) from the other deck (playing) into the focused one (loaded, paused): codjd picks the transition for the two tempos (a 32 bar blend, a 16 bar filter blend, or a crossfade) and starts it on the playing deck's next 8 bar phrase. Like vim's `gt` (next tab): on to the next track. The status line says what it chose (`automix into deck B: blend · 32 bars · bar 97`)|
|`0` or `gg`|Go to the start of the track (the first downbeat when the grid is known, else 0 s). A `0` after other digits is part of a count (`10w`)|
|`G` or `$`|Go to the outro: 32 bars before the end (60 s before the end without a grid). Seeking does not keep the beat phase, jumps do|
|`m{a-h}`|Mark: save a hot cue at the playhead (eight slots, `a` to `h`; beat-snapped with quantize)|
|`'{a-h}` or `` `{a-h} ``|Jump to a mark (quantized while playing). An empty mark says so and does nothing|
|`v`|Loop: starts a loop of the loop length (4 beats, or what `f` + the loop label set) on the focused deck; `v` again leaves it. `8v` loops 8 beats; while looping a count resizes the loop (`16v`) and plain `v` leaves it. The status line shows `-- LOOP 8 --` while the focused deck loops|
|`+` / `-`|Pitch of the focused deck up / down 0.1 % per tap (`4+` is 0.4 %); holding ramps slowly (1 to 4 %/s). `+` is `Shift + =`|
|`=`|Pitch back to 0 %|
|`R`|Reset the focused deck: EQ flat, filter off, fader up, pitch 0 (each change is undoable with `u`)|
|`[` / `]`|A **playing** deck: nudge its tempo down / up (hold = continuous bend where releases are reported, tap = a short 0.3 s bend). A **paused** deck: roll back / forward to find a spot, like the knob's jog wheel (the `JOG` step a press, 0.1 s unless changed; faster while held); then `c` or a knob press cues there. **With a control picked through the hints they step that control instead**, see below|
|`.`|Repeat the last action on the focused deck (a motion with its count, a mark, a loop, sync, play, a pitch step). A count replaces the old one: `2.`|
|`u` / `Ctrl-r`|Undo / redo the last change of a control made by hand (keys, hints, wheel, knob, `:` commands). A held ramp or edits within 0.6 s of each other are one step; 50 steps are kept|
|`f`|[Control hints](#control-hints-and-editing): label every control, type a label to pick it|
|`\`|Reset the picked control|
|`;`|Toggle between the last two picked controls|
|`Tab`|Browse the [library](#library-vim-keys) with vim keys (`j` / `k`, `a` / `b` load); `Tab` or `Esc` come back. The same while the [tutor](#learning) is open|
|`/`|Straight into the library search (same regex and `artist:`/`title:`/`genre:` filters as the list); `Enter` keeps the filter and browses, `Esc` clears it|
|`Up` / `Down`, `PageUp` / `PageDown`|Move in the library without leaving the decks|
|`q`|In the library: add the highlighted track to the player's queue. The **UP NEXT** box beside the library (100 columns and up; else the library title) shows the coming automix transition (`CRIA` · `blend in 1:12`) and the rest of the queue|
|`Enter`|Load the highlighted track onto the focused deck; when that one is playing, onto the other (never over a playing deck: with both playing it says so and loads nothing)|
|`:`|DJ command line, see below|
|`?`|[The key map on the screen](#learning), every key explained (`q` or `Esc` closes it; on a small screen `j` `k` `Ctrl-d` `Ctrl-u` `g` `G` scroll it)|
|`F1` / `F2`|Headphone cue on deck A / B (`:hp`)|
|`F5`|Quantize on / off (mixer-wide, `:quantize`)|
|`F6` / `F7` / `F8`|[Set](#sets-in-dj-mode): arm the next mix / auto mode on-off / disarm (`:arm next`, `:auto`, `:disarm`); `F8` also cancels a planned transition and rolls back a running one, like `u`|
|`:split`|One pair of headphones: the master (what the room hears) in the left ear, the cue (the deck you prepare, its headphone button on) in the right. `:split` again goes back to master and headphone outputs. In codjd's config: `[audio] mode = "split"`|
|`F9`|[Record](#capture-and-recording) a mix into the active set on / off (same as `:rec`)|
|`F10` / `F11`|[Editor pane / AI prompt box](#editor-pane-and-ai-prompt-box) (`:e`, `:ai`); the same key closes it again, like `F12` for the patch pane|
|`F12`|[Patch pane](#patch-decks): synths and drum machines in four slots, with a sequencer grid (same as `:patch`)|
|Mouse wheel|Edits the picked control (up = +); with nothing picked it scrolls the library. Clicks do nothing in DJ mode|
|`Esc`|Closes the set error overlay if open; else drops a half-typed count or mark; cancels the hints if they are up; clears the picked control. It never leaves DJ mode|

Nothing else is bound on the decks: no other letter, digit or `Alt` combination plays (`gt` is two keys on purpose, so a stray key never starts a mix), and there is no `x` / `d` that could unload a track in the middle of a set.

**Commands** (`:` then Enter; the mixer is set here and through the hints):

|Command|Does|
|-------|----|
|`:q`|Leave DJ mode back to the normal view (also `:quit`, `:q!`, `:wq`, `:x`); music keeps playing|
|`:fader [a\|b] <0..1>`|Channel fader of the focused deck (or the named one); `off` is 0, `full` 1|
|`:low` / `:mid` / `:high [a\|b] <-1..1>`|EQ band of the focused deck; `kill` is -1, `flat` or `0` is 0. `:low kill`, `:low b flat`|
|`:filter [a\|b] <-1..1>`|Filter: below 0 muffles, above 0 thins the bass; `reset` is 0|
|`:xfade <-1..1>`|Crossfader, -1 is all deck A, 1 all deck B; `a`, `b` and `center` work too|
|`:delm a` / `:delm a c` / `:delm b-d` / `:delm!`|Delete marks of the focused deck (like vim's `:delmarks`); `:delm!` deletes all eight|
|`:focus a\|b`|Focus a deck|
|`:hp [a\|b] [on\|off]`, `:quantize [on\|off]`|Headphone cue (the focused deck unless named), quantize|
|`:load a`, `:load b`, `:grid a x2` / `/2` / `x1.5` / `/1.5` / `reset`|Load the highlighted library track; correct a deck's detected tempo (saved per track). A yellow `?` after the BPM means the tempo detectors disagree, so the grid is likely half, double or 1.5× off. Sync refuses changes over ±16 % and says which tempos clash|
|`:help [term]`, `:tutor [reset]`, `:mix new [recipe] [bars]`|See [Learning](#learning)|
|`:patch ...`|See [Patch decks](#patch-decks)|

Values outside the range are refused with a usage note, nothing is sent. Every value goes through the normal `DjSet` path, so codjd sees a manual move, and `u` takes it back.

Tap vs hold: pitch taps step 0.1 %; holding `+` / `-` ramps after 0.2 s (slowly, 1 to 4 %/s). Hints edits step 0.05 (see below). Values are sent at most about 30 times a second per control and coalesced, so a slow bus never builds a backlog; the screen always shows what codjd reports.

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
|`ca` / `cs`|loop length A / B (for a `v` without a count)||
|`ba`|master|`bs`|cue mix|
|`bd`|sync range (the largest tempo change a synced play may need)||

   The table is fixed, so a label never moves with the window size. A control the screen has no room for at the moment (small terminals drop master / cue mix / sync range first, then gain) gets no label and cannot be picked.
3. The control is now selected: it is drawn inverted, the right end of the status line shows a chip like `◆ A filter -0.30`, and `f` brings the labels back for another pick. `;` swaps with the control selected before it (two controls you play together: filter and crossfader). `Esc` clears the selection; the hint labels are only typed while they are up, so they never collide with the deck keys (`w` is a motion, and a label too).

While a control is picked `]` / `[` / `\` edit it; **with nothing picked `]` / `[` nudge the focused deck** and `\` says so. A picked control is shown in the status line, so you can always tell which of the two the bracket keys do.

Editing the selected control:

|Control|`]` / `[` step|`\` or knob press|
|-------|---------------|-------------------|
|EQ, filter, crossfader, channel fader, cue mix, master|0.05|EQ / filter 0, crossfader 0, fader 1, cue mix 0, master 1|
|Pitch|0.1 %|0 %|
|Gain|0.5 dB|the track's auto-gain (what the deck loaded with)|
|Loop length|×2 / ÷2 along 1/4 1/2 1 2 4 8 16 32 beats|4 beats|
|Sync range|0.01 (1 %)|0.16 (codjd's default)|

Everything goes through the normal `DjSet` path (coalesced, at most ~30 a second per control), so codjd sees a manual move: it cancels the automation of that control and records it into a mix recording. The loop length is only used by a `v` without a count; it lives in codj, not in codjd. The decks show it as `loop 4` (idle loops) and `gain +0.0` in the time line when it has room; the mixer line shows `MST`, `CUE` and `RNG` on wide terminals (72 columns and up).

Codjd does not report a manual gain trim or the sync range, so codj shows the last value it sent itself (a track loaded afterwards starts from its own auto-gain again). The sync range is assumed to be codjd's default 0.16 whenever DJ mode opens, since codjd cannot be asked for it.

**Acceleration** (keys, wheel and knob alike): detents less than 60 ms apart multiply the step, x2 for the second, x4 from the third on, until you pause or reverse. A slow turn is always one step per detent; a held `]` accelerates through the keyboard's auto-repeat. For the loop length a x4 detent moves up to four notches.

**Mouse wheel**: enabled in DJ mode for the wheel only. With a selected control the wheel edits it (up = +); with none it scrolls the library list. It is ignored while the set editor or the AI prompt box is open.

## Hardware knob

A rotary encoder that sends volume keys (the knob on many keyboards' media cluster, e.g. the Ajazz AK820's) is codj's volume in the normal view and a jog wheel and control knob in DJ mode. It is off until the config names the device:

```toml
[knob]
device = "AK820 Consumer Control"   # part of the input device's name (see `evtest`); empty = off
up = 115                            # Linux input key codes, these are the defaults:
down = 114                          #   KEY_VOLUMEUP, KEY_VOLUMEDOWN,
press = 113                         #   KEY_MUTE
grab = true                         # EVIOCGRAB the device while codj runs
```

- **Normal view:** turning changes codjd's volume 2 % a detent; a press mutes / unmutes.
- **DJ mode**, first match wins:
  - a control is picked (`f` and its label): turning edits it with the same acceleration (up = +), a press resets it;
  - the library is browsed (`Tab`): turning scrolls it, a press loads the highlighted track like `Enter` (the focused deck, else the other);
  - otherwise the knob is the **volume** (2 % a notch), as in the normal view. **A knob press** switches it to the **jog wheel** of the focused deck and back (the status line says which): paused, turning scrubs the position to find a spot by ear, by the `JOG` step a detent (0.1 s to start), x2 / x4 / x8 when spun fast; playing, turning bends the tempo (±2 %) while the knob moves, to line up the beats. `JOG 0.1s` on the crossfader line is a control like the others (`f` `bq`): the knob or `]` `[` go through 0.02 to 5 s, a knob press puts it back to 0.1 s. `jog = 0.5` under `[knob]` sets where it starts.
- Only the three configured codes are acted on; everything else the device sends is dropped unseen (never logged or stored), because the device may also be your keyboard.
- With `grab = true` the device is taken exclusively while codj runs, so the system volume does not move with it; quitting codj releases it (and so does the kernel if codj dies or panics, because the grab belongs to the open file). A device that also has letter keys is never grabbed, it would swallow your typing; use the consumer-control node (`device` matches the first node whose name contains it and that has one of the configured keys).
- It needs read access to `/dev/input/event*` (the `input` group). A missing device, no permission or an unplugged knob shows one note in the status line; DJ mode works without it.
- The knob is read by the `evdev` crate on its own thread and handed to the screen over a channel; `cargo test` includes a test with a virtual device that is skipped when `/dev/uinput` is not writable.

## Sets in DJ mode

A set is a text file (`docs/set-language.md` in the codjd repo): tracks, cues and the transitions between them in bars and beats. codjd runs the mixes; DJ mode shows the set and arms mixes. All of it needs a codjd with the "Set runtime (D-Bus)" methods (`docs/dj-api.md`); against an older codjd the strip never appears and a set command says `codjd has no set runtime: update codjd`.

Mixes and tracks are numbered from 1 on screen (the bus counts from 0).

|Command|Does|
|-------|----|
|`:set`|List the sets with their error counts in the status line|
|`:set <name>`|Make a set active (name, file name or stem); `:set none` deactivates|
|`:set new <name>`|Make a set: writes `<name>.codj` with a header and pointers (`:mix new`, `F9`, `F10`) into the set folder and makes it active. An existing name is refused; the status line says `:set <name>` uses it|
|`:arm <n>` / `:arm next`|Arm mix `n`: its outgoing track must be on a deck, the incoming one is loaded on the other deck, synced; the mix runs by itself when the outgoing deck reaches the mix start. `next` picks the mix after a running one, else the first whose outgoing track is on a deck (a playing one first)|
|`:disarm`|Cancel the armed or running mix's remaining automation; controls stay where they are|
|`:auto on` / `:auto off`|Auto mode: after a mix ends, the next one whose outgoing track is on air is armed (`:auto` alone toggles)|
|`:preview <n> [bars]`|Audition mix `n`: both tracks loaded, the outgoing deck `bars` (default 8) before the mix start, armed and playing|
|`:load <n>`|Load track `n` of the set onto the free deck with the set's grid/gain overrides (`:load a` / `:load b` still load the library selection)|
|`:e`|The in-TUI editor pane on the active set (also `F10`), see below|
|`:ai <text>`|Run the AI agent on a request (also `F11`), see below. The prompts of a DJ session are one conversation|
|`:ai new`|Start a new conversation with the agent|
|`:errors`|The active set's errors as `file:line:col: message` in an overlay; `Esc` closes it|
|`:edit`|Open the set file in `$EDITOR` (else `$VISUAL`, else `vi`). The terminal is handed over (raw mode, alternate screen, mouse capture and keyboard-protocol flags are given back), the audio keeps playing, and everything is restored when the editor exits; codjd reloads the file when you save|

|Key|Does|
|---|----|
|`F6`|Arm the next mix (same as `:arm next`); a mix that is armed but not yet started is not re-armed|
|`F7`|Auto mode on / off|
|`F8`|Disarm|
|`F10`|Editor pane (same as `:e`)|
|`F11`|AI prompt box (same as `:ai`); brings back a hidden run|

`Enter` is not used for this: it loads the library selection onto the focused deck.

While a set is active a strip sits between the decks and the mixer: the set name, `✓ no errors` or `✗ N errors` in red, `AUTO` when auto mode is on, the track list with the track on air (`▶`, green) and the incoming one (`»`, yellow) and the mixes as segments between them (`───16b▶` is a 16-bar mix, bold when armed). For the armed mix there is a progress bar over its length in bars with the current bar and a `◆` at each upcoming automation item, and below it the next items (`bar 16 · swap low`). Without an armed mix the strip lists the mixes and which one F6 would arm. It is 6 rows on a tall screen, 4 on a medium one and one line on a small one. `SetState` is polled at about 7 Hz while a set is active, once a second otherwise.

## Capture and recording

Playing by hand turns into set code: codjd writes the active set's file itself (line-level edits, comments and formatting stay, git commit or `.bak` per edit, see "Editing from codj" in `docs/set-language.md`). All of it needs an active set (`:set <name>`) and a codjd with the "Set editing" methods (`docs/dj-api.md`); the commands answer in the status line with codjd's reply.

|Command|Does|
|-------|----|
|`:cue <name> [a\|b]`|Capture the deck's position as cue `name` (`cue drop bar 33` in that track's block; a track not yet in the set is added). Status line: `cue drop → aft bar 33`|
|`:loopsave <name> [a\|b]`|Capture the deck's active loop as loop `name`|
|`:gridsave [a\|b]`|Write the deck's current grid (after `:grid` corrections) into the set as a `grid` line|
|`:rec audio` / `:rec audio on` / `:rec audio off`|Record what the room hears (the master, never the headphones) to a WAV file: `~/Music/codj-mixes/mix-<date>-<time>.wav`, or `[record] dir` in codjd's config. The header shows `● REC 12:34` while it runs. It runs in codjd, so it keeps going if codj is closed; a crash or a full disk still leaves a playable file up to that point|
|`:rec` / `:rec on` / `:rec off`, `F9`|Start / stop recording a mix. While recording every control change on either deck (keys, MCP, D-Bus) is logged; stopping writes the `mix` block and says e.g. `recorded mix 3: aft → smk, 16 bars, 9 items` (mixes count from 1 on the screen, like `:arm 1`)|

Names are set-language identifiers (letters, digits, `_`, `-`, starting with a letter or `_`). Without `a` or `b` the command uses the focused deck (`h` / `l`); an empty focused deck says so and asks for the other one by name.

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

**Which AI.** By default `:ai` runs Claude Code (`claude -p`, your Claude login). For any other model, codj runs its own agent loop on an OpenAI-compatible chat API, with the same codj tools and the same `--live` guardrail:

```toml
[agent]
provider = "openrouter"                 # one key, most models: OPENROUTER_API_KEY
model = "anthropic/claude-sonnet-4.5"   # any OpenRouter model name (this is the default)

# provider = "openai"                   # OPENAI_API_KEY, model e.g. "gpt-4.1"
# base_url = "http://127.0.0.1:11434/v1"  # any OpenAI-compatible API: a local server, another router
# model = "qwen3"
# api_key_env = "MY_ROUTER_KEY"         # the variable holding its key; keys never go in the config
```

The loop asks the model, runs every tool call of its reply against codj's MCP server and returns the results by call id, and asks again until the model answers without a tool call; it stops after 20 turns or when the model repeats the same call three times. The prompts of a DJ session are one conversation (`:ai new` starts another); the system message carries the current decks and sets afresh each time. API errors read as words in red: `no credits left`, `rate limited`, `the API key was refused`. `provider = "codex"` / `"grok"` are not supported yet. `examples/native_agent_e2e.sh` tests the loop against a fake API on localhost.

**What the agent is doing.** With `claude` the pane shows the agent's work as it happens, from Claude Code's `--output-format stream-json` events: its text, every tool call as `→ set_write name=claude-trio text=<12 lines> base_hash=3f2a…` (whole texts are never dumped), each result as `✓ ok 9c1e…` or `✗ set changed since you read it` (red), and a closing `-- done: 4 turns, 12.3 s, $0.07`, which stays above the diff in the review. `output = "text"` under `[agent]` shows the plain output instead; `output = "stream-json"` reads events from another command that prints them (grok's headless `--output-format streaming-messages-json` copies the format).

**One conversation.** With `claude` as the command, the prompts of one DJ session are one conversation: the first runs with `--session-id <uuid>`, later ones with `--resume <uuid>`, so "make it shorter" knows what "it" is. Every prompt still carries the current decks and sets. The box's title says which turn it is; `:ai new` forgets the conversation and the next prompt starts another (so does leaving and opening DJ mode). Other agent commands run one prompt at a time, as before.

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
|`mcp_tools`|all set and patch tools (+ live ones when `live`)|Allow-list of MCP tool names, without the `mcp__codj__` prefix|

## Learning

DJ mode explains itself, for someone who has never DJ'd. All the wording lives in one module (`src/dj/glossary.rs`), so the key map, the status line, `:help` and the tutor say the same thing.

**`?` key map.** Draws the keys on the real screen, which-key style: one panel over the decks lists the focused deck's keys (in two columns on a wide screen; its title names the focused deck), the area over the mixer and the library lists the mixer's keys (the hints and the `:` commands) and the global ones, every entry with a line of explanation (`sync: match B's tempo and beats to A`, `cue: a saved spot; hold to hear it in headphones`, `low: the bass — keep only one bassline at a time`, `crossfader: which deck the room hears`). `q`, `Esc` or any key other than the scroll keys closes it and does nothing else; the DJ keys are not played while it is up. When the panels do not fit (a small terminal or a phone), every key goes into one list with a heading per section, which `j` `k` `Ctrl-d` `Ctrl-u` `g` `G` scroll like vim's help; its title says which lines show and `j more` while there are more.

**Hints explain too.** While the `f` labels are up the status line says what the typed letters lead to (`mid: vocals, synths, snares; the body of a track  (TA A mid · TS B mid)`); once a control is selected its one-liner sits next to the chip (`low: the bass — keep only one bassline at a time ◆ B low -0.30`).

**`:help [term]`.** A popup with the glossary entry (a one-liner and one to three sentences); any key closes it. `:help` alone lists the terms, `:help keys` opens the key map. Case, spaces and `-` / `/` do not matter (`:help Bass-Swap`). Terms: bpm, beat, bar, phrase, downbeat, sync, phase, cue, hot cue, loop, eq, low/mid/high, kill, bass swap, filter, fader, crossfader, headphones/cue, gain, key, set, mix, recipe, quantize, and nudge, pitch, grid, jump. A word that is not there gets a "did you mean" note.

### `:tutor`

A guided first mix with your own library, like vimtutor. A pane shares the bottom of the screen with the library (the decks keep at least 10 rows; the set strip shrinks to a line on a medium screen; on a screen under 100 columns the pane takes the whole bottom). Each step has a title, 1 to 3 sentences of why, the exact keys and a live checklist. **Steps finish by themselves**, a moment after the state shows they were done (the step stays up for at least 1.2 s so it can be read). `:tutor next` skips the current step (`Tab` browses the library as always), `Esc` pauses and closes the pane (an open hint selection is cleared first), `:tutor` resumes, `:tutor reset` starts over. Progress (`{"version":1,"step":N}`) is saved to `$XDG_STATE_HOME/codj/tutor.json` (`~/.local/state/codj/tutor.json`, else under the data directory) after every step.

|#|Step|Keys|Done when (polled `DjDecks` / `DjMixer`)|
|-|----|----|----------------------------------------|
|1|Choose two tracks: codj proposes a pair from the library with tempos within ±6 % and marks them `★A` / `★B`|`Tab` browses the library, `a` loads the highlighted track on A, `b` on B, `Tab` comes back (after A is loaded the highlight moves to the second track)|both decks loaded, with different files|
|2|Play deck A: what BPM, beat and bar are, and the phase meter|`h Space` (h focuses A; loading a deck focuses it)|A playing|
|3|Listen to B in headphones: what cue is for|`F2`, and `:xfade a` so the room only hears A|B's headphone send on, or 45 s (with one output, `internal` mode, there is nothing to hear apart; the step says so)|
|4|Sync B to A: what sync does, the ±16 % limit, what a yellow `?` next to the BPM means, `:grid`|`l s` (focus B, sync)|B synced|
|5|Start B on a phrase: phrases of 16 / 32 bars, quantize. The pane counts A's bars and says when to press|`Space` (B has the focus)|B playing|
|6|Bring B in without its bass: why two basslines clash|`f q`, `[` down to -1 (or wheel / knob; or `:low b kill`), `:xfade 0`|B low <= -0.9|
|7|Swap the bass on a bar line|`:low a kill`; `:low b flat` (or `f q`, `\`)|A low <= -0.9 and B low >= -0.1|
|8|Fade A out, then stop it|`:xfade b` (or `:fader a 0`), then `h` `Space`|A stopped and (crossfader >= 0.5 or A's fader <= 0.1)|
|9|Done: record the next one (`F9`), `:mix new blend`, `:help`|||

Step 1 spends at most 14 `GetGrid` calls: the first track with an analysed, steady grid (of the first 6) is the seed, then up to 8 candidates, the ones `SimilarTo` names first and then the following library tracks (the indices `SimilarTo` returns are taken as positions in the library list); the first with a tempo within ±6 % is proposed. When no pair is within ±6 % (or an old codjd has no `GetGrid`, or nothing is analysed yet) the pane says "no two tracks within 6 % found: pick any two, sync bends one to the other". **Bars**: with the deck's grid (`GetGrid`: first beat, downbeat offset, beats per bar) bar = `floor((k - offset) / beats_per_bar) + 1` for the beat index `k = floor((position - first_beat) * track_bpm / 60)`; without a grid reply `floor(position * track_bpm / 60 / 4) + 1`. The pane shows `A is on bar 29, beat 1: the phrase line is bar 33, in 4 bars`, and `press Space now` in the last beats before a phrase line and the first ones after it. Resetting with both decks still loaded passes step 1 at once.

### `:mix new <recipe> [bars]`

Writes a transition for the two tracks on the decks into the active set: **outgoing** is the playing deck on air (the one that is playing, the louder when both are, the crossfader's side on a tie; this rule is about what the room hears, so it does not use the focus), **incoming** the other. Both must be tracks of the active set (matched by file path against `SetState.tracks`), else `add the track to the set first: :cue on it or edit`. `:mix new` alone pops up the recipes with their explanations and default bars (`SetRecipes`); `:mix new blend` or `:mix new blend 32` takes the block from `SetRecipeText(recipe, from id, to id, bars)` (a full, commented `mix` block), reads the set with `SetSource`, appends the block after a blank line, checks the whole text with `SetCheck` and saves it with `SetWrite` (author `human`, on the hash it was read at, so a save made meanwhile refuses it). The status line says `added blend Kaiku → Aftertekno (8 bars) as mix 3: :arm 3`. When a `mix <from> -> <to>` block for the same two tracks exists the new one is appended and the note names the older block's line; if `SetCheck` objects to the duplicate, the older block is replaced instead and the note says `replaced ...`. Against an older codjd the commands say `codjd has no recipes: update codjd`.

**Set warnings.** `SetWarnings(set)` (non-fatal findings of the set) is read when the active set changes and every 3 s: the strip shows a yellow `⚠ 2 warnings (:errors)` next to the red error badge (`⚠2` on a one-line strip) and `:errors` lists them after the errors as `warning: file:line:col: message` in yellow. An older codjd without the method just shows none.

## Patch decks

Besides the two decks there are four **patch slots**, P1 to P4: synths, drum machines and sequencers that play in time with the decks, built from a graph of nodes (modeled on NoiseCraft; the file format, the node catalog and the D-Bus methods are in `docs/patches.md` of the codjd repo). A patch is a JSON file in the sets folder's `patches/` that you, the AI agent or both edit. A slot is a mixer channel like a deck (fader, filter, low) whose sequencers are clocked by the beat grid. It needs a codjd with the patch methods; against an older one the pane says `codjd has no patch decks: update codjd`.

**The pane** (`F12`, or `:patch`) takes the rows of the library and the set strip; the decks and the mixer stay above it. It shows one block per slot: the patch's name (a yellow `●` when it has unsaved edits), a **LIVE** (red: the room hears it) or **CUE** (cyan: headphones only) badge, its cpu share (red from 25 %, the budget codjd mutes a slot at), a red `✖ MUTED: cpu overrun` when codjd muted it, the slot's fader, filter and low, its knobs as a name, a bar and a value, and its sequencers as `drums A→C step 6` (the pattern that plays, the one queued for the next bar, the step that plays now). **The deck keys are off while the pane is open** (the header says so); `f`, `]`, `[`, `\`, `;`, the mouse wheel and a hardware knob keep working, so you can still pick and turn a control.

|Key (pane open)|Does|
|---|----|
|`1`-`4`|Open the sequencer grid of that slot (an empty slot says how to load one)|
|`j` / `k`|Select the next / previous slot (overview)|
|`Tab` / `Shift + Tab`|Next / previous sequencer, across the slots|
|`Esc`|Leave the grid, then close the pane (`F12` closes it at once)|
|`p`|Send the selected slot LIVE to the room, or back to headphones only (CUE)|
|`u` / `Ctrl-r`|Undo / redo the slot's edits (`PatchUndo` / `PatchRedo`)|
|`:w [name]`|Save the selected slot's patch, with its live edits, under its own name or `name` (`PatchSave`, author `human`)|
|`?`|The keys of the pane (any key closes)|

**The grid** shows one sequencer: the eight patterns `A B C D E F G H` on top (the edited one inverted, the playing one green, the queued one yellow and underlined), a beat ruler, and a row per drum sound (`kick`, `snare`, `hat` ... from the node's `row_names`) or per scale degree with its note (`8 A3` down to `1 A2`, highest on top, from the node's `scale` and `root`). Cells are `█▇▅▃▁` by velocity on drum rows and `●` on note rows; the playing step is a moving column (and a `▾` over the ruler) while the edited pattern is the one that plays. The grid is read once with `PatchSource` when a slot loads (and again when its unsaved flag clears), because `PatchState` does not carry cells; codj mirrors its own cell edits, so `u` / `Ctrl-r` keep it in step as long as the edits are made from the pane.

|Key (grid open)|Does|
|---|----|
|`h` `j` `k` `l` or the arrows|Move the cursor|
|`w` / `b`, `0`, `$`|Next / previous beat (4 steps), first step, last step|
|`x`|Toggle the step. A note step holds one note: turning one on clears the rest of its column|
|`1`-`9`|Velocity of the drum cell under the cursor: `d / 9`, so `9` is full and `1` a ghost note (0.11). On an empty cell it turns it on; `x` then uses the last one. Not on note rows|
|`A`-`H` (Shift + letter)|Choose the pattern you edit. Edits go to it, whichever pattern plays|
|`Enter`|Queue the edited pattern: it plays at the next bar (`PatchAction {"op":"pattern",...,"now":false}`)|

Every cell change is one `PatchAction {"op":"cell",...}`: it applies at once, never recompiles and stays an unsaved edit of the slot until `:w`.

|Command|Does|
|---|----|
|`:patch`|Open the pane|
|`:patch new <name> [drums\|acid\|pad]`|Write a starter patch (a 4-row drum machine with a basic beat, an acid bassline with cutoff and reso knobs, or a slow pad) as `<name>` through `PatchWrite` (empty base hash, author `human`) and load it into the first free slot, CUE|
|`:patch load [slot] <name>` / `:patch unload <slot>`|Load a patch into a slot (the first free one without a number) / empty it. Slots are numbered 1-4|
|`:patch live <slot> [on\|off]`|LIVE or CUE; without `on` / `off` it toggles|
|`:patch import <file.ncft> [name]`|Import a NoiseCraft file (`PatchImport`); the note says what was dropped|
|`:patch list`|The patch files with their titles|
|`:patch edit <name>`|The patch's JSON in the [editor pane](#editor-pane-and-ai-prompt-box) (`PatchSource` / `PatchCheck` / `PatchWrite`). `PatchCheck` errors are `<node>: message`; a marker goes on the line of that node (its key `"<id>": {` or its `"name"`), errors for the whole file (`patch: no AudioOut node`) show under the text|

**Hints for patch controls.** With the pane open `f` labels every slot's knobs (up to 9 each) and its fader, filter and low, in the left-hand letters like everything else: slot P1 uses labels starting with `t`, P2 `g`, P3 `x`, P4 `c`; the second letter is the knob's position (`d q w e r t v g b`, the order in the slot's knob list) or `z` / `x` / `c` for the slot's fader / filter / low (so P1's cutoff, if it is the first knob, is `td`; P3's fader is `xz`). Then `]` / `[` / the wheel / a hardware knob edit it: a knob moves by 1/20 of its range per step and goes out as `PatchAction {"op":"knob",...}` (coalesced like the mixer values), `\` returns to the value in the patch file; the slot's mixer controls go out as `DjSet` on deck `2 + slot`. codjd reports a slot's fader but not its filter and low, so those show the last value sent from here.

**AI.** The `:ai` preamble lists the four slots (patch, LIVE or CUE, muted reason, knobs with values, sequencers with their patterns) and the rules: patches only through the patch tools, `patch_node_types` then JSON then `patch_check` then `patch_write` with the `base_hash`; a LIVE slot is what the audience hears, leave it alone unless the request says otherwise. The default tool list of the agent command now includes the always-allowed patch tools (and the live ones when `live = true`). See the MCP tools below.

**MCP tools** (`codj mcp`): `patch_list`, `patch_source` (`{json, hash}`), `patch_check` (`{valid, errors}`), `patch_write` (`base_hash` required, `""` only to create; author `agent:<client>` like sets), `patch_import`, `patch_state` (the slots, knob names and values, sequencers with pattern letters, queued pattern or `null`), `patch_node_types` (the node catalog with inputs, defaults, outputs and params, plus the file format rules; it needs no daemon) and `patch_action` (one op as an object; a JSON string holding one is accepted) work without `--live`; `patch_load`, `patch_unload` and `patch_live` need `--live` (refused with the same message as the other live tools, before anything reaches D-Bus). `patch_action` is always allowed, as the contract has it, so an agent can turn knobs of a slot that is LIVE; the prompt rules tell it to leave a LIVE slot alone unless asked. Workflow: `patch_node_types` -> write the JSON -> `patch_check` -> `patch_write` -> (live) `patch_load` -> `patch_action` for knobs and cells -> `patch_live`.

## Hold and release

Hold-to-preview cue (`c`), hold-to-nudge (`[` `]`) and the pitch ramp (`+` `-`) need to know when a key is released. In terminals with the kitty keyboard protocol (kitty, foot, WezTerm, Ghostty, Alacritty) DJ mode asks for key press/repeat/release events while it is open (`DISAMBIGUATE_ESCAPE_CODES | REPORT_EVENT_TYPES | REPORT_ALL_KEYS_AS_ESCAPE_CODES`; letters only report releases with the last flag) and gives the extra flags back when you leave DJ mode, on exit and in the panic hook. While you type in the search box or command line the extra flags are off, so typed text arrives as text.

Without release events (other terminals, tmux) DJ mode says so in the status line and falls back:

- cue (`c`): press starts the preview, press again ends it (on a playing deck a press just stops at the cue point)
- nudge (`[` `]`): a tap bends the tempo for 0.3 s; holding the key keeps it alive through the keyboard's auto-repeat

- pitch (`+` / `-`): one step per key event, so holding steps at your key repeat rate

Auto-repeat of a tap key (play, sync, loop, marks, motions, counts) is ignored, so holding `w` jumps once; type `8w` for more.

## Testing without codjd's engine

`examples/fake_codjd.rs` serves the deck API (and a fake set runtime: sets `demo` and `broken`, 3 tracks, 2 mixes, an armed mix that advances) with plausible changing state and logs every call, `examples/dj_e2e.sh` runs codj against it in tmux (sets, capture and recording included, and phase 5: control hints, label selection, `] [ \ ;` edits, the mouse wheel as SGR mouse reports, the beat-jump keys `w` `b`; it also serves a set's text and hash, with conflict detection on stale writes, for the editing methods), and phase 6: the `?` key map, `:help`, hint explanations, `:tutor` (first steps advancing as the fake's state changes, `Tab`, `Esc`, resume, reset, the progress file in an isolated `XDG_STATE_HOME`) and `:mix new` through the fake's `SetRecipes` / `SetRecipeText` / `SetWarnings` / `GetGrid` / `SimilarTo` (`ONLY=6` runs just that phase), and phase 7: the patch decks (`F12`, `:patch new d1`, the grid with `hjkl x 1-9 A-H Enter p u Ctrl-r`, a hinted knob edited with `]`, `:w`, `:patch list|edit|live|unload|load|import`) against the fake's in-memory patch files and four slots with a moving step counter (`ONLY=7` runs just that phase), and phase 8: the vim keys (focus, `4w` `12b` `2}` `3W`, marks, `v` / `8v`, `0` `gg` `G`, `.`, `+` `u` `Ctrl-r`, nudge, `:fader` `:low` `:filter` `:xfade` `:delm`, the status-line count, `Esc` staying and `:q` leaving) with every call checked in what the fake received (`ONLY=8` runs just that phase; `TMUX_SOCK` names the tmux server, `W` the scratch directory, `BIN` the build directory); `examples/mcp_e2e.sh` drives `codj mcp` over JSON-RPC against it (set and patch tools, stale/fresh `base_hash`, the `--live` guardrail), all on an isolated bus (`dbus-run-session`); neither touches the real session bus.

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
