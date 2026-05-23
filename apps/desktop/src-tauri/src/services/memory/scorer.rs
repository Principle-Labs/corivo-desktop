//! Recall scoring helpers (memory-system-spec §7.4).
//!
//! Kept tiny on purpose — every coefficient lives in the per-layer
//! pack functions in `mod.rs` so the §15 evaluation harness can tune
//! one number at a time without chasing across files.

use chrono::{DateTime, Utc};

/// Exponential decay with half-life expressed in days.
///
/// `recency_decay(now, now, _) == 1.0` and the value approaches 0 as
/// `ts` recedes. Future timestamps clamp to 1 (no inflation).
pub fn recency_decay(now: DateTime<Utc>, ts: DateTime<Utc>, half_life_days: f32) -> f32 {
    let age_seconds = (now - ts).num_seconds();
    if age_seconds <= 0 {
        return 1.0;
    }
    let age_days = age_seconds as f32 / 86_400.0;
    (-age_days / half_life_days * std::f32::consts::LN_2).exp()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    #[test]
    fn decay_at_zero_age_is_one() {
        let now = Utc::now();
        let v = recency_decay(now, now, 7.0);
        assert!((v - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn decay_at_half_life_is_half() {
        let now = Utc::now();
        let then = now - Duration::days(7);
        let v = recency_decay(now, then, 7.0);
        assert!((v - 0.5).abs() < 1e-3);
    }

    #[test]
    fn future_ts_clamps_to_one() {
        let now = Utc::now();
        let future = now + Duration::days(10);
        let v = recency_decay(now, future, 7.0);
        assert_eq!(v, 1.0);
    }
}
