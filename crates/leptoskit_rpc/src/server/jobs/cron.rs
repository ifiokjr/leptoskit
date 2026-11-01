//! `#[rpc::cron("…")]`: jobs enqueued on a schedule, once per tick across
//! every instance.

use chrono::Datelike;
use chrono::Timelike;
use leptoskit_db::Db;
use leptoskit_db::Timestamp;
use leptoskit_query::BoxFuture;

use super::Enqueue;
use super::JobRow;
use crate::Job;
use crate::Result;

/// When a cron job runs: the minutes, hours, days of the month, months, and
/// days of the week a five-field cron expression allows, in UTC, as bit
/// sets. `#[rpc::cron]` parses the expression when it compiles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Schedule {
	/// Bit `n` for minute `n`, 0 to 59.
	pub minutes: u64,
	/// Bit `n` for hour `n`, 0 to 23.
	pub hours: u32,
	/// Bit `n` for day `n` of the month, 1 to 31.
	pub days: u32,
	/// Bit `n` for month `n`, 1 to 12.
	pub months: u16,
	/// Bit `n` for weekday `n`, Sunday 0 to Saturday 6.
	pub weekdays: u8,
	/// Whether the day-of-month field was `*`.
	pub any_day: bool,
	/// Whether the day-of-week field was `*`.
	pub any_weekday: bool,
}

impl Schedule {
	/// Whether the minute starting at `at` is a tick.
	#[must_use]
	pub fn matches(&self, at: Timestamp) -> bool {
		self.minutes & (1 << at.minute()) != 0
			&& self.hours & (1 << at.hour()) != 0
			&& self.months & (1 << at.month()) != 0
			&& self.day_matches(at)
	}

	/// Days follow cron's rule: when both day fields are restricted, a day
	/// matching either one is a tick.
	fn day_matches(&self, at: Timestamp) -> bool {
		let day = self.days & (1 << at.day()) != 0;
		let weekday = self.weekdays & (1 << at.weekday().num_days_from_sunday()) != 0;

		match (self.any_day, self.any_weekday) {
			(true, true) => true,
			(true, false) => weekday,
			(false, true) => day,
			(false, false) => day || weekday,
		}
	}

	/// The first tick after `after`, within about eight years, or `None`
	/// for a schedule that never ticks, such as the 31st of February.
	#[must_use]
	pub fn next_after(&self, after: Timestamp) -> Option<Timestamp> {
		let mut at = after
			.with_second(0)?
			.with_nanosecond(0)?
			.checked_add_signed(chrono::Duration::minutes(1))?;
		let limit = after.checked_add_signed(chrono::Duration::days(366 * 8))?;

		while at <= limit {
			if self.months & (1 << at.month()) == 0 || !self.day_matches(at) {
				// The rest of this day can't tick.
				at = (at.date_naive() + chrono::Days::new(1))
					.and_hms_opt(0, 0, 0)?
					.and_utc();
				continue;
			}

			if self.hours & (1 << at.hour()) == 0 {
				at = at.with_minute(0)? + chrono::Duration::hours(1);

				continue;
			}

			if self.minutes & (1 << at.minute()) != 0 {
				return Some(at);
			}

			at += chrono::Duration::minutes(1);
		}

		None
	}
}

/// Enqueues one tick of a cron job: whether this call added it.
pub type CronEnqueue = fn(Db, String, Timestamp) -> BoxFuture<'static, Result<bool>>;

/// One `#[rpc::cron]`, as a server build registers it.
pub struct CronRegistration {
	/// The job's name.
	pub name: &'static str,
	/// The cron expression, as written.
	pub expression: &'static str,
	/// When it runs.
	pub schedule: Schedule,
	/// Enqueues one tick.
	#[doc(hidden)]
	pub enqueue: CronEnqueue,
}

inventory::collect!(CronRegistration);

/// Enqueues `job` for `tick`, keyed `key`, unless that tick's job exists.
/// Public for the macro's expansion.
#[doc(hidden)]
pub fn enqueue_tick<J: Job>(
	job: J,
	db: Db,
	key: String,
	tick: Timestamp,
) -> BoxFuture<'static, Result<bool>> {
	Box::pin(async move {
		let exists = JobRow::query()
			.filter(|row| row.key.eq(key.clone()))
			.exists(&db)
			.await?;
		if exists {
			return Ok(false);
		}

		Enqueue::on(db, job, None).key(key).at(tick).await?;
		Ok(true)
	})
}

#[cfg(test)]
mod tests {
	use chrono::TimeZone;
	use chrono::Utc;

	use super::*;

	fn at(text: &str) -> Timestamp {
		chrono::DateTime::parse_from_rfc3339(text)
			.unwrap()
			.with_timezone(&Utc)
	}

	const EVERY_FIVE: Schedule = Schedule {
		minutes: 0x0084_2108_4210_8421,
		hours: (1 << 24) - 1,
		days: !1,
		months: !1 & 0x1fff,
		weekdays: 0x7f,
		any_day: true,
		any_weekday: true,
	};

	#[test]
	fn every_five_minutes_ticks_on_the_fives() {
		assert_eq!(
			EVERY_FIVE.next_after(at("2026-10-31T10:02:30Z")),
			Some(at("2026-10-31T10:05:00Z"))
		);
		assert_eq!(
			EVERY_FIVE.next_after(at("2026-10-31T10:05:00Z")),
			Some(at("2026-10-31T10:10:00Z"))
		);
		assert_eq!(
			EVERY_FIVE.next_after(at("2026-12-31T23:59:00Z")),
			Some(at("2027-01-01T00:00:00Z"))
		);
		assert!(EVERY_FIVE.matches(at("2026-10-31T10:55:00Z")));
		assert!(!EVERY_FIVE.matches(at("2026-10-31T10:56:00Z")));
	}

	#[test]
	fn days_follow_crons_either_rule() {
		// 09:30 on the 1st of the month or on Mondays.
		let schedule = Schedule {
			minutes: 1 << 30,
			hours: 1 << 9,
			days: 1 << 1,
			months: !1 & 0x1fff,
			weekdays: 1 << 1,
			any_day: false,
			any_weekday: false,
		};
		// Saturday the 31st of October 2026: next is Monday the 2nd.
		assert_eq!(
			schedule.next_after(at("2026-10-31T12:00:00Z")),
			Some(at("2026-11-01T09:30:00Z"))
		);
		assert_eq!(
			schedule.next_after(at("2026-11-01T09:30:00Z")),
			Some(at("2026-11-02T09:30:00Z"))
		);
		let mondays = Schedule {
			any_day: true,
			..schedule
		};
		assert_eq!(
			mondays.next_after(at("2026-10-31T12:00:00Z")),
			Some(at("2026-11-02T09:30:00Z"))
		);
		let firsts = Schedule {
			any_weekday: true,
			..schedule
		};
		assert_eq!(
			firsts.next_after(at("2026-11-01T09:30:00Z")),
			Some(at("2026-12-01T09:30:00Z"))
		);
	}

	#[test]
	fn an_impossible_date_never_ticks() {
		let february_31 = Schedule {
			minutes: 1,
			hours: 1,
			days: 1 << 31,
			months: 1 << 2,
			weekdays: 0x7f,
			any_day: false,
			any_weekday: true,
		};
		assert_eq!(
			february_31.next_after(Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap()),
			None
		);
	}
}
