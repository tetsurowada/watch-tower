pub mod macos;
#[cfg(target_os = "macos")]
pub mod native;

/// What a notification is about. Native notifications show it as an image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Done,
    Wait,
    Input,
}

impl State {
    /// The image file in the app bundle's Resources directory.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub fn image(self) -> &'static str {
        match self {
            Self::Done => "state-done.png",
            Self::Wait => "state-wait.png",
            Self::Input => "state-input.png",
        }
    }
}

/// What a sink shows to the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notification {
    pub message: String,
    pub title: String,
    pub subtitle: String,
    pub sound: String,
    /// Notifications with the same thread are grouped together. Empty means no grouping.
    pub thread: String,
    pub state: Option<State>,
}

pub trait Sink: Send + Sync {
    fn send(&self, notification: &Notification) -> Result<(), String>;
}
