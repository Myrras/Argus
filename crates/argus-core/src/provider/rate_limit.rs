//! Rate limits: when a platform says "slow down", the provider stops calling
//! it until the time the platform named, and the poll loop skips it until
//! then (see [`ReviewProvider::rate_limited_until`](super::ReviewProvider::rate_limited_until)).

use std::sync::Mutex;

use chrono::{DateTime, Duration, Local, TimeZone as _, Utc};

use super::model::ProviderId;
use crate::error::CoreError;

/// The wait when a platform rate-limits without saying for how long.
const DEFAULT_WAIT_SECS: i64 = 60;
/// The longest wait honoured: GitHub's and GitLab's windows are an hour at
/// most, so anything further out is a bad header or a skewed clock.
const MAX_WAIT_SECS: i64 = 60 * 60;

/// The marker [`is_rate_limited`] looks for; part of every rate-limit message.
const MARKER: &str = " is rate limited until ";

/// The error for a provider that is rate-limited until `until`, with the
/// time in the user's local clock: "GitHub is rate limited until 14:05".
pub(crate) fn rate_limited(provider: ProviderId, until: DateTime<Utc>) -> CoreError {
    let local = until.with_timezone(&Local);
    CoreError::Provider(format!(
        "{provider}{MARKER}{}; Argus tries again then",
        local.format("%H:%M")
    ))
}

/// Whether `e` is a [`rate_limited`] error. A message check because the
/// error has no variant of its own (the IPC mapping matches every variant).
pub(crate) fn is_rate_limited(e: &CoreError) -> bool {
    matches!(e, CoreError::Provider(m) if m.contains(MARKER))
}

/// When a rate-limited response says the limit lifts: `Retry-After` (seconds
/// or an HTTP date) first, else a reset epoch (`X-RateLimit-Reset` on GitHub,
/// `RateLimit-Reset` on GitLab), else [`DEFAULT_WAIT_SECS`]. Clamped to
/// between a second and an hour from `now`.
pub(crate) fn reset_at(
    header: impl Fn(&str) -> Option<String>,
    now: DateTime<Utc>,
) -> DateTime<Utc> {
    let retry_after = header("retry-after").and_then(|v| {
        let v = v.trim();
        match v.parse::<i64>() {
            Ok(secs) => Some(now + Duration::seconds(secs)),
            Err(_) => DateTime::parse_from_rfc2822(v)
                .ok()
                .map(|d| d.with_timezone(&Utc)),
        }
    });
    let reset = || {
        ["x-ratelimit-reset", "ratelimit-reset"]
            .iter()
            .find_map(|name| header(name))
            .and_then(|v| v.trim().parse::<i64>().ok())
            .and_then(|epoch| Utc.timestamp_opt(epoch, 0).single())
    };
    let until = retry_after
        .or_else(reset)
        .unwrap_or(now + Duration::seconds(DEFAULT_WAIT_SECS));
    until.clamp(
        now + Duration::seconds(1),
        now + Duration::seconds(MAX_WAIT_SECS),
    )
}

/// The default wait from `now`, for a limit hit without headers (`gh search`
/// only reports it on stderr).
pub(crate) fn default_reset(now: DateTime<Utc>) -> DateTime<Utc> {
    now + Duration::seconds(DEFAULT_WAIT_SECS)
}

/// One provider's backoff state.
#[derive(Default)]
pub(crate) struct Backoff {
    until: Mutex<Option<DateTime<Utc>>>,
}

impl Backoff {
    /// The time the limit lifts, while it is still ahead of the clock.
    pub fn active(&self) -> Option<DateTime<Utc>> {
        let now = Utc::now();
        let mut until = self.until.lock().expect("backoff lock poisoned");
        if until.is_some_and(|u| u <= now) {
            *until = None;
        }
        *until
    }

    /// Fail fast while limited: no call goes out until the limit lifts.
    pub fn check(&self, provider: ProviderId) -> Result<(), CoreError> {
        match self.active() {
            Some(until) => Err(rate_limited(provider, until)),
            None => Ok(()),
        }
    }

    /// Record a limit until `until` (the later one wins when calls running
    /// in parallel hit it with different resets) and return its error.
    pub fn hit(&self, provider: ProviderId, until: DateTime<Utc>) -> CoreError {
        let mut current = self.until.lock().expect("backoff lock poisoned");
        let until = current.map_or(until, |c| c.max(until));
        *current = Some(until);
        tracing::debug!("{provider} rate limited until {until}");
        rate_limited(provider, until)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 14, 10, 0, 0).unwrap()
    }

    fn headers(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let pairs: Vec<(String, String)> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |name| {
            pairs
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(name))
                .map(|(_, v)| v.clone())
        }
    }

    #[test]
    fn reads_retry_after_then_the_reset_epoch() {
        let n = now();
        assert_eq!(
            reset_at(headers(&[("Retry-After", "120")]), n),
            n + Duration::seconds(120)
        );
        assert_eq!(
            reset_at(
                headers(&[("retry-after", "Wed, 14 Oct 2026 10:05:00 GMT")]),
                n
            ),
            n + Duration::minutes(5)
        );
        let epoch = (n + Duration::minutes(7)).timestamp().to_string();
        assert_eq!(
            reset_at(headers(&[("X-RateLimit-Reset", &epoch)]), n),
            n + Duration::minutes(7)
        );
        assert_eq!(reset_at(headers(&[]), n), n + Duration::seconds(60));
        // Clamped: never in the past, never more than an hour out.
        assert_eq!(
            reset_at(headers(&[("Retry-After", "-5")]), n),
            n + Duration::seconds(1)
        );
        assert_eq!(
            reset_at(headers(&[("Retry-After", "999999")]), n),
            n + Duration::hours(1)
        );
    }

    #[test]
    fn the_message_names_the_provider_and_local_time() {
        let until = now() + Duration::minutes(5);
        let e = rate_limited(ProviderId::Gitlab, until);
        let hhmm = until.with_timezone(&Local).format("%H:%M").to_string();
        assert!(e
            .to_string()
            .contains(&format!("GitLab is rate limited until {hhmm}")));
        assert!(is_rate_limited(&e));
        assert!(!is_rate_limited(&CoreError::Provider("other".into())));
        assert!(!e.is_unavailable());
    }

    #[test]
    fn backoff_holds_until_the_reset_and_keeps_the_later_one() {
        let b = Backoff::default();
        assert!(b.check(ProviderId::Github).is_ok());
        let soon = Utc::now() + Duration::minutes(2);
        let later = soon + Duration::minutes(3);
        b.hit(ProviderId::Github, later);
        b.hit(ProviderId::Github, soon);
        assert_eq!(b.active(), Some(later));
        assert!(is_rate_limited(&b.check(ProviderId::Github).unwrap_err()));
        // A reset in the past is over.
        let past = Backoff::default();
        past.hit(ProviderId::Github, Utc::now() - Duration::seconds(1));
        assert_eq!(past.active(), None);
    }
}
