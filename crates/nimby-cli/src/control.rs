//! Error policy, cancellation and the stdin control channel.
//!
//! `nimby install` runs as a child process of the GUI, so it cannot read the
//! keyboard. With `--control stdin` it reads one word per line:
//!
//! - `cancel` stops after the current file and keeps everything it verified
//! - `retry` / `skip` / `abort` answer a `needsAction` event
//!
//! Without the channel the process is non-interactive and `--on-error ask`
//! behaves like `abort`, so scripts and CI never hang.

use std::io::BufRead;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Mutex;
use std::time::Duration;

use clap::ValueEnum;

/// What to do when a mod cannot be installed after the automatic retries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum OnError {
    /// Stop the whole install.
    Abort,
    /// Keep asking for another attempt.
    Retry,
    /// Leave this mod as is and continue with the next one.
    Skip,
    /// Ask the user (falls back to `abort` when there is no control channel).
    Ask,
}

impl OnError {
    pub fn as_str(&self) -> &'static str {
        match self {
            OnError::Abort => "abort",
            OnError::Retry => "retry",
            OnError::Skip => "skip",
            OnError::Ask => "ask",
        }
    }
}

/// The user's answer to a `needsAction` event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Retry,
    Skip,
    Abort,
}

impl Action {
    pub fn as_str(&self) -> &'static str {
        match self {
            Action::Retry => "retry",
            Action::Skip => "skip",
            Action::Abort => "abort",
        }
    }

    pub fn parse(value: &str) -> Option<Action> {
        match value.trim().to_ascii_lowercase().as_str() {
            "retry" | "r" => Some(Action::Retry),
            "skip" | "s" => Some(Action::Skip),
            "abort" | "cancel" | "a" | "q" => Some(Action::Abort),
            _ => None,
        }
    }
}

pub struct Control {
    cancel: std::sync::Arc<AtomicBool>,
    answers: Option<Mutex<Receiver<String>>>,
}

impl Control {
    /// No channel: `cancel` is never set and asking degrades to `abort`.
    pub fn none() -> Self {
        Self {
            cancel: std::sync::Arc::new(AtomicBool::new(false)),
            answers: None,
        }
    }

    /// Read control words from stdin on a background thread.
    pub fn stdin() -> Self {
        let cancel = std::sync::Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        let flag = std::sync::Arc::clone(&cancel);
        std::thread::spawn(move || {
            let stdin = std::io::stdin();
            let mut lock = stdin.lock();
            let mut line = String::new();
            loop {
                line.clear();
                match lock.read_line(&mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        let word = line.trim().to_string();
                        // `cancel` must work whether or not the installer is
                        // currently waiting for an answer.
                        if word.eq_ignore_ascii_case("cancel") {
                            flag.store(true, Ordering::Relaxed);
                        }
                        if tx.send(word).is_err() {
                            break;
                        }
                    }
                }
            }
            // End of input: never leave a waiting install stuck.
            let _ = tx.send("abort".to_string());
        });
        Self {
            cancel,
            answers: Some(Mutex::new(rx)),
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    pub fn request_cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// Cancellation flag handed to the core installer.
    pub fn cancel_flag(&self) -> &AtomicBool {
        &self.cancel
    }

    pub fn can_ask(&self) -> bool {
        self.answers.is_some()
    }

    /// Block until the user answered. `cancel` also aborts; an invalid word is
    /// ignored so a stray newline cannot decide for the user.
    pub fn wait_answer(&self) -> Action {
        let Some(answers) = &self.answers else {
            return Action::Abort;
        };
        let Ok(receiver) = answers.lock() else {
            return Action::Abort;
        };
        loop {
            if self.is_cancelled() {
                return Action::Abort;
            }
            match receiver.recv() {
                Ok(word) => {
                    if word.eq_ignore_ascii_case("cancel") {
                        self.request_cancel();
                        return Action::Abort;
                    }
                    if let Some(action) = Action::parse(&word) {
                        return action;
                    }
                }
                Err(_) => return Action::Abort,
            }
        }
    }
}

/// Backoff between two attempts of the same mod.
pub fn backoff(attempt: u32) -> Duration {
    Duration::from_millis(250u64 << attempt.min(4))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_answers() {
        assert_eq!(Action::parse("retry"), Some(Action::Retry));
        assert_eq!(Action::parse(" SKIP "), Some(Action::Skip));
        assert_eq!(Action::parse("abort"), Some(Action::Abort));
        assert_eq!(Action::parse("maybe"), None);
    }

    #[test]
    fn no_channel_never_waits() {
        let control = Control::none();
        assert!(!control.can_ask());
        assert_eq!(control.wait_answer(), Action::Abort);
        assert!(!control.is_cancelled());
    }

    #[test]
    fn backoff_grows_and_is_capped() {
        assert_eq!(backoff(0), Duration::from_millis(250));
        assert_eq!(backoff(2), Duration::from_millis(1000));
        assert_eq!(backoff(9), Duration::from_millis(4000));
    }
}
