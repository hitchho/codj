use std::str::FromStr;

pub struct CommandContext {
    pub name: CommandName,
    pub args: Vec<String>,
}

impl<'a> CommandContext {
    pub fn new(name: CommandName, args: Vec<String>) -> Self {
        Self { name, args }
    }
}

#[derive(Debug)]
pub enum CommandName {
    Move,
    Play,
    Pause,
    Resume,
    Reload,
    AddToPlaylist,
    CreatePlaylist,
    Quit,
    ToggleMute,
    RenamePlaylist,
    RemovePlaylist,
    SoulseekSearch,
    SoulseekConnect,
    SoulseekDisconnect,
    Settings,
    Duplicates,
    Dj,
    /// `:automix [on|off]`: the player's beat-matched transitions
    Automix,
}

impl CommandName {
    pub fn number_of_required_args(&self) -> usize {
        match self {
            // no arguments
            CommandName::Pause => 0,
            // no arguments
            CommandName::Resume => 0,
            // no arguments
            CommandName::Quit => 0,
            // no arguments
            CommandName::ToggleMute => 0,
            // no arguments
            CommandName::Reload => 0,
            // direction up/down
            CommandName::Move => 1,
            CommandName::Play => 1,
            // playlist name
            CommandName::AddToPlaylist => 1,
            // playlist name
            CommandName::CreatePlaylist => 1,
            // Old and New names
            CommandName::RenamePlaylist => 2,
            // Playlist name
            CommandName::RemovePlaylist => 1,
            // search query
            // no query just opens the panel
            CommandName::SoulseekSearch => 0,
            // no arguments
            CommandName::SoulseekConnect => 0,
            // no arguments
            CommandName::SoulseekDisconnect => 0,
            CommandName::Settings => 0,
            CommandName::Duplicates => 0,
            CommandName::Dj => 0,
            CommandName::Automix => 0,
        }
    }
}

impl FromStr for CommandName {
    type Err = std::io::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "move" => Ok(Self::Move),
            "play" => Ok(Self::Play),
            "add_to_playlist" => Ok(Self::AddToPlaylist),
            "create_playlist" => Ok(Self::CreatePlaylist),
            "rename_playlist" => Ok(Self::RenamePlaylist),
            "remove_playlist" => Ok(Self::RemovePlaylist),
            "toggle_mute" | "mute" => Ok(Self::ToggleMute),
            "pause" => Ok(Self::Pause),
            "reload" => Ok(Self::Reload),
            "resume" => Ok(Self::Resume),
            "q" | "q!" | "quit" | "qa" | "qa!" | "wq" | "x" => Ok(Self::Quit),
            "slsk" | "soulseek" | "slsk_search" => Ok(Self::SoulseekSearch),
            "slsk_connect" => Ok(Self::SoulseekConnect),
            "slsk_disconnect" => Ok(Self::SoulseekDisconnect),
            "settings" | "set" => Ok(Self::Settings),
            "dupes" | "duplicates" => Ok(Self::Duplicates),
            "dj" => Ok(Self::Dj),
            "automix" => Ok(Self::Automix),
            _ => Err(std::io::Error::other(format!(
                "Unkown command variant {}",
                s
            ))),
        }
    }
}
