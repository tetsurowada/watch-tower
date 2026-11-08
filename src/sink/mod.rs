pub mod macos;

/// What a sink shows to the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notification {
    pub message: String,
    pub title: String,
    pub subtitle: String,
    pub sound: String,
}

pub trait Sink: Send + Sync {
    fn send(&self, notification: &Notification) -> Result<(), String>;
}
