//! `Trigger::next_after` + validation.
//!
//! Each variant gets a dedicated `next_*` helper rather than going
//! through a unified cron downgrade. `Interval` is anchored to "now"
//! (not absolute wall-clock), `Daily` / `Weekly` evaluate HH:MM in
//! the configured IANA timezone, `Once` is a simple comparison, and
//! `Cron` defers to the `cron` crate after padding the input to its
//! native 6-field format.

use std::str::FromStr;

use chrono::{Datelike, DateTime, Duration, LocalResult, NaiveDate, TimeZone, Utc};
use chrono_tz::Tz;
use cron::Schedule;

use crate::domain::workflow::{Trigger, Weekday};

impl Trigger {
    /// Next firing time strictly after `t`. Returns `None` for `Once`
    /// triggers whose instant has already passed, or when the
    /// expression is malformed (defensive — the IPC layer rejects bad
    /// triggers before they reach the store).
    pub fn next_after(&self, t: DateTime<Utc>) -> Option<DateTime<Utc>> {
        match self {
            Trigger::Interval { minutes } => {
                Some(t + Duration::minutes(i64::from(*minutes)))
            }
            Trigger::Daily { hour, minute, tz } => {
                next_daily(t, *hour, *minute, tz)
            }
            Trigger::Weekly {
                weekdays,
                hour,
                minute,
                tz,
            } => next_weekly(t, weekdays, *hour, *minute, tz),
            Trigger::Once { at } => (*at > t).then_some(*at),
            Trigger::Cron { expr, tz } => next_cron(t, expr, tz),
        }
    }

    /// Returns `Ok(())` if the trigger is structurally sound. Surfaces
    /// human-readable error strings so the IPC layer can echo them
    /// back to the create/update drawer.
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Trigger::Interval { minutes } => {
                if *minutes == 0 {
                    Err("interval 必须 ≥ 1 分钟".into())
                } else {
                    Ok(())
                }
            }
            Trigger::Daily { hour, minute, tz } => {
                validate_hm(*hour, *minute)?;
                validate_tz(tz)
            }
            Trigger::Weekly {
                weekdays,
                hour,
                minute,
                tz,
            } => {
                if weekdays.is_empty() {
                    return Err("weekly 触发器至少选一个 weekday".into());
                }
                validate_hm(*hour, *minute)?;
                validate_tz(tz)
            }
            Trigger::Once { .. } => Ok(()),
            Trigger::Cron { expr, tz } => {
                let normalized = pad_to_six_field(expr);
                Schedule::from_str(&normalized)
                    .map_err(|e| format!("cron 表达式解析失败: {e}"))?;
                validate_tz(tz)
            }
        }
    }
}

fn validate_hm(hour: u8, minute: u8) -> Result<(), String> {
    if hour > 23 {
        return Err(format!("hour {hour} 越界 (0–23)"));
    }
    if minute > 59 {
        return Err(format!("minute {minute} 越界 (0–59)"));
    }
    Ok(())
}

fn validate_tz(tz: &str) -> Result<(), String> {
    Tz::from_str(tz).map_err(|e| format!("无效时区 {tz}: {e}"))?;
    Ok(())
}

fn parse_tz(tz: &str) -> Option<Tz> {
    Tz::from_str(tz).ok()
}

fn next_daily(
    t: DateTime<Utc>,
    hour: u8,
    minute: u8,
    tz: &str,
) -> Option<DateTime<Utc>> {
    let zone = parse_tz(tz)?;
    let local = t.with_timezone(&zone);
    let today = local.date_naive();

    for offset in 0..=2 {
        let day = today + Duration::days(offset);
        if let Some(utc) = local_hm_to_utc(zone, day, hour, minute) {
            if utc > t {
                return Some(utc);
            }
        }
    }
    None
}

fn next_weekly(
    t: DateTime<Utc>,
    weekdays: &[Weekday],
    hour: u8,
    minute: u8,
    tz: &str,
) -> Option<DateTime<Utc>> {
    let zone = parse_tz(tz)?;
    let allowed: Vec<chrono::Weekday> =
        weekdays.iter().map(|w| w.to_chrono()).collect();
    let today = t.with_timezone(&zone).date_naive();

    for offset in 0..=7 {
        let day = today + Duration::days(offset);
        if !allowed.contains(&day.weekday()) {
            continue;
        }
        if let Some(utc) = local_hm_to_utc(zone, day, hour, minute) {
            if utc > t {
                return Some(utc);
            }
        }
    }
    None
}

/// Convert `day @ hour:minute` in `zone` back to UTC. Returns `None`
/// when the requested instant doesn't exist (DST spring-forward gap);
/// callers retry with the next day. On DST fall-back ambiguity, picks
/// the earlier of the two — matches "first occurrence wins".
fn local_hm_to_utc(
    zone: Tz,
    day: NaiveDate,
    hour: u8,
    minute: u8,
) -> Option<DateTime<Utc>> {
    let naive = day.and_hms_opt(u32::from(hour), u32::from(minute), 0)?;
    let local = match zone.from_local_datetime(&naive) {
        LocalResult::Single(dt) => dt,
        LocalResult::Ambiguous(earlier, _later) => earlier,
        LocalResult::None => return None,
    };
    Some(local.with_timezone(&Utc))
}

fn pad_to_six_field(expr: &str) -> String {
    // The `cron` crate's native format is 6-field (Quartz-style:
    // `sec min hour dom mon dow`). Our UI surface accepts standard
    // 5-field POSIX cron and we transparently prepend a `0 ` seconds
    // slot. A power user who already speaks the 6-field dialect can
    // pass that through unchanged.
    let count = expr.split_whitespace().count();
    if count == 5 {
        format!("0 {expr}")
    } else {
        expr.to_string()
    }
}

fn next_cron(t: DateTime<Utc>, expr: &str, tz: &str) -> Option<DateTime<Utc>> {
    let zone = parse_tz(tz)?;
    let normalized = pad_to_six_field(expr);
    let schedule = Schedule::from_str(&normalized).ok()?;
    let after_local = t.with_timezone(&zone);
    schedule
        .after(&after_local)
        .next()
        .map(|d| d.with_timezone(&Utc))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn utc(y: i32, m: u32, d: u32, hh: u32, mm: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, hh, mm, 0).unwrap()
    }

    #[test]
    fn interval_advances_by_minutes() {
        let trigger = Trigger::Interval { minutes: 15 };
        let now = utc(2026, 5, 21, 10, 0);
        let next = trigger.next_after(now).unwrap();
        assert_eq!(next, utc(2026, 5, 21, 10, 15));
    }

    #[test]
    fn interval_validate_rejects_zero() {
        assert!(Trigger::Interval { minutes: 0 }.validate().is_err());
        assert!(Trigger::Interval { minutes: 1 }.validate().is_ok());
    }

    #[test]
    fn daily_picks_today_if_hm_is_future() {
        // 09:00 Asia/Shanghai (+08:00) = 01:00 UTC.
        let trigger = Trigger::Daily {
            hour: 9,
            minute: 0,
            tz: "Asia/Shanghai".into(),
        };
        // Now is 00:30 UTC on May 21 → 08:30 Shanghai. Next is today
        // 09:00 Shanghai = 01:00 UTC same date.
        let now = utc(2026, 5, 21, 0, 30);
        let next = trigger.next_after(now).unwrap();
        assert_eq!(next, utc(2026, 5, 21, 1, 0));
    }

    #[test]
    fn daily_rolls_to_tomorrow_if_hm_already_passed() {
        let trigger = Trigger::Daily {
            hour: 9,
            minute: 0,
            tz: "Asia/Shanghai".into(),
        };
        // Now is 02:00 UTC May 21 → 10:00 Shanghai (already past 09:00).
        // Next must be May 22, 01:00 UTC.
        let now = utc(2026, 5, 21, 2, 0);
        let next = trigger.next_after(now).unwrap();
        assert_eq!(next, utc(2026, 5, 22, 1, 0));
    }

    #[test]
    fn daily_crosses_utc_midnight_correctly() {
        // 00:30 Asia/Shanghai is 16:30 UTC the previous day.
        let trigger = Trigger::Daily {
            hour: 0,
            minute: 30,
            tz: "Asia/Shanghai".into(),
        };
        // Now is 15:00 UTC May 21 → 23:00 Shanghai same day. Next is
        // May 22 00:30 Shanghai → May 21 16:30 UTC.
        let now = utc(2026, 5, 21, 15, 0);
        let next = trigger.next_after(now).unwrap();
        assert_eq!(next, utc(2026, 5, 21, 16, 30));
    }

    #[test]
    fn weekly_skips_to_next_allowed_weekday() {
        // Monday at 10:00 Asia/Shanghai = 02:00 UTC.
        let trigger = Trigger::Weekly {
            weekdays: vec![Weekday::Mon],
            hour: 10,
            minute: 0,
            tz: "Asia/Shanghai".into(),
        };
        // 2026-05-21 is a Thursday. Next Monday is 2026-05-25.
        let now = utc(2026, 5, 21, 5, 0);
        let next = trigger.next_after(now).unwrap();
        assert_eq!(next, utc(2026, 5, 25, 2, 0));
    }

    #[test]
    fn weekly_fires_today_when_weekday_matches_and_hm_is_future() {
        // 2026-05-21 is a Thursday.
        let trigger = Trigger::Weekly {
            weekdays: vec![Weekday::Thu, Weekday::Mon],
            hour: 18,
            minute: 0,
            tz: "Asia/Shanghai".into(),
        };
        // 18:00 Shanghai Thursday = 10:00 UTC Thursday.
        let now = utc(2026, 5, 21, 8, 0);
        let next = trigger.next_after(now).unwrap();
        assert_eq!(next, utc(2026, 5, 21, 10, 0));
    }

    #[test]
    fn weekly_validate_rejects_empty_weekdays() {
        let trigger = Trigger::Weekly {
            weekdays: vec![],
            hour: 9,
            minute: 0,
            tz: "Asia/Shanghai".into(),
        };
        assert!(trigger.validate().is_err());
    }

    #[test]
    fn once_returns_none_when_already_fired() {
        let trigger = Trigger::Once {
            at: utc(2026, 5, 20, 0, 0),
        };
        let now = utc(2026, 5, 21, 0, 0);
        assert_eq!(trigger.next_after(now), None);
    }

    #[test]
    fn once_returns_at_when_in_future() {
        let trigger = Trigger::Once {
            at: utc(2026, 5, 22, 0, 0),
        };
        let now = utc(2026, 5, 21, 0, 0);
        assert_eq!(trigger.next_after(now), Some(utc(2026, 5, 22, 0, 0)));
    }

    #[test]
    fn cron_accepts_5_field_input_and_picks_next_match() {
        // "0 9 * * *" = daily at 09:00 local. tz=Asia/Shanghai →
        // 01:00 UTC.
        let trigger = Trigger::Cron {
            expr: "0 9 * * *".into(),
            tz: "Asia/Shanghai".into(),
        };
        let now = utc(2026, 5, 21, 2, 0);
        let next = trigger.next_after(now).unwrap();
        assert_eq!(next, utc(2026, 5, 22, 1, 0));
    }

    #[test]
    fn cron_rejects_garbage_expressions() {
        let trigger = Trigger::Cron {
            expr: "nope this is bad".into(),
            tz: "Asia/Shanghai".into(),
        };
        assert!(trigger.validate().is_err());
    }

    #[test]
    fn cron_validate_rejects_bad_tz() {
        let trigger = Trigger::Cron {
            expr: "0 9 * * *".into(),
            tz: "Mars/Olympus".into(),
        };
        assert!(trigger.validate().is_err());
    }

    #[test]
    fn pad_to_six_field_passes_through_native_format() {
        assert_eq!(pad_to_six_field("0 0 9 * * *"), "0 0 9 * * *");
        assert_eq!(pad_to_six_field("0 9 * * *"), "0 0 9 * * *");
    }
}
