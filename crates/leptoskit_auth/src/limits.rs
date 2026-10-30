//! Rate limits: how often one source may try one action.

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use leptoskit_db::Db;
use leptoskit_db::DbError;

use crate::AuthError;
use crate::NewRateLimit;
use crate::RateLimit;
use crate::Result;

/// At most `max` attempts per `window`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limit {
	/// Attempts allowed in a window.
	pub max: u32,
	/// How long a window lasts.
	pub window: Duration,
}

impl Limit {
	/// At most `max` attempts per `window`.
	pub const fn new(max: u32, window: Duration) -> Self {
		Self { max, window }
	}
}

/// A future a [`RateLimiter`] returns.
pub type LimitFuture<'a> = Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>>;

/// Counts attempts. The default keeps its counters in `auth.rate_limits`;
/// implement this to keep them somewhere faster, such as Redis.
pub trait RateLimiter: Send + Sync + 'static {
	/// Counts an attempt under `key`, or fails with
	/// [`AuthError::RateLimited`] when `limit` is already used up. The key
	/// is a hash, never an address.
	fn hit<'a>(&'a self, key: &'a [u8], limit: Limit) -> LimitFuture<'a>;
}

/// Fixed-window counters in the database, updated with compare-and-set, so
/// concurrent attempts on Postgres can't both take the last one.
#[derive(Clone)]
pub struct DbRateLimiter {
	db: Db,
}

impl DbRateLimiter {
	/// Counters in `db`'s `auth.rate_limits`.
	pub fn new(db: &Db) -> Self {
		Self {
			db: db.system("auth: count attempts"),
		}
	}

	async fn count(&self, key: &[u8], limit: Limit) -> Result<()> {
		let window = chrono::TimeDelta::from_std(limit.window)
			.map_err(|_| AuthError::Config("a rate limit window is too long".into()))?;
		let max = i32::try_from(limit.max).unwrap_or(i32::MAX);

		// Each pass reads the counter, then changes it only if nobody else did
		// in between. A lost race reads again; a busy key that keeps losing
		// is refused rather than let through.
		for _ in 0..8 {
			let now = leptoskit_db::now();
			let row = RateLimit::query()
				.filter(|limit| limit.key.eq(key.to_vec()))
				.first(&self.db)
				.await?;
			let Some(row) = row else {
				let inserted = NewRateLimit {
					key: key.to_vec(),
					window_start: now,
					count: 1,
				}
				.insert(&self.db)
				.await;

				match inserted {
					Ok(_) => return Ok(()),
					Err(DbError::Conflict { .. }) => continue,
					Err(error) => return Err(error.into()),
				}
			};
			let ends = row.window_start + window;
			let (window_start, count) = if now >= ends {
				(now, 1)
			} else if row.count >= max {
				return Err(AuthError::RateLimited {
					retry_after: (ends - now).to_std().unwrap_or_default(),
				});
			} else {
				(row.window_start, row.count + 1)
			};
			let changed = RateLimit::update()
				.filter(|limit| {
					limit.id.eq(row.id)
						& limit.window_start.eq(row.window_start)
						& limit.count.eq(row.count)
				})
				.set(|limit| limit.window_start, window_start)
				.set(|limit| limit.count, count)
				.run(&self.db)
				.await?;

			if changed == 1 {
				return Ok(());
			}
		}

		Err(AuthError::RateLimited {
			retry_after: Duration::from_secs(1),
		})
	}
}

impl RateLimiter for DbRateLimiter {
	fn hit<'a>(&'a self, key: &'a [u8], limit: Limit) -> LimitFuture<'a> {
		Box::pin(self.count(key, limit))
	}
}

/// The limits on each sign-in step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
	/// Codes sent to one address.
	pub code_per_address: Limit,
	/// Codes requested from one IP address.
	pub code_per_ip: Limit,
	/// Codes tried from one IP address: emailed codes, authenticator codes,
	/// and recovery codes.
	pub verify_per_ip: Limit,
	/// Authenticator and recovery codes tried for one account, from
	/// anywhere.
	pub second_factor_per_account: Limit,
}

impl Default for Limits {
	fn default() -> Self {
		Self {
			code_per_address: Limit::new(5, Duration::from_mins(15)),
			code_per_ip: Limit::new(20, Duration::from_hours(1)),
			verify_per_ip: Limit::new(30, Duration::from_mins(15)),
			second_factor_per_account: Limit::new(10, Duration::from_mins(15)),
		}
	}
}
