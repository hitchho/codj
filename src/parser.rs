use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[allow(dead_code)]
#[derive(Deserialize, Serialize, Debug, Copy, Clone, Default)]
pub enum Sorting {
    #[default]
    ByTitleAscending,
    ByTitleDescending,
    ByDurationAscending,
    ByDurationDescending,
    Shuffle,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct Scripts {
    pub list: Option<PathBuf>,
    pub lyrics: Option<PathBuf>,
    pub actions: Option<PathBuf>,
    pub seeker: Option<PathBuf>,
    pub volume: Option<PathBuf>,
    pub all: Option<PathBuf>,
}

#[derive(Deserialize, Serialize, Debug, Clone, Default)]
pub struct SoulseekSettings {
    pub enabled: Option<bool>,
    pub username: Option<String>,
    pub password: Option<String>,
    pub server_host: Option<String>,
    pub server_port: Option<u16>,
    pub listen_port: Option<u16>,
    pub max_results: Option<usize>,
    /// directory to save downloaded files
    pub download_dir: Option<PathBuf>,
    /// directory to share (optional, uses dummy file if not set)
    pub share_dir: Option<PathBuf>,
    /// result formats to show, earlier = preferred (e.g. ["flac", "mp3"]); unset = all audio
    pub formats: Option<Vec<String>>,
    /// hide lossy results below this kbps; lossless and unknown bitrates are kept
    pub min_bitrate: Option<u32>,
    /// hide results bigger than this many MiB
    pub max_size_mb: Option<u32>,
    /// rank peers with free upload slots higher (default true)
    pub prefer_free_slots: Option<bool>,
    /// downloads running at once, the rest wait in list order (default 3)
    pub max_concurrent_downloads: Option<u32>,
}

/// `[agent]`: the AI agent the DJ screen's prompt box (`:ai`, F11) runs. codj holds no AI keys,
/// the command brings its own.
#[derive(Deserialize, Serialize, Debug, Clone, Default)]
pub struct AgentSettings {
    /// the agent command line, split on spaces (quotes group), default `claude -p`
    pub command: Option<String>,
    /// let the agent's MCP server change what is playing (`codj mcp --live`), default false
    pub live: Option<bool>,
    /// how the prompt reaches the command: "stdin" (default) or "arg" (last argument)
    pub prompt_via: Option<String>,
    /// MCP tool names the agent may use (without the `mcp__codj__` prefix); unset = all set tools
    /// (plus the live ones when `live`)
    pub mcp_tools: Option<Vec<String>>,
    /// what the command prints: "stream-json" (Claude Code's event lines: text, tool calls,
    /// results, a closing summary) or "text"; default stream-json for `claude`, text otherwise
    pub output: Option<String>,
}

/// `[knob]`: a hardware knob (rotary encoder sending volume keys): volume, jog wheel. Off unless `device`
/// is set. Only the three key codes below are ever read from the device.
#[derive(Deserialize, Serialize, Debug, Clone, Default, PartialEq)]
pub struct KnobSettings {
    /// part of the input device's name (`evtest` lists them), e.g. "AK820 Consumer Control"
    pub device: Option<String>,
    /// Linux input key codes: default 115 KEY_VOLUMEUP, 114 KEY_VOLUMEDOWN, 113 KEY_MUTE
    pub up: Option<u16>,
    pub down: Option<u16>,
    pub press: Option<u16>,
    /// take the device exclusively while codj runs, so the system volume stays put
    /// (default true; never done for a device that also types letters)
    pub grab: Option<bool>,
    /// seconds a paused deck moves per detent when the knob is the jog wheel (and per `[` `]`
    /// press); spinning fast multiplies it by up to 8. Default about 1/16 beat.
    pub jog: Option<f64>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(tag = "config")]
pub struct Config {
    pub fps: Option<u32>,
    pub lyrics: Option<bool>,
    pub genre: Option<bool>,
    pub scripts: Option<Scripts>,
    pub step_size: Option<usize>,
    pub soulseek: Option<SoulseekSettings>,
    pub agent: Option<AgentSettings>,
    pub knob: Option<KnobSettings>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            fps: Some(30),
            lyrics: Some(false),
            genre: Some(false),
            scripts: None,
            step_size: Some(7),
            soulseek: None,
            agent: None,
            knob: None,
        }
    }
}

impl Config {
    pub fn parse_config(_path: &str) -> std::io::Result<Self> {
        let conf_content = std::fs::read_to_string(_path)?;

        let config: Config = basic_toml::from_str(&conf_content)
            .map_err(|e| std::io::Error::other(format!("{}", e)))?;
        Ok(config)
    }
}

mod test {
    #[allow(unused_imports)]
    use super::*;
    #[test]
    fn read_config() {
        match Config::parse_config("./config.example.toml") {
            Ok(_) => {}
            Err(_) => panic!(),
        }
    }

    #[test]
    fn parse_soulseek_settings() {
        let conf: Config = basic_toml::from_str(
            "[soulseek]\nserver_host = \"h\"\nserver_port = 1\nlisten_port = 2\nmax_results = 3\n",
        )
        .unwrap();
        let s = conf.soulseek.unwrap();
        assert_eq!(s.server_host.as_deref(), Some("h"));
        assert_eq!(s.server_port, Some(1));
        assert_eq!(s.listen_port, Some(2));
        assert_eq!(s.max_results, Some(3));
    }

    #[test]
    fn parse_agent_settings() {
        let conf: Config = basic_toml::from_str(
            "[agent]\ncommand = \"my-agent --x\"\nlive = true\nprompt_via = \"arg\"\nmcp_tools = [\"set_check\"]\n",
        )
        .unwrap();
        let a = conf.agent.unwrap();
        assert_eq!(a.command.as_deref(), Some("my-agent --x"));
        assert_eq!(a.live, Some(true));
        assert_eq!(a.prompt_via.as_deref(), Some("arg"));
        assert_eq!(a.mcp_tools, Some(vec!["set_check".to_string()]));
        let none: Config = basic_toml::from_str("fps = 20\n").unwrap();
        assert!(none.agent.is_none());
    }

    #[test]
    fn parse_knob_settings() {
        let conf: Config = basic_toml::from_str(
            "[knob]\ndevice = \"AK820 Consumer Control\"\nup = 200\ngrab = false\n",
        )
        .unwrap();
        let k = conf.knob.unwrap();
        assert_eq!(k.device.as_deref(), Some("AK820 Consumer Control"));
        assert_eq!(k.up, Some(200));
        assert_eq!(k.down, None);
        assert_eq!(k.grab, Some(false));
        let none: Config = basic_toml::from_str("fps = 20\n").unwrap();
        assert!(none.knob.is_none());
    }
}
