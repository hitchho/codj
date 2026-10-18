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

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(tag = "config")]
pub struct Config {
    pub fps: Option<u32>,
    pub lyrics: Option<bool>,
    pub genre: Option<bool>,
    pub scripts: Option<Scripts>,
    pub step_size: Option<usize>,
    pub soulseek: Option<SoulseekSettings>,
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
}
