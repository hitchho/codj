//! Minimal status-bar mode (`musa status`, `musa toggle`, ...) for polybar, waybar and i3blocks.
//! Runs before any terminal setup and never touches the TUI.

use std::{
    io::{self, Write},
    path::Path,
    time::Duration,
};

use futures::StreamExt;
use zbus::Connection;

use crate::{types::PlayingStatus, ServerProxy};

pub const DEFAULT_FORMAT: &str = "{icon} {title} — {artist} {elapsed}/{total}";
const VOLUME_STEP: f32 = 0.05;
const RETRY: Duration = Duration::from_secs(2);

/// Names that are treated as subcommands; any other first argument is a config path.
pub fn is_subcommand(arg: &str) -> bool {
    matches!(arg, "status" | "toggle" | "next" | "prev" | "vol+" | "vol-")
}

#[derive(Debug, Clone, PartialEq)]
pub enum Snapshot {
    Stopped,
    Track {
        paused: bool,
        title: String,
        artist: String,
        path: String,
        elapsed: u64,
        total: u64,
    },
}

pub fn mmss(secs: u64) -> String {
    format!("{:02}:{:02}", secs / 60, secs % 60)
}

/// Keeps at most `max` chars, ending in "…" when something was cut. 0 means unlimited.
pub fn truncate(s: &str, max: usize) -> String {
    if max == 0 || s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// Title for display: the tag, or the file name without extension for untagged tracks.
pub fn display_title(title: &str, path: &str) -> String {
    if !missing(title) {
        return title.trim().to_string();
    }
    Path::new(path)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Empty, or the "Unknown" musad fills in for a missing tag
fn missing(tag: &str) -> bool {
    let tag = tag.trim();
    tag.is_empty() || tag.eq_ignore_ascii_case("unknown")
}

pub fn render(format: &str, snap: &Snapshot, max_width: usize) -> String {
    let Snapshot::Track {
        paused,
        title,
        artist,
        path,
        elapsed,
        total,
    } = snap
    else {
        return "■".to_string();
    };
    let mut f = format.to_string();
    let artist = if missing(artist) { "" } else { artist.as_str() };
    if artist.is_empty() {
        // drop the separator together with the empty artist
        f = f.replace(" — {artist}", "").replace("{artist}", "");
    }
    f.replace("{icon}", if *paused { "⏸" } else { "▶" })
        .replace("{title}", &truncate(&display_title(title, path), max_width))
        .replace("{artist}", artist.trim())
        .replace("{elapsed}", &mmss(*elapsed))
        .replace("{total}", &mmss(*total))
}

async fn snapshot(p: &ServerProxy<'_>) -> zbus::Result<Snapshot> {
    if matches!(p.get_playing_status().await?, PlayingStatus::Stopped) {
        return Ok(Snapshot::Stopped);
    }
    let paused = matches!(p.get_playing_status().await?, PlayingStatus::Pausing);
    let m = p.playing().await?;
    let elapsed = p.played_duration().await?;
    Ok(Snapshot::Track {
        paused,
        title: m.title,
        artist: m.artist,
        path: crate::search::file_name(&m.path),
        elapsed: elapsed.max(0.0) as u64,
        total: m.length.as_secs(),
    })
}

/// Prints a line; false when stdout is gone (e.g. the bar closed the pipe).
fn emit(line: &str) -> bool {
    let mut out = io::stdout().lock();
    writeln!(out, "{line}").and_then(|_| out.flush()).is_ok()
}

struct Opts {
    format: String,
    max_width: usize,
    offline: String,
    follow: bool,
}

fn parse_opts(args: &[String]) -> Result<Opts, String> {
    let mut o = Opts {
        format: DEFAULT_FORMAT.into(),
        max_width: 0,
        offline: String::new(),
        follow: false,
    };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut val = |name: &str| it.next().cloned().ok_or(format!("{name} needs a value"));
        match a.as_str() {
            "--follow" | "-f" => o.follow = true,
            "--format" => o.format = val("--format")?,
            "--offline" => o.offline = val("--offline")?,
            "--max-width" => {
                o.max_width = val("--max-width")?
                    .parse()
                    .map_err(|_| "--max-width needs a number".to_string())?
            }
            other => return Err(format!("unknown option {other}")),
        }
    }
    Ok(o)
}

async fn connect() -> Option<ServerProxy<'static>> {
    let conn = Connection::session().await.ok()?;
    let p = ServerProxy::new(&conn).await.ok()?;
    p.get_playing_index().await.ok()?;
    Some(p)
}

async fn follow(o: &Opts) {
    let mut last: Option<String> = None;
    let mut show = |line: String| -> bool {
        if last.as_deref() == Some(&line) {
            return true;
        }
        let ok = emit(&line);
        last = Some(line);
        ok
    };
    loop {
        let Some(p) = connect().await else {
            if !show(o.offline.clone()) {
                return;
            }
            tokio::time::sleep(RETRY).await;
            continue;
        };
        let (Ok(a), Ok(b), Ok(c), Ok(d)) = (
            p.receive_music_played().await,
            p.receive_paused().await,
            p.receive_resumed().await,
            p.receive_ended().await,
        ) else {
            tokio::time::sleep(RETRY).await;
            continue;
        };
        let mut signals = futures::stream::select_all(vec![
            a.map(|_| ()).boxed(),
            b.map(|_| ()).boxed(),
            c.map(|_| ()).boxed(),
            d.map(|_| ()).boxed(),
        ]);
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        loop {
            // musad going away makes the calls fail (or time out), which sends us back to reconnecting
            let snap = tokio::time::timeout(Duration::from_secs(3), snapshot(&p)).await;
            let Ok(Ok(snap)) = snap else { break };
            if !show(render(&o.format, &snap, o.max_width)) {
                return;
            }
            tokio::select! {
                _ = tick.tick() => {}
                _ = signals.next() => {}
            }
        }
        if !show(o.offline.clone()) {
            return;
        }
        tokio::time::sleep(RETRY).await;
    }
}

/// Handles a bar subcommand and returns the process exit code.
/// Play/pause; when stopped (nothing loaded) start the current track instead
async fn toggle(p: &ServerProxy<'_>) -> zbus::Result<()> {
    if matches!(p.get_playing_status().await?, PlayingStatus::Stopped) {
        let index = p.get_playing_index().await?;
        p.play_from_index(index).await.map(|_| ())
    } else {
        p.toggle_play().await.map(|_| ())
    }
}

pub async fn run(args: &[String]) -> i32 {
    let cmd = args[0].as_str();
    let rest = &args[1..];
    if cmd == "status" {
        let o = match parse_opts(rest) {
            Ok(o) => o,
            Err(e) => {
                eprintln!("musa status: {e}");
                return 2;
            }
        };
        if o.follow {
            follow(&o).await;
            return 0;
        }
        let Some(p) = connect().await else {
            emit(&o.offline);
            return 1;
        };
        return match snapshot(&p).await {
            Ok(s) => {
                emit(&render(&o.format, &s, o.max_width));
                0
            }
            Err(e) => {
                eprintln!("musa status: {e}");
                1
            }
        };
    }
    let Some(p) = connect().await else {
        eprintln!("musa {cmd}: musad is not running");
        return 1;
    };
    let res = match cmd {
        "toggle" => toggle(&p).await,
        "next" => p.play_next().await.map(|_| ()),
        "prev" => p.play_previous().await.map(|_| ()),
        _ => {
            let delta = if cmd == "vol+" { VOLUME_STEP } else { -VOLUME_STEP };
            match p.get_volume().await {
                Ok(v) => p
                    .volume(((v + delta) * 100.0).round().clamp(0.0, 100.0) / 100.0)
                    .await
                    .map(|_| ()),
                Err(e) => Err(e),
            }
        }
    };
    match res {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("musa {cmd}: {e}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(title: &str, artist: &str, path: &str, paused: bool) -> Snapshot {
        Snapshot::Track {
            paused,
            title: title.into(),
            artist: artist.into(),
            path: path.into(),
            elapsed: 138,
            total: 437,
        }
    }

    #[test]
    fn default_format() {
        let s = track("Aftertekno", "Pneumatix", "/m/a.mp3", false);
        assert_eq!(render(DEFAULT_FORMAT, &s, 0), "▶ Aftertekno — Pneumatix 02:18/07:17");
        let s = track("Aftertekno", "Pneumatix", "/m/a.mp3", true);
        assert!(render(DEFAULT_FORMAT, &s, 0).starts_with("⏸ "));
    }

    #[test]
    fn empty_artist_drops_separator() {
        let s = track("Song", " ", "/m/a.mp3", false);
        assert_eq!(render(DEFAULT_FORMAT, &s, 0), "▶ Song 02:18/07:17");
    }

    #[test]
    fn untagged_uses_file_stem() {
        let s = track("", "", "/m/dir/My Song.v2.flac", false);
        assert_eq!(render(DEFAULT_FORMAT, &s, 0), "▶ My Song.v2 02:18/07:17");
    }

    #[test]
    fn stopped() {
        assert_eq!(render(DEFAULT_FORMAT, &Snapshot::Stopped, 0), "■");
    }

    #[test]
    fn truncation_counts_chars() {
        assert_eq!(truncate("abc", 3), "abc");
        assert_eq!(truncate("abcdef", 4), "abc…");
        assert_eq!(truncate("ääääää", 4), "äää…");
        assert_eq!(truncate("日本語のタイトル", 3), "日本…");
        assert_eq!(truncate("abcdef", 0), "abcdef");
        assert_eq!(truncate("abc", 1), "…");
    }

    #[test]
    fn max_width_applies_to_title_only() {
        let s = track("A very long title indeed", "X", "/m/a.mp3", false);
        assert_eq!(render(DEFAULT_FORMAT, &s, 8), "▶ A very … — X 02:18/07:17");
    }

    #[test]
    fn custom_format_and_time() {
        let s = track("T", "A", "p", false);
        assert_eq!(render("{title}|{elapsed}", &s, 0), "T|02:18");
        assert_eq!(mmss(3599), "59:59");
        assert_eq!(mmss(3600), "60:00");
    }

    #[test]
    fn subcommands_and_options() {
        assert!(is_subcommand("vol+") && !is_subcommand("/home/x/config.toml"));
        let o = parse_opts(&["--follow".into(), "--max-width".into(), "40".into()]).unwrap();
        assert!(o.follow && o.max_width == 40);
        assert!(parse_opts(&["--max-width".into(), "x".into()]).is_err());
        assert!(parse_opts(&["--bogus".into()]).is_err());
    }
}
