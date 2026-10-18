//! Editable `[soulseek]` settings: what the settings panel lists, how each value is shown and
//! parsed, and writing a change back to the config file without losing its comments.

use std::path::{Path, PathBuf};

use crate::parser::SoulseekSettings;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Bool,
    Number,
    Text,
    /// comma separated list
    List,
    /// shown masked
    Secret,
}

pub struct Setting {
    pub key: &'static str,
    pub kind: Kind,
    pub help: &'static str,
    /// takes effect on the next search instead of the next connect
    pub ranking: bool,
}

pub const SOULSEEK: &[Setting] = &[
    Setting {
        key: "formats",
        kind: Kind::List,
        help: "only show these formats, earlier = preferred (empty: all audio)",
        ranking: true,
    },
    Setting {
        key: "min_bitrate",
        kind: Kind::Number,
        help: "hide lossy files below this kbps, 0 = off (lossless always kept)",
        ranking: true,
    },
    Setting {
        key: "max_size_mb",
        kind: Kind::Number,
        help: "hide files bigger than this many MiB, 0 = no limit",
        ranking: true,
    },
    Setting {
        key: "prefer_free_slots",
        kind: Kind::Bool,
        help: "rank peers with free upload slots higher",
        ranking: true,
    },
    Setting {
        key: "max_concurrent_downloads",
        kind: Kind::Number,
        help: "downloads running at once, the rest wait in list order (Shift+J/K to reorder)",
        ranking: true,
    },
    Setting {
        key: "max_results",
        kind: Kind::Number,
        help: "how many results to show",
        ranking: false,
    },
    Setting {
        key: "download_dir",
        kind: Kind::Text,
        help: "where downloads are saved",
        ranking: false,
    },
    Setting {
        key: "enabled",
        kind: Kind::Bool,
        help: "use soulseek at all",
        ranking: false,
    },
    Setting {
        key: "username",
        kind: Kind::Text,
        help: "soulseek account (created on first login)",
        ranking: false,
    },
    Setting {
        key: "password",
        kind: Kind::Secret,
        help: "soulseek password",
        ranking: false,
    },
    Setting {
        key: "server_host",
        kind: Kind::Text,
        help: "soulseek server",
        ranking: false,
    },
    Setting {
        key: "server_port",
        kind: Kind::Number,
        help: "soulseek server port",
        ranking: false,
    },
    Setting {
        key: "listen_port",
        kind: Kind::Number,
        help: "port peers connect to",
        ranking: false,
    },
];

/// Current value as shown in the panel (and pre-filled when editing, except secrets)
pub fn display(s: &SoulseekSettings, key: &str) -> String {
    fn opt<T: ToString>(v: &Option<T>, default: &str) -> String {
        v.as_ref().map_or(default.to_string(), |v| v.to_string())
    }
    match key {
        "formats" => s
            .formats
            .as_ref()
            .filter(|f| !f.is_empty())
            .map_or("all audio".to_string(), |f| f.join(", ")),
        "min_bitrate" => opt(&s.min_bitrate, "0"),
        "max_size_mb" => opt(&s.max_size_mb, "0"),
        "prefer_free_slots" => yes_no(s.prefer_free_slots.unwrap_or(true)),
        "max_results" => opt(&s.max_results, "50"),
        "max_concurrent_downloads" => opt(&s.max_concurrent_downloads, "3"),
        "download_dir" => s
            .download_dir
            .as_ref()
            .map_or("~/Music/soulseek".to_string(), |p| p.display().to_string()),
        "enabled" => yes_no(s.enabled.unwrap_or(false)),
        "username" => opt(&s.username, ""),
        "password" => match &s.password {
            Some(p) if !p.is_empty() => "•".repeat(p.chars().count().min(12)),
            _ => String::new(),
        },
        "server_host" => opt(&s.server_host, "server.slsknet.org"),
        "server_port" => opt(&s.server_port, "2242"),
        "listen_port" => opt(&s.listen_port, "2234"),
        _ => String::new(),
    }
}

fn yes_no(b: bool) -> String {
    if b { "yes" } else { "no" }.to_string()
}

/// Text the edit box starts with
pub fn edit_text(s: &SoulseekSettings, key: &str) -> String {
    match key {
        "password" => String::new(),
        "formats" => s.formats.clone().unwrap_or_default().join(", "),
        _ => display(s, key),
    }
}

/// Flips a yes/no setting
pub fn toggle(s: &mut SoulseekSettings, key: &str) {
    match key {
        "prefer_free_slots" => s.prefer_free_slots = Some(!s.prefer_free_slots.unwrap_or(true)),
        "enabled" => s.enabled = Some(!s.enabled.unwrap_or(false)),
        _ => {}
    }
}

/// Parses `input` into the setting, or says why it can't
pub fn set(s: &mut SoulseekSettings, key: &str, input: &str) -> Result<(), String> {
    let input = input.trim();
    fn num<T: std::str::FromStr>(input: &str) -> Result<T, String> {
        input
            .parse()
            .map_err(|_| format!("'{}' is not a valid number", input))
    }
    match key {
        "formats" => {
            let formats: Vec<String> = input
                .split([',', ' '])
                .map(|f| f.trim().trim_start_matches('.').to_lowercase())
                .filter(|f| !f.is_empty())
                .collect();
            s.formats = (!formats.is_empty()).then_some(formats);
        }
        "min_bitrate" => s.min_bitrate = Some(num(input)?),
        "max_size_mb" => s.max_size_mb = Some(num(input)?),
        "max_results" => s.max_results = Some(num(input)?),
        "max_concurrent_downloads" => match num::<u32>(input)? {
            0 => return Err("at least 1".to_string()),
            n => s.max_concurrent_downloads = Some(n),
        },
        "server_port" => s.server_port = Some(port(input)?),
        "listen_port" => s.listen_port = Some(port(input)?),
        "download_dir" => s.download_dir = Some(expand_home(input)),
        "username" => s.username = Some(input.to_string()),
        "password" => s.password = Some(input.to_string()),
        "server_host" => s.server_host = Some(input.to_string()),
        "prefer_free_slots" | "enabled" => match input {
            "yes" | "true" | "on" | "1" => set_bool(s, key, true),
            "no" | "false" | "off" | "0" => set_bool(s, key, false),
            _ => return Err("use yes or no".to_string()),
        },
        _ => return Err(format!("unknown setting {}", key)),
    }
    Ok(())
}

fn set_bool(s: &mut SoulseekSettings, key: &str, v: bool) {
    match key {
        "prefer_free_slots" => s.prefer_free_slots = Some(v),
        "enabled" => s.enabled = Some(v),
        _ => {}
    }
}

fn port(input: &str) -> Result<u16, String> {
    match input.parse::<u16>() {
        Ok(p) if p > 0 => Ok(p),
        _ => Err(format!("'{}' is not a port (1-65535)", input)),
    }
}

fn expand_home(input: &str) -> PathBuf {
    match (input.strip_prefix("~/"), std::env::var("HOME")) {
        (Some(rest), Ok(home)) => Path::new(&home).join(rest),
        _ => PathBuf::from(input),
    }
}

/// Writes one setting into the `[soulseek]` table of the config file, keeping everything else
/// (comments, order, other sections) as it was
pub fn save(path: &Path, s: &SoulseekSettings, key: &str) -> Result<(), String> {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let mut doc: toml_edit::DocumentMut = text
        .parse()
        .map_err(|e| format!("can't parse {}: {}", path.display(), e))?;
    if !doc.contains_key("soulseek") {
        doc["soulseek"] = toml_edit::table();
    }
    let table = &mut doc["soulseek"];
    let value: Option<toml_edit::Item> = match key {
        "formats" => s.formats.as_ref().map(|f| {
            toml_edit::value(f.iter().map(|x| x.as_str()).collect::<toml_edit::Array>())
        }),
        "min_bitrate" => s.min_bitrate.map(|v| toml_edit::value(v as i64)),
        "max_size_mb" => s.max_size_mb.map(|v| toml_edit::value(v as i64)),
        "max_results" => s.max_results.map(|v| toml_edit::value(v as i64)),
        "max_concurrent_downloads" => s
            .max_concurrent_downloads
            .map(|v| toml_edit::value(v as i64)),
        "server_port" => s.server_port.map(|v| toml_edit::value(v as i64)),
        "listen_port" => s.listen_port.map(|v| toml_edit::value(v as i64)),
        "prefer_free_slots" => s.prefer_free_slots.map(toml_edit::value),
        "enabled" => s.enabled.map(toml_edit::value),
        "download_dir" => s
            .download_dir
            .as_ref()
            .map(|p| toml_edit::value(p.display().to_string())),
        "username" => s.username.clone().map(toml_edit::value),
        "password" => s.password.clone().map(toml_edit::value),
        "server_host" => s.server_host.clone().map(toml_edit::value),
        _ => return Err(format!("unknown setting {}", key)),
    };
    match value {
        Some(v) => table[key] = v,
        None => {
            if let Some(t) = table.as_table_like_mut() {
                t.remove(key);
            }
        }
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(path, doc.to_string()).map_err(|e| format!("can't write config: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_parses_and_validates() {
        let mut s = SoulseekSettings::default();
        set(&mut s, "formats", "FLAC, .mp3 ogg").unwrap();
        assert_eq!(s.formats.as_deref().unwrap(), ["flac", "mp3", "ogg"]);
        set(&mut s, "formats", "").unwrap();
        assert!(s.formats.is_none());
        set(&mut s, "min_bitrate", "256").unwrap();
        assert_eq!(s.min_bitrate, Some(256));
        assert!(set(&mut s, "min_bitrate", "lots").is_err());
        assert!(set(&mut s, "listen_port", "70000").is_err());
        assert!(set(&mut s, "listen_port", "0").is_err());
        set(&mut s, "prefer_free_slots", "no").unwrap();
        assert_eq!(s.prefer_free_slots, Some(false));
        toggle(&mut s, "prefer_free_slots");
        assert_eq!(s.prefer_free_slots, Some(true));
    }

    #[test]
    fn password_is_masked() {
        let s = SoulseekSettings {
            password: Some("hunter2".into()),
            ..Default::default()
        };
        assert_eq!(display(&s, "password"), "•••••••");
        assert_eq!(edit_text(&s, "password"), "");
    }

    #[test]
    fn save_keeps_comments_and_other_sections() {
        let dir = std::env::temp_dir().join(format!("musa-settings-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(
            &path,
            "# my config\n[config]\nfps = 30\n\n[soulseek]\n# account\nusername = \"me\"\n",
        )
        .unwrap();

        let mut s = SoulseekSettings::default();
        set(&mut s, "formats", "flac, mp3").unwrap();
        save(&path, &s, "formats").unwrap();
        set(&mut s, "min_bitrate", "320").unwrap();
        save(&path, &s, "min_bitrate").unwrap();

        let out = std::fs::read_to_string(&path).unwrap();
        assert!(out.contains("# my config"));
        assert!(out.contains("# account"));
        assert!(out.contains("fps = 30"));
        assert!(out.contains("username = \"me\""));
        assert!(out.contains("formats = [\"flac\", \"mp3\"]"));
        assert!(out.contains("min_bitrate = 320"));
        // and it still parses as our config
        let cfg = crate::parser::Config::parse_config(path.to_str().unwrap()).unwrap();
        assert_eq!(cfg.soulseek.unwrap().min_bitrate, Some(320));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
