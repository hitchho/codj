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
    interface = "net.rotko.Musa.Player",
    default_service = "net.rotko.Musa",
    default_path = "/net/rotko/Musa"
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
    async fn slsk_search(&self, query: String) -> Result<String>;
    async fn slsk_get_results(&self) -> Result<Vec<RankedFile>>;
    async fn slsk_download(&self, username: String, filename: String) -> Result<String>;
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

/// Checks that musad answers; if not, starts it (from PATH or next to this binary) in its own
/// session so it keeps running after musa exits, and waits up to 5 s for it
async fn ensure_musad(proxy: &ServerProxy<'_>) -> bool {
    if proxy.get_playing_index().await.is_ok() {
        return true;
    }
    let beside = env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("musad")))
        .filter(|p| p.is_file());
    let in_path = || {
        env::var_os("PATH").and_then(|paths| {
            env::split_paths(&paths)
                .map(|dir| dir.join("musad"))
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
    eprintln!("starting musad...");
    for _ in 0..50 {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        if proxy.get_playing_index().await.is_ok() {
            return true;
        }
    }
    false
}

/// ~/.cache/musad/musad.log, for a musad started by musa
fn dirs_log_file() -> Option<std::path::PathBuf> {
    let dir = env::var_os("XDG_CACHE_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".cache")))?
        .join("musad");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir.join("musad.log"))
}

pub fn init_panic_hook() {
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        // intentionally ignore errors here since we're already in a panic
        let _ = stdout().execute(PopKeyboardEnhancementFlags);
        let _ = stdout().execute(DisableMouseCapture);
        disable_raw_mode().unwrap();
        stdout().execute(LeaveAlternateScreen).unwrap();
        original_hook(panic_info);
    }));
}

#[tokio::main]
async fn main() -> Result<()> {
    let connection = Connection::session().await.unwrap_or_else(|_| {
        panic!("Could not connect to the bus address, aborting...");
    });
    let proxy = ServerProxy::new(&connection).await?;
    if !ensure_musad(&proxy).await {
        eprintln!(
            "musad is not running and couldn't be started.\n\
             Start it with `musad` (or `cargo run` in the musad repo), or put musad in your PATH \
             so musa can start it."
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
        stdout().execute(PushKeyboardEnhancementFlags(
            KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES,
        ))?;
    }
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;

    let args: Vec<String> = env::args().collect();
    let config_path = if args.len() > 1 {
        args[1].clone()
    } else {
        // ~/.config/musa, or the config from before the rename
        let home = env::var("HOME").unwrap().to_string();
        let new = format!("{home}/.config/musa/config.toml");
        let old = format!("{home}/.config/mplayer-client/config.toml");
        if !std::path::Path::new(&new).exists() && std::path::Path::new(&old).exists() {
            old
        } else {
            new
        }
    };
    let config = if args.len() > 1 {
        Config::parse_config(&config_path).unwrap()
    } else {
        Config::parse_config(&config_path).unwrap_or_default()
    };

    let mut ui = ui::UI::new(proxy);
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
        ui.update_state().await;
        if last_soulseek_poll.map_or(true, |t| t.elapsed() >= soulseek_poll) {
            ui.poll_soulseek_results().await;
            ui.poll_queue().await;
            ui.poll_analysis().await;
            last_soulseek_poll = Some(std::time::Instant::now());
        }
        let frame = ui.fps_controller.frame_duration();
        should_quit = handle_events(&mut ui, frame).await?;
    }

    if enhanced {
        stdout().execute(PopKeyboardEnhancementFlags)?;
    }
    stdout().execute(DisableMouseCapture)?;
    disable_raw_mode()?;
    stdout().execute(LeaveAlternateScreen)?;
    Ok(())
}

/// Waits up to one frame for input, then handles every queued key before the next draw. Handling
/// only one key per frame let held keys (key repeat) pile up and keep scrolling after release.
async fn handle_events<'a>(ui: &mut ui::UI<'a>, frame: std::time::Duration) -> io::Result<bool> {
    let mut wait = frame;
    while event::poll(wait)? {
        match event::read()? {
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
