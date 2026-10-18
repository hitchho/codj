use std::fs::OpenOptions;
use std::io::prelude::*;

#[allow(unused)]
pub trait StringFeatures {
    /// insert [content] if the requested [String] is empty
    fn insert_if_empty(&mut self, content: &str);
}

#[allow(dead_code)]
/// logs _data_ to a _file_ in a incremantive manner
///
/// Panics:
/// - failed to create or write to the file
pub fn log(data: &str, filename: &str) -> std::io::Result<std::fs::File> {
    let mut f = OpenOptions::new()
        .append(true)
        .create(true)
        .open(filename)?;
    writeln!(f, "{}", data)?;
    std::io::Result::Ok(f)
}

impl StringFeatures for String {
    fn insert_if_empty(&mut self, content: &str) {
        if self.is_empty() {
            self.push_str(content);
        }
    }
}

#[derive(
    PartialEq,
    Eq,
    Debug,
    Ord,
    PartialOrd,
    Clone,
    serde::Deserialize,
    serde::Serialize,
    zbus::zvariant::Type,
)]
pub struct RunStatus {
    error_messge: String,
    status_type: StatusOption,
}

#[derive(
    PartialEq,
    Eq,
    Debug,
    Ord,
    PartialOrd,
    Clone,
    serde::Deserialize,
    serde::Serialize,
    zbus::zvariant::Type,
)]
pub enum StatusOption {
    Ok,
    OutOfRange,
    CoudntPreformAction,
    CoudntGetSHandler,
    CoudntSeek,
    CoudntPauseManager,
    CoudntPauseHandler,
    CoudntResumeManager,
    CoudntResumeHandler,
    WrongPath,
    CoudntReadMusicData,
}

// TODO: rework/rewrite this
// NOTE: this looks so ugly
#[allow(unused)]
impl RunStatus {
    fn new(msg: String, status: StatusOption) -> Self {
        Self {
            error_messge: msg,
            status_type: status,
        }
    }

    fn ok() -> Self {
        Self::new(String::from(""), StatusOption::Ok)
    }

    fn is_ok(&self) -> bool {
        matches!(self.status_type, StatusOption::Ok)
    }

    fn handler_errror() -> Self {
        RunStatus::new(
            "coudn't get stream handler!".to_string(),
            StatusOption::CoudntGetSHandler,
        )
    }
}

pub struct FPSController {
    fps: u32,
    frame_window: f64,
}

impl Default for FPSController {
    fn default() -> Self {
        Self {
            fps: 30,
            frame_window: 1.0 / 30.0,
        }
    }
}

impl FPSController {
    #[allow(unused)]
    pub fn new(fps: u32) -> Self {
        Self {
            fps,
            frame_window: 1.0 / fps as f64,
        }
    }

    /// Time budget for one frame
    pub fn frame_duration(&self) -> std::time::Duration {
        std::time::Duration::from_secs_f64(self.frame_window)
    }

    pub fn change_fps(&mut self, fps: u32) {
        self.fps = fps;
        self.frame_window = 1.0 / fps as f64;
    }
}
