//! Retry policy shared by the HTTP layer and the installer.
//!
//! Transient failures (timeouts, resets, 5xx, truncated ranges) are retried with
//! exponential backoff before the caller is asked what to do. Everything else
//! (404, bad manifest, wrong hash from a local mirror) fails immediately, since
//! another attempt would produce the same result.

use std::time::Duration;

use crate::error::{Error, Result};

#[derive(Clone, Copy, Debug)]
pub struct RetryPolicy {
    /// Total attempts, including the first one. 1 disables retrying.
    pub attempts: u32,
    pub base_delay: Duration,
    pub max_delay: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            attempts: 3,
            base_delay: Duration::from_millis(250),
            max_delay: Duration::from_secs(4),
        }
    }
}

impl RetryPolicy {
    pub fn with_attempts(attempts: u32) -> Self {
        Self {
            attempts: attempts.max(1),
            ..Default::default()
        }
    }

    /// Delay before attempt `attempt` (1-based, so the first retry waits
    /// `base_delay`).
    pub fn delay(&self, attempt: u32) -> Duration {
        let factor = 1u32 << (attempt.saturating_sub(1).min(5));
        (self.base_delay * factor).min(self.max_delay)
    }
}

/// Report of a failed attempt that is about to be retried.
pub struct RetryNotice<'a> {
    /// Attempt that just failed, 1-based.
    pub attempt: u32,
    pub attempts: u32,
    pub delay: Duration,
    pub error: &'a Error,
}

/// Run `f` until it succeeds, `f` fails with a non-transient error, or the
/// policy is exhausted. `f` receives the 1-based attempt number.
pub fn with_retry<T>(
    policy: RetryPolicy,
    mut on_retry: impl FnMut(RetryNotice<'_>),
    mut f: impl FnMut(u32) -> Result<T>,
) -> Result<T> {
    let attempts = policy.attempts.max(1);
    let mut attempt = 1u32;
    loop {
        match f(attempt) {
            Ok(value) => return Ok(value),
            Err(err) => {
                let retryable = err.is_transient() && attempt < attempts;
                if !retryable {
                    return Err(err);
                }
                let delay = policy.delay(attempt);
                on_retry(RetryNotice {
                    attempt,
                    attempts,
                    delay,
                    error: &err,
                });
                std::thread::sleep(delay);
                attempt += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retries_transient_failures_then_succeeds() {
        let mut calls = 0;
        let policy = RetryPolicy {
            attempts: 3,
            base_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(2),
        };
        let notices = std::cell::Cell::new(0);

        let value: u32 = with_retry(
            policy,
            |_| notices.set(notices.get() + 1),
            |attempt| {
                calls += 1;
                if attempt < 3 {
                    Err(Error::Io(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "slow",
                    )))
                } else {
                    Ok(7)
                }
            },
        )
        .unwrap();

        assert_eq!(value, 7);
        assert_eq!(calls, 3);
        assert_eq!(notices.get(), 2);
    }

    #[test]
    fn gives_up_on_permanent_errors() {
        let mut calls = 0;
        let result: Result<()> = with_retry(
            RetryPolicy::with_attempts(5),
            |_| {},
            |_| {
                calls += 1;
                Err(Error::HttpStatus {
                    url: "x".into(),
                    status: 404,
                })
            },
        );
        assert!(result.is_err());
        assert_eq!(calls, 1, "a 404 must not be retried");
    }

    #[test]
    fn stops_after_the_policy_is_exhausted() {
        let mut calls = 0;
        let result: Result<()> = with_retry(
            RetryPolicy {
                attempts: 3,
                base_delay: Duration::from_millis(1),
                max_delay: Duration::from_millis(1),
            },
            |_| {},
            |_| {
                calls += 1;
                Err(Error::HttpStatus {
                    url: "x".into(),
                    status: 503,
                })
            },
        );
        assert!(result.is_err());
        assert_eq!(calls, 3);
    }

    #[test]
    fn backoff_grows_and_is_capped() {
        let policy = RetryPolicy {
            attempts: 10,
            base_delay: Duration::from_millis(100),
            max_delay: Duration::from_millis(400),
        };
        assert_eq!(policy.delay(1), Duration::from_millis(100));
        assert_eq!(policy.delay(2), Duration::from_millis(200));
        assert_eq!(policy.delay(3), Duration::from_millis(400));
        assert_eq!(policy.delay(9), Duration::from_millis(400));
    }
}
