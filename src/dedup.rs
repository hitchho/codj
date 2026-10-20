//! Hides extra copies of the same song in the music list: re-downloads (`track (1).mp3`), the
//! same album ripped twice, an mp3 next to its flac. Only the best copy is shown: lossless over
//! lossy, then the bigger file.

use std::{collections::HashMap, path::Path, time::Duration};

use crate::types::Music;

/// Same title and artist, and lengths this close, count as the same song
const SAME_LENGTH: Duration = Duration::from_secs(2);

/// Returns the list without duplicates (order kept) and how many were hidden.
/// `quality` ranks a copy; the highest is kept.
pub fn hide_duplicates<Q: Ord>(
    list: Vec<Music>,
    quality: impl Fn(&Music) -> Q,
) -> (Vec<Music>, usize) {
    // song key -> positions of its copies in `list`
    let mut groups: HashMap<(String, String), Vec<usize>> = HashMap::new();
    for (i, m) in list.iter().enumerate() {
        if let Some(key) = song_key(m) {
            groups.entry(key).or_default().push(i);
        }
    }
    let mut hidden = vec![false; list.len()];
    for positions in groups.values().filter(|p| p.len() > 1) {
        // within a title/artist group, only copies of about the same length are the same song
        let mut clusters: Vec<Vec<usize>> = Vec::new();
        for &i in positions {
            let len = list[i].length;
            match clusters
                .iter_mut()
                .find(|c| list[c[0]].length.abs_diff(len) <= SAME_LENGTH)
            {
                Some(c) => c.push(i),
                None => clusters.push(vec![i]),
            }
        }
        for cluster in clusters.iter().filter(|c| c.len() > 1) {
            let best = *cluster
                .iter()
                .max_by_key(|&&i| quality(&list[i]))
                .expect("cluster is not empty");
            for &i in cluster {
                hidden[i] = i != best;
            }
        }
    }
    let count = hidden.iter().filter(|h| **h).count();
    let kept = list
        .into_iter()
        .zip(hidden)
        .filter_map(|(m, h)| (!h).then_some(m))
        .collect();
    (kept, count)
}

/// Untagged files (no artist) are never merged: their titles are just file names
fn song_key(m: &Music) -> Option<(String, String)> {
    let title = m.title.trim().to_lowercase();
    let artist = m.artist.trim().to_lowercase();
    (!title.is_empty() && !artist.is_empty() && artist != "unknown").then_some((title, artist))
}

/// Lossless first, then file size
pub fn file_quality(m: &Music) -> (bool, u64) {
    let path: &Path = (&m.path).into();
    const LOSSLESS: [&str; 5] = ["flac", "wav", "aif", "aiff", "alac"];
    let lossless = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| LOSSLESS.iter().any(|l| l.eq_ignore_ascii_case(e)));
    let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    (lossless, size)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn music(title: &str, artist: &str, secs: u64, path: &str) -> Music {
        Music {
            title: title.into(),
            artist: artist.into(),
            length: Duration::from_secs(secs),
            path: PathBuf::from(path).into(),
            ..Default::default()
        }
    }

    fn paths(list: &[Music]) -> Vec<String> {
        list.iter()
            .map(|m| <&Path>::from(&m.path).display().to_string())
            .collect()
    }

    #[test]
    fn keeps_the_best_copy_in_place() {
        let list = vec![
            music("Cadillac", "Aasi & Swooh", 198, "/m/cadillac (1).mp3"),
            music("Cadillac", "Aasi & Swooh", 198, "/m/cadillac.mp3"),
            music("Desire", "DJ Nobu", 416, "/m/03 - Desire.flac"),
            music("Desire", "DJ Nobu", 417, "/m/03. Desire.flac"),
            music("Smokin", "Aasi", 300, "/m/smokin.mp3"),
        ];
        // stand-in quality: longer path wins
        let (kept, hidden) = hide_duplicates(list, |m| <&Path>::from(&m.path).as_os_str().len());
        assert_eq!(hidden, 2);
        assert_eq!(
            paths(&kept),
            ["/m/cadillac (1).mp3", "/m/03 - Desire.flac", "/m/smokin.mp3"]
        );
    }

    #[test]
    fn different_lengths_are_different_songs() {
        let list = vec![
            music("Intro", "X", 60, "/m/a.mp3"),
            music("Intro", "X", 240, "/m/b.mp3"),
        ];
        let (kept, hidden) = hide_duplicates(list, |_| 0);
        assert_eq!((kept.len(), hidden), (2, 0));
    }

    #[test]
    fn untagged_files_are_never_merged() {
        let list = vec![
            music("track01", "", 100, "/m/a/track01.mp3"),
            music("track01", "", 100, "/m/b/track01.mp3"),
            music("Song", "Unknown", 100, "/m/c.mp3"),
            music("Song", "Unknown", 100, "/m/d.mp3"),
        ];
        let (kept, hidden) = hide_duplicates(list, |_| 0);
        assert_eq!((kept.len(), hidden), (4, 0));
    }

    #[test]
    fn lossless_beats_bigger_lossy() {
        let dir = std::env::temp_dir().join(format!("codj-dedup-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mp3 = dir.join("a.mp3");
        let flac = dir.join("a.flac");
        std::fs::write(&mp3, vec![0u8; 2000]).unwrap();
        std::fs::write(&flac, vec![0u8; 1000]).unwrap();
        let list = vec![
            music("A", "B", 100, mp3.to_str().unwrap()),
            music("A", "B", 100, flac.to_str().unwrap()),
        ];
        let (kept, _) = hide_duplicates(list, file_quality);
        assert_eq!(paths(&kept), [flac.display().to_string()]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
