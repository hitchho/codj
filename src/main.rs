use std::{
    collections::HashMap,
    env,
    io::{self, stdout},
};

use crossterm::{
    event::{self, Event},
    event::{
        DisableMouseCapture, EnableMouseCapture, KeyboardEnhancementFlags,
        PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
    },
    terminal::{
        disable_raw_mode, enable_raw_mode, supports_keyboard_enhancement, EnterAlternateScreen,
        LeaveAlternateScreen,
    },
    ExecutableCommand,
};
use parser::*;
use ratatui::{backend::CrosstermBackend, Terminal};
use soulseek_types::{ConnectionState, Download, RankedFile};
use types::Lyrics;
use types::Metadata;
use types::OuterMusic;
use types::PlayerStatus;
use types::PlayingStatus;
use types::Playlist;
use types::Repeat;
use types::Sort;
use utils::RunStatus;
use zbus::{proxy, Connection, Result};

mod allowed_commands;
mod bar;
mod dedup;
mod dj;
mod dj_types;
mod knob;
mod mcp;
mod search;
mod settings;
mod parser;
mod soulseek_types;
mod states;
mod style;
mod types;
mod ui;
mod utils;

#[allow(unused_imports)]
use utils::log;

use crate::types::{OuterPlaylist, PlaylistOpperation};

// NOTE: the return status of most functions is not currently documented, tho
// usually they represent the success of the opperaiton/call.
#[proxy(
    interface = "net.rotko.Codj.Player",
    default_service = "net.rotko.Codj",
    default_path = "/net/rotko/Codj"
)]
pub trait Server {
    /// Toggle playing status
    fn toggle_play(&self) -> Result<RunStatus>;
    /// Gets the player playing status
    fn get_playing_status(&self) -> Result<PlayingStatus>;
    /// Gets the sorting status of the current playlist
    fn get_sort(&self) -> Result<Sort>;
    /// Gets next music on the playlist music list
    fn get_next_music(&self) -> Result<OuterMusic>;
    /// Gets previous music on the playlist music list
    fn get_previous_music(&self) -> Result<OuterMusic>;
    /// Gets the index of the currently playing music
    fn get_playing_index(&self) -> Result<u32>;
    // /// Gets the index of the currently playing music
    // fn get_playlist_name(&self) -> Result<u32>;
    /// Gets the played duration for the currently playing music
    fn played_duration(&self) -> Result<f32>;
    /// Plays previous music on the playlist music list
    fn play_previous(&self) -> Result<RunStatus>;
    /// Plays next music on the playlist music list
    fn play_next(&self) -> Result<RunStatus>;
    /// Plays a music from its indoex on the playlist
    fn play_from_index(&self, index: u32) -> Result<RunStatus>;
    /// queue the playlist track at `index`; `front` makes it play next
    fn queue_add(&self, index: u32, front: bool) -> Result<RunStatus>;
    fn queue_remove(&self, pos: u32) -> Result<RunStatus>;
    fn queue_move(&self, pos: u32, up: bool) -> Result<RunStatus>;
    fn queue_play(&self, pos: u32) -> Result<RunStatus>;
    fn get_queue(&self) -> Result<Vec<OuterMusic>>;
    /// (analyzed, queued) of the server's audio analysis
    fn analysis_status(&self) -> Result<(u32, u32)>;
    /// (bpm, loudness dBFS, analyzed)
    fn get_features(&self, path: &str) -> Result<(f64, f64, bool)>;
    /// queues the `n` tracks that sound most like `path`; returns how many were added
    fn queue_similar(&self, path: &str, n: u32) -> Result<u32>;
    /// Sorts playlist
    fn sort(&self, sort: Sort) -> Result<RunStatus>;
    /// Changes the repeat state of plalyist
    fn repeat(&self, repeat: Repeat) -> Result<RunStatus>;
    /// Returns current playlist
    fn playlist(&self) -> Result<OuterPlaylist>;
    fn get_playlists(&self) -> Result<HashMap<String, Playlist>>;
    fn get_playlists_names(&self) -> Result<Vec<String>>;
    fn use_playlist(&self, title: String) -> Result<()>;
    /// Returns the lyrics of the current music
    fn lyrics(&self) -> Result<Lyrics>;
    /// Returns metadata of the current music
    fn metadata(&self) -> Result<Metadata>;
    /// Play the currently paused music
    fn play(&self) -> Result<RunStatus>;
    /// Gets the repeat status of the current playlist
    fn get_repeat(&self) -> Result<Repeat>;
    /// Returns the player status:
    /// - status: <Playing|Pausing|Paused>
    fn player_status(&self) -> Result<PlayerStatus>;
    /// Plays the music from the file path, returns true if no panic happened
    /// Terminate playing, returns true if no panic happened
    fn end(&self) -> Result<RunStatus>;
    /// Resumes playing the currently paused song, returns true if no panic happened
    fn resume(&self) -> Result<RunStatus>;
    /// Pauses playing the currently playing song, returns true if no panic happened
    fn pause(&self) -> Result<RunStatus>;
    /// Played duration over the the total duration of the music
    /// format: full length / played duration
    fn timer(&self) -> Result<(f32, f32)>;
    /// Seeks the player by the given duration relative to the current playing timer
    /// negative number meens seking backward and vice versa
    ///
    /// - if state is playing:
    ///     - seeks by the give nduration
    /// - if state is Stopping it:
    ///     - plays the preivously played song
    ///     - seeks by the given duration
    /// - if state is pausing it:
    ///     - resumes the currently playing song
    ///     - seeks by the given duration
    fn seek(&self, duration: f64) -> Result<RunStatus>;
    /// changes the volume of the player (valueb between 0 and 1)
    fn volume(&self, amount: f32) -> Result<RunStatus>;
    async fn rename_playlist(&self, old: &str, new: &str) -> Result<RunStatus>;
    async fn remove_playlist(&self, new: &str) -> Result<RunStatus>;
    async fn reload_config(&self) -> Result<()>;
    async fn create_playlist(&self, id: String) -> Result<()>;
    async fn save_to_playlist(
        &self,
        index: usize,
        source: String,
        destination: String,
    ) -> Result<RunStatus>;
    /// Gets current playlist name
    async fn get_playlist_name(&self) -> Result<String>;
    /// Gets player volume (between 0 and 1)
    fn get_volume(&self) -> Result<f32>;
    /// Gets the currently playing [Music]
    async fn playing(&self) -> Result<OuterMusic>;
    fn toggle_mute(&self) -> Result<RunStatus>;
    /// Signal fires when volume changes
    /// Decks (DJ groundwork): replies "ok" or an error message
    async fn dj_load(&self, deck: u32, path: &str) -> Result<String>;
    async fn dj_play(&self, deck: u32) -> Result<String>;
    async fn dj_pause(&self, deck: u32) -> Result<String>;
    async fn dj_seek(&self, deck: u32, seconds: f64) -> Result<String>;
    /// controls: rate, gain, fader, cue (per deck); crossfader, master, cue_mix
    async fn dj_set(&self, control: &str, deck: u32, value: f64) -> Result<String>;
    /// "internal" or "external"
    async fn dj_mode(&self, mode: &str) -> Result<String>;
    /// per deck: (position s, playing, decoded s, decoding finished)
    async fn dj_state(&self) -> Result<Vec<(f64, bool, f64, bool)>>;
    /// CDJ cue button: press sets / previews / jumps to the cue point, release ends the preview
    async fn dj_cue(&self, deck: u32, pressed: bool) -> Result<String>;
    /// match this deck's tempo and beat phase to the other deck
    async fn dj_sync(&self, deck: u32) -> Result<String>;
    /// corrects a deck's detected tempo by a factor (2, 0.5, 1.5, 0.6667; 0 resets), saved per
    /// track; replies "ok <bpm>" or an error
    async fn dj_grid_scale(&self, deck: u32, factor: f64) -> Result<String>;
    /// (bpm, first_beat_s, downbeat_offset, beats_per_bar, confidence, variable, octave_doubt,
    /// analyzed)
    async fn get_grid(&self, path: &str) -> Result<(f64, f64, u32, u32, f64, bool, bool, bool)>;
    /// temporary tempo bend, -1..1 (= -4..+4 %), 0 releases
    async fn dj_nudge(&self, deck: u32, amount: f64) -> Result<String>;
    /// jump by `beats` (negative = back), keeping the beat phase
    async fn dj_beat_jump(&self, deck: u32, beats: f64) -> Result<String>;
    /// loop of `beats` from the current beat; 0 exits the loop
    async fn dj_loop(&self, deck: u32, beats: f64) -> Result<String>;
    /// slot 0..7, action "set" | "jump" | "clear"
    async fn dj_hotcue(&self, deck: u32, slot: u32, action: &str) -> Result<String>;
    /// state of both decks, for polling
    async fn dj_decks(&self) -> Result<Vec<dj_types::DeckInfo>>;
    async fn dj_mixer(&self) -> Result<dj_types::MixerInfo>;
    /// waveform overview: u32 LE column count, u32 LE duration ms, then 6 bytes per column
    // set runtime, see "Set runtime (D-Bus)" in docs/dj-api.md; mixes and tracks count from 0
    async fn set_use(&self, name: &str) -> Result<String>;
    async fn set_arm(&self, mix: u32) -> Result<String>;
    async fn set_disarm(&self) -> Result<String>;
    async fn set_auto(&self, on: bool) -> Result<String>;
    async fn set_preview(&self, mix: u32, bars_before: f64) -> Result<String>;
    async fn set_load(&self, track: u32) -> Result<String>;
    async fn set_state(&self) -> Result<dj_types::SetState>;
    /// (name, file, error count) of every set file in the set folder
    async fn set_list(&self) -> Result<Vec<(String, String, u32)>>;
    /// `file:line:col: message` per error of a set's latest save
    async fn set_errors(&self, name: &str) -> Result<Vec<String>>;

    // set editing and sources, see "Set editing (D-Bus)" and "Set sources" in docs/dj-api.md
    /// capture the deck's position as cue `name` into the active set
    async fn set_cue(&self, deck: u32, name: &str) -> Result<String>;
    /// capture the deck's active loop as loop `name`
    async fn set_loop_capture(&self, deck: u32, name: &str) -> Result<String>;
    /// write the deck's current grid into the set as a `grid` line
    async fn set_grid_capture(&self, deck: u32) -> Result<String>;
    /// start / stop recording a mix; stopping replies `ok mix 2: aft -> smk, 16 bars, 9 items`
    async fn set_record(&self, on: bool) -> Result<String>;
    /// (recording, elapsed beats on the outgoing deck's clock, events logged)
    async fn set_recording(&self) -> Result<(bool, f64, u32)>;
    /// (text, sha-256 hex hash) of a set file as on disk
    async fn set_source(&self, name: &str) -> Result<(String, String)>;
    /// errors of `text` as if saved, empty = valid
    async fn set_check(&self, text: &str) -> Result<Vec<String>>;
    /// save after checking: `ok <new hash>` or the reason; `author` is `human` or `agent:<name>`
    async fn set_write(&self, name: &str, text: &str, base_hash: &str, author: &str) -> Result<String>;

    /// transition recipes: (name, explanation, default bars)
    async fn set_recipes(&self) -> Result<Vec<(String, String, u32)>>;
    /// a full commented `mix` block of `recipe` for the two tracks of the active set
    async fn set_recipe_text(&self, recipe: &str, from_id: &str, to_id: &str, bars: u32) -> Result<String>;
    /// warnings (not errors) of the set `name`, `file:line:col: message`
    async fn set_warnings(&self, name: &str) -> Result<Vec<String>>;

    // patch decks, see docs/patches.md in the codjd repo; slots count from 0
    /// (name, title) of every patch file
    async fn patch_list(&self) -> Result<Vec<(String, String)>>;
    /// (json, sha-256 hex hash) of a patch file
    async fn patch_source(&self, name: &str) -> Result<(String, String)>;
    /// errors of `json` as if saved (`<node>: message`), empty = valid
    async fn patch_check(&self, json: &str) -> Result<Vec<String>>;
    /// save after checking: `ok <new hash>` or the reason; empty `base_hash` creates
    async fn patch_write(&self, name: &str, json: &str, base_hash: &str, author: &str) -> Result<String>;
    /// import a NoiseCraft file: `ok <hash> (<notes>)`
    async fn patch_import(&self, ncft_path: &str, name: &str, author: &str) -> Result<String>;
    async fn patch_load(&self, slot: u32, name: &str) -> Result<String>;
    async fn patch_unload(&self, slot: u32) -> Result<String>;
    /// edit a loaded patch live: `action_json` is one op of the contract
    async fn patch_action(&self, slot: u32, action_json: &str) -> Result<String>;
    async fn patch_undo(&self, slot: u32) -> Result<String>;
    async fn patch_redo(&self, slot: u32) -> Result<String>;
    async fn patch_save(&self, slot: u32, name: &str, author: &str) -> Result<String>;
    async fn patch_live(&self, slot: u32, live: bool) -> Result<String>;
    async fn patch_state(&self) -> Result<Vec<dj_types::PatchSlot>>;

    async fn get_waveform(&self, path: &str, columns: u32) -> Result<Vec<u8>>;
    /// (xruns, second output underruns, second output skips, sample rate)
    async fn audio_health(&self) -> Result<(u64, u64, u64, u32)>;
    async fn audio_devices(&self) -> Result<Vec<String>>;
    /// playlist indices of the n tracks most like `path`, nearest first
    async fn similar_to(&self, path: &str, n: u32) -> Result<Vec<u32>>;

    #[zbus(signal)]
    async fn volume_changed(&self, amount: f32) -> zbus::Result<()>;
    /// Signal fires when player is in pausing state
    #[zbus(signal)]
    async fn paused(&self) -> zbus::Result<()>;
    /// Signal fires when playing resumed from pausing
    #[zbus(signal)]
    async fn resumed(&self) -> zbus::Result<()>;
    /// Signal fires when the player at the stop state
    #[zbus(signal)]
    async fn ended(&self) -> zbus::Result<()>;
    /// Signal fires when a music starts playing
    #[zbus(signal)]
    async fn music_played(&self) -> zbus::Result<()>;
    /// Signal fires when the sorting status changes
    #[zbus(signal)]
    async fn sorted(&self, sort: Sort) -> zbus::Result<()>;
    /// Signal fires when the repeat status changes
    #[zbus(signal)]
    async fn repeat_changed(&self, sort: Repeat) -> zbus::Result<()>;

    /// Signal fires when a finished download was added to the library
    #[zbus(signal)]
    async fn music_added(&self, path: &str) -> zbus::Result<()>;
    /// a file appeared in the music folder from outside codjd
    #[zbus(signal)]
    async fn library_changed(&self, path: &str) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn playlists_updated(
        &self,
        opperatioin: PlaylistOpperation,
        id: &str,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    #[allow(unused)]
    async fn playlist_loaded(&self, id: &str) -> zbus::Result<()>;

    #[zbus(signal)]
    #[allow(unused)]
    async fn playlist_deleted(&mut self, id: &str) -> zbus::Result<()>;

    #[zbus(signal)]
    #[allow(unused)]
    async fn playlist_created(&mut self, id: &str) -> zbus::Result<()>;

    // soulseek methods
    async fn slsk_connect(
        &self,
        username: String,
        password: String,
        download_dir: String,
        server_host: String,
        server_port: u32,
        listen_port: u32,
        max_results: u32,
    ) -> Result<String>;
    async fn slsk_disconnect(&self) -> Result<String>;
    async fn slsk_state(&self) -> Result<ConnectionState>;
    /// last connect outcome or error
    async fn slsk_status(&self) -> Result<String>;
    async fn slsk_search(&self, query: String) -> Result<String>;
    async fn slsk_get_results(&self) -> Result<Vec<RankedFile>>;
    async fn slsk_download(&self, username: String, filename: String) -> Result<String>;
    async fn slsk_download_folder(&self, username: String, filename: String) -> Result<String>;
    async fn slsk_get_downloads(&self) -> Result<Vec<Download>>;
    async fn slsk_cancel_download(&self, id: u32) -> Result<String>;
    async fn slsk_clear_finished(&self) -> Result<String>;
    async fn slsk_remove_download(&self, id: u32) -> Result<String>;
    async fn slsk_set_preferences(
        &self,
        formats: Vec<String>,
        min_bitrate: u32,
        max_size_mb: u32,
        prefer_free_slots: bool,
        max_concurrent_downloads: u32,
    ) -> Result<String>;
    async fn slsk_move_download(&self, id: u32, up: bool) -> Result<String>;
}

/// Checks that codjd answers; if not, starts it (from PATH or next to this binary) in its own
/// session so it keeps running after codj exits, and waits up to 5 s for it
async fn ensure_codjd(proxy: &ServerProxy<'_>) -> bool {
    if proxy.get_playing_index().await.is_ok() {
        return true;
    }
    let beside = env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("codjd")))
        .filter(|p| p.is_file());
    let in_path = || {
        env::var_os("PATH").and_then(|paths| {
            env::split_paths(&paths)
                .map(|dir| dir.join("codjd"))
                .find(|p| p.is_file())
        })
    };
    // `setsid -f` reports success even when the program is missing, so look first
    let Some(program) = beside.or_else(in_path) else {
        return false;
    };
    let log = dirs_log_file();
    let mut command = std::process::Command::new("setsid");
    command.arg("-f").arg(&program).stdin(std::process::Stdio::null());
    match log.and_then(|p| std::fs::File::create(p).ok()) {
        Some(file) => {
            let err = file.try_clone().ok();
            command.stdout(file);
            command.stderr(err.map_or(std::process::Stdio::null(), Into::into));
        }
        None => {
            command.stdout(std::process::Stdio::null());
            command.stderr(std::process::Stdio::null());
        }
    }
    if command.status().map_or(true, |s| !s.success()) {
        return false;
    }
    eprintln!("starting codjd...");
    for _ in 0..50 {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        if proxy.get_playing_index().await.is_ok() {
            return true;
        }
    }
    false
}

/// ~/.cache/codjd/codjd.log, for a codjd started by codj
fn dirs_log_file() -> Option<std::path::PathBuf> {
    let dir = env::var_os("XDG_CACHE_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".cache")))?
        .join("codjd");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir.join("codjd.log"))
}

/// How many keyboard-enhancement levels this process has pushed, so the panic hook and exit can
/// pop exactly that many (DJ mode pushes a second one for key-release reporting)
static KBD_DEPTH: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

pub fn kbd_push(flags: KeyboardEnhancementFlags) {
    if stdout().execute(PushKeyboardEnhancementFlags(flags)).is_ok() {
        KBD_DEPTH.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
}

pub fn kbd_pop() {
    if KBD_DEPTH.load(std::sync::atomic::Ordering::SeqCst) > 0
        && stdout().execute(PopKeyboardEnhancementFlags).is_ok()
    {
        KBD_DEPTH.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}

fn kbd_pop_all() {
    while KBD_DEPTH.load(std::sync::atomic::Ordering::SeqCst) > 0 {
        let before = KBD_DEPTH.load(std::sync::atomic::Ordering::SeqCst);
        kbd_pop();
        if KBD_DEPTH.load(std::sync::atomic::Ordering::SeqCst) == before {
            break;
        }
    }
}

pub fn init_panic_hook() {
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        // intentionally ignore errors here since we're already in a panic
        kbd_pop_all();
        let _ = stdout().execute(DisableMouseCapture);
        disable_raw_mode().unwrap();
        stdout().execute(LeaveAlternateScreen).unwrap();
        original_hook(panic_info);
    }));
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();
    if args.len() > 1 && bar::is_subcommand(&args[1]) {
        std::process::exit(bar::run(&args[1..]).await);
    }
    // MCP server on stdio: no terminal setup, stdout carries only protocol
    if args.len() > 1 && args[1] == "mcp" {
        std::process::exit(mcp::run(&args[2..]).await);
    }
    let connection = Connection::session().await.unwrap_or_else(|_| {
        panic!("Could not connect to the bus address, aborting...");
    });
    let proxy = ServerProxy::new(&connection).await?;
    if !ensure_codjd(&proxy).await {
        eprintln!(
            "codjd is not running and couldn't be started.\n\
             Start it with `codjd` (or `cargo run` in the codjd repo), or put codjd in your PATH \
             so codj can start it."
        );
        std::process::exit(1);
    }

    init_panic_hook();
    enable_raw_mode()?;
    stdout().execute(EnterAlternateScreen)?;
    stdout().execute(EnableMouseCapture)?;
    // lets terminals that speak the kitty protocol report Ctrl+Enter apart from Enter
    let enhanced = supports_keyboard_enhancement().unwrap_or(false);
    if enhanced {
        kbd_push(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES);
    }
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;

    let config_path = if args.len() > 1 {
        args[1].clone()
    } else {
        // ~/.config/codj, or a config from before a rename (musa, mplayer-client)
        let home = env::var("HOME").unwrap().to_string();
        let new = format!("{home}/.config/codj/config.toml");
        [
            format!("{home}/.config/musa/config.toml"),
            format!("{home}/.config/mplayer-client/config.toml"),
        ]
        .into_iter()
        .find(|old| !std::path::Path::new(&new).exists() && std::path::Path::new(old).exists())
        .unwrap_or(new)
    };
    let config = if args.len() > 1 {
        Config::parse_config(&config_path).unwrap()
    } else {
        Config::parse_config(&config_path).unwrap_or_default()
    };

    let mut ui = ui::UI::new(proxy);
    ui.keyboard_enhanced = enhanced;
    ui.config_path = Some(config_path.into());

    ui.update_from_config(&config);
    ui.restore_state().await;
    ui.set_frame(&mut terminal.get_frame());
    let mut should_quit = false;
    // soulseek results trickle in from peers, twice a second is plenty
    let soulseek_poll = std::time::Duration::from_millis(500);
    let mut last_soulseek_poll: Option<std::time::Instant> = None;
    while !should_quit {
        terminal.draw(|frame| {
            ui.render(frame);
        })?;
        let frame = if ui.dj_active() {
            // live mode: the deck state comes from its own 30 Hz poller; no library or Soulseek
            // polling while performing (the signal streams are only drained, which is free)
            ui.check_state_streams().await;
            std::time::Duration::from_millis(33)
        } else {
            ui.update_state().await;
            if last_soulseek_poll.map_or(true, |t| t.elapsed() >= soulseek_poll) {
                ui.poll_soulseek_results().await;
                ui.poll_queue().await;
                ui.poll_analysis().await;
                last_soulseek_poll = Some(std::time::Instant::now());
            }
            ui.fps_controller.frame_duration()
        };
        should_quit = handle_events(&mut ui, frame).await?;
        if let Some(path) = ui.dj_take_edit() {
            let text = edit_in_terminal(&mut ui, &mut terminal, &path);
            ui.dj_edited(text);
        }
        ui.dj_tick();
    }

    ui.leave_dj();
    kbd_pop_all();
    stdout().execute(DisableMouseCapture)?;
    disable_raw_mode()?;
    stdout().execute(LeaveAlternateScreen)?;
    Ok(())
}

/// `:edit`: gives the terminal to `$EDITOR` and takes it back. Leaves raw mode, mouse capture and
/// the alternate screen and pops every keyboard-enhancement level, runs the editor on the file
/// (codjd reloads a set when it is saved), then restores all of it, including the levels DJ mode
/// had pushed, and forces a full redraw. The restore runs whatever the editor did.
fn edit_in_terminal<B: ratatui::backend::Backend>(
    ui: &mut ui::UI<'_>,
    terminal: &mut Terminal<B>,
    path: &std::path::Path,
) -> String {
    let levels = ui.kbd_levels();
    kbd_pop_all();
    let _ = stdout().execute(DisableMouseCapture);
    let _ = disable_raw_mode();
    let _ = stdout().execute(LeaveAlternateScreen);

    let line = dj::editor_line(env::var("EDITOR").ok().as_deref(), env::var("VISUAL").ok().as_deref());
    let result = std::process::Command::new("sh").arg("-c").arg(&line).arg(path).status();

    let _ = enable_raw_mode();
    let _ = stdout().execute(EnterAlternateScreen);
    let _ = stdout().execute(EnableMouseCapture);
    for flags in levels {
        kbd_push(flags);
    }
    let _ = terminal.hide_cursor();
    let _ = terminal.clear();
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    match result {
        Ok(status) if status.success() => format!("edited {name}: codjd reloads it on save (:errors shows problems)"),
        Ok(status) => format!("editor exited with {status}"),
        Err(e) => format!("could not run the editor: {e}"),
    }
}

/// Waits up to one frame for input, then handles every queued key before the next draw. Handling
/// only one key per frame let held keys (key repeat) pile up and keep scrolling after release.
async fn handle_events<'a>(ui: &mut ui::UI<'a>, frame: std::time::Duration) -> io::Result<bool> {
    let mut wait = frame;
    while event::poll(wait)? {
        match event::read()? {
            Event::Key(key) if ui.dj_active() => ui.dj_key(&key),
            // the normal view's geometry is stale behind the DJ screen: a click must not seek or
            // play anything. Only the wheel is used: it edits the selected control.
            Event::Mouse(m) if ui.dj_active() => match m.kind {
                event::MouseEventKind::ScrollUp => ui.dj_wheel(true),
                event::MouseEventKind::ScrollDown => ui.dj_wheel(false),
                _ => {}
            },
            Event::Key(key) => {
                if ui::Region::handle_events(ui, &key).await? {
                    return Ok(true);
                }
            }
            Event::Mouse(mouse) => ui.handle_mouse(&mouse).await,
            _ => {}
        }
        wait = std::time::Duration::ZERO;
    }
    Ok(false)
}
