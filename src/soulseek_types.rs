// soulseek types for dbus communication with server

use serde::{Deserialize, Serialize};

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, zbus::zvariant::Type,
)]
pub enum ConnectionState {
    #[default]
    Disconnected,
    Connecting,
    Connected,
    LoggingIn,
    Ready,
    Error,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, zbus::zvariant::Type,
)]
pub enum DownloadState {
    #[default]
    Pending,
    Connecting,
    Downloading,
    Complete,
    Failed,
    Cancelled,
    /// waiting in the uploader's queue
    Queued,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, zbus::zvariant::Type)]
pub struct SharedFile {
    pub filename: String,
    pub size: u64,
    pub bitrate: u32,
    pub length_secs: u64,
    pub vbr: bool,
    pub sample_rate: u32,
}

impl SharedFile {
    pub fn extension(&self) -> Option<&str> {
        self.filename.rsplit('.').next()
    }

    pub fn basename(&self) -> &str {
        self.filename
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(&self.filename)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, zbus::zvariant::Type)]
pub struct RankedFile {
    pub file: SharedFile,
    pub username: String,
    pub free_slots: bool,
    pub upload_speed: u32,
    pub queue_size: u32,
    pub quality_score: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, zbus::zvariant::Type)]
pub struct Download {
    pub id: u32,
    pub username: String,
    pub remote_path: String,
    pub local_path: std::path::PathBuf,
    pub size: u64,
    pub downloaded: u64,
    pub state: DownloadState,
    pub quality_score: u32,
    pub display_name: String,
    pub token: u32,
    /// 0 for the source picked by the user, n for the nth other user tried
    pub attempt: u32,
    /// how many other users may be tried in total
    pub max_attempts: u32,
    /// why the previous source was given up on, or why this one failed
    pub last_error: String,
}

impl Download {
    pub fn progress(&self) -> f64 {
        if self.size == 0 {
            0.0
        } else {
            self.downloaded as f64 / self.size as f64
        }
    }

    pub fn progress_percent(&self) -> String {
        format!("{:.1}%", self.progress() * 100.0)
    }

    /// what the downloads table says about retries: "retry 2/3 from bob" while another user
    /// is being tried, the reason once it failed for good
    pub fn retry_info(&self) -> String {
        let retry = (self.attempt > 0)
            .then(|| format!("retry {}/{} from {}", self.attempt, self.max_attempts, self.username));
        match (self.state, retry) {
            (DownloadState::Failed, Some(r)) if !self.last_error.is_empty() => {
                format!("{}: {}", r, self.last_error)
            }
            (DownloadState::Failed, None) => self.last_error.clone(),
            (_, Some(r)) => r,
            (_, None) => String::new(),
        }
    }
}
