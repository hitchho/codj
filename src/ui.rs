// TODO: rename actions to pannel
// TODO: add actions box for searching and commands e.g:
// TODO: make a keybind to create a playlist
// TODO: make a keybind to add a music to a playlist
// TODO: make search golobal command, when canceling, you go back to original region when search is
// done (this also allows for the after search effect, you temporary get transported to the list
// region)
// TODO: implement :delete_from_this_playlist and :delete_from_playlist [playlist_name] [name or index of music]
// TODO: implement :add_from_this_playlist [destination] and :add_from_playlist [source] [name or
// index] [destination]
use crate::parser::Scripts;
use crate::style::UIStyle;
use crate::types::Music;
use crate::{Repeat, Sort};
use crossterm::event::Event as CrosstermEvent;
use crossterm::event::{
    self, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use futures::StreamExt;
use mlua::prelude::*;
use mlua::Lua;
use mlua::{UserData, UserDataMethods};
use ratatui::{prelude::*, style::Stylize, widgets::*};
use serde::{Deserialize, Serialize};
use std::io;
use std::str::FromStr;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};
use tokio::time::Interval;
use tui_input::{Input, InputRequest, StateChanged};

use crate::allowed_commands::{CommandContext, CommandName};
use crate::soulseek_types::{ConnectionState, Download, DownloadState, RankedFile};
use crate::states::State;
use crate::utils::{log, FPSController};
use crate::{Config, Lyrics, PlayingStatus, ServerProxy};

pub struct ViewConfiguration {
    lyrics: bool,
    search: bool,
    command: bool,
    playlists: bool,
}

/// ⏮ ⏸ ⏭ ⏹, three columns each
const BAR_CONTROLS_WIDTH: u16 = 12;
/// "vol 100%"
const VOLUME_LABEL_WIDTH: u16 = 8;

pub struct ViewGeometry {
    frame: Rect,
    list: Rect,
    lyrics: Option<Rect>,
    playlists: Option<Rect>,
    actions: Rect,
    volume: Rect,
    slider: Rect,
    bar: Rect,
    elapsed: Rect,
    total: Rect,
    search: Option<Rect>,
    command: Option<Rect>,
    configuration: ViewConfiguration,
}

impl ViewGeometry {
    pub fn new(
        frame: Rect,
        list: Rect,
        lyrics: Option<Rect>,
        playlists: Option<Rect>,
        search: Option<Rect>,
        command: Option<Rect>,
        actions: Rect,
        slider: Rect,
        volume: Rect,
        configuration: ViewConfiguration,
    ) -> Self {
        Self {
            frame,
            list,
            lyrics,
            playlists,
            actions,
            volume,
            slider,
            bar: frame,
            elapsed: frame,
            total: frame,
            search,
            command,
            configuration,
        }
    }

    fn default(frame: Rect) -> Self {
        let mut init = Self::new(
            frame,
            frame,
            None,
            None,
            None,
            None,
            frame,
            frame,
            frame,
            ViewConfiguration {
                search: false,
                lyrics: false,
                playlists: false,
                command: false,
            },
        );
        init.calculate_gemoetry();
        init
    }

    /// Splits the screen: the list on top, the search/command input under it, lyrics or
    /// playlists below that, and the 3-row now-playing bar at the bottom
    pub fn calculate_gemoetry(&mut self) {
        let [main, bar] =
            Layout::vertical([Constraint::Min(0), Constraint::Length(3)]).areas(self.frame);
        let panel_open = self.configuration.lyrics || self.configuration.playlists;
        let (top, panel) = if panel_open {
            let [top, panel] =
                Layout::vertical([Constraint::Percentage(50), Constraint::Percentage(50)])
                    .areas(main);
            (top, Some(panel))
        } else {
            (main, None)
        };
        let input_open = self.configuration.search || self.configuration.command;
        let (list, input) = if input_open {
            let [list, input] =
                Layout::vertical([Constraint::Min(0), Constraint::Length(3)]).areas(top);
            (list, Some(input))
        } else {
            (top, None)
        };
        self.list = list;
        self.lyrics = panel.filter(|_| self.configuration.lyrics);
        self.playlists = panel.filter(|_| self.configuration.playlists);
        self.search = input.filter(|_| self.configuration.search);
        self.command = input.filter(|_| self.configuration.command);

        // inside the bar: controls, elapsed, seek bar, total, volume
        self.bar = bar;
        let inner = Block::bordered().inner(bar);
        let [controls, _, elapsed, slider, total, _, volume] = Layout::horizontal([
            Constraint::Length(BAR_CONTROLS_WIDTH),
            Constraint::Length(1),
            Constraint::Length(6),
            Constraint::Min(4),
            Constraint::Length(6),
            Constraint::Length(2),
            Constraint::Length(16),
        ])
        .areas(inner);
        self.actions = controls;
        self.elapsed = elapsed;
        self.slider = slider;
        self.total = total;
        self.volume = volume;
    }

    pub fn bar_geo(&self) -> Rect {
        self.bar
    }

    pub fn set_frame(&mut self, frame: Rect) {
        self.frame = frame;
    }

    pub fn list_geo(&self) -> Rect {
        self.list
    }

    pub fn lyrics_geo(&self) -> Option<Rect> {
        self.lyrics
    }

    pub fn playlists_geo(&self) -> Option<Rect> {
        self.playlists
    }

    pub fn actions_geo(&self) -> Rect {
        self.actions
    }

    pub fn volume_geo(&self) -> Rect {
        self.volume
    }

    pub fn slider_geo(&self) -> Rect {
        self.slider
    }

    pub fn search_geo(&self) -> Option<Rect> {
        self.search
    }

    pub fn command_geo(&self) -> Option<Rect> {
        self.command
    }

    pub fn enable_search(&mut self) {
        self.configuration.search = true;
    }

    pub fn enable_command(&mut self) {
        self.configuration.command = true;
    }

    pub fn disable_search(&mut self) {
        self.configuration.search = false;
    }

    pub fn disable_command(&mut self) {
        self.configuration.command = false;
    }

    #[allow(unused)]
    pub fn disable_toggled(&mut self) {
        self.configuration.lyrics = false;
        self.configuration.playlists = false;
    }

    pub fn toggle_lyrics(&mut self) {
        self.configuration.lyrics = !self.configuration.lyrics;
    }

    pub fn disable_lyrics(&mut self) {
        self.configuration.lyrics = false;
    }

    #[allow(unused)]
    pub fn enable_lyrics(&mut self) {
        self.configuration.lyrics = true;
    }

    pub fn toggle_playlists(&mut self) {
        self.configuration.playlists = !self.configuration.playlists;
    }

    pub fn disable_playlists(&mut self) {
        self.configuration.playlists = false;
    }

    #[allow(unused)]
    pub fn enable_playlists(&mut self) {
        self.configuration.playlists = true;
    }
}

#[derive(Default, Debug, Deserialize, Serialize)]
pub enum ListMode {
    Command,
    Search,
    #[default]
    Select,
    AfterSearch,
}

#[derive(Default)]
/// Represents a two chars combo in a keybind
pub enum AncitipationMode {
    #[default]
    /// no anticipation
    Normal,
    /// contains the anticipated next character
    Char(char),
}

pub struct UI<'a> {
    /// list of all the musics to play (*dsiplayed* musics)
    pub music_list: Musics,
    /// bar indecate the playing timer
    pub power_bar: PowerBar,
    /// currently selected action
    pub action: PowerActions,
    /// currently selected region
    pub region: Region,
    /// UI style
    pub style: UIStyle,
    /// slection mod <Search, AfterSearch, Select>
    pub mode: ListMode,
    /// search buffer, used to search through the musics list
    search_bufr: Input,
    /// command buffer, used to input commands
    command_bufr: Input,
    /// `Ctrl-w` was pressed: the next j/k/h/l moves between panels
    pending_window: bool,
    /// vim count typed before a motion (`5j`)
    count: Option<usize>,
    /// the last `/` search, for `n` / `N`
    last_search: Option<String>,
    /// Tab completion of `:` commands: (what was typed, index of the shown match)
    completion: Option<(String, usize)>,
    /// error from the last submitted command, shown in the command box title
    command_error: Option<String>,
    /// helps reading a combination of keys like `gg`
    pub anticipation_mode: AncitipationMode,
    /// what to repeat <ThisMusic, AllMusics, None>
    displayed_repeat: Repeat,
    /// playlist order
    displayed_sort: Sort,
    /// used to fetch playing state from server
    pub state: State<'a>,
    // TODO: make this private
    pub fps_controller: FPSController,
    /// Displayed lyrics
    displayed_lyrics: DisplayedLyrics,
    /// Displayed Playlists
    displayed_playlists: DisplayedPlaylists,
    /// Views/widgets geometry
    pub geometry: ViewGeometry,
    /// Used for event checking
    pub internal_clock: Interval,
    /// `AutomixState`, polled with the clock: (on, next title, seconds to the transition, what)
    automix: (bool, String, f64, String),
    /// Lua scripts
    pub scripts: Option<Functions>,
    /// Lua interpreter
    pub lua: Lua,
    /// Ranked search results from server (sorted by quality)
    pub soulseek_ranked: Vec<RankedFile>,
    /// soulseek_ranked narrowed by the name filter; the table, cursor and downloads use this
    soulseek_shown: Vec<RankedFile>,
    /// regex on result names, typed after `f` in the results
    soulseek_filter: Input,
    /// the name filter is being typed
    soulseek_filtering: bool,
    /// first `d` of a `dd` in the soulseek panel
    soulseek_pending_d: bool,
    playlists_pending_g: bool,
    /// config file the settings panel writes to
    pub config_path: Option<std::path::PathBuf>,
    /// the settings panel, when open
    settings_view: Option<SettingsView>,
    /// play queue as last fetched from the server
    queue: Vec<crate::types::OuterMusic>,
    /// the queue panel, when open
    queue_view: Option<QueueView>,
    /// (analyzed, queued) of the server's audio analysis
    analysis_progress: (u32, u32),
    /// path, bpm and loudness of the highlighted track, once analyzed
    selected_features: Option<(String, f64, f64)>,
    /// short-lived message in the music list title
    list_note: Option<(String, Instant)>,
    /// search to run once the soulseek connect finishes
    soulseek_pending_query: Option<String>,
    /// where the repeat and sort labels were drawn in the bar, for clicks
    bar_repeat: Rect,
    bar_sort: Rect,
    /// last list click (row index, time), a second click on the same row plays it
    last_click: Option<(usize, Instant)>,
    /// Soulseek status message
    pub soulseek_status: String,
    /// Active downloads for display
    pub soulseek_download_list: Vec<Download>,
    /// Selected index in soulseek results
    pub soulseek_selected: usize,
    /// the user moved the results cursor since the last search; until then it stays on the top
    /// row while results stream in instead of following whichever result arrived first
    soulseek_selection_moved: bool,
    /// Selected index in the soulseek downloads list
    pub soulseek_download_selected: usize,
    /// Which soulseek list currently has focus
    pub soulseek_focus: SoulseekFocus,
    /// Table state of the results table (keeps scroll offset)
    pub soulseek_results_state: TableState,
    /// selected result whose name is scrolling, and when it was selected
    soulseek_marquee: (Option<(String, String)>, std::time::Instant),
    /// Table state of the downloads table (keeps scroll offset)
    pub soulseek_downloads_state: TableState,
    /// A search was started from this session, open the panel when results arrive
    pub soulseek_open_on_results: bool,
    /// The command input was opened from the soulseek region, go back there afterwards
    pub command_from_soulseek: bool,
    /// `:playlists` opened the panel: focus it once the command line closes
    pub focus_playlists: bool,
    /// Whether showing soulseek results panel
    pub soulseek_visible: bool,
    /// the `?` key list of the normal view
    help_open: bool,
    /// the last key or mouse event: codj only swaps to a newer build after a quiet spell
    pub last_input: std::time::Instant,
    /// list position handed over by the codj this one replaced, applied once the list is in
    pub pending_select: Option<usize>,
    /// Soulseek connection state
    pub soulseek_connected: bool,
    /// Soulseek config from config file (for connect params)
    pub soulseek_config: Option<crate::parser::SoulseekSettings>,
    /// `[agent]` from the config file, for the DJ screen's prompt box
    pub agent_config: crate::parser::AgentSettings,
    /// `[knob]` from the config
    pub knob_config: crate::parser::KnobSettings,
    /// the hardware knob while the normal view has it: volume and mute; DJ mode borrows it
    knob: Option<crate::knob::Knob>,
    /// what the knob reader had to say while the normal view had it, shown in DJ mode
    knob_note: Option<String>,
    /// the terminal speaks the kitty keyboard protocol (set by main)
    pub keyboard_enhanced: bool,
    /// live DJ mode, when on; it replaces the whole normal view
    dj: Option<Box<crate::dj::DjScreen>>,
    /// DJ mode pushed the extra keyboard flags that report key releases
    dj_kbd_pushed: bool,
    /// while the DJ search box or command line is open, release reporting is switched off again
    /// so typed text arrives as text
    dj_text_pushed: bool,
}

#[allow(unused)]
pub struct Functions {
    pub all: Option<mlua::Function>,
    pub list: Option<mlua::Function>,
    pub lyrics: Option<mlua::Function>,
    pub actions: Option<mlua::Function>,
    pub seeker: Option<mlua::Function>,
    pub volume: Option<mlua::Function>,
}

impl Functions {
    fn new(
        all: Option<mlua::Function>,
        list: Option<mlua::Function>,
        lyrics: Option<mlua::Function>,
        actions: Option<mlua::Function>,
        seeker: Option<mlua::Function>,
        volume: Option<mlua::Function>,
    ) -> Self {
        Self {
            all,
            list,
            lyrics,
            actions,
            seeker,
            volume,
        }
    }
}

/// Focused list inside the soulseek panel
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub enum SoulseekFocus {
    #[default]
    Results,
    Downloads,
}

/// Index of the previously selected item in a refreshed list, so that the selection stays on the
/// same item when the list is re-sorted. Falls back to the old index if the item is gone, and the
/// result is always in range (0 for an empty list).
fn anchored_index<T>(
    old: &[T],
    new: &[T],
    selected: usize,
    same: impl Fn(&T, &T) -> bool,
) -> usize {
    let found = old
        .get(selected)
        .and_then(|prev| new.iter().position(|n| same(prev, n)));
    found.unwrap_or(selected).min(new.len().saturating_sub(1))
}

/// Window of `text` that fits in `width` columns, scrolled by time so a long name can be read in
/// full: holds at the start, scrolls to the end, holds there, then starts over.
fn marquee(text: &str, width: usize, elapsed_secs: f64) -> String {
    const HOLD: f64 = 1.0;
    const CHARS_PER_SEC: f64 = 6.0;
    let chars: Vec<char> = text.chars().collect();
    if width == 0 || chars.len() <= width {
        return text.to_string();
    }
    let max_offset = chars.len() - width;
    let scroll = max_offset as f64 / CHARS_PER_SEC;
    let t = elapsed_secs % (HOLD + scroll + HOLD);
    let offset = if t < HOLD {
        0
    } else if t < HOLD + scroll {
        (((t - HOLD) * CHARS_PER_SEC) as usize).min(max_offset)
    } else {
        max_offset
    };
    chars[offset..offset + width].iter().collect()
}

/// State of the open queue panel
#[derive(Default)]
struct QueueView {
    selected: usize,
    /// first `d` of a `dd`
    pending_d: bool,
}

/// State of the open settings panel
struct SettingsView {
    /// the `[soulseek]` settings being edited
    data: crate::parser::SoulseekSettings,
    selected: usize,
    /// edit box for the selected value
    editing: Option<Input>,
    /// result of the last change
    message: String,
}

/// Results whose file name matches the filter regex (all of them for an empty filter)
fn filter_results(results: &[RankedFile], filter: &str) -> Vec<RankedFile> {
    let filter = filter.trim();
    if filter.is_empty() {
        return results.to_vec();
    }
    let re = crate::search::build_regex(filter);
    results
        .iter()
        .filter(|rf| re.is_match(rf.file.basename()))
        .cloned()
        .collect()
}

fn same_result(a: &RankedFile, b: &RankedFile) -> bool {
    a.username == b.username && a.file.filename == b.file.filename
}

/// all the displayed region
#[derive(Clone, Deserialize, Serialize, Default)]
pub enum Region {
    #[default]
    List,
    Action,
    Seeker,
    Volume,
    Lyrics,
    Playlist,
    Search,
    Command,
    Soulseek,
}

/// Ctrl+Enter searches soulseek even when the library has matches. Terminals without the kitty
/// keyboard protocol send Ctrl+Enter as plain Enter, so Ctrl+J and Alt+Enter do the same.
fn is_soulseek_enter(key: &KeyEvent) -> bool {
    match key.code {
        KeyCode::Enter => key.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT),
        KeyCode::Char('j') => key.modifiers == KeyModifiers::CONTROL,
        _ => false,
    }
}

impl ListMode {
    pub async fn handle_search<'a>(ui: &mut UI<'a>, key: &KeyEvent) {
        if key.kind == event::KeyEventKind::Press {
            ui.run_list_script(key, ListMode::Search);
            if is_soulseek_enter(key) {
                let query = ui.search_bufr.value().trim().to_string();
                if !query.is_empty() {
                    ui.leave_search();
                    ui.soulseek_search(&query).await;
                }
                return;
            }
            if key.modifiers == KeyModifiers::NONE {
                match key.code {
                    KeyCode::Enter => {
                        let query = ui.search_bufr.value().trim().to_string();
                        if ui.music_list.musics.is_empty() && !query.is_empty() {
                            // nothing in the library, look for it on soulseek instead
                            ui.leave_search();
                            ui.soulseek_search(&query).await;
                            return;
                        }
                        // like vim's /: Enter puts the cursor on the match, n / N go on from there;
                        // a second Enter plays it
                        if !query.is_empty() {
                            ui.last_search = Some(query);
                        }
                        ui.leave_search();
                        return;
                    }
                    KeyCode::Esc => {
                        ui.leave_search();
                        return;
                    }
                    // j/k are typed into the query, arrows move through the matches
                    KeyCode::Down => {
                        ui.list_down();
                        return;
                    }
                    KeyCode::Up => {
                        ui.list_up();
                        return;
                    }
                    _ => {}
                }
            }
            ui.register_search_query(key.to_owned()).await;
        }
    }

    pub async fn handle_after_search<'a>(ui: &mut UI<'a>, key: &KeyEvent) -> std::io::Result<bool> {
        if key.kind == event::KeyEventKind::Press {
            ui.run_list_script(key, ListMode::AfterSearch);
            if is_soulseek_enter(key) {
                let query = ui.search_bufr.value().trim().to_string();
                if !query.is_empty() {
                    ui.leave_search();
                    ui.soulseek_search(&query).await;
                }
                return Ok(false);
            }
            match key.modifiers {
                KeyModifiers::NONE => match key.code {
                    KeyCode::Char(c) => match c {
                        'j' => ui.list_down(),
                        'k' => ui.list_up(),
                        '/' | 'f' => ui.change_list_mode(ListMode::Search),
                        'q' => ui.change_list_mode(ListMode::Select),
                        _ => {}
                    },
                    KeyCode::Enter => ui.play_after_search().await,
                    KeyCode::Esc => ui.change_list_mode(ListMode::Select),
                    _ => {}
                },
                KeyModifiers::ALT => {
                    if let KeyCode::Char(c) = key.code {
                        if c == '/' {
                            ui.change_list_mode(ListMode::Search)
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(false)
    }

    pub async fn handle_select<'a>(ui: &mut UI<'a>, key: &KeyEvent) -> std::io::Result<bool> {
        if key.kind == event::KeyEventKind::Press {
            ui.run_list_script(key, ListMode::Select);
            match key.modifiers {
                // No modifiers
                KeyModifiers::NONE => {
                    match ui.anticipation_mode {
                        // No `g` key pressed beforehand
                        AncitipationMode::Normal => match key.code {
                            KeyCode::Enter => ui.play_selected_music().await,
                            // a count: digits before a motion (`0` alone is not one)
                            KeyCode::Char(d @ '0'..='9') if d != '0' || ui.count.is_some() => {
                                let n = ui.count.unwrap_or(0) * 10 + d.to_digit(10).unwrap() as usize;
                                ui.count = Some(n.min(9999));
                                return Ok(false);
                            }
                            KeyCode::Char(c) => match c {
                                'j' => {
                                    for _ in 0..ui.count.take().unwrap_or(1) {
                                        ui.list_down_clamped();
                                    }
                                }
                                'k' => {
                                    for _ in 0..ui.count.take().unwrap_or(1) {
                                        ui.list_up_clamped();
                                    }
                                }
                                '/' | 'f' => {
                                    ui.select_region(Region::Search);
                                    ui.change_list_mode(ListMode::Search)
                                }
                                ':' => {
                                    ui.command_from_soulseek = false;
                                    ui.change_list_mode(ListMode::Command);
                                    ui.select_region(Region::Command);
                                }
                                's' => ui.goto_playing(),
                                'n' => ui.search_next(true),
                                'g' => ui.anticipate('g'),
                                'q' => ui.queue_selected(false).await,
                                'm' => ui.queue_similar().await,
                                'h' => {
                                    for _ in 0..ui.count.take().unwrap_or(1).min(60) {
                                        ui.previous_5s().await;
                                    }
                                }
                                'l' => {
                                    for _ in 0..ui.count.take().unwrap_or(1).min(60) {
                                        ui.next_5s().await;
                                    }
                                }
                                // some terminals send shifted letters without the SHIFT flag
                                c if c.is_ascii_uppercase() => {
                                    if ui.list_upper_key(c).await {
                                        return ui.quit();
                                    }
                                }
                                _ => {}
                            },
                            _ => {}
                        },
                        AncitipationMode::Char(c) => {
                            if c == 'g' {
                                if let KeyCode::Char(c) = key.code {
                                    if c == 'g' {
                                        ui.goto_top()
                                    }
                                }
                            }
                            ui.anticipation_mode = AncitipationMode::Normal;
                        }
                    }
                }
                KeyModifiers::SHIFT => match key.code {
                    // like M: DJ mode (terminals without the kitty keyboard protocol send plain Enter)
                    KeyCode::Enter => ui.enter_dj().await,
                    KeyCode::Char(c) => {
                        if ui.list_upper_key(c).await {
                            return ui.quit();
                        }
                    }
                    _ => {}
                },
                KeyModifiers::CONTROL => {
                    if let KeyCode::Char(c) = key.code {
                        match c {
                            'd' => ui.scroll_list_down(),
                            'u' => ui.scroll_list_up(),
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
        // a count only applies to the next motion
        if key.kind == event::KeyEventKind::Press {
            ui.count = None;
        }
        Ok(false)
    }

    async fn handle_command(ui: &mut UI<'_>, key: &KeyEvent) -> std::io::Result<bool> {
        if key.kind == event::KeyEventKind::Press {
            ui.run_list_script(key, ListMode::Search);
            // Tab / Shift-Tab complete the command name, like vim's wildmenu
            if matches!(key.code, KeyCode::Tab | KeyCode::BackTab) {
                let typed = ui.command_bufr.value().to_string();
                // command names, and `pl` with its subcommands
                if ui.completion.is_some() || !complete_command(&typed).is_empty() {
                    let (base, i) = match ui.completion.take() {
                        Some((base, i)) if key.code == KeyCode::BackTab => (base, i.wrapping_sub(1)),
                        Some((base, i)) => (base, i + 1),
                        None => (typed.clone(), 0),
                    };
                    let matches = complete_command(&base);
                    if !matches.is_empty() {
                        let i = i % matches.len();
                        ui.command_bufr = Input::new(matches[i].to_string());
                        ui.completion = Some((base, i));
                        ui.command_error = (matches.len() > 1).then(|| matches.join("  "));
                    }
                }
                return Ok(false);
            }
            ui.completion = None;
            if key.modifiers == KeyModifiers::NONE {
                match key.code {
                    KeyCode::Enter => {
                        match ui.run_command().await {
                            Ok(true) => return Ok(true),
                            Ok(false) => {}
                            Err(e) => {
                                // keep the input open so the command can be fixed
                                ui.command_error = Some(e.to_string());
                                return Ok(false);
                            }
                        }
                        ui.clear_command();
                        ui.change_list_mode(ListMode::Select);
                        ui.leave_command_region();
                        return Ok(false);
                    }
                    KeyCode::Esc => {
                        ui.clear_command();
                        ui.change_list_mode(ListMode::Select);
                        ui.leave_command_region();
                        return Ok(false);
                    }
                    _ => {}
                }
            }
            ui.register_command_query(key.to_owned()).await;
        }
        Ok(false)
    }
}

impl Region {
    pub async fn handle_events<'a>(ui: &mut UI<'a>, key: &KeyEvent) -> io::Result<bool> {
        // the key list closes on any key, which does nothing else
        if ui.help_open {
            if key.kind == KeyEventKind::Press {
                ui.help_open = false;
            }
            return Ok(false);
        }
        // Ctrl-w j / k / h / l: move between panels (the same moves as Alt + j / k)
        if ui.pending_window && key.kind == KeyEventKind::Press {
            ui.pending_window = false;
            if let KeyCode::Char(c @ ('j' | 'k' | 'h' | 'l' | 'w')) = key.code {
                let c = if c == 'w' { 'j' } else { c };
                let alt = KeyEvent::new(KeyCode::Char(c), KeyModifiers::ALT);
                Region::handle_global(ui, &alt).await;
            }
            return Ok(false);
        }
        if key.code == KeyCode::Char('?') && key.kind == KeyEventKind::Press && !ui.typing() {
            ui.help_open = true;
            return Ok(false);
        }
        // the settings panel sits on top and takes every key while open
        if ui.settings_view.is_some() {
            ui.handle_settings_key(key).await;
            return Ok(false);
        }
        // Space pauses/continues everywhere, except while typing
        if key.code == KeyCode::Char(' ')
            && key.modifiers == KeyModifiers::NONE
            && key.kind == KeyEventKind::Press
            && !ui.typing()
        {
            ui.toggle_play().await;
            return Ok(false);
        }
        if ui.queue_view.is_some() {
            Region::handle_global(ui, key).await;
            ui.handle_queue_key(key).await;
            return Ok(false);
        }
        Region::handle_global(ui, key).await;
        // keybind depend on reagion
        match ui.region {
            Region::List => {
                if Region::handle_list(ui, key)
                    .await
                    .is_ok_and(|should_quit| should_quit)
                {
                    return Ok(true);
                }
            }
            Region::Seeker => {
                if Region::handle_seeker(ui, key)
                    .await
                    .is_ok_and(|should_quit| should_quit)
                {
                    return Ok(true);
                }
            }
            Region::Volume => {
                if Region::handle_volume(ui, key)
                    .await
                    .is_ok_and(|should_quit| should_quit)
                {
                    return Ok(true);
                }
            }
            Region::Action => {
                if Region::handle_action(ui, key)
                    .await
                    .is_ok_and(|should_quit| should_quit)
                {
                    return Ok(true);
                }
            }
            Region::Lyrics => {
                if Region::handle_lyrics(ui, key)
                    .await
                    .is_ok_and(|should_quit| should_quit)
                {
                    return Ok(true);
                }
            }
            Region::Search => {
                if Region::handle_search(ui, key)
                    .await
                    .is_ok_and(|should_quit| should_quit)
                {
                    return Ok(true);
                }
            }
            Region::Command => {
                if Region::handle_command(ui, key)
                    .await
                    .is_ok_and(|should_quit| should_quit)
                {
                    return Ok(true);
                }
            }
            Region::Playlist => {
                if Region::handle_playlist(ui, key)
                    .await
                    .is_ok_and(|should_quit| should_quit)
                {
                    return Ok(true);
                }
            }
            Region::Soulseek => {
                if Region::handle_soulseek(ui, key)
                    .await
                    .is_ok_and(|should_quit| should_quit)
                {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    async fn handle_soulseek<'a>(ui: &mut UI<'a>, key: &KeyEvent) -> io::Result<bool> {
        if key.kind == event::KeyEventKind::Press && ui.soulseek_filtering {
            // typing the name filter: keys go into it, arrows still move
            match key.code {
                KeyCode::Enter => ui.soulseek_filtering = false,
                KeyCode::Esc => {
                    ui.soulseek_filter.reset();
                    ui.soulseek_filtering = false;
                    ui.soulseek_refilter();
                }
                KeyCode::Down => ui.soulseek_down(),
                KeyCode::Up => ui.soulseek_up(),
                _ => {
                    ui.soulseek_filter
                        .handle_event(&crossterm::event::Event::Key(*key));
                    ui.soulseek_refilter();
                }
            }
            return Ok(false);
        }
        if key.kind == event::KeyEventKind::Press {
            // `dd` only counts when the two d's are back to back
            let pending_d = std::mem::take(&mut ui.soulseek_pending_d);
            match key.modifiers {
                KeyModifiers::NONE => match key.code {
                    KeyCode::Char('j') => ui.soulseek_down(),
                    KeyCode::Char('k') => ui.soulseek_up(),
                    KeyCode::Char('f') | KeyCode::Char('/') => {
                        ui.soulseek_focus = SoulseekFocus::Results;
                        ui.soulseek_filtering = true;
                    }
                    KeyCode::Esc if !ui.soulseek_filter.value().is_empty() => {
                        // first Esc drops the filter, the next one closes the panel
                        ui.soulseek_filter.reset();
                        ui.soulseek_refilter();
                    }
                    KeyCode::Char('q') | KeyCode::Char('z') | KeyCode::Esc => {
                        ui.soulseek_visible = false;
                        ui.region = Region::List;
                    }
                    KeyCode::Enter => {
                        ui.soulseek_download_selected().await;
                    }
                    KeyCode::Char('a') => ui.soulseek_download_folder_selected().await,
                    KeyCode::Char('x') | KeyCode::Delete => ui.soulseek_remove_key().await,
                    KeyCode::Char('J') => ui.soulseek_move_download(false).await,
                    KeyCode::Char('K') => ui.soulseek_move_download(true).await,
                    KeyCode::Char('d') if pending_d => ui.soulseek_remove_key().await,
                    KeyCode::Char('d') => ui.soulseek_pending_d = true,
                    KeyCode::Char('c') => {
                        ui.soulseek_clear_finished().await;
                    }
                    KeyCode::Char('G') => {
                        ui.soulseek_goto_bottom();
                    }
                    KeyCode::Char(':') => {
                        ui.command_from_soulseek = true;
                        ui.change_list_mode(ListMode::Command);
                        ui.select_region(Region::Command);
                    }
                    KeyCode::Tab | KeyCode::BackTab => ui.soulseek_switch_focus(),
                    _ => {}
                },
                KeyModifiers::SHIFT => {
                    match key.code {
                        KeyCode::BackTab => ui.soulseek_switch_focus(),
                        KeyCode::Char('G') => ui.soulseek_goto_bottom(),
                        KeyCode::Char('J') => ui.soulseek_move_download(false).await,
                        KeyCode::Char('K') => ui.soulseek_move_download(true).await,
                        _ => {}
                    }
                }
                KeyModifiers::CONTROL => match key.code {
                    KeyCode::Char('d') => ui.soulseek_scroll_down(),
                    KeyCode::Char('u') => ui.soulseek_scroll_up(),
                    _ => {}
                },
                _ => {}
            }
        }
        Ok(false)
    }

    pub async fn handle_global<'a>(ui: &mut UI<'a>, key: &KeyEvent) {
        if key.kind == event::KeyEventKind::Press {
            ui.run_global_script(key, ui.region.clone());
            match key.modifiers {
                // plain letters belong to the region (list, queue, ...): no globals here; they
                // used to fire together with the list's own m / n
                KeyModifiers::ALT => {
                    if let ListMode::Select = ui.mode {
                        if let KeyCode::Char(c) = key.code {
                            match c {
                                'j' => match ui.region {
                                    Region::List => {
                                        if ui.displayed_lyrics.displayable {
                                            ui.select_lyrics_region()
                                        } else if ui.displayed_playlists.displayable {
                                            ui.select_playlist_region()
                                        } else {
                                            ui.select_action_region()
                                        }
                                    }
                                    Region::Lyrics | Region::Playlist => ui.select_action_region(),
                                    Region::Action => ui.select_bar_region(),
                                    Region::Seeker => ui.select_list_region(),
                                    Region::Volume => ui.select_list_region(),
                                    Region::Search | Region::Command | Region::Soulseek => {}
                                },
                                'k' => match ui.region {
                                    Region::Search | Region::Command | Region::Soulseek => {}
                                    Region::List => ui.select_volume_region(),
                                    Region::Lyrics | Region::Playlist => ui.select_list_region(),
                                    Region::Action => {
                                        if ui.displayed_lyrics.displayable {
                                            ui.select_lyrics_region()
                                        } else if ui.displayed_playlists.displayable {
                                            ui.select_playlist_region()
                                        } else {
                                            ui.select_list_region()
                                        }
                                    }
                                    Region::Seeker => ui.select_action_region(),
                                    Region::Volume => ui.select_action_region(),
                                },
                                'h' | 'l' => match ui.region {
                                    Region::Seeker => ui.select_volume_region(),
                                    Region::Volume => ui.select_bar_region(),
                                    _ => {}
                                },
                                _ => {}
                            }
                        }
                    }
                }
                KeyModifiers::CONTROL => {
                    if let KeyCode::Char(c) = key.code {
                        match c {
                            'l' => ui.toggle_lyrics(),
                            // next / previous track, like vim's Ctrl-n / Ctrl-p
                            'n' => ui.play_next().await,
                            'p' => ui.play_preivous().await,
                            's' => ui.toggle_soulseek(),
                            // Ctrl-w then j/k: the panel below / above, like vim windows
                            'w' => ui.pending_window = true,
                            _ => {}
                        }
                    }
                }
                KeyModifiers::SHIFT => {
                    if let KeyCode::Char(c) = key.code {
                        match c {
                            'D' => ui.download_lyrics(),
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
    }

    pub async fn handle_list<'a>(ui: &mut UI<'a>, key: &KeyEvent) -> std::io::Result<bool> {
        match ui.mode {
            ListMode::AfterSearch => {
                if ListMode::handle_after_search(ui, key)
                    .await
                    .is_ok_and(|x| x)
                {
                    return Ok(true);
                }
            }
            ListMode::Select => {
                if ListMode::handle_select(ui, key).await.is_ok_and(|x| x) {
                    return Ok(true);
                }
            }
            _ => {}
        }
        Ok(false)
    }

    pub async fn handle_action<'a>(ui: &mut UI<'a>, key: &KeyEvent) -> std::io::Result<bool> {
        if key.kind == event::KeyEventKind::Press {
            ui.run_actions_script(key);
            match key.modifiers {
                KeyModifiers::SHIFT => {
                    if key.code == KeyCode::BackTab {
                        ui.cycle_back();
                    }
                }
                KeyModifiers::NONE => match key.code {
                    KeyCode::Char(c) => match c {
                        'z' => ui.select_list_region(),
                        'l' => ui.next_action(),
                        'h' => ui.previous_action(),
                        _ => {}
                    },
                    KeyCode::Enter => {
                        ui.preform_action().await;
                    }
                    KeyCode::Tab => {
                        ui.cycle();
                    }
                    _ => {}
                },
                _ => {}
            }
        }
        Ok(false)
    }

    pub async fn handle_seeker<'a>(ui: &mut UI<'a>, key: &KeyEvent) -> std::io::Result<bool> {
        if key.kind == event::KeyEventKind::Press {
            ui.run_seeker_script(key);
            if key.modifiers == KeyModifiers::NONE {
                if let KeyCode::Char(c) = key.code {
                    match c {
                        'l' => ui.next_5s().await,
                        'h' => ui.previous_5s().await,
                        'k' => ui.toggle_play().await,
                        'z' => ui.select_list_region(),
                        _ => {}
                    }
                }
            }
        }
        Ok(false)
    }

    pub async fn handle_volume<'a>(ui: &mut UI<'a>, key: &KeyEvent) -> std::io::Result<bool> {
        if key.kind == event::KeyEventKind::Press {
            ui.run_volume_script(key);
            match key.modifiers {
                KeyModifiers::NONE => {
                    if let KeyCode::Char(c) = key.code {
                        match c {
                            'k' | 'l' => ui.increase_volume().await,
                            'h' | 'j' => ui.decrease_volume().await,
                            'z' => ui.select_list_region(),
                            _ => {}
                        }
                    }
                }
                KeyModifiers::SHIFT => {
                    if let KeyCode::Char(c) = key.code {
                        match c {
                            'j' | 'h' => ui.increase_volume().await,
                            'k' | 'l' => ui.decrease_volume().await,
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(false)
    }

    pub async fn handle_lyrics(ui: &mut UI<'_>, key: &KeyEvent) -> std::io::Result<bool> {
        if key.kind == event::KeyEventKind::Press {
            ui.run_lyrics_script(key);
            ui.displayed_lyrics.last_active_interaction = Instant::now();
            match key.modifiers {
                KeyModifiers::NONE => match ui.anticipation_mode {
                    AncitipationMode::Normal => match key.code {
                        KeyCode::Char(c) => match c {
                            'z' => ui.select_list_region(),
                            'j' => ui.select_next_lyrics_line(),
                            'k' => ui.select_previous_lyrics_line(),
                            'g' => ui.anticipate('g'),
                            _ => {}
                        },
                        KeyCode::Enter => {
                            ui.play_music_from_lyrics().await;
                        }
                        _ => {}
                    },
                    AncitipationMode::Char(c) => {
                        if c == 'g' {
                            if let KeyCode::Char(c) = key.code {
                                if c == 'g' {
                                    ui.goto_top_lyrics()
                                }
                            }
                        }
                        ui.anticipation_mode = AncitipationMode::Normal;
                    }
                },
                KeyModifiers::SHIFT => {
                    if let KeyCode::Char(c) = key.code {
                        if c == 'G' {
                            ui.goto_bottom_lyrics()
                        }
                    }
                }
                KeyModifiers::CONTROL => {
                    if let KeyCode::Char(c) = key.code {
                        match c {
                            'd' => ui.scroll_lyrics_down(),
                            'u' => ui.scroll_lyrics_up(),
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(false)
    }

    async fn handle_playlist(ui: &mut UI<'_>, key: &KeyEvent) -> std::io::Result<bool> {
        if key.kind == event::KeyEventKind::Press
            && matches!(key.modifiers, KeyModifiers::NONE | KeyModifiers::SHIFT)
        {
            // `gg` only counts when the two g's are back to back
            let pending_g = std::mem::take(&mut ui.playlists_pending_g);
            match key.code {
                KeyCode::Char('j') | KeyCode::Down => ui.select_next_playlist_name(),
                KeyCode::Char('k') | KeyCode::Up => ui.select_previous_playlist_name(),
                KeyCode::Char('g') if pending_g => ui.displayed_playlists.state.select_first(),
                KeyCode::Char('g') => ui.playlists_pending_g = true,
                KeyCode::Char('G') | KeyCode::End => ui.displayed_playlists.state.select_last(),
                KeyCode::Home => ui.displayed_playlists.state.select_first(),
                // leave the panel open, go back to the tracks
                KeyCode::Char('z') | KeyCode::Tab | KeyCode::BackTab => ui.select_list_region(),
                KeyCode::Char('q') | KeyCode::Esc => {
                    ui.disable_playlists();
                    ui.select_list_region();
                }
                KeyCode::Char(':') => {
                    // back to the panel after the command, like `:` from the Soulseek panel
                    ui.focus_playlists = true;
                    ui.change_list_mode(ListMode::Command);
                    ui.select_region(Region::Command);
                }
                KeyCode::Enter | KeyCode::Char('l') => {
                    ui.switch_to_selected_playlist().await;
                    ui.select_list_region();
                }
                _ => {}
            }
        }
        Ok(false)
    }

    async fn handle_search(ui: &mut UI<'_>, key: &KeyEvent) -> std::io::Result<bool> {
        if let ListMode::Search = ui.mode {
            ListMode::handle_search(ui, key).await;
        }
        // Never quit on search mode
        Ok(false)
    }

    async fn handle_command(ui: &mut UI<'_>, key: &KeyEvent) -> std::io::Result<bool> {
        match ui.mode {
            ListMode::Command => ListMode::handle_command(ui, key).await,
            _ => Ok(false),
        }
    }
}

pub fn to_input_request(evt: &CrosstermEvent) -> Option<InputRequest> {
    use InputRequest::*;
    use KeyCode::*;
    match evt {
        CrosstermEvent::Key(KeyEvent {
            code,
            modifiers,
            kind,
            state: _,
        }) if *kind == KeyEventKind::Press => match (*code, *modifiers) {
            (Backspace, KeyModifiers::NONE) | (Char('h'), KeyModifiers::CONTROL) => {
                Some(DeletePrevChar)
            }
            (Delete, KeyModifiers::NONE) => Some(DeleteNextChar),
            (Tab, KeyModifiers::NONE) => None,
            (Left, KeyModifiers::NONE) | (Char('b'), KeyModifiers::CONTROL) => Some(GoToPrevChar),
            (Left, KeyModifiers::CONTROL) | (Char('b'), KeyModifiers::META) => Some(GoToPrevWord),
            (Right, KeyModifiers::NONE) | (Char('f'), KeyModifiers::CONTROL) => Some(GoToNextChar),
            (Right, KeyModifiers::CONTROL) | (Char('f'), KeyModifiers::META) => Some(GoToNextWord),
            (Char('u'), KeyModifiers::CONTROL) => Some(DeleteLine),

            (Char('w'), KeyModifiers::CONTROL)
            | (Char('d'), KeyModifiers::META)
            | (Backspace, KeyModifiers::META)
            | (Backspace, KeyModifiers::ALT) => Some(DeletePrevWord),

            (Delete, KeyModifiers::CONTROL) => Some(DeleteNextWord),
            (Char('k'), KeyModifiers::CONTROL) => Some(DeleteTillEnd),
            (Char('a'), KeyModifiers::CONTROL) | (Home, KeyModifiers::NONE) => Some(GoToStart),
            (Char('e'), KeyModifiers::CONTROL) | (End, KeyModifiers::NONE) => Some(GoToEnd),
            (Char(c), KeyModifiers::NONE) => Some(InsertChar(c)),
            (Char(c), KeyModifiers::SHIFT) => Some(InsertChar(c)),
            (_, _) => None,
        },
        _ => None,
    }
}

/// Import this trait to implement `Input::handle_event()` for crossterm.
pub trait EventHandler {
    /// Handle crossterm event.
    fn handle_event(&mut self, evt: &CrosstermEvent) -> Option<StateChanged>;
}

impl EventHandler for Input {
    /// Handle crossterm event.
    fn handle_event(&mut self, evt: &CrosstermEvent) -> Option<StateChanged> {
        to_input_request(evt).and_then(|req| self.handle(req))
    }
}

impl<'a> UI<'a> {
    fn run_script(
        &self,
        f: &Option<mlua::Function>,
        key: &KeyEvent,
        region: Region,
        mode: Option<ListMode>,
    ) -> mlua::Result<()> {
        if let Some(function) = f {
            let ev = Event::new(key.modifiers.into(), key.code.into(), region, mode);
            let userdata = self.lua.create_userdata(ev)?;
            return function.call::<()>(userdata);
        }
        Ok(())
    }

    pub fn run_list_script(&self, key: &KeyEvent, mode: ListMode) {
        if let Some(scripts) = &self.scripts {
            if let Some(function) = &scripts.list {
                let ev = Event::new(
                    key.modifiers.into(),
                    key.code.into(),
                    Region::List,
                    Some(mode),
                );
                let userdata = self.lua.create_userdata(ev);
                function.call::<()>(userdata).unwrap();
            }
        }
    }

    pub fn run_global_script(&self, key: &KeyEvent, region: Region) {
        if let Some(scripts) = &self.scripts {
            let _ = self.run_script(&scripts.all, key, region, None);
        }
    }

    pub fn run_seeker_script(&self, key: &KeyEvent) {
        if let Some(script) = &self.scripts {
            let _ = self.run_script(&script.seeker, key, Region::Seeker, None);
        }
    }

    pub fn run_actions_script(&self, key: &KeyEvent) {
        if let Some(script) = &self.scripts {
            let _ = self.run_script(&script.seeker, key, Region::Action, None);
        }
    }

    pub fn run_volume_script(&self, key: &KeyEvent) {
        if let Some(script) = &self.scripts {
            let _ = self.run_script(&script.seeker, key, Region::Volume, None);
        }
    }

    pub fn run_lyrics_script(&self, key: &KeyEvent) {
        if let Some(script) = &self.scripts {
            let _ = self.run_script(&script.seeker, key, Region::Lyrics, None);
        }
    }

    pub fn new(proxy: ServerProxy<'a>) -> Self {
        let lua = unsafe { mlua::Lua::unsafe_new() };
        UI {
            scripts: None,
            power_bar: PowerBar::default(),
            music_list: Musics::default(),
            region: Region::default(),
            style: UIStyle::default(),
            action: PowerActions::BackwardSkip,
            state: State::new(proxy),
            mode: ListMode::default(),
            search_bufr: Input::default(),
            command_bufr: Input::default(),
            pending_window: false,
            count: None,
            last_search: None,
            completion: None,
            command_error: None,
            anticipation_mode: AncitipationMode::default(),
            displayed_repeat: Repeat::default(),
            displayed_sort: Sort::default(),
            displayed_lyrics: DisplayedLyrics::default(),
            displayed_playlists: DisplayedPlaylists::default(),
            fps_controller: FPSController::default(),
            geometry: ViewGeometry::default(Rect::default()),
            internal_clock: tokio::time::interval(Duration::from_secs(1)),
            automix: (false, String::new(), -1., String::new()),
            lua,
            soulseek_ranked: Vec::new(),
            soulseek_shown: Vec::new(),
            soulseek_filter: Input::default(),
            soulseek_filtering: false,
            soulseek_pending_d: false,
            playlists_pending_g: false,
            config_path: None,
            settings_view: None,
            queue: Vec::new(),
            queue_view: None,
            analysis_progress: (0, 0),
            selected_features: None,
            list_note: None,
            soulseek_pending_query: None,
            bar_repeat: Rect::default(),
            bar_sort: Rect::default(),
            last_click: None,
            soulseek_status: String::new(),
            soulseek_download_list: Vec::new(),
            soulseek_selected: 0,
            soulseek_selection_moved: false,
            soulseek_download_selected: 0,
            soulseek_focus: SoulseekFocus::default(),
            soulseek_results_state: TableState::new(),
            soulseek_marquee: (None, std::time::Instant::now()),
            soulseek_downloads_state: TableState::new(),
            soulseek_open_on_results: false,
            command_from_soulseek: false,
            focus_playlists: false,
            soulseek_visible: false,
            help_open: false,
            last_input: std::time::Instant::now(),
            pending_select: None,
            soulseek_connected: false,
            soulseek_config: None,
            agent_config: Default::default(),
            knob_config: Default::default(),
            knob: None,
            knob_note: None,
            keyboard_enhanced: false,
            dj: None,
            dj_kbd_pushed: false,
            dj_text_pushed: false,
        }
    }

    pub fn dj_active(&self) -> bool {
        self.dj.is_some()
    }

    /// Nothing on screen would be lost by swapping codj for a newer build: the plain list, no
    /// typing, no panel or DJ mode, and no key for a while
    pub fn idle_for_restart(&self, quiet: std::time::Duration) -> bool {
        !self.dj_active()
            && matches!(self.mode, ListMode::Select)
            && matches!(self.region, Region::List)
            && self.settings_view.is_none()
            && self.queue_view.is_none()
            && !self.help_open
            && !self.soulseek_visible
            && self.last_input.elapsed() >= quiet
    }

    /// DJ mode with nothing open that a swap would lose, and no key for a while: the newer
    /// build comes back in DJ mode with the same deck focused
    pub fn dj_idle_for_restart(&self, quiet: std::time::Duration) -> Option<u8> {
        let dj = self.dj.as_ref()?;
        (dj.idle_for_restart() && self.last_input.elapsed() >= quiet).then(|| dj.focus())
    }

    /// Back in DJ mode after a swap, the same deck focused
    pub async fn resume_dj(&mut self, focus: u8) {
        self.enter_dj().await;
        if let Some(dj) = self.dj.as_mut() {
            dj.set_focus(focus);
        }
    }

    /// Where the cursor is in the list (handed to a newer codj)
    pub fn list_position(&self) -> usize {
        self.music_list.selected
    }

    /// Puts the cursor where the replaced codj had it, once the list has loaded
    pub fn apply_pending_select(&mut self) {
        if let Some(n) = self.pending_select.filter(|_| !self.music_list.musics.is_empty()) {
            self.music_list.selected = n.min(self.music_list.musics.len() - 1);
            self.pending_select = None;
        }
    }

    /// `:dj` / `M`: switches to the live DJ screen. With a kitty-protocol terminal this also asks
    /// for key release events, which hold-to-preview and hold-to-nudge need.
    pub async fn enter_dj(&mut self) {
        if self.dj.is_some() {
            return;
        }
        self.fetch_playlist_data().await;
        let library = self.music_list.unfiltered_music_list.clone();
        if self.keyboard_enhanced {
            use crossterm::event::KeyboardEnhancementFlags as F;
            crate::kbd_push(
                F::DISAMBIGUATE_ESCAPE_CODES | F::REPORT_EVENT_TYPES | F::REPORT_ALL_KEYS_AS_ESCAPE_CODES,
            );
            self.dj_kbd_pushed = true;
        }
        let conn = self.state.proxy.inner().connection().clone();
        let mut dj = crate::dj::DjScreen::new(conn, library, self.dj_kbd_pushed);
        dj.set_agent(self.agent_config.clone());
        dj.set_knob(self.knob.take());
        dj.set_jog_seconds(self.knob_config.jog);
        if let Some(n) = self.knob_note.take() {
            dj.note_from_outside(n);
        }
        self.dj = Some(Box::new(dj));
    }

    /// Back to the normal view (audio keeps playing; nothing is stopped)
    pub fn leave_dj(&mut self) {
        let Some(mut dj) = self.dj.take() else { return };
        // release held keys and send pending taps before the session goes away
        dj.settle(Instant::now());
        self.knob = dj.take_knob();
        drop(dj);
        if self.dj_text_pushed {
            crate::kbd_pop();
            self.dj_text_pushed = false;
        }
        if self.dj_kbd_pushed {
            crate::kbd_pop();
            self.dj_kbd_pushed = false;
        }
    }

    /// Typed text needs the plain protocol (shifted symbols arrive as the key plus Shift while
    /// every key is reported as an escape code), so typing pops back to the base level
    fn sync_dj_kbd(&mut self) {
        if !self.dj_kbd_pushed {
            return;
        }
        let typing = self.dj.as_ref().is_some_and(|d| d.typing());
        if typing && !self.dj_text_pushed {
            crate::kbd_push(crossterm::event::KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES);
            self.dj_text_pushed = true;
        } else if !typing && self.dj_text_pushed {
            crate::kbd_pop();
            self.dj_text_pushed = false;
        }
    }

    pub fn dj_key(&mut self, key: &KeyEvent) {
        let Some(dj) = self.dj.as_mut() else { return };
        if dj.handle_key(key, Instant::now()) == crate::dj::Outcome::Exit {
            self.leave_dj();
        } else {
            self.sync_dj_kbd();
        }
    }

    /// A mouse wheel detent in DJ mode (clicks are ignored there)
    pub fn dj_wheel(&mut self, up: bool) {
        if let Some(dj) = self.dj.as_mut() {
            dj.wheel(up, Instant::now());
        }
    }

    /// `:edit`: the file to open in `$EDITOR`, once
    pub fn dj_take_edit(&mut self) -> Option<std::path::PathBuf> {
        self.dj.as_mut()?.take_edit_request()
    }

    pub fn dj_edited(&mut self, text: String) {
        if let Some(dj) = self.dj.as_mut() {
            dj.edited(text);
        }
    }

    /// the keyboard-enhancement levels to push again after the terminal was given away
    pub fn kbd_levels(&self) -> Vec<crossterm::event::KeyboardEnhancementFlags> {
        crate::dj::kbd_levels(self.keyboard_enhanced, self.dj_kbd_pushed, self.dj_text_pushed)
    }

    pub fn dj_tick(&mut self) {
        if let Some(dj) = self.dj.as_mut() {
            dj.tick(Instant::now());
        }
        // a pane may have closed by itself (:wq): give the keyboard flags back
        self.sync_dj_kbd();
    }

    pub async fn check_state_streams(&mut self) {
        let mut ctx = Context::from_waker(futures::task::noop_waker_ref());
        let paused_stream = self.state.dbus_streams.paused_stream.as_mut().unwrap();
        let resumed_stream = self.state.dbus_streams.resumed_stream.as_mut().unwrap();
        let stopped_stream = self.state.dbus_streams.stopped_stream.as_mut().unwrap();
        let playlist_updated_stream = self.state.dbus_streams.playlist_updated.as_mut().unwrap();
        let volume_stream = self
            .state
            .dbus_streams
            .volume_changed_stream
            .as_mut()
            .unwrap();
        let music_played = self
            .state
            .dbus_streams
            .music_played_stream
            .as_mut()
            .unwrap();
        let playlist_created_stream = self.state.dbus_streams.playlist_created.as_mut().unwrap();
        let playlist_deleted_stream = self.state.dbus_streams.playlist_deleted.as_mut().unwrap();
        let playlist_loaded_stream = self.state.dbus_streams.playlist_loaded.as_mut().unwrap();
        let sorted_stream = self.state.dbus_streams.sorted_stream.as_mut().unwrap();

        if let Poll::Ready(_) = playlist_updated_stream.poll_next_unpin(&mut ctx) {
            let playlist = self.state.proxy.playlist().await.unwrap_or_default();
            self.music_list.unfiltered_music_list.clear();
            for (index, music) in playlist.musics.iter().enumerate() {
                let music = music.to_owned();
                self.music_list.unfiltered_music_list.push(Music {
                    path: music.path,
                    title: music.title,
                    length: music.length,
                    artist: music.artist,
                    genre: music.genre,
                    index,
                });
            }
            for m in &mut self.music_list.unfiltered_music_list {
                if m.title.trim().is_empty() {
                    m.title = crate::search::file_name(&m.path);
                }
            }
            self.music_list.musics = self.music_list.unfiltered_music_list.clone();
            self.music_list.playing_index = playlist.playing_index as usize;
            self.state.batch.metadata = self.state.proxy.metadata().await.unwrap_or_default();

            let timer = self.state.proxy.timer().await.unwrap_or_default();
            let played = timer.0;
            let full = timer.1;
            (
                self.state.batch.played_duration,
                self.state.batch.music_duration,
            ) = (
                Duration::from_secs_f32(played),
                Duration::from_secs_f32(full),
            );
            self.state.batch.playing_music = self.state.proxy.playing().await.unwrap_or_default();
        }

        if let Poll::Ready(_) = paused_stream.poll_next_unpin(&mut ctx) {
            self.state.batch.status = PlayingStatus::Pausing;
        }

        if let Poll::Ready(_) = resumed_stream.poll_next_unpin(&mut ctx) {
            self.state.batch.status = PlayingStatus::Playing;
        }

        if let Poll::Ready(_) = stopped_stream.poll_next_unpin(&mut ctx) {
            self.state.batch.status = PlayingStatus::Stopped;
        }

        if let Poll::Ready(_) = music_played.poll_next_unpin(&mut ctx) {
            self.state.batch.metadata.lyrics = self.state.proxy.lyrics().await.unwrap_or_default();
            self.music_list.playing_index =
                self.state.proxy.get_playing_index().await.unwrap() as usize;
            self.state.batch.playing_music = self.state.proxy.playing().await.unwrap();
            self.state.batch.status = PlayingStatus::Playing;
        }

        // drain every pending volume signal; echoes of our own changes are already applied
        // locally, applying them late would undo presses made since
        while let Poll::Ready(Some(v)) = volume_stream.poll_next_unpin(&mut ctx) {
            if self.state.batch.own_volume_changes > 0 {
                self.state.batch.own_volume_changes -= 1;
                continue;
            }
            if let Ok(args) = v.args() {
                self.state.batch.volume = args.amount.clamp(0.0, 1.0);
            }
        }

        if let Poll::Ready(v) = playlist_created_stream.poll_next_unpin(&mut ctx) {
            self.state
                .batch
                .playlists_names
                .push(v.unwrap().args().unwrap().id().to_string());
        }

        if let Poll::Ready(v) = playlist_deleted_stream.poll_next_unpin(&mut ctx) {
            let id = v.unwrap().args().unwrap().id().to_string();
            if let Some(pos) = self
                .state
                .batch
                .playlists_names
                .iter()
                .position(|x| x.eq(&id))
            {
                self.state.batch.playlists_names.remove(pos);
            }
        }

        if let Poll::Ready(_) = playlist_loaded_stream.poll_next_unpin(&mut ctx) {
            self.state.batch.status = PlayingStatus::Stopped;

            let playlist = self.state.proxy.playlist().await.unwrap_or_default();
            self.music_list.unfiltered_music_list.clear();
            for (index, music) in playlist.musics.iter().enumerate() {
                let music = music.to_owned();
                self.music_list.unfiltered_music_list.push(Music {
                    path: music.path,
                    title: music.title,
                    length: music.length,
                    artist: music.artist,
                    genre: music.genre,
                    index,
                });
            }
            for m in &mut self.music_list.unfiltered_music_list {
                if m.title.trim().is_empty() {
                    m.title = crate::search::file_name(&m.path);
                }
            }
            self.music_list.musics = self.music_list.unfiltered_music_list.clone();
            self.music_list.playing_index = playlist.playing_index as usize;
            self.state.batch.metadata = self.state.proxy.metadata().await.unwrap_or_default();

            let timer = self.state.proxy.timer().await.unwrap_or_default();
            let played = timer.0;
            let full = timer.1;
            (
                self.state.batch.played_duration,
                self.state.batch.music_duration,
            ) = (
                Duration::from_secs_f32(played),
                Duration::from_secs_f32(full),
            );
            self.state.batch.playing_music = self.state.proxy.playing().await.unwrap_or_default();
        }

        if let Poll::Ready(v) = sorted_stream.poll_next_unpin(&mut ctx) {
            self.state.batch.sort = v.unwrap().args().unwrap().sort;
            self.fetch_playlist_data().await;
        }

        let added = self.state.dbus_streams.music_added.as_mut().and_then(|stream| {
            match stream.poll_next_unpin(&mut ctx) {
                Poll::Ready(Some(signal)) => signal.args().ok().map(|a| a.path().to_string()),
                _ => None,
            }
        });
        if let Some(path) = added {
            self.on_music_added(&path).await;
        }

        // drain: a whole album moved in arrives as many signals, one refresh is enough
        let mut changed = false;
        if let Some(stream) = self.state.dbus_streams.library_changed.as_mut() {
            while let Poll::Ready(Some(_)) = stream.poll_next_unpin(&mut ctx) {
                changed = true;
            }
        }
        if changed {
            self.on_library_changed().await;
        }
    }

    /// Files arrived from outside: reload the list, keeping the cursor on the same track
    async fn on_library_changed(&mut self) {
        let selected = self.selected_path();
        self.fetch_playlist_data().await;
        if matches!(self.mode, ListMode::Search | ListMode::AfterSearch) {
            self.music_list.search(self.search_bufr.value().to_owned());
        }
        if let Some(i) = selected.and_then(|path| {
            self.music_list.musics.iter().position(|m| {
                <&std::path::Path>::from(&m.path) == std::path::Path::new(&path)
            })
        }) {
            self.music_list.selected = i;
        }
    }

    /// A finished download joined the library: reload the list and put the cursor on it, unless
    /// the user is in the middle of a search
    async fn on_music_added(&mut self, path: &str) {
        self.fetch_playlist_data().await;
        if matches!(self.mode, ListMode::Search | ListMode::AfterSearch) {
            self.music_list.search(self.search_bufr.value().to_owned());
            return;
        }
        let target = std::path::Path::new(path);
        if let Some(i) = self.music_list.musics.iter().position(|m| {
            let p: &std::path::Path = (&m.path).into();
            p == target
        }) {
            self.music_list.selected = i;
        }
    }

    pub fn update_scripts(&mut self, scripts: &Option<Scripts>) {
        self.scripts = match scripts {
            Some(scripts) => {
                let fall = match &scripts.all {
                    Some(path) => {
                        let file_content = std::fs::read_to_string(path).unwrap();
                        let func: mlua::Function = self.lua.load(&file_content).eval().unwrap();
                        Some(func)
                    }
                    None => None,
                };

                let flist = match &scripts.list {
                    None => None,
                    Some(path) => {
                        let file_content = std::fs::read_to_string(path).unwrap();
                        let func: mlua::Function = self.lua.load(&file_content).eval().unwrap();
                        Some(func)
                    }
                };

                let flyrics = match &scripts.lyrics {
                    None => None,
                    Some(path) => {
                        let file_content = std::fs::read_to_string(path).unwrap();
                        let func: mlua::Function = self.lua.load(&file_content).eval().unwrap();
                        Some(func)
                    }
                };

                let factions = match &scripts.actions {
                    Some(path) => {
                        let file_content = std::fs::read_to_string(path).unwrap();
                        let func: mlua::Function = self.lua.load(&file_content).eval().unwrap();
                        Some(func)
                    }
                    None => None,
                };

                let fseeker = match &scripts.seeker {
                    Some(path) => {
                        let file_content = std::fs::read_to_string(path).unwrap();
                        let func: mlua::Function = self.lua.load(&file_content).eval().unwrap();
                        Some(func)
                    }
                    None => None,
                };

                let fvolume = match &scripts.volume {
                    Some(path) => {
                        let file_content = std::fs::read_to_string(path).unwrap();
                        let func: mlua::Function = self.lua.load(&file_content).eval().unwrap();
                        Some(func)
                    }
                    None => None,
                };

                Some(Functions::new(
                    fall, flist, flyrics, factions, fseeker, fvolume,
                ))
            }
            None => None,
        }
    }

    pub fn update_from_config(&mut self, config: &Config) {
        self.update_scripts(&config.scripts);
        let config = config.clone();
        self.fps_controller.change_fps(config.fps.unwrap_or(30));
        self.displayed_lyrics.displayable = config.lyrics.unwrap_or(false);
        self.music_list.display_genre = config.genre.unwrap_or(false);
        self.music_list.setp_szie = config.step_size.unwrap_or(7);
        self.geometry.configuration.lyrics = self.displayed_lyrics.displayable;
        self.agent_config = config.agent.clone().unwrap_or_default();
        let knob = config.knob.clone().unwrap_or_default();
        // (re)open the knob when its settings change; the old reader is dropped first, which
        // releases its grab
        if knob != self.knob_config || (self.knob.is_none() && self.dj.is_none()) {
            self.knob = None;
            if let Some(dj) = self.dj.as_mut() {
                dj.set_knob(None);
                dj.set_knob(crate::knob::Knob::start(&knob));
            } else {
                self.knob = crate::knob::Knob::start(&knob);
            }
        }
        self.knob_config = knob;

        // store soulseek config for later connection via dbus
        if let Some(slsk) = &config.soulseek {
            if crate::settings::enabled(slsk) {
                self.soulseek_config = Some(slsk.clone());
                self.soulseek_status = "configured (use :slsk_connect)".to_string();
            }
        }
    }

    pub fn previous_action(&mut self) {
        self.action = match self.action {
            PowerActions::Sort => PowerActions::Repeat,
            PowerActions::Repeat => PowerActions::Stop,
            PowerActions::Stop => PowerActions::ForwardSkip,
            PowerActions::ForwardSkip => PowerActions::TogglePlay,
            PowerActions::TogglePlay => PowerActions::BackwardSkip,
            PowerActions::BackwardSkip => PowerActions::Sort,
        }
    }

    pub fn next_action(&mut self) {
        self.action = match self.action {
            PowerActions::BackwardSkip => PowerActions::TogglePlay,
            PowerActions::TogglePlay => PowerActions::ForwardSkip,
            PowerActions::ForwardSkip => PowerActions::Stop,
            PowerActions::Stop => PowerActions::Repeat,
            PowerActions::Repeat => PowerActions::Sort,
            PowerActions::Sort => PowerActions::BackwardSkip,
        }
    }

    pub async fn preform_action(&mut self) {
        // match the selected action
        match self.action {
            // we are on the toggle play botton
            PowerActions::TogglePlay => self.toggle_play().await,
            PowerActions::ForwardSkip => self.play_next().await,
            PowerActions::BackwardSkip => self.play_previous().await,
            PowerActions::Stop => self.stop().await,
            PowerActions::Sort => self.sort().await,
            PowerActions::Repeat => self.repeat().await,
        }
    }

    /// plays the *selected* song in the music list
    pub async fn play_selected_music(&mut self) {
        if let Some(music) = self.music_list.musics.get(self.music_list.selected) {
            self.state.play_from_index(music.index).await;
        }
    }

    /// Moves slection to the `List` region
    pub fn select_list_region(&mut self) {
        self.region = Region::List;
    }

    /// Moves slection to the `Bar` region
    pub fn select_bar_region(&mut self) {
        self.region = Region::Seeker;
    }

    /// Moves slection to the `Action` region
    pub fn select_action_region(&mut self) {
        self.region = Region::Action
    }

    fn render_list_with_genre(&mut self, frame: &mut Frame) {
        let mut rows = vec![];
        let playing_index = self.music_list.playing_index;
        for music in self.music_list.musics.iter() {
            let mut title = music.title.to_owned();
            let artist = music.artist.to_owned();
            let time = UI::duration_to_string(music.length.as_secs());
            let genre = music.genre.to_owned();
            if playing_index == music.index && !matches!(self.state.status(), PlayingStatus::Stopped) {
                title.insert_str(0, self.style.list_style.playing_selector.as_str());
                rows.push(
                    Row::new(vec![title, artist, genre, time])
                        .style(self.style.list_style.playing_region_color),
                )
            } else {
                rows.push(Row::new(vec![title, artist, genre, time]))
            }
        }

        let widths = [
            Constraint::Fill(3),
            Constraint::Fill(2),
            Constraint::Fill(1),
            Constraint::Length(8),
        ];

        let block = match self.region {
            Region::List => match self.mode {
                ListMode::Command => Block::default()
                    .title(self.musics_title())
                    .borders(Borders::ALL)
                    .fg(self.style.list_style.active_command_region_color),
                ListMode::Search => Block::default()
                    .title(self.musics_title())
                    .borders(Borders::ALL)
                    .fg(self.style.list_style.active_search_region_color),
                ListMode::AfterSearch => Block::default()
                    .title(self.musics_title())
                    .borders(Borders::ALL)
                    .fg(self.style.list_style.active_after_search_region_color),
                ListMode::Select => Block::default()
                    .title(self.musics_title())
                    .borders(Borders::ALL)
                    .fg(self.style.list_style.active_region_color),
            },
            _ => Block::default()
                .title(self.musics_title())
                .borders(Borders::ALL)
                .fg(self.style.list_style.passive_region_color),
        };

        let table = Table::new(rows, widths)
            .block(block)
            .highlight_style(
                Style::new()
                    .add_modifier(Modifier::REVERSED)
                    .fg(self.style.list_style.hilight_color),
            )
            .highlight_symbol(self.style.list_style.selector.as_str())
            .header(
                Row::new(vec!["Title", "artist", "genre", "duration"])
                    .style(Style::new().bold().italic()),
            );

        frame.render_stateful_widget(table, self.geometry.list_geo(), &mut self.music_list.state);
    }

    fn render_list_without_genre(&mut self, frame: &mut Frame) {
        let mut rows = vec![];
        let playing_index = self.music_list.playing_index;
        for music in self.music_list.musics.iter() {
            let mut title = music.title.to_owned();
            let artist = music.artist.to_owned();
            let time = UI::duration_to_string(music.length.as_secs());
            if playing_index == music.index && !matches!(self.state.status(), PlayingStatus::Stopped) {
                title.insert_str(0, self.style.list_style.playing_selector.as_str());
                rows.push(
                    Row::new(vec![title, artist, time])
                        .style(self.style.list_style.playing_region_color),
                )
            } else {
                rows.push(Row::new(vec![title, artist, time]))
            }
        }

        let widths = [
            Constraint::Fill(3),
            Constraint::Fill(2),
            Constraint::Length(8),
        ];

        let block = match self.region {
            Region::List => match self.mode {
                ListMode::Search => Block::default()
                    .title(self.musics_title())
                    .borders(Borders::ALL)
                    .fg(self.style.list_style.active_search_region_color),
                ListMode::AfterSearch => Block::default()
                    .title(self.musics_title())
                    .borders(Borders::ALL)
                    .fg(self.style.list_style.active_after_search_region_color),
                ListMode::Select => Block::default()
                    .title(self.musics_title())
                    .borders(Borders::ALL)
                    .fg(self.style.list_style.active_region_color),
                ListMode::Command => Block::default()
                    .title(self.musics_title())
                    .borders(Borders::ALL)
                    .fg(self.style.list_style.active_command_region_color),
            },
            _ => Block::default()
                .title(self.musics_title())
                .borders(Borders::ALL)
                .fg(self.style.list_style.passive_region_color),
        };

        let table = Table::new(rows, widths)
            .block(block)
            .highlight_style(
                Style::new()
                    .add_modifier(Modifier::REVERSED)
                    .fg(self.style.list_style.hilight_color),
            )
            .highlight_symbol(self.style.list_style.selector.as_str())
            .header(
                Row::new(vec!["Title", "artist", "duration"]).style(Style::new().bold().italic()),
            );

        match self.mode {
            ListMode::Command => {
                if let Some(geo) = self.geometry.command_geo() {
                    frame.render_widget(
                        Paragraph::new(self.command_bufr.value())
                            .block(Block::bordered().title("Querry")),
                        geo,
                    );
                }
            }
            ListMode::Search | ListMode::AfterSearch => {
                if let Some(geo) = self.geometry.search_geo() {
                    frame.render_widget(
                        Paragraph::new(self.search_bufr.value())
                            .block(Block::bordered().title("Querry")),
                        geo,
                    );
                }
            }
            ListMode::Select => {}
        }

        frame.render_stateful_widget(table, self.geometry.list_geo(), &mut self.music_list.state);
    }

    /// Renders the region of the music list
    pub fn render_list(&mut self, frame: &mut Frame) {
        if self.music_list.display_genre {
            self.render_list_with_genre(frame)
        } else {
            self.render_list_without_genre(frame)
        }
        // an empty library says what to do instead of showing a blank list
        if self.music_list.unfiltered_music_list.is_empty() {
            let area = self.geometry.list_geo();
            let inner = Rect::new(area.x + 2, area.y + 3, area.width.saturating_sub(4), area.height.saturating_sub(4));
            let lines = vec![
                Line::from("No music in the library yet.").bold(),
                Line::from(""),
                Line::from("codjd plays ~/Music, or the folder set as `path` in ~/.config/codjd/config.toml."),
                Line::from("Files copied there show up by themselves."),
                Line::from(""),
                Line::from(vec![Span::from(":slsk <artist or title>").bold().cyan(), Span::from("  search Soulseek and download")]),
                Line::from(vec![Span::from(":set").bold().cyan(), Span::from("                     settings  ·  "), Span::from("?").bold().cyan(), Span::from(" all keys")]),
            ];
            Paragraph::new(lines).wrap(Wrap { trim: false }).render(inner, frame.buffer_mut());
        }
    }

    fn get_action_index(&self) -> usize {
        match self.action {
            PowerActions::BackwardSkip => 0,
            PowerActions::TogglePlay => 1,
            PowerActions::ForwardSkip => 2,
            PowerActions::Stop => 3,
            PowerActions::Repeat => 4,
            PowerActions::Sort => 5,
        }
    }

    /// Converst timer in the u64 form to a string of form xx:yy
    pub fn duration_to_string(time: u64) -> String {
        let seconds = time % 60;
        let minities = time / 60;

        let sseconds = if seconds > 9 {
            format!("{}", seconds)
        } else {
            format!("0{}", seconds)
        };

        let sminutes = if minities > 9 {
            format!("{}", minities)
        } else {
            format!("0{}", minities)
        };

        format!("{}:{}", sminutes, sseconds)
    }

    /// Calculates the percentage of the seeker with respect with the full song length
    fn seeker_percent(&self) -> f64 {
        let current = self.state.played_duration().as_secs();
        let music_length = self.state.playing_music().length.as_secs();
        if music_length != 0 {
            let mut percent = current as f64 / music_length as f64;
            percent = percent.max(0.0).min(1.0);
            percent
        } else {
            0.0
        }
    }

    /// Seeks playing time forward by 5 seconds
    pub async fn next_5s(&mut self) {
        let current = self.state.played_duration();
        let max = self.state.playing_music_duration();
        if current + Duration::from_secs(5) < max {
            self.power_bar.current_timer = current + Duration::from_secs(5);
            self.state
                .seek(self.power_bar.current_timer.as_secs_f64())
                .await;
        } else {
            self.play_next().await;
        }
    }

    /// Seeks playing time to the target duration if it fits, otherwise
    /// plays the next music
    pub async fn goto_xs(&mut self, target: Duration) {
        self.state.seek(target.as_secs_f64()).await;
    }

    /// Seeks playing time backward by 5 seconds
    pub async fn previous_5s(&mut self) {
        let current = self.state.played_duration();
        match current.checked_sub(Duration::from_secs(5)) {
            Some(dur) => {
                self.power_bar.current_timer = dur;
                self.state.seek(dur.as_secs_f64()).await;
            }
            None => {
                self.play_preivous().await;
            }
        }
    }

    /// Mouse: click a track to select it and again to play, wheel scrolls; in the bar, click
    /// the controls, click/drag the seek bar, click or wheel the volume, click repeat/sort to
    /// change them
    pub async fn handle_mouse(&mut self, m: &MouseEvent) {
        let at = Position::new(m.column, m.row);
        let overlay =
            self.soulseek_visible || self.queue_view.is_some() || self.settings_view.is_some();
        let list = self.geometry.list_geo();
        let in_list = !overlay && list.contains(at) && !matches!(self.mode, ListMode::Command);
        match m.kind {
            MouseEventKind::ScrollDown | MouseEventKind::ScrollUp => {
                let down = matches!(m.kind, MouseEventKind::ScrollDown);
                if in_list {
                    self.scroll_list_by(if down { 3 } else { -3 });
                } else if self.geometry.slider_geo().contains(at) {
                    if down { self.previous_5s().await } else { self.next_5s().await }
                } else if self.geometry.volume_geo().contains(at) {
                    if down { self.decrease_volume().await } else { self.increase_volume().await }
                }
            }
            MouseEventKind::Down(MouseButton::Left) => {
                if in_list {
                    self.click_list(at, list).await;
                } else if self.geometry.slider_geo().contains(at) {
                    self.seek_to_column(m.column).await;
                } else if self.geometry.volume_geo().contains(at) {
                    self.volume_to_column(m.column).await;
                } else if self.geometry.actions_geo().contains(at) {
                    let i = (m.column - self.geometry.actions_geo().x) / 3;
                    self.action = match i {
                        0 => PowerActions::BackwardSkip,
                        1 => PowerActions::TogglePlay,
                        2 => PowerActions::ForwardSkip,
                        _ => PowerActions::Stop,
                    };
                    self.preform_action().await;
                } else if self.bar_repeat.contains(at) {
                    self.action = PowerActions::Repeat;
                    self.cycle();
                    self.repeat().await;
                } else if self.bar_sort.contains(at) {
                    self.action = PowerActions::Sort;
                    self.cycle();
                    self.sort().await;
                }
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                if self.geometry.slider_geo().contains(at) {
                    self.seek_to_column(m.column).await;
                } else if self.geometry.volume_geo().contains(at) {
                    self.volume_to_column(m.column).await;
                }
            }
            _ => {}
        }
    }

    async fn click_list(&mut self, at: Position, list: Rect) {
        // border and header take the first two rows
        let first = list.y + 2;
        if at.y < first || at.y + 1 >= list.y + list.height {
            return;
        }
        let index = self.music_list.state.offset() + (at.y - first) as usize;
        if index >= self.music_list.musics.len() {
            return;
        }
        self.select_list_region();
        let again = self
            .last_click
            .is_some_and(|(i, t)| i == index && t.elapsed() < Duration::from_millis(500));
        self.music_list.selected = index;
        if again {
            self.last_click = None;
            self.play_selected_music().await;
            if matches!(self.mode, ListMode::Search | ListMode::AfterSearch) {
                self.leave_search();
            }
        } else {
            self.last_click = Some((index, Instant::now()));
        }
    }

    /// Moves the selection by `rows`, stopping at the ends (the wheel shouldn't wrap around)
    fn scroll_list_by(&mut self, rows: isize) {
        let len = self.music_list.musics.len();
        if len == 0 {
            return;
        }
        let target = self.music_list.selected as isize + rows;
        self.music_list.selected = target.clamp(0, len as isize - 1) as usize;
    }

    async fn seek_to_column(&mut self, column: u16) {
        let slider = self.geometry.slider_geo();
        let ratio = column.saturating_sub(slider.x) as f64 / slider.width.max(1) as f64;
        let ratio = ratio.clamp(0., 1.);
        let length = self.state.playing_music_duration();
        let target = length.mul_f64(ratio);
        self.power_bar.current_timer = target;
        self.state.batch.played_duration = target;
        self.goto_xs(target).await;
    }

    async fn volume_to_column(&mut self, column: u16) {
        // the gauge draws "vol 100%" and a space before its line
        let area = self.geometry.volume_geo();
        let line_x = area.x + VOLUME_LABEL_WIDTH + 1;
        let width = area.width.saturating_sub(VOLUME_LABEL_WIDTH + 1).max(1);
        let ratio = column.saturating_sub(line_x) as f32 / (width - 1).max(1) as f32;
        let ratio = ratio.clamp(0., 1.);
        self.state.set_volume(ratio).await;
    }

    /// Renders the now-playing bar: track, controls, seek bar and volume in one bordered row
    pub fn render_bar(&mut self, frame: &mut Frame) {
        let active = matches!(self.region, Region::Action | Region::Seeker | Region::Volume);
        let border = if active {
            self.style.action_style.active_region_color
        } else {
            self.style.action_style.passive_region_color
        };
        let hilight = Style::new()
            .fg(self.style.action_style.hilight_color)
            .add_modifier(Modifier::REVERSED);
        let action_index = self.get_action_index();
        let in_actions = matches!(self.region, Region::Action);
        let chosen = |i: usize| in_actions && action_index == i;

        let music = self.state.playing_music();
        let name = if music.title.trim().is_empty() {
            crate::search::file_stem(&music.path)
        } else {
            music.title.to_owned()
        };
        let mut title = vec![Span::from(" "), Span::from(name).bold()];
        if !music.artist.trim().is_empty() {
            title.push(Span::from(format!(" — {}", music.artist)));
        }
        title.push(Span::from(" "));

        // repeat and sort: small, on the right; in the controls h/l reach them and Enter changes
        // them (italic until applied)
        let mut repeat = Span::from(repeat_label(&self.displayed_repeat));
        let mut sort = Span::from(sort_label(&self.displayed_sort));
        if self.displayed_repeat != self.state.batch.repeat {
            repeat = repeat.italic();
        }
        if self.displayed_sort != self.state.batch.sort {
            sort = sort.italic();
        }
        repeat = if chosen(4) { repeat.patch_style(hilight) } else { repeat.dim() };
        sort = if chosen(5) { sort.patch_style(hilight) } else { sort.dim() };
        let bar = self.geometry.bar_geo();
        let (repeat_w, sort_w) = (repeat.width() as u16, sort.width() as u16);
        // right-aligned in the top border, before the corner and one space
        let sort_x = (bar.x + bar.width).saturating_sub(2 + sort_w);
        let repeat_x = sort_x.saturating_sub(3 + repeat_w);
        self.bar_sort = Rect::new(sort_x, bar.y, sort_w, 1);
        self.bar_repeat = Rect::new(repeat_x, bar.y, repeat_w, 1);
        let right = Line::from(vec![
            Span::from(" "),
            repeat,
            Span::from(" · ").dim(),
            sort,
            Span::from(" "),
        ])
        .right_aligned();

        // the one hint a first-time user needs
        let hint = Line::from(vec![
            Span::from(" ? ").bold(),
            Span::from("keys · "),
            Span::from(":dj").bold(),
            Span::from(" mix · "),
            Span::from("/").bold(),
            Span::from(" search · "),
            Span::from(":q").bold(),
            Span::from(" quit "),
        ])
        .dim()
        .right_aligned();
        let mut block = Block::bordered()
            .title(Line::from(title))
            .title(right)
            .title_bottom(hint);
        if let Some(next) = automix_label(&self.automix) {
            block = block.title_bottom(Line::from(vec![Span::from(" "), Span::from(next), Span::from(" ")]).left_aligned());
        }
        block
            .border_style(Style::new().fg(border))
            .render(self.geometry.bar_geo(), frame.buffer_mut());

        let playing = matches!(self.state.status(), PlayingStatus::Playing);
        let icons = ["⏮", if playing { "⏸" } else { "⏵" }, "⏭", "⏹"];
        let controls: Vec<Span> = icons
            .iter()
            .enumerate()
            .map(|(i, icon)| {
                let span = Span::from(format!(" {icon} "));
                if chosen(i) {
                    span.style(hilight)
                } else {
                    span
                }
            })
            .collect();
        Paragraph::new(Line::from(controls))
            .render(self.geometry.actions_geo(), frame.buffer_mut());

        let elapsed = UI::duration_to_string(self.state.played_duration().as_secs());
        let total = UI::duration_to_string(self.state.playing_music().length.as_secs());
        let time_style = Style::new().fg(self.style.seeker_style.passive_region_color);
        Paragraph::new(elapsed)
            .style(time_style)
            .right_aligned()
            .render(self.geometry.elapsed, frame.buffer_mut());
        Paragraph::new(total)
            .style(time_style)
            .right_aligned()
            .render(self.geometry.total, frame.buffer_mut());

        let seeker_fg = if matches!(self.region, Region::Seeker) {
            self.style.seeker_style.active_region_color
        } else {
            self.style.seeker_style.fg_seeker_color
        };
        LineGauge::default()
            .unfilled_style(Style::default().fg(Color::DarkGray))
            .line_set(symbols::line::THICK)
            .filled_style(
                Style::default()
                    .fg(seeker_fg)
                    .bg(self.style.seeker_style.bg_seeker_color),
            )
            .label("")
            .ratio(self.seeker_percent())
            .render(self.geometry.slider_geo(), frame.buffer_mut());

        let volume_fg = if matches!(self.region, Region::Volume) {
            self.style.volume_style.active_region_color
        } else {
            self.style.volume_style.fg_volume_color
        };
        let volume = self.state.volume().clamp(0., 1.);
        LineGauge::default()
            .style(Style::new().fg(self.style.volume_style.passive_region_color))
            .unfilled_style(Style::default().fg(Color::DarkGray))
            .filled_style(
                Style::default()
                    .fg(volume_fg)
                    .bg(self.style.volume_style.bg_volume_color),
            )
            .label(format!("vol {:>3}%", (volume * 100.).round() as u32))
            .ratio(volume as f64)
            .render(self.geometry.volume_geo(), frame.buffer_mut());
    }

    /// Updates the music playing state
    pub async fn update_state(&mut self) {
        self.check_state_streams().await;
        self.update_music_selection_state().await;
        self.update_played_duration().await;
        self.update_lyrics_state().await;
    }

    /// fetch the currently playing music data, e.g. photo, lyrics, etc
    pub async fn fetch_playlist_data(&mut self) {
        let playlist = self.state.proxy.playlist().await.unwrap_or_default();
        self.music_list.unfiltered_music_list.clear();
        for (index, music) in playlist.musics.iter().enumerate() {
            let music = music.to_owned();
            let title = if music.title.trim().is_empty() {
                crate::search::file_stem(&music.path)
            } else {
                music.title
            };
            self.music_list.unfiltered_music_list.push(Music {
                path: music.path,
                title,
                length: music.length,
                artist: music.artist,
                genre: music.genre,
                index,
            });
        }
        self.music_list.hidden_duplicates = 0;
        if !self.music_list.show_duplicates {
            let all = std::mem::take(&mut self.music_list.unfiltered_music_list);
            let (kept, hidden) = crate::dedup::hide_duplicates(all, crate::dedup::file_quality);
            self.music_list.unfiltered_music_list = kept;
            self.music_list.hidden_duplicates = hidden;
        }
        self.music_list.musics = self.music_list.unfiltered_music_list.clone();
        self.music_list.playing_index = playlist.playing_index as usize;
        self.state.fetch_playlist_data().await
    }

    /// Renders the displayed UI
    pub fn render(&mut self, frame: &mut Frame) {
        if let Some(dj) = self.dj.as_mut() {
            dj.render(frame);
            return;
        }
        self.update_geometry(frame);
        self.render_list(frame);
        self.render_search(frame);
        self.render_lyrics(frame);
        self.render_playlists(frame);
        self.render_commands(frame);
        self.render_bar(frame);
        self.render_soulseek(frame);
        self.render_queue(frame);
        self.render_settings(frame);
        self.render_help(frame);
    }

    /// `?`: the normal view's keys, vim style
    fn render_help(&self, frame: &mut Frame) {
        if !self.help_open {
            return;
        }
        let rows: &[(&str, &str)] = &[
            ("j k  5j  gg G  Ctrl-d Ctrl-u", "move (counts work: 5j, 3l)"),
            ("Enter", "play the track under the cursor"),
            ("Space", "pause / continue"),
            ("h l", "seek 5 s back / forward"),
            ("H L", "volume down / up"),
            ("s", "jump to the playing track"),
            ("/  n N", "search (regex; artist: title: genre:), next / prev match"),
            ("q  Q", "queue the track · show the queue"),
            ("m", "queue 5 similar-sounding tracks"),
            (":dj  M  S-Enter", "DJ mode: two decks, mixing, sets"),
            (":automix", "beat-matched transitions between tracks on / off"),
            (":autoplay", "never run out: queue tracks that blend, on / off"),
            (":tutor", "a guided first mix (opens DJ mode)"),
            (":slsk <query>", "search Soulseek and download"),
            (":set", "settings"),
            ("Ctrl-n Ctrl-p", "next / previous track"),
            ("Ctrl-w j k", "move between panels"),
            (":lyrics :pl :mute", "panels, mute"),
            (":pl new|add|mv|rm NAME", "make, add the track to, rename, delete"),
            (":q  ZZ", "quit (music keeps playing in codjd)"),
        ];
        let area = frame.area();
        let w = 76.min(area.width.saturating_sub(4));
        let h = (rows.len() as u16 + 2).min(area.height.saturating_sub(2));
        let r = Rect::new(area.x + (area.width - w) / 2, area.y + (area.height - h) / 2, w, h);
        let lines: Vec<Line> = rows
            .iter()
            .map(|(k, d)| Line::from(vec![Span::from(format!(" {k:<26}")).bold().cyan(), Span::from(*d)]))
            .collect();
        frame.render_widget(Clear, r);
        Paragraph::new(lines)
            .block(Block::bordered().title(" keys · any key closes · :dj ? shows the DJ keys "))
            .render(r, frame.buffer_mut());
    }

    fn render_queue(&mut self, frame: &mut Frame) {
        let Some(view) = &self.queue_view else {
            return;
        };
        let full = frame.area();
        let area = Rect {
            x: full.x + full.width / 2,
            y: full.y,
            width: full.width - full.width / 2,
            height: full.height.saturating_sub(6),
        };
        frame.render_widget(Clear, area);
        let rows: Vec<Row> = self
            .queue
            .iter()
            .enumerate()
            .map(|(i, m)| {
                let title = if m.title.trim().is_empty() {
                    crate::search::file_name(&m.path)
                } else {
                    m.title.clone()
                };
                Row::new(vec![
                    Cell::from(format!("{}", i + 1)),
                    Cell::from(title),
                    Cell::from(m.artist.clone()),
                    Cell::from(UI::duration_to_string(m.length.as_secs())),
                ])
            })
            .collect();
        let table = Table::new(
            rows,
            [
                Constraint::Length(3),
                Constraint::Fill(3),
                Constraint::Fill(2),
                Constraint::Length(6),
            ],
        )
        .block(
            Block::bordered()
                .title(format!(" queue ({}) ", self.queue.len()))
                .title_bottom(" J/K move · x/dd remove · Enter play · z close ")
                .border_style(Style::default().fg(Color::Yellow)),
        )
        .highlight_style(Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD));
        // `then`, not `then_some`: the index must not be computed for an empty queue
        let selected = (!self.queue.is_empty()).then(|| view.selected.min(self.queue.len() - 1));
        let mut state = TableState::new().with_selected(selected);
        frame.render_stateful_widget(table, area, &mut state);
    }

    /// Title of the music list, with the queue length once something is queued
    fn musics_title(&self) -> String {
        let mut title = "Musics".to_string();
        if !self.queue.is_empty() {
            title += &format!(" · queue {}", self.queue.len());
        }
        if self.music_list.hidden_duplicates > 0 {
            title += &format!(" · {} copies hidden", self.music_list.hidden_duplicates);
        }
        if let Some((note, at)) = &self.list_note {
            if at.elapsed() < Duration::from_secs(4) {
                return format!("{title} · {note}");
            }
        }
        if let Some((path, bpm, db)) = &self.selected_features {
            if self.selected_path().as_deref() == Some(path.as_str()) {
                // 0: no beat found
                if *bpm >= 1. {
                    title += &format!(" · ~{bpm:.0} bpm");
                }
                title += &format!(" {db:.0} dB");
            }
        }
        let (done, total) = self.analysis_progress;
        if done < total {
            title += &format!(" · analyzing {done}/{total}");
        }
        title
    }

    fn selected_path(&self) -> Option<String> {
        self.music_list
            .musics
            .get(self.music_list.selected)
            .map(|m| <&std::path::Path>::from(&m.path).to_string_lossy().into_owned())
    }

    /// Fetches analysis progress and the highlighted track's tempo/loudness
    pub async fn poll_analysis(&mut self) {
        if let Ok(progress) = self.state.proxy.analysis_status().await {
            self.analysis_progress = progress;
        }
        let Some(path) = self.selected_path() else {
            self.selected_features = None;
            return;
        };
        if let Ok((bpm, db, true)) = self.state.proxy.get_features(&path).await {
            self.selected_features = Some((path, bpm, db));
        }
    }

    /// A text field has the keys (search, command line, soulseek filter)
    fn typing(&self) -> bool {
        matches!(self.mode, ListMode::Search | ListMode::Command)
            || matches!(self.region, Region::Search | Region::Command)
            || self.soulseek_filtering
    }

    /// `m`: queue tracks that sound like the highlighted one
    async fn queue_similar(&mut self) {
        let Some(path) = self.selected_path() else { return };
        let note = match self.state.proxy.queue_similar(&path, 5).await {
            Ok(0) if self.analysis_progress.0 < self.analysis_progress.1 => format!(
                "not analyzed yet ({}/{})",
                self.analysis_progress.0, self.analysis_progress.1
            ),
            Ok(0) => "nothing similar to queue".to_string(),
            Ok(n) => format!("queued {n} similar"),
            Err(e) => format!("similar failed: {e}"),
        };
        self.list_note = Some((note, Instant::now()));
        self.poll_queue().await;
    }

    /// Shift+letter in the music list; returns true to quit
    async fn list_upper_key(&mut self, c: char) -> bool {
        let quit_armed = matches!(self.anticipation_mode, AncitipationMode::Char('Z'));
        self.anticipation_mode = AncitipationMode::Normal;
        match c {
            'G' => self.goto_bottom(),
            'N' => self.search_next(false),
            'Q' => self.toggle_queue().await,
            // D is taken by the lyrics download
            'M' => self.enter_dj().await,
            'H' => self.decrease_volume().await,
            'L' => self.increase_volume().await,
            // ZZ quits, like vim
            'Z' if quit_armed => return true,
            'Z' => self.anticipate('Z'),
            _ => {}
        }
        false
    }

    /// `q`: add the highlighted track to the queue (`front` = play it next)
    async fn queue_selected(&mut self, front: bool) {
        let Some(index) = self
            .music_list
            .musics
            .get(self.music_list.selected)
            .map(|m| m.index as u32)
        else {
            return;
        };
        let _ = self.state.proxy.queue_add(index, front).await;
        self.poll_queue().await;
    }

    async fn toggle_queue(&mut self) {
        if self.queue_view.take().is_none() {
            self.poll_queue().await;
            self.queue_view = Some(QueueView::default());
        }
    }

    /// Fetches the play queue (it shrinks as queued tracks get played)
    pub async fn poll_queue(&mut self) {
        if let Ok(queue) = self.state.proxy.get_queue().await {
            self.queue = queue;
        }
        if let Some(view) = self.queue_view.as_mut() {
            view.selected = view.selected.min(self.queue.len().saturating_sub(1));
        }
    }

    async fn handle_queue_key(&mut self, key: &KeyEvent) {
        if key.kind != event::KeyEventKind::Press {
            return;
        }
        let Some(view) = self.queue_view.as_mut() else {
            return;
        };
        let pending_d = std::mem::take(&mut view.pending_d);
        let len = self.queue.len();
        let pos = view.selected;
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => view.selected = (pos + 1).min(len.saturating_sub(1)),
            KeyCode::Char('k') | KeyCode::Up => view.selected = pos.saturating_sub(1),
            KeyCode::Char('J') if pos + 1 < len => {
                view.selected = pos + 1;
                let _ = self.state.proxy.queue_move(pos as u32, false).await;
                self.poll_queue().await;
            }
            KeyCode::Char('K') if pos > 0 => {
                view.selected = pos - 1;
                let _ = self.state.proxy.queue_move(pos as u32, true).await;
                self.poll_queue().await;
            }
            KeyCode::Char('d') if !pending_d => view.pending_d = true,
            KeyCode::Char('x') | KeyCode::Char('d') | KeyCode::Delete if len > 0 => {
                let _ = self.state.proxy.queue_remove(pos as u32).await;
                self.poll_queue().await;
            }
            KeyCode::Enter if len > 0 => {
                let _ = self.state.proxy.queue_play(pos as u32).await;
                self.poll_queue().await;
            }
            KeyCode::Char('z') | KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc => {
                self.queue_view = None
            }
            _ => {}
        }
    }

    fn render_settings(&mut self, frame: &mut Frame) {
        let Some(view) = &self.settings_view else {
            return;
        };
        let full = frame.area();
        let width = full.width.saturating_sub(4).min(100);
        let height = (crate::settings::SOULSEEK.len() as u16 + 7).min(full.height);
        let area = Rect {
            x: full.x + (full.width - width) / 2,
            y: full.y + (full.height.saturating_sub(height)) / 2,
            width,
            height,
        };
        frame.render_widget(Clear, area);
        let block = Block::bordered()
            .title(" soulseek settings ")
            .title_bottom(" j/k move · Enter edit · Space toggle · Esc close ")
            .border_style(Style::default().fg(Color::Yellow));
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let [list_area, help_area] =
            Layout::vertical([Constraint::Min(1), Constraint::Length(3)]).areas(inner);

        let rows: Vec<Row> = crate::settings::SOULSEEK
            .iter()
            .map(|s| {
                Row::new(vec![
                    Cell::from(s.key),
                    Cell::from(crate::settings::display(&view.data, s.key)),
                ])
            })
            .collect();
        let table = Table::new(rows, [Constraint::Length(26), Constraint::Min(10)])
            .highlight_style(Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD));
        let mut state = TableState::new().with_selected(Some(view.selected));
        frame.render_stateful_widget(table, list_area, &mut state);

        let setting = &crate::settings::SOULSEEK[view.selected];
        let mut lines = vec![Line::from(setting.help).dim()];
        match &view.editing {
            Some(input) => lines.push(Line::from(format!("{} = {}▏", setting.key, input.value()))),
            None => lines.push(Line::from(view.message.clone())),
        }
        frame.render_widget(Paragraph::new(lines), help_area);
    }

    /// Opens the settings panel with what's in the config file right now
    fn open_settings(&mut self) {
        let from_file = self
            .config_path
            .as_ref()
            .and_then(|p| Config::parse_config(&p.to_string_lossy()).ok())
            .and_then(|c| c.soulseek);
        let data = from_file
            .or_else(|| self.soulseek_config.clone())
            .unwrap_or_default();
        self.settings_view = Some(SettingsView {
            data,
            selected: 0,
            editing: None,
            message: String::new(),
        });
    }

    async fn handle_settings_key(&mut self, key: &KeyEvent) {
        if key.kind != event::KeyEventKind::Press {
            return;
        }
        let Some(view) = self.settings_view.as_mut() else {
            return;
        };
        let count = crate::settings::SOULSEEK.len();
        let setting = &crate::settings::SOULSEEK[view.selected];

        if let Some(input) = view.editing.as_mut() {
            match key.code {
                KeyCode::Esc => view.editing = None,
                KeyCode::Enter => {
                    let text = input.value().to_string();
                    view.editing = None;
                    match crate::settings::set(&mut view.data, setting.key, &text) {
                        Ok(()) => self.apply_setting(setting.key, setting.ranking).await,
                        Err(e) => view.message = e,
                    }
                }
                _ => {
                    input.handle_event(&crossterm::event::Event::Key(*key));
                }
            }
            return;
        }

        match key.code {
            KeyCode::Char('j') | KeyCode::Down => view.selected = (view.selected + 1) % count,
            KeyCode::Char('k') | KeyCode::Up => view.selected = (view.selected + count - 1) % count,
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('z') => self.settings_view = None,
            KeyCode::Char(' ') | KeyCode::Enter if setting.kind == crate::settings::Kind::Bool => {
                crate::settings::toggle(&mut view.data, setting.key);
                self.apply_setting(setting.key, setting.ranking).await;
            }
            KeyCode::Enter => {
                view.editing = Some(Input::new(crate::settings::edit_text(
                    &view.data,
                    setting.key,
                )));
            }
            _ => {}
        }
    }

    /// Saves the changed setting to the config file and starts using it
    async fn apply_setting(&mut self, key: &str, ranking: bool) {
        let Some(view) = self.settings_view.as_mut() else {
            return;
        };
        let saved = match &self.config_path {
            Some(path) => crate::settings::save(path, &view.data, key),
            None => Err("no config file to save to".to_string()),
        };
        let data = view.data.clone();
        view.message = match saved {
            Ok(()) if ranking => "saved, applied to the results".to_string(),
            Ok(()) => "saved, takes effect on the next connect".to_string(),
            Err(e) => e,
        };
        self.soulseek_config = crate::settings::enabled(&data).then(|| data.clone());
        if ranking {
            let _ = self
                .state
                .proxy
                .slsk_set_preferences(
                    data.formats.clone().unwrap_or_default(),
                    data.min_bitrate.unwrap_or(0),
                    data.max_size_mb.unwrap_or(0),
                    data.prefer_free_slots.unwrap_or(true),
                    data.max_concurrent_downloads.unwrap_or(3),
                )
                .await;
        }
    }

    /// Renders soulseek search results and downloads
    fn render_soulseek(&mut self, frame: &mut Frame) {
        if !self.soulseek_visible {
            return;
        }

        let area = frame.area();
        // take right half of screen for soulseek panel
        let slsk_area = Rect {
            x: area.width / 2,
            y: 0,
            width: area.width / 2,
            height: area.height.saturating_sub(7),
        };

        // the panel overlays the music list, wipe what is underneath
        frame.render_widget(Clear, slsk_area);

        let is_selected = matches!(self.region, Region::Soulseek)
            || (self.command_from_soulseek && matches!(self.region, Region::Command));
        let border_for = |focused: bool| {
            if is_selected && focused {
                Style::default().fg(Color::Yellow)
            } else {
                Style::default().fg(Color::Gray)
            }
        };
        let results_focused = self.soulseek_focus == SoulseekFocus::Results;

        // split into results and downloads
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(70), Constraint::Percentage(30)])
            .split(slsk_area);

        // render search results
        self.render_soulseek_results(frame, chunks[0], border_for(results_focused));

        // render downloads
        self.render_soulseek_downloads(frame, chunks[1], border_for(!results_focused));
    }

    fn render_soulseek_results(&mut self, frame: &mut Frame, area: Rect, border_style: Style) {
        let filter = self.soulseek_filter.value();
        let title = if self.soulseek_filtering || !filter.is_empty() {
            format!(
                " soulseek results ({} of {}) · filter: {}{} ",
                self.soulseek_shown.len(),
                self.soulseek_ranked.len(),
                filter,
                if self.soulseek_filtering { "▏" } else { "" }
            )
        } else {
            format!(" soulseek results ({}) ", self.soulseek_ranked.len())
        };

        let widths = [
            Constraint::Percentage(35),
            Constraint::Length(5),
            Constraint::Length(6),
            Constraint::Length(8),
            Constraint::Length(5),
            Constraint::Percentage(20),
        ];
        // same column layout the table computes, to know how much of the name fits
        let inner = area.inner(Margin::new(1, 1));
        let name_width = Layout::horizontal(widths)
            .flex(layout::Flex::Start)
            .spacing(1)
            .split(inner)[0]
            .width as usize;

        // restart the scroll whenever another result gets selected
        let selected_key = self
            .soulseek_shown
            .get(self.soulseek_selected)
            .map(|rf| (rf.username.clone(), rf.file.filename.clone()));
        if self.soulseek_marquee.0 != selected_key {
            self.soulseek_marquee = (selected_key, std::time::Instant::now());
        }
        let scroll_name = self.soulseek_focus == SoulseekFocus::Results;
        let elapsed = self.soulseek_marquee.1.elapsed().as_secs_f64();
        let selected = self.soulseek_selected;

        let rows: Vec<Row> = self
            .soulseek_shown
            .iter()
            .enumerate()
            .map(|(i, rf)| {
                let format = rf.file.extension().unwrap_or("?");
                let bitrate = if rf.file.bitrate > 0 {
                    format!("{}k", rf.file.bitrate)
                } else {
                    "-".to_string()
                };
                let size_mb = rf.file.size as f64 / 1_048_576.0;
                let score = rf.quality_score;

                let name = if scroll_name && i == selected {
                    marquee(rf.file.basename(), name_width, elapsed)
                } else {
                    rf.file.basename().to_string()
                };

                Row::new(vec![
                    Cell::from(name),
                    Cell::from(format.to_uppercase()),
                    Cell::from(bitrate),
                    Cell::from(format!("{:.1}MB", size_mb)),
                    Cell::from(format!("{}", score)),
                    Cell::from(rf.username.clone()),
                ])
            })
            .collect();

        let table = Table::new(rows, widths)
            .header(
                Row::new(vec!["name", "fmt", "kbps", "size", "score", "user"])
                    .style(Style::default().add_modifier(Modifier::BOLD)),
            )
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(border_style)
                    .title(title)
                    .title_bottom(
                        Line::from(" Enter get · a album · / filter · Tab · Esc ")
                            .dim()
                            .right_aligned(),
                    ),
            )
            .highlight_style(self.soulseek_highlight(true));

        self.soulseek_results_state
            .select((!self.soulseek_shown.is_empty()).then_some(self.soulseek_selected));
        frame.render_stateful_widget(table, area, &mut self.soulseek_results_state);
        // nothing searched yet: say how
        if self.soulseek_ranked.is_empty() {
            let inner = Rect::new(area.x + 2, area.y + 3, area.width.saturating_sub(4), area.height.saturating_sub(4));
            let account = self
                .soulseek_config
                .as_ref()
                .is_some_and(|c| c.username.as_deref().is_some_and(|u| !u.is_empty()));
            let mut lines = vec![];
            if !account {
                lines.push(Line::from(vec![
                    Span::from("First time: ").bold(),
                    Span::from(":set").bold().cyan(),
                    Span::from(" → username and password (any new name creates an account)"),
                ]));
                lines.push(Line::from(""));
            }
            lines.extend([
                Line::from(vec![Span::from(":slsk <artist or title>").bold().cyan(), Span::from("  search the Soulseek network")]),
                Line::from("Results stream in over a few seconds, best quality first."),
                Line::from("Downloads land in the library and show up in the list."),
            ]);
            Paragraph::new(lines).wrap(Wrap { trim: false }).render(inner, frame.buffer_mut());
        }
    }

    /// Highlight style of the selected row, only visible in the focused list
    fn soulseek_highlight(&self, results: bool) -> Style {
        let focused = (self.soulseek_focus == SoulseekFocus::Results) == results;
        if focused {
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        }
    }

    fn render_soulseek_downloads(&mut self, frame: &mut Frame, area: Rect, border_style: Style) {
        let title = format!(
            " downloads ({}) {} ",
            self.soulseek_download_list.len(),
            self.soulseek_status
        );

        let rows: Vec<Row> = self
            .soulseek_download_list
            .iter()
            .map(|dl| {
                let (state_str, style) = match dl.state {
                    DownloadState::Pending => ("pending", Style::default().fg(Color::Gray)),
                    DownloadState::Queued => ("queued", Style::default().fg(Color::Blue)),
                    DownloadState::Connecting => ("connecting", Style::default().fg(Color::Yellow)),
                    DownloadState::Downloading => ("downloading", Style::default().fg(Color::Cyan)),
                    DownloadState::Complete => ("complete", Style::default().fg(Color::Green)),
                    DownloadState::Failed => ("failed", Style::default().fg(Color::Red)),
                    DownloadState::Cancelled => ("cancelled", Style::default().fg(Color::Red)),
                };

                let progress = if dl.state == DownloadState::Downloading {
                    dl.progress_percent()
                } else if dl.state == DownloadState::Complete {
                    "100%".to_string()
                } else {
                    "-".to_string()
                };

                // progress bar
                let bar_width = 10;
                let filled = ((dl.progress() * bar_width as f64) as usize).min(bar_width);
                let bar: String =
                    format!("[{}{}]", "=".repeat(filled), " ".repeat(bar_width - filled));

                Row::new(vec![
                    Cell::from(dl.display_name.clone()),
                    Cell::from(state_str),
                    Cell::from(bar),
                    Cell::from(progress),
                    Cell::from(dl.retry_info()),
                ])
                .style(style)
            })
            .collect();

        let widths = [
            Constraint::Percentage(40),
            Constraint::Length(12),
            Constraint::Length(12),
            Constraint::Length(7),
            Constraint::Fill(1),
        ];

        let table = Table::new(rows, widths)
            .header(
                Row::new(vec!["file", "state", "progress", "%", "source"])
                    .style(Style::default().add_modifier(Modifier::BOLD)),
            )
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(border_style)
                    .title(title)
                    .title_bottom(
                        Line::from(" x/dd remove · J/K reorder · Tab ")
                            .dim()
                            .right_aligned(),
                    ),
            )
            .highlight_style(self.soulseek_highlight(false));

        self.soulseek_downloads_state.select(
            (!self.soulseek_download_list.is_empty()).then_some(self.soulseek_download_selected),
        );
        frame.render_stateful_widget(table, area, &mut self.soulseek_downloads_state);
    }

    /// Handles music selection in the music list
    pub async fn update_music_selection_state(&mut self) {
        self.music_list.state.select(Some(self.music_list.selected));
    }

    /// Selectes the upper element in the music list (goes up by 1)
    pub fn list_up(&mut self) {
        let quesize = self.music_list.musics.len();
        let selected_index = self.music_list.selected;
        if selected_index == 0 {
            self.music_list.selected = quesize.saturating_sub(1);
        } else {
            self.music_list.selected -= 1;
        }
    }

    /// Select the next element in the music list (goes down by 1)
    pub fn list_down(&mut self) {
        let quesize = self.music_list.musics.len();
        let selected_index = self.music_list.selected;

        if Some(selected_index) == quesize.checked_sub(1) {
            self.music_list.selected = 0;
        } else {
            self.music_list.selected += 1;
        }
    }

    /// Appends the char to the existing search buffer
    pub async fn register_search_query(&mut self, c: KeyEvent) {
        self.search_bufr
            .handle_event(&crossterm::event::Event::Key(c));
        self.music_list.search(self.search_bufr.value().to_owned());
    }

    /// Appends the char to the existing command buffer
    /// Empties the command input and forgets the last command error
    fn clear_command(&mut self) {
        self.command_bufr.reset();
        self.command_error = None;
    }

    pub async fn register_command_query(&mut self, c: KeyEvent) {
        self.command_bufr
            .handle_event(&crossterm::event::Event::Key(c));
    }

    /// Resets the search querry to an empty string
    pub fn reset_querry(&mut self) {
        self.music_list.reset_search();
    }

    /// Selects the first element in the music list
    pub fn goto_top(&mut self) {
        self.music_list.selected = 0;
    }

    /// Selects the first element in the lyrics list
    pub fn goto_top_lyrics(&mut self) {
        if self.displayed_lyrics.displayable {
            self.displayed_lyrics.state.select(Some(0))
        }
    }

    /// Selects the currently playing song in the music list
    pub fn goto_playing(&mut self) {
        let playing = self.music_list.playing_index;
        if let Some(i) = self.music_list.musics.iter().position(|m| m.index == playing) {
            self.music_list.selected = i;
        }
    }

    /// Selects the last element in the music list
    pub fn goto_bottom(&mut self) {
        self.music_list.selected = self.music_list.musics.len() - 1;
    }

    /// Selects the last element in the lyrics list
    pub fn goto_bottom_lyrics(&mut self) {
        if self.displayed_lyrics.displayable {
            self.displayed_lyrics
                .state
                .select(Some(self.displayed_lyrics.lyrics.lines.len() - 1))
        }
    }

    /// The knob in the normal view: turning steps the volume 2 % per detent, pressing mutes
    pub async fn poll_knob(&mut self) {
        use crate::knob::{KnobAction, Msg};
        let Some(knob) = self.knob.as_ref() else { return };
        for msg in knob.drain() {
            match msg {
                Msg::Note(n) => self.knob_note = Some(n),
                Msg::Action(KnobAction::Up, _) => self.state.set_volume(self.state.volume() + 0.02).await,
                Msg::Action(KnobAction::Down, _) => self.state.set_volume(self.state.volume() - 0.02).await,
                Msg::Action(KnobAction::Press, _) => self.state.toggle_mute().await,
            }
        }
    }

    /// Increases volume
    pub async fn increase_volume(&mut self) {
        self.state.increase_volume().await;
    }

    /// Decreases volume
    pub async fn decrease_volume(&mut self) {
        self.state.decrease_volume().await;
    }

    /// selects the volume reagion in the ui
    pub fn select_volume_region(&mut self) {
        self.region = Region::Volume
    }

    /// Plays the previous music in the list
    async fn play_preivous(&mut self) {
        self.state.play_preivous().await;
    }

    /// Plays the next song in the music list
    async fn play_next(&mut self) {
        self.state.play_next().await;
    }

    /// Plays the next song in the music list
    async fn play_previous(&mut self) {
        self.state.play_preivous().await;
    }

    /// Toggls the playing music
    /// - if playing:
    ///     - pause
    /// - if pausing:
    ///     - resume
    /// - if stopping
    ///     - play
    pub async fn toggle_play(&mut self) {
        // nothing loaded yet: start the highlighted track
        if matches!(self.state.status(), PlayingStatus::Stopped) {
            self.play_selected_music().await;
        } else {
            self.state.toggle_play().await;
        }
    }

    /// derives the displayed lyrics state:  
    /// - highlight index
    async fn derive_lyrics_state(&mut self) {
        let lyrics = self.state.metadata().lyrics;
        // duration since the last action is greater than 2 secs
        if Instant::now().duration_since(self.displayed_lyrics.last_active_interaction)
            > Duration::from_secs(2)
        {
            // music has a lyrics, and the lyrics timestamp is correct
            if self.displayed_lyrics.displayable && lyrics.time_is_correct {
                // get the first line
                lyrics.lines.first().unwrap();
                let current = self.state.played_duration();
                let index = lyrics.lines.binary_search_by(|l| l.timestamp.cmp(&current));
                if let Err(i) = index {
                    if i == 0 || i == 1 {
                        self.displayed_lyrics.state.select(Some(0));
                    } else {
                        self.displayed_lyrics.state.select(Some(i - 1));
                    }
                } else {
                    let index = index.unwrap();
                    if index == 0 || index == 1 {
                        self.displayed_lyrics.state.select(Some(0));
                    } else {
                        self.displayed_lyrics.state.select(Some(index - 1));
                    }
                }
            }
        }
    }

    /// restore the displayable lyrics **widget** state
    async fn restore_displayable_lyrics(&mut self) {
        self.fetch_playlist_data().await;
        self.derive_lyrics().await;
        self.derive_lyrics_state().await;
    }

    /// Restores playing state
    pub async fn restore_state(&mut self) {
        //----------------------------------------------------
        self.state.dbus_streams.paused_stream =
            Some(self.state.proxy.receive_paused().await.unwrap());
        self.state.dbus_streams.resumed_stream =
            Some(self.state.proxy.receive_resumed().await.unwrap());
        self.state.dbus_streams.stopped_stream =
            Some(self.state.proxy.receive_ended().await.unwrap());
        self.state.dbus_streams.music_played_stream =
            Some(self.state.proxy.receive_music_played().await.unwrap());
        self.state.dbus_streams.volume_changed_stream =
            Some(self.state.proxy.receive_volume_changed().await.unwrap());
        self.state.dbus_streams.sorted_stream =
            Some(self.state.proxy.receive_sorted().await.unwrap());
        self.state.dbus_streams.playlist_updated =
            Some(self.state.proxy.receive_playlists_updated().await.unwrap());
        self.state.dbus_streams.playlist_loaded =
            Some(self.state.proxy.receive_playlist_loaded().await.unwrap());
        self.state.dbus_streams.playlist_deleted =
            Some(self.state.proxy.receive_playlist_deleted().await.unwrap());
        self.state.dbus_streams.playlist_created =
            Some(self.state.proxy.receive_playlist_created().await.unwrap());
        self.state.dbus_streams.music_added = self.state.proxy.receive_music_added().await.ok();
        self.state.dbus_streams.library_changed =
            self.state.proxy.receive_library_changed().await.ok();
        //----------------------------------------------------
        //TODO: make this something more readable
        self.displayed_repeat = self.state.get_repeat();
        self.displayed_sort = self.state.get_order();
        let player_state = self.state.proxy.player_status().await.unwrap();
        self.state.batch.volume = player_state.volume.max(0.0).min(1.0);
        self.state.batch.status = player_state.status;
        self.music_list.playing_index = player_state.index as usize;
        //----------------------------------------------------
        self.restore_displayable_lyrics().await;
    }

    pub fn set_frame(&mut self, frame: &mut Frame) {
        self.geometry.frame = frame.area()
    }

    /// Cycles through actions in this orders
    /// - sort action:
    ///     ByTitleAscending -> ByTitleDescending -> ByDurationAscending -> ByDurationDescending -> Shuffle -> ByTitleAscending
    /// - repeat action:
    ///     ThisMusic -> AllMusics -> Dont -> ThisMusic
    pub fn cycle_back(&mut self) {
        // FIXME: prefor sorts?
        match self.action {
            PowerActions::Sort => match self.displayed_sort {
                Sort::ByTitleAscending => self.displayed_sort = Sort::Shuffle,
                Sort::ByTitleDescending => self.displayed_sort = Sort::ByTitleAscending,
                Sort::ByDurationAscending => self.displayed_sort = Sort::ByTitleDescending,
                Sort::ByDurationDescending => self.displayed_sort = Sort::ByDurationAscending,
                Sort::ArtistAscending => self.displayed_sort = Sort::ByDurationDescending,
                Sort::ArtistDescending => self.displayed_sort = Sort::ArtistAscending,
                Sort::Shuffle => self.displayed_sort = Sort::ArtistDescending,
            },
            // ThisMusic -> AllMusics -> Dont
            PowerActions::Repeat => match self.displayed_repeat {
                Repeat::SameMusic => self.displayed_repeat = Repeat::Dont,
                Repeat::AllMusics => self.displayed_repeat = Repeat::SameMusic,
                Repeat::Dont => self.displayed_repeat = Repeat::AllMusics,
            },
            _ => {}
        }
    }

    /// Scrolls through the music list *up* by 7 units
    pub fn scroll_list_up(&mut self) {
        let queue_size = self.music_list.musics.len();
        let selected_index = self.music_list.selected;
        if selected_index == 0 {
            self.music_list.selected = queue_size - 1;
        } else if selected_index.saturating_sub(self.music_list.setp_szie) == 0 {
            self.music_list.selected = 0;
        } else {
            self.music_list.selected -= 7;
        }
    }

    /// Scrolls through the music list *down* by 7 units
    pub fn scroll_list_down(&mut self) {
        let queue_size = self.music_list.musics.len();
        let selected_index = self.music_list.selected;

        if selected_index == queue_size - 1 {
            self.music_list.selected = 0;
        } else if selected_index.saturating_add(self.music_list.setp_szie) >= queue_size {
            self.music_list.selected = queue_size - 1;
        } else {
            self.music_list.selected = self
                .music_list
                .selected
                .saturating_add(self.music_list.setp_szie);
        }
    }

    pub async fn toggle_mute(&self) {
        self.state.toggle_mute().await;
    }

    async fn stop(&self) {
        self.state.end().await;
    }

    fn change_list_mode(&mut self, mode: ListMode) {
        match mode {
            ListMode::Select => {
                self.geometry.disable_search();
                self.geometry.disable_command();
                match self.mode {
                    ListMode::Command => self.swap_list_index_to_normal_from_command(),
                    ListMode::Search => self.swap_list_index_to_normal_from_search(),
                    ListMode::AfterSearch => self.swap_list_index_to_normal_from_aftersearch(),
                    _ => {}
                }
                self.reset_querry()
            }
            ListMode::Search => {
                self.geometry.enable_search();
                self.swap_list_index_to_search_mode();
            }
            ListMode::Command => {
                self.geometry.enable_command();
            }
            _ => {}
        }
        self.mode = mode;
    }

    /// Closes the search and shows the full list again, keeping the cursor on the track that was
    /// highlighted in the results
    /// `j` with a count: stops at the end instead of wrapping around like a single `j`
    fn list_down_clamped(&mut self) {
        if self.music_list.selected + 1 < self.music_list.musics.len() {
            self.music_list.selected += 1;
        }
    }

    fn list_up_clamped(&mut self) {
        self.music_list.selected = self.music_list.selected.saturating_sub(1);
    }

    /// `n` / `N`: the next / previous track matching the last `/` search, wrapping around
    fn search_next(&mut self, forward: bool) {
        let Some(query) = self.last_search.clone() else {
            self.list_note = Some(("no previous search (/ to search)".into(), Instant::now()));
            return;
        };
        let hits: std::collections::HashSet<usize> = crate::search::search_musics(&query, &self.music_list.musics)
            .into_iter()
            .map(|m| m.index)
            .collect();
        let len = self.music_list.musics.len();
        let from = self.music_list.selected;
        let found = (1..=len)
            .map(|step| if forward { (from + step) % len } else { (from + len - step % len) % len })
            .find(|&i| hits.contains(&self.music_list.musics[i].index));
        match found {
            Some(i) => self.music_list.selected = i,
            None => self.list_note = Some((format!("no match for /{query}"), Instant::now())),
        }
    }

    fn leave_search(&mut self) {
        let keep = self
            .music_list
            .musics
            .get(self.music_list.selected)
            .map(|m| m.index);
        self.search_bufr.reset();
        self.reset_querry();
        self.mode = ListMode::Select;
        self.geometry.disable_search();
        self.select_region(Region::List);
        self.music_list.selected = keep
            .and_then(|i| self.music_list.musics.iter().position(|m| m.index == i))
            .unwrap_or(0);
    }

    async fn play_after_search(&mut self) {
        self.play_selected_music().await;
        self.reset_querry();
        self.change_list_mode(ListMode::Select);
    }

    fn quit(&self) -> Result<bool, std::io::Error> {
        Ok(true)
    }

    fn anticipate(&mut self, arg: char) {
        self.anticipation_mode = AncitipationMode::Char(arg);
    }

    // TODO: improve this to download the file respecting the lrc standard
    fn download_lyrics(&self) {
        log(
            &format!("{:#?}", self.state.metadata().lyrics),
            &format!("{}.lrc", self.state.metadata().title),
        )
        .unwrap();
    }

    // TODO: add a way to handle time_is_correct per line instead
    fn render_lyrics(&mut self, frame: &mut Frame) {
        if self.displayed_lyrics.displayable {
            if let Some(geo) = self.geometry.lyrics_geo() {
                let mut lyrics = self.state.metadata().lyrics;
                let mut lines = vec![];
                let widths = [Constraint::Fill(4)];

                for line in lyrics.lines.iter_mut() {
                    lines.push(Row::new(vec![line.content.as_str()]))
                }

                let style = match self.region {
                    Region::Lyrics => Style::new().fg(self.style.lyrics_style.active_region_color),
                    _ => Style::new().fg(self.style.lyrics_style.passive_region_color),
                };

                let block = match self.region {
                    Region::Lyrics => Block::default()
                        .title("Lyrics")
                        .borders(Borders::ALL)
                        .fg(self.style.lyrics_style.active_region_color),
                    _ => Block::default()
                        .title("Lyrics")
                        .borders(Borders::ALL)
                        .fg(self.style.lyrics_style.passive_region_color),
                };

                let table = Table::new(lines, widths)
                    .block(block)
                    .style(style)
                    .highlight_symbol(self.style.lyrics_style.selector.as_str())
                    .highlight_style(
                        Style::new()
                            .add_modifier(Modifier::REVERSED)
                            .fg(self.style.list_style.hilight_color),
                    );
                frame.render_stateful_widget(table, geo, &mut self.displayed_lyrics.state)
            }
        }
    }

    /// updates the lyrics state, by changing the displayble property
    /// according to the music metadata
    async fn update_lyrics_state(&mut self) {
        self.derive_lyrics_state().await;
    }

    async fn derive_lyrics(&mut self) {
        self.displayed_lyrics.lyrics = self.state.metadata().lyrics;
    }

    // TODO: make moving between lyrics as natural as in spotify
    fn select_next_lyrics_line(&mut self) {
        self.displayed_lyrics.state.select_next();
    }

    // TODO: make moving between lyrics as natural as in spotify
    fn select_previous_lyrics_line(&mut self) {
        self.displayed_lyrics.state.select_previous();
    }

    fn select_lyrics_region(&mut self) {
        self.region = Region::Lyrics;
    }

    fn select_region(&mut self, region: Region) {
        self.region = region;
    }

    fn select_playlist_region(&mut self) {
        self.region = Region::Playlist;
    }

    fn toggle_lyrics(&mut self) {
        self.disable_playlists();
        self.displayed_lyrics.displayable = !self.displayed_lyrics.displayable;
        self.geometry.toggle_lyrics();
    }

    fn update_geometry(&mut self, frame: &mut Frame) {
        self.geometry.set_frame(frame.area());
        self.geometry.calculate_gemoetry();
    }

    async fn play_music_from_lyrics(&mut self) {
        if let Some(i) = self.displayed_lyrics.state.selected() {
            if self.state.metadata().lyrics.time_is_correct {
                if let Some(target_time) = self.displayed_lyrics.lyrics.lines.get(i) {
                    self.goto_xs(target_time.timestamp).await;
                }
            }
        }
    }

    fn scroll_lyrics_down(&mut self) {
        let quesize = self.displayed_lyrics.lyrics.lines.len();
        let selected_index = self.displayed_lyrics.state.selected_mut().unwrap_or(0);

        if selected_index == quesize - 1 {
            self.displayed_lyrics.state.select_first();
        } else {
            self.displayed_lyrics.state.select(Some(selected_index + 7));
        }
    }

    fn scroll_lyrics_up(&mut self) {
        let selected_index = self.displayed_lyrics.state.selected().unwrap_or(0);

        if selected_index == 0 {
            self.displayed_lyrics.state.select_last();
        } else {
            self.displayed_lyrics.state.select(Some(selected_index - 7));
        }
    }

    async fn sort(&mut self) {
        self.state.sort(self.displayed_sort).await;
        self.fetch_playlist_data().await;
    }

    async fn repeat(&mut self) {
        self.state.repat(self.displayed_repeat).await;
        self.fetch_repeat().await;
    }

    async fn update_played_duration(&mut self) {
        tokio::select! {
            _ = self.internal_clock.tick() => {
                let time = Box::pin(self.state.proxy.played_duration()).await.unwrap();
                self.state.batch.played_duration =
                    Duration::from_secs_f32(time);
                // an older codjd has no automix: nothing to show
                self.automix = Box::pin(self.state.proxy.automix_state())
                    .await
                    .unwrap_or((false, String::new(), -1., String::new()));
            },
            _ = futures::future::ready(()) => {
            },
        }
    }

    fn cycle(&mut self) {
        match self.action {
            PowerActions::Sort => match self.displayed_sort {
                Sort::ByTitleAscending => self.displayed_sort = Sort::ByTitleDescending,
                Sort::ByTitleDescending => self.displayed_sort = Sort::ByDurationAscending,
                Sort::ByDurationAscending => self.displayed_sort = Sort::ByDurationDescending,
                Sort::ByDurationDescending => self.displayed_sort = Sort::ArtistAscending,
                Sort::ArtistAscending => self.displayed_sort = Sort::ArtistDescending,
                Sort::ArtistDescending => self.displayed_sort = Sort::Shuffle,
                Sort::Shuffle => self.displayed_sort = Sort::ByTitleAscending,
            },
            // ThisMusic -> AllMusics -> Dont
            PowerActions::Repeat => match self.displayed_repeat {
                Repeat::SameMusic => self.displayed_repeat = Repeat::AllMusics,
                Repeat::AllMusics => self.displayed_repeat = Repeat::Dont,
                Repeat::Dont => self.displayed_repeat = Repeat::SameMusic,
            },
            _ => {}
        }
    }

    async fn fetch_repeat(&mut self) {
        self.state.batch.repeat = self.state.proxy.get_repeat().await.unwrap_or_default();
    }

    fn swap_list_index_to_normal_from_command(&mut self) {
        self.music_list.switch_selected_buffer = self.music_list.selected;
        self.music_list.switch_selected_buffer = 0
    }

    fn swap_list_index_to_normal_from_aftersearch(&mut self) {
        self.music_list.switch_selected_buffer = self.music_list.selected;
        self.music_list.selected = 0;
        self.music_list.switch_selected_buffer = 0
    }

    fn swap_list_index_to_normal_from_search(&mut self) {
        self.music_list.switch_selected_buffer = self.music_list.selected;
        self.music_list.selected = 0;
        self.music_list.switch_selected_buffer = 0
    }

    fn swap_list_index_to_search_mode(&mut self) {
        self.music_list.selected = self.music_list.switch_selected_buffer;
        self.music_list.switch_selected_buffer = 0
    }

    fn disable_playlists(&mut self) {
        self.displayed_playlists.displayable = false;
        self.geometry.disable_playlists();
    }

    fn disable_lyrics(&mut self) {
        self.displayed_lyrics.displayable = false;
        self.geometry.disable_lyrics();
    }

    fn toggle_playlists(&mut self) {
        self.disable_lyrics();
        self.displayed_playlists.displayable = !self.displayed_playlists.displayable;
        self.geometry.toggle_playlists();
    }

    fn render_playlists(&mut self, frame: &mut Frame<'_>) {
        if self.displayed_playlists.displayable {
            if let Some(geo) = self.geometry.playlists_geo() {
                let names = self.state.playlist_names();

                let names = names
                    .iter()
                    .map(|v| Row::new(vec![v.as_str()]))
                    .collect::<Vec<Row<'_>>>();

                let widths = [Constraint::Fill(4)];

                let style = match self.region {
                    Region::Playlist => {
                        Style::new().fg(self.style.playlists_style.active_region_color)
                    }
                    _ => Style::new().fg(self.style.playlists_style.passive_region_color),
                };

                let block = match self.region {
                    Region::Playlist => Block::default()
                        .title("Playlists")
                        .title_bottom(" j/k move · Enter open · Tab tracks · Esc close ")
                        .borders(Borders::ALL)
                        .fg(self.style.playlists_style.active_region_color),
                    _ => Block::default()
                        .title("Playlists")
                        .borders(Borders::ALL)
                        .fg(self.style.playlists_style.passive_region_color),
                };

                let table = Table::new(names, widths)
                    .block(block)
                    .style(style)
                    .highlight_symbol(self.style.playlists_style.selector.as_str())
                    .highlight_style(
                        Style::new()
                            .add_modifier(Modifier::REVERSED)
                            .fg(self.style.list_style.hilight_color),
                    );
                frame.render_stateful_widget(table, geo, &mut self.displayed_playlists.state)
            }
        }
    }

    fn select_next_playlist_name(&mut self) {
        self.displayed_playlists.state.select_next();
    }

    fn select_previous_playlist_name(&mut self) {
        self.displayed_playlists.state.select_previous();
    }

    async fn switch_to_selected_playlist(&mut self) {
        if let Some(index) = self.displayed_playlists.state.selected() {
            let titles = self.state.playlist_names();
            if let Some(title) = titles.get(index) {
                let _ = self.state.proxy.use_playlist(title.clone()).await;
            }
        }
    }

    fn render_commands(&self, frame: &mut Frame<'_>) {
        if matches!(self.mode, ListMode::Command) {
            if let Some(geo) = self.geometry.command_geo() {
                frame.render_widget(
                    Paragraph::new(self.command_bufr.value()).block(
                        Block::bordered().title(match &self.command_error {
                            Some(e) => format!("Command ({})", e),
                            None => "Command".to_string(),
                        }),
                    ),
                    geo,
                );
            }
        }
    }

    async fn run_command(&mut self) -> std::io::Result<bool> {
        let command = self.command_bufr.to_string();
        let mut commands: Vec<String> = command.split_whitespace().map(|v| v.to_string()).collect();
        if !commands.is_empty() {
            let mut command_name = commands.remove(0);
            let mut command_args = commands;
            // short forms: `:pl new|add|mv|rm NAME`, and `:pl NAME` opens one
            if matches!(command_name.as_str(), "pl" | "playlists") && !command_args.is_empty() {
                let long = match command_args[0].as_str() {
                    "new" => Some("create_playlist"),
                    "add" => Some("add_to_playlist"),
                    "mv" => Some("rename_playlist"),
                    "rm" => Some("remove_playlist"),
                    _ => None,
                };
                match long {
                    Some(long) => {
                        command_args.remove(0);
                        command_name = long.to_string();
                    }
                    None => {
                        let name = command_args.join(" ");
                        if !self.state.playlist_names().contains(&name) {
                            return Err(std::io::Error::other(format!(
                                "no playlist {name} (:pl new {name} makes it)"
                            )));
                        }
                        let _ = self.state.proxy.use_playlist(name).await;
                        return Ok(false);
                    }
                }
            }
            // DJ-mode commands work from here too: switch to DJ mode and run them there
            if matches!(command_name.as_str(), "tutor" | "patch") {
                self.clear_command();
                self.change_list_mode(ListMode::Select);
                self.leave_command_region();
                self.enter_dj().await;
                if let Some(dj) = self.dj.as_mut() {
                    dj.run_command(command.trim());
                }
                return Ok(false);
            }
            match command_name.as_str() {
                "lyrics" => {
                    self.toggle_lyrics();
                    return Ok(false);
                }
                "playlists" | "pl" => {
                    self.toggle_playlists();
                    if self.displayed_playlists.displayable {
                        self.focus_playlists = true;
                        if self.displayed_playlists.state.selected().is_none() {
                            self.displayed_playlists.state.select(Some(0));
                        }
                    }
                    return Ok(false);
                }
                _ => {}
            }
            // vim's :help opens the key list
            if matches!(command_name.as_str(), "h" | "help") {
                self.help_open = true;
                self.change_list_mode(ListMode::Select);
                self.select_list_region();
                return Ok(false);
            }
            match CommandName::from_str(&command_name) {
                Ok(name) => return self.preform(CommandContext::new(name, command_args)).await,
                Err(_) => {
                    return Err(io::Error::other(format!(
                        "unknown command: {} (:help lists the keys and commands)",
                        command_name
                    )))
                }
            }
        }
        Ok(false)
    }

    fn render_search(&self, frame: &mut Frame<'_>) {
        if matches!(self.mode, ListMode::Search | ListMode::AfterSearch) {
            if let Some(geo) = self.geometry.search_geo() {
                let title = if self.search_bufr.value().trim().is_empty() {
                    "Search (regex, artist:/genre:/title:)".to_string()
                } else if self.music_list.musics.is_empty() {
                    "Search (no matches, Enter: search soulseek)".to_string()
                } else {
                    format!(
                        "Search ({} matches, Ctrl+Enter: search soulseek)",
                        self.music_list.musics.len()
                    )
                };
                frame.render_widget(
                    Paragraph::new(self.search_bufr.value()).block(Block::bordered().title(title)),
                    geo,
                );
            }
        }
    }

    async fn preform(&mut self, context: CommandContext) -> Result<bool, io::Error> {
        let args = context.args;
        // a search query may contain spaces, so it takes every remaining word
        let takes_rest = matches!(context.name, CommandName::SoulseekSearch);
        if !takes_rest && args.len() > context.name.number_of_required_args() {
            return Err(std::io::Error::other("Too many arguments"));
        } else if args.len() < context.name.number_of_required_args() {
            return Err(std::io::Error::other("Few arguments"));
        }
        match context.name {
            CommandName::Play => {
                // guarded by upper if check
                let direction = args.first().unwrap();
                match direction.as_str() {
                    "previous" => self.play_preivous().await,
                    "next" => self.play_next().await,
                    _ => {}
                }
            }
            CommandName::Move => {
                // guarded by upper if check
                let direction = args.first().unwrap();
                match direction.as_str() {
                    "up" => self.list_up(),
                    "down" => self.list_down(),
                    _ => {}
                }
            }
            CommandName::AddToPlaylist => {
                let name = args.first().unwrap();
                self.add_selected_to_playlist(name).await;
            }
            CommandName::Quit => return Ok(true),
            CommandName::CreatePlaylist => {
                let name = args.first().unwrap();
                self.crate_playlist(name).await;
            }
            CommandName::ToggleMute => self.toggle_mute().await,
            CommandName::Pause => self.state.pause_stream(),
            CommandName::Resume => self.state.resume_stream(),
            CommandName::Reload => self.reload().await,
            CommandName::RenamePlaylist => {
                let old = args.first().unwrap();
                let new = args.get(1).unwrap();
                self.rename_playlist(old, new).await;
            }
            CommandName::RemovePlaylist => {
                let name = args.first().unwrap();
                self.remove_playlist(name).await;
            }
            CommandName::SoulseekSearch => {
                if args.is_empty() {
                    // plain `:slsk` opens the panel; return there after the command input
                    self.soulseek_visible = true;
                    self.command_from_soulseek = true;
                } else {
                    let query = args.join(" ");
                    self.soulseek_search(&query).await;
                }
            }
            CommandName::SoulseekConnect => {
                self.soulseek_connect().await;
            }
            CommandName::SoulseekDisconnect => {
                self.soulseek_disconnect().await;
            }
            CommandName::Settings => self.open_settings(),
            CommandName::Dj => self.enter_dj().await,
            CommandName::Automix => {
                let on = match args.first().map(String::as_str) {
                    Some("on") => true,
                    Some("off") => false,
                    _ => !self.automix.0,
                };
                let note = match self.state.proxy.automix(on).await {
                    Ok(r) if r == "ok" => {
                        self.automix.0 = on;
                        if on { "automix on: tracks blend into each other on the beat" } else { "automix off: tracks play to their end" }.to_string()
                    }
                    Ok(r) => format!("automix: {r}"),
                    Err(_) => "automix: this codjd has none, update codjd".to_string(),
                };
                self.list_note = Some((note, Instant::now()));
            }
            CommandName::Autoplay => {
                let current = self.state.proxy.autoplay_state().await.map(|s| s.0).unwrap_or(false);
                let on = match args.first().map(String::as_str) {
                    Some("on") => true,
                    Some("off") => false,
                    _ => !current,
                };
                let note = match self.state.proxy.autoplay(on).await {
                    Ok(r) if r == "ok" => {
                        if on { "autoplay on: when the queue runs low, tracks that blend are queued for you" } else { "autoplay off: the list plays in order" }.to_string()
                    }
                    Ok(r) => format!("autoplay: {r}"),
                    Err(_) => "autoplay: this codjd has none, update codjd".to_string(),
                };
                self.list_note = Some((note, Instant::now()));
            }
            CommandName::Duplicates => {
                self.music_list.show_duplicates = !self.music_list.show_duplicates;
                let selected = self.selected_path();
                self.fetch_playlist_data().await;
                if let Some(i) = selected.and_then(|p| {
                    self.music_list.musics.iter().position(|m| {
                        <&std::path::Path>::from(&m.path) == std::path::Path::new(&p)
                    })
                }) {
                    self.music_list.selected = i;
                }
                let note = if self.music_list.show_duplicates {
                    "showing every copy"
                } else {
                    "hiding duplicate copies"
                };
                self.list_note = Some((note.to_string(), Instant::now()));
            }
        }
        Ok(false)
    }

    async fn soulseek_connect(&mut self) {
        if let Some(config) = &self.soulseek_config {
            // check if already connected
            if let Ok(state) = self.state.proxy.slsk_state().await {
                if state == ConnectionState::Ready {
                    self.soulseek_status = "already connected".to_string();
                    self.soulseek_connected = true;
                    return;
                }
            }

            // get connection params from config
            let username = config.username.clone().unwrap_or_default();
            let password = config.password.clone().unwrap_or_default();
            let download_dir = config
                .download_dir
                .clone()
                .unwrap_or_else(|| {
                    dirs::audio_dir()
                        .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
                        .join("soulseek")
                })
                .to_string_lossy()
                .to_string();

            let server_host = config
                .server_host
                .clone()
                .unwrap_or_else(|| "server.slsknet.org".to_string());
            let server_port = config.server_port.unwrap_or(2242) as u32;
            let listen_port = config.listen_port.unwrap_or(2234) as u32;
            let max_results = config.max_results.unwrap_or(50) as u32;

            self.soulseek_status = "connecting...".to_string();

            match self
                .state
                .proxy
                .slsk_connect(
                    username,
                    password,
                    download_dir,
                    server_host,
                    server_port,
                    listen_port,
                    max_results,
                )
                .await
            {
                Ok(msg) => {
                    self.soulseek_status = msg;
                    self.soulseek_connected = true;
                }
                Err(e) => {
                    self.soulseek_status = format!("connect failed: {}", e);
                    self.soulseek_connected = false;
                }
            }
        } else {
            self.soulseek_status = "no soulseek config - add [soulseek] to config.toml".to_string();
        }
    }

    async fn soulseek_disconnect(&mut self) {
        match self.state.proxy.slsk_disconnect().await {
            Ok(msg) => {
                self.soulseek_status = msg;
            }
            Err(e) => {
                self.soulseek_status = format!("disconnect error: {}", e);
            }
        }
        self.soulseek_connected = false;
        self.soulseek_open_on_results = false;
        self.soulseek_ranked.clear();
        self.soulseek_shown.clear();
        self.soulseek_selected = 0;
        self.soulseek_selection_moved = false;
    }

    async fn soulseek_search(&mut self, query: &str) {
        // connect on first use, using the [soulseek] config. codjd connects in the background;
        // the search runs from poll_soulseek_results once it's logged in
        match self.state.proxy.slsk_state().await {
            Ok(ConnectionState::Ready) => {}
            state => {
                let connecting = matches!(
                    state,
                    Ok(ConnectionState::Connecting
                        | ConnectionState::Connected
                        | ConnectionState::LoggingIn)
                );
                if !connecting {
                    self.soulseek_connect().await;
                }
                if self.soulseek_config.is_some() {
                    self.soulseek_pending_query = Some(query.to_string());
                    self.soulseek_status = format!("connecting... (then searching: {query})");
                }
                return;
            }
        }
        // ranking settings live in the client config, the server ranks with them
        if let Some(cfg) = &self.soulseek_config {
            let _ = self
                .state
                .proxy
                .slsk_set_preferences(
                    cfg.formats.clone().unwrap_or_default(),
                    cfg.min_bitrate.unwrap_or(0),
                    cfg.max_size_mb.unwrap_or(0),
                    cfg.prefer_free_slots.unwrap_or(true),
                    cfg.max_concurrent_downloads.unwrap_or(3),
                )
                .await;
        }
        match self.state.proxy.slsk_search(query.to_string()).await {
            Ok(msg) => {
                self.soulseek_status = msg;
                self.soulseek_ranked.clear();
                self.soulseek_shown.clear();
                self.soulseek_filter.reset();
                self.soulseek_filtering = false;
                self.soulseek_selected = 0;
                self.soulseek_selection_moved = false;
                self.soulseek_open_on_results = true;
            }
            Err(e) => {
                self.soulseek_status = format!("search failed: {}", e);
            }
        }
    }

    pub async fn poll_soulseek_results(&mut self) {
        // a :slsk_connect started earlier: show how it went
        if self.soulseek_pending_query.is_none() && self.soulseek_status == "connecting..." {
            if let Ok(msg) = self.state.proxy.slsk_status().await {
                self.soulseek_status = msg;
            }
        }
        if let Some(query) = self.soulseek_pending_query.clone() {
            match self.state.proxy.slsk_state().await {
                Ok(ConnectionState::Ready) => {
                    self.soulseek_pending_query = None;
                    self.soulseek_search(&query).await;
                }
                Ok(ConnectionState::Connecting
                | ConnectionState::Connected
                | ConnectionState::LoggingIn) => {}
                _ => {
                    // the connect failed: say why
                    self.soulseek_pending_query = None;
                    self.soulseek_connected = false;
                    if let Ok(msg) = self.state.proxy.slsk_status().await {
                        self.soulseek_status = msg;
                    }
                }
            }
        }
        // poll search results from server
        if let Ok(results) = self.state.proxy.slsk_get_results().await {
            self.apply_soulseek_results(results);
        }

        // poll download list from server
        if let Ok(downloads) = self.state.proxy.slsk_get_downloads().await {
            self.apply_soulseek_downloads(downloads);
        }
    }

    /// Replaces the results, keeping the selection on the same (username, filename)
    fn apply_soulseek_results(&mut self, results: Vec<RankedFile>) {
        let shown = filter_results(&results, self.soulseek_filter.value());
        self.soulseek_selected = if self.soulseek_selection_moved {
            anchored_index(
                &self.soulseek_shown,
                &shown,
                self.soulseek_selected,
                same_result,
            )
        } else {
            0
        };
        self.soulseek_shown = shown;
        if !results.is_empty() && results.len() != self.soulseek_ranked.len() {
            self.soulseek_status = format!("found {} files (sorted by quality)", results.len());
        }
        let has_results = !results.is_empty();
        self.soulseek_ranked = results;

        // only open the panel for a search started from this session, and never while typing
        // a command
        if has_results
            && self.soulseek_open_on_results
            && !matches!(self.mode, ListMode::Command | ListMode::Search)
        {
            self.soulseek_open_on_results = false;
            self.soulseek_visible = true;
            self.soulseek_focus = SoulseekFocus::Results;
            self.region = Region::Soulseek;
        }
    }

    /// Replaces the downloads, keeping the selection on the same download id
    fn apply_soulseek_downloads(&mut self, downloads: Vec<Download>) {
        self.soulseek_download_selected = anchored_index(
            &self.soulseek_download_list,
            &downloads,
            self.soulseek_download_selected,
            |a, b| a.id == b.id,
        );
        self.soulseek_download_list = downloads;
    }

    /// Re-applies the name filter after it was edited, keeping the cursor on the same result
    /// when it still matches
    fn soulseek_refilter(&mut self) {
        let shown = filter_results(&self.soulseek_ranked, self.soulseek_filter.value());
        self.soulseek_selected =
            anchored_index(&self.soulseek_shown, &shown, self.soulseek_selected, same_result);
        self.soulseek_shown = shown;
    }

    fn soulseek_switch_focus(&mut self) {
        self.soulseek_focus = match self.soulseek_focus {
            SoulseekFocus::Results => SoulseekFocus::Downloads,
            SoulseekFocus::Downloads => SoulseekFocus::Results,
        };
    }

    /// Returns from the command input to the region it was opened from
    fn leave_command_region(&mut self) {
        if std::mem::take(&mut self.focus_playlists) && self.displayed_playlists.displayable {
            self.select_playlist_region();
        } else if self.command_from_soulseek && self.soulseek_visible {
            self.select_region(Region::Soulseek);
        } else {
            self.select_region(Region::List);
        }
        self.command_from_soulseek = false;
    }

    /// Selection and length of the focused soulseek list
    fn soulseek_focused(&mut self) -> (&mut usize, usize) {
        if self.soulseek_focus == SoulseekFocus::Results {
            self.soulseek_selection_moved = true;
        }
        match self.soulseek_focus {
            SoulseekFocus::Results => (&mut self.soulseek_selected, self.soulseek_shown.len()),
            SoulseekFocus::Downloads => (
                &mut self.soulseek_download_selected,
                self.soulseek_download_list.len(),
            ),
        }
    }

    fn soulseek_up(&mut self) {
        let (sel, _) = self.soulseek_focused();
        *sel = sel.saturating_sub(1);
    }

    fn soulseek_down(&mut self) {
        let (sel, len) = self.soulseek_focused();
        *sel = (*sel + 1).min(len.saturating_sub(1));
    }

    fn soulseek_goto_bottom(&mut self) {
        let (sel, len) = self.soulseek_focused();
        *sel = len.saturating_sub(1);
    }

    fn soulseek_scroll_up(&mut self) {
        let (sel, _) = self.soulseek_focused();
        *sel = sel.saturating_sub(10);
    }

    fn soulseek_scroll_down(&mut self) {
        let (sel, len) = self.soulseek_focused();
        *sel = (*sel + 10).min(len.saturating_sub(1));
    }

    async fn soulseek_download_selected(&mut self) {
        if self.soulseek_focus != SoulseekFocus::Results {
            return;
        }
        let Some(rf) = self.soulseek_shown.get(self.soulseek_selected) else {
            return;
        };
        let (username, filename) = (rf.username.clone(), rf.file.filename.clone());
        match self.state.proxy.slsk_download(username, filename).await {
            Ok(msg) => {
                self.soulseek_status = msg;
            }
            Err(e) => {
                self.soulseek_status = format!("download error: {}", e);
            }
        }
    }

    /// `a`: queue every file the selected result's user has in the same folder (the album)
    async fn soulseek_download_folder_selected(&mut self) {
        if self.soulseek_focus != SoulseekFocus::Results {
            self.soulseek_status = "Tab to the results, then a to download the folder".to_string();
            return;
        }
        let Some(rf) = self.soulseek_shown.get(self.soulseek_selected) else {
            return;
        };
        let (username, filename) = (rf.username.clone(), rf.file.filename.clone());
        self.soulseek_status = match self.state.proxy.slsk_download_folder(username, filename).await {
            Ok(msg) => msg,
            Err(e) => format!("download error: {}", e),
        };
    }

    /// Shift+J/K: move the selected download down/up; waiting downloads start in list order.
    /// The selection follows it (anchored by id) when the list is refreshed.
    async fn soulseek_move_download(&mut self, up: bool) {
        if self.soulseek_focus != SoulseekFocus::Downloads {
            return;
        }
        let Some(id) = self
            .soulseek_download_list
            .get(self.soulseek_download_selected)
            .map(|dl| dl.id)
        else {
            return;
        };
        let _ = self.state.proxy.slsk_move_download(id, up).await;
        if let Ok(downloads) = self.state.proxy.slsk_get_downloads().await {
            self.apply_soulseek_downloads(downloads);
        }
    }

    /// `x` / `dd`: remove the selected download (cancelling it if it's still running)
    async fn soulseek_remove_key(&mut self) {
        if self.soulseek_focus != SoulseekFocus::Downloads {
            self.soulseek_status = "Tab to the downloads list, then x or dd to remove".to_string();
            return;
        }
        let Some(id) = self
            .soulseek_download_list
            .get(self.soulseek_download_selected)
            .map(|dl| dl.id)
        else {
            return;
        };
        match self.state.proxy.slsk_remove_download(id).await {
            Ok(msg) => self.soulseek_status = msg,
            Err(e) => self.soulseek_status = format!("remove error: {}", e),
        }
        // refresh now so the row disappears without waiting for the next poll
        if let Ok(downloads) = self.state.proxy.slsk_get_downloads().await {
            self.apply_soulseek_downloads(downloads);
        }
    }

    async fn soulseek_clear_finished(&mut self) {
        match self.state.proxy.slsk_clear_finished().await {
            Ok(msg) => {
                self.soulseek_status = msg;
            }
            Err(e) => {
                self.soulseek_status = format!("clear error: {}", e);
            }
        }
    }

    fn toggle_soulseek(&mut self) {
        self.soulseek_visible = !self.soulseek_visible;
        if self.soulseek_visible {
            self.region = Region::Soulseek;
        } else {
            self.region = Region::List;
        }
    }

    async fn add_selected_to_playlist(&self, destination: &String) {
        let index = self
            .music_list
            .musics
            .get(self.music_list.selected)
            .unwrap()
            .index;

        let source = self.state.proxy.get_playlist_name().await.unwrap();

        self.state
            .proxy
            .save_to_playlist(index, source.clone(), destination.clone())
            .await
            .unwrap();
    }

    async fn crate_playlist(&self, name: &String) {
        self.state
            .proxy
            .create_playlist(name.clone())
            .await
            .unwrap();
    }

    async fn rename_playlist(&self, old: &str, new: &str) {
        self.state.proxy.rename_playlist(old, new).await.unwrap();
    }

    async fn remove_playlist(&self, name: &str) {
        self.state.proxy.remove_playlist(name).await.unwrap();
    }

    async fn reload(&self) {
        self.state.proxy.reload_config().await.unwrap();
    }
}

#[derive(PartialEq, Eq, Debug, Clone)]
pub struct DisplayedLyrics {
    /// Used to disable lyrics, feched from config or changed
    /// via a keybind
    displayable: bool,
    /// State/Index of the lyrics list pointer
    state: TableState,
    /// Lyrics
    lyrics: Lyrics,
    /// Used to sync current lyrics line with the audio
    pub last_active_interaction: Instant,
}

#[derive(Debug, Clone, Default)]
pub struct DisplayedPlaylists {
    /// Used to toggle playlists view
    displayable: bool,
    /// State/Index of the playlist list pointer
    state: TableState,
}

impl Default for DisplayedLyrics {
    fn default() -> Self {
        Self {
            displayable: false,
            state: TableState::new(),
            lyrics: Lyrics::default(),
            last_active_interaction: Instant::now().checked_add(Duration::from_secs(1)).unwrap(),
        }
    }
}

#[derive(Default, Debug)]
pub struct Musics {
    /// weather we should display lyrics or not
    pub display_genre: bool,
    /// displayed music list
    /// TODO: change this to a HashMap
    pub musics: Vec<Music>,
    /// index of the currently *selected* music in the displayed music list
    pub selected: usize,
    /// Buffer used to switch between selected index values
    /// Holds the previously selected index
    /// search/aftersearch <===> normal
    pub switch_selected_buffer: usize,
    pub state: TableState,
    /// full music list (not filtered)
    pub unfiltered_music_list: Vec<Music>,
    /// playlist index (`Music::index`) of the currently playing song
    playing_index: usize,
    pub setp_szie: usize,
    /// list every copy of a song instead of only the best one (`:dupes` toggles)
    pub show_duplicates: bool,
    /// copies left out of the list
    pub hidden_duplicates: usize,
}

impl Musics {
    /// search the music list
    pub fn search(&mut self, search_bufr: String) {
        self.musics = crate::search::search_musics(&search_bufr, &self.unfiltered_music_list);
    }

    pub fn reset_search(&mut self) {
        self.musics = self.unfiltered_music_list.clone();
    }
}

#[derive(Default)]
pub struct PowerBar {
    current_timer: Duration,
}

#[derive(Debug)]
pub enum PowerActions {
    TogglePlay,
    ForwardSkip,
    BackwardSkip,
    Repeat,
    Sort,
    Stop,
}

#[derive(Deserialize, Serialize, Clone, Copy)]
enum Kmodifier {
    SHIFT,
    CONTROL,
    ALT,
    SUPER,
    HYPER,
    META,
    NONE,
}

impl From<KeyModifiers> for Kmodifier {
    fn from(value: KeyModifiers) -> Self {
        match value {
            KeyModifiers::SHIFT => Kmodifier::SHIFT,
            KeyModifiers::CONTROL => Kmodifier::CONTROL,
            KeyModifiers::ALT => Kmodifier::ALT,
            KeyModifiers::SUPER => Kmodifier::SUPER,
            KeyModifiers::HYPER => Kmodifier::HYPER,
            KeyModifiers::META => Kmodifier::META,
            KeyModifiers::NONE => Kmodifier::NONE,
            _ => todo!(),
        }
    }
}

#[derive(Deserialize, Serialize, Clone, Copy)]
enum KCode {
    /// Backspace key.
    Backspace,
    /// Enter key.
    Enter,
    /// Left arrow key.
    Left,
    /// Right arrow key.
    Right,
    /// Up arrow key.
    Up,
    /// Down arrow key.
    Down,
    /// Home key.
    Home,
    /// End key.
    End,
    /// Page up key.
    PageUp,
    /// Page down key.
    PageDown,
    /// Tab key.
    Tab,
    /// Shift + Tab key.
    BackTab,
    /// Delete key.
    Delete,
    /// Insert key.
    Insert,
    /// F key.
    ///
    /// `KeyCode::F(1)` represents F1 key, etc.
    F(u8),
    /// A character.
    ///
    /// `KeyCode::Char('c')` represents `c` character, etc.
    Char(char),
    /// Null.
    Null,
    /// Escape key.
    Esc,
    /// Caps Lock key.
    ///
    /// **Note:** this key can only be read if
    /// [`KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES`] has been enabled with
    /// [`PushKeyboardEnhancementFlags`].
    CapsLock,
    /// Scroll Lock key.
    ///
    /// **Note:** this key can only be read if
    /// [`KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES`] has been enabled with
    /// [`PushKeyboardEnhancementFlags`].
    ScrollLock,
    /// Num Lock key.
    ///
    /// **Note:** this key can only be read if
    /// [`KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES`] has been enabled with
    /// [`PushKeyboardEnhancementFlags`].
    NumLock,
    /// Print Screen key.
    ///
    /// **Note:** this key can only be read if
    /// [`KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES`] has been enabled with
    /// [`PushKeyboardEnhancementFlags`].
    PrintScreen,
    /// Pause key.
    ///
    /// **Note:** this key can only be read if
    /// [`KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES`] has been enabled with
    /// [`PushKeyboardEnhancementFlags`].
    Pause,
    /// Menu key.
    ///
    /// **Note:** this key can only be read if
    /// [`KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES`] has been enabled with
    /// [`PushKeyboardEnhancementFlags`].
    Menu,
    /// The "Begin" key (often mapped to the 5 key when Num Lock is turned on).
    ///
    /// **Note:** this key can only be read if
    /// [`KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES`] has been enabled with
    /// [`PushKeyboardEnhancementFlags`].
    KeypadBegin,
    /// A media key.
    ///
    /// **Note:** these keys can only be read if
    /// [`KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES`] has been enabled with
    /// [`PushKeyboardEnhancementFlags`].
    Media(MediaKey),
    /// A modifier key.
    ///
    /// **Note:** these keys can only be read if **both**
    /// [`KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES`] and
    /// [`KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES`] have been enabled with
    /// [`PushKeyboardEnhancementFlags`].
    Modifier(ModifierKey),
}

#[derive(Deserialize, Serialize, Clone, Copy)]
enum MediaKey {
    /// Play media key.
    Play,
    /// Pause media key.
    Pause,
    /// Play/Pause media key.
    PlayPause,
    /// Reverse media key.
    Reverse,
    /// Stop media key.
    Stop,
    /// Fast-forward media key.
    FastForward,
    /// Rewind media key.
    Rewind,
    /// Next-track media key.
    TrackNext,
    /// Previous-track media key.
    TrackPrevious,
    /// Record media key.
    Record,
    /// Lower-volume media key.
    LowerVolume,
    /// Raise-volume media key.
    RaiseVolume,
    /// Mute media key.
    MuteVolume,
}

#[derive(Deserialize, Serialize, Clone, Copy)]
pub enum ModifierKey {
    /// Left Shift key.
    LeftShift,
    /// Left Control key.
    LeftControl,
    /// Left Alt key.
    LeftAlt,
    /// Left Super key.
    LeftSuper,
    /// Left Hyper key.
    LeftHyper,
    /// Left Meta key.
    LeftMeta,
    /// Right Shift key.
    RightShift,
    /// Right Control key.
    RightControl,
    /// Right Alt key.
    RightAlt,
    /// Right Super key.
    RightSuper,
    /// Right Hyper key.
    RightHyper,
    /// Right Meta key.
    RightMeta,
    /// Iso Level3 Shift key.
    IsoLevel3Shift,
    /// Iso Level5 Shift key.
    IsoLevel5Shift,
}

impl From<KeyCode> for KCode {
    fn from(value: KeyCode) -> Self {
        match value {
            KeyCode::Backspace => KCode::Backspace,
            KeyCode::Enter => KCode::Enter,
            KeyCode::Left => KCode::Left,
            KeyCode::Right => KCode::Right,
            KeyCode::Up => KCode::Up,
            KeyCode::Down => KCode::Down,
            KeyCode::Home => KCode::Home,
            KeyCode::End => KCode::End,
            KeyCode::PageUp => KCode::PageUp,
            KeyCode::PageDown => KCode::PageDown,
            KeyCode::Tab => KCode::Tab,
            KeyCode::BackTab => KCode::BackTab,
            KeyCode::Delete => KCode::Delete,
            KeyCode::Insert => KCode::Insert,
            KeyCode::F(k) => KCode::F(k),
            KeyCode::Char(c) => KCode::Char(c),
            KeyCode::Null => KCode::Null,
            KeyCode::Esc => KCode::Esc,
            KeyCode::CapsLock => KCode::CapsLock,
            KeyCode::ScrollLock => KCode::ScrollLock,
            KeyCode::NumLock => KCode::Null,
            KeyCode::PrintScreen => KCode::PrintScreen,
            KeyCode::Pause => KCode::Pause,
            KeyCode::Menu => KCode::Menu,
            KeyCode::KeypadBegin => KCode::KeypadBegin,
            KeyCode::Media(_) => todo!(),
            KeyCode::Modifier(_) => todo!(),
        }
    }
}

impl KCode {}

#[derive(Deserialize, Serialize)]
struct Event {
    modifier: Kmodifier,
    code: KCode,
    region: Region,
    list_mode: Option<ListMode>,
}

impl Event {
    fn modifier(&self) -> Kmodifier {
        self.modifier
    }

    fn code(&self) -> KCode {
        self.code
    }

    fn region(&self) -> Region {
        self.region.clone()
    }
}

impl UserData for Event {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("modifier", |_lua, this, ()| {
            Ok(_lua.to_value(&this.modifier()))
        });
        methods.add_method("code", |_lua, this, ()| Ok(_lua.to_value(&this.code())));
        methods.add_method("region", |_lua, this, ()| Ok(_lua.to_value(&this.region())));
    }
}

impl Event {
    fn new(modifier: Kmodifier, code: KCode, region: Region, list_mode: Option<ListMode>) -> Self {
        Self {
            modifier,
            code,
            region,
            list_mode,
        }
    }
}

#[cfg(test)]
mod soulseek_tests {
    use super::*;
    use crate::soulseek_types::SharedFile;

    #[test]
    fn filter_results_by_name_regex() {
        let all = vec![
            rf("a", "Music\\Aphex Twin\\01 Xtal.flac"),
            rf("b", "x/02 Tha.mp3"),
            rf("c", "Aphex\\03 Pulsewidth.m4a"),
        ];
        assert_eq!(filter_results(&all, "").len(), 3);
        // matches the file name only, not the folders
        assert_eq!(filter_results(&all, "aphex").len(), 0);
        let hits = filter_results(&all, "xtal|PULSE");
        assert_eq!(hits.iter().map(|r| r.username.as_str()).collect::<Vec<_>>(), vec!["a", "c"]);
        // half-typed regex is matched literally
        assert!(filter_results(&all, "(").is_empty());
    }

    fn dl(state: DownloadState, attempt: u32, err: &str) -> Download {
        Download {
            id: 1,
            username: "carol".into(),
            remote_path: "a\\b.flac".into(),
            local_path: "/tmp/b.flac".into(),
            size: 10,
            downloaded: 0,
            state,
            quality_score: 0,
            display_name: "b.flac".into(),
            token: 0,
            attempt,
            max_attempts: 3,
            last_error: err.into(),
        }
    }

    #[test]
    fn download_signature_matches_codjd() {
        // codjd's Download over D-Bus; both ends must change together
        assert_eq!(
            <Download as zbus::zvariant::Type>::SIGNATURE.to_string(),
            "(usssttuusuuus)"
        );
    }

    #[test]
    fn download_retry_info() {
        assert_eq!(dl(DownloadState::Downloading, 0, "").retry_info(), "");
        assert_eq!(dl(DownloadState::Failed, 0, "denied: Banned").retry_info(), "denied: Banned");
        assert_eq!(
            dl(DownloadState::Queued, 2, "peer connection failed").retry_info(),
            "retry 2/3 from carol"
        );
        assert_eq!(
            dl(DownloadState::Failed, 3, "upload failed").retry_info(),
            "retry 3/3 from carol: upload failed"
        );
    }

    #[test]
    fn marquee_short_name_does_not_scroll() {
        assert_eq!(marquee("short.mp3", 20, 5.0), "short.mp3");
    }

    #[test]
    fn marquee_holds_scrolls_and_wraps() {
        let name = "0123456789abcdef"; // 16 chars in 10 columns: 6 to scroll, 1s at 6/s
        assert_eq!(marquee(name, 10, 0.5), "0123456789"); // hold at start
        assert_eq!(marquee(name, 10, 1.5), "3456789abc"); // halfway
        assert_eq!(marquee(name, 10, 2.5), "6789abcdef"); // hold at end
        assert_eq!(marquee(name, 10, 3.5), "0123456789"); // starts over
    }

    #[test]
    fn marquee_handles_multibyte() {
        let s = marquee("ØTTA - Ünïcødé name.flac", 8, 10.0);
        assert_eq!(s.chars().count(), 8);
    }

    fn rf(user: &str, name: &str) -> RankedFile {
        RankedFile {
            file: SharedFile {
                filename: name.to_string(),
                ..Default::default()
            },
            username: user.to_string(),
            free_slots: true,
            upload_speed: 0,
            queue_size: 0,
            quality_score: 0,
        }
    }

    #[test]
    fn selection_follows_item_when_resorted() {
        let old = vec![rf("a", "1"), rf("b", "2"), rf("c", "3")];
        let new = vec![rf("c", "3"), rf("x", "9"), rf("a", "1"), rf("b", "2")];
        assert_eq!(anchored_index(&old, &new, 1, same_result), 3);
    }

    #[test]
    fn same_filename_other_user_is_different() {
        let old = vec![rf("a", "1"), rf("b", "1")];
        let new = vec![rf("b", "1"), rf("a", "1")];
        assert_eq!(anchored_index(&old, &new, 1, same_result), 0);
    }

    #[test]
    fn missing_item_clamps_to_length() {
        let old = vec![rf("a", "1"), rf("b", "2"), rf("c", "3")];
        let new = vec![rf("a", "1")];
        assert_eq!(anchored_index(&old, &new, 2, same_result), 0);
        let new = vec![rf("a", "1"), rf("d", "4")];
        assert_eq!(anchored_index(&old, &new, 2, same_result), 1);
    }

    #[test]
    fn empty_lists_never_out_of_range() {
        let old: Vec<RankedFile> = vec![];
        let new = vec![rf("a", "1")];
        assert_eq!(anchored_index(&old, &new, 5, same_result), 0);
        assert_eq!(anchored_index(&new, &old, 0, same_result), 0);
        assert_eq!(anchored_index(&old, &old, 3, same_result), 0);
    }

    #[test]
    fn download_selection_anchored_by_id() {
        let old = vec![(1u32, 'a'), (2, 'b'), (3, 'c')];
        let new = vec![(3u32, 'c'), (2, 'b')];
        assert_eq!(anchored_index(&old, &new, 1, |a, b| a.0 == b.0), 1);
        assert_eq!(anchored_index(&old, &new, 0, |a, b| a.0 == b.0), 0);
    }
}

fn repeat_label(repeat: &Repeat) -> &'static str {
    match repeat {
        Repeat::SameMusic => "repeat one",
        Repeat::AllMusics => "repeat all",
        Repeat::Dont => "no repeat",
    }
}

fn sort_label(sort: &Sort) -> &'static str {
    match sort {
        Sort::ByTitleAscending => "title ↑",
        Sort::ByTitleDescending => "title ↓",
        Sort::ByDurationAscending => "length ↑",
        Sort::ByDurationDescending => "length ↓",
        Sort::ArtistAscending => "artist ↑",
        Sort::ArtistDescending => "artist ↓",
        Sort::Shuffle => "shuffle",
    }
}

/// The play bar's note of the coming transition: `next: CRIA · blend in 1:12`, `mixing into CRIA ·
/// blend`; None when automix is off or nothing is coming
pub(crate) fn automix_label((on, title, secs, what): &(bool, String, f64, String)) -> Option<String> {
    if !on || title.is_empty() || *secs < 0. {
        return None;
    }
    // "blend · 32 bars" -> "blend"
    let what = what.split(" · ").next().unwrap_or(what);
    if *secs <= 0. {
        return Some(format!("mixing into {title} · {what}"));
    }
    let s = secs.ceil() as u64;
    Some(format!("next: {title} · {what} in {}:{:02}", s / 60, s % 60))
}

#[cfg(test)]
mod automix_label_tests {
    use super::automix_label;

    #[test]
    fn says_what_comes_and_when() {
        let st = |on, t: &str, s, w: &str| (on, t.to_string(), s, w.to_string());
        assert_eq!(automix_label(&st(true, "CRIA", 71.2, "blend · 32 bars")).as_deref(), Some("next: CRIA · blend in 1:12"));
        assert_eq!(automix_label(&st(true, "CRIA", 0., "crossfade · 8 s")).as_deref(), Some("mixing into CRIA · crossfade"));
        assert_eq!(automix_label(&st(true, "", -1., "")), None);
        assert_eq!(automix_label(&st(false, "CRIA", 10., "blend · 32 bars")), None);
    }
}

/// `:` commands, for completion (the main name of each; aliases still work when typed)
const COMMANDS: &[&str] = &[
    "dj", "automix", "autoplay", "tutor", "patch", "slsk", "set", "dupes", "help", "lyrics", "playlists", "mute", "q", "reload", "pause", "resume", "toggle_mute",
    "create_playlist", "add_to_playlist", "rename_playlist", "remove_playlist", "slsk_connect",
    "slsk_disconnect", "pl new ", "pl add ", "pl mv ", "pl rm ",
];

/// Commands starting with `typed`, in a stable order
fn complete_command(typed: &str) -> Vec<&'static str> {
    COMMANDS.iter().copied().filter(|c| c.starts_with(typed)).collect()
}

#[cfg(test)]
mod completion_tests {
    use super::*;

    #[test]
    fn completes_command_names() {
        assert_eq!(complete_command("d"), ["dj", "dupes"]);
        assert_eq!(complete_command("au"), ["automix", "autoplay"]);
        assert_eq!(complete_command("t"), ["tutor", "toggle_mute"]);
        assert_eq!(complete_command("sl"), ["slsk", "slsk_connect", "slsk_disconnect"]);
        assert!(complete_command("zz").is_empty());
        assert_eq!(complete_command("pl"), ["playlists", "pl new ", "pl add ", "pl mv ", "pl rm "]);
        assert_eq!(complete_command("pl m"), ["pl mv "]);
        assert!(complete_command("pl new warmup").is_empty(), "a name is not completed");
        assert_eq!(complete_command("").len(), COMMANDS.len());
    }
}
