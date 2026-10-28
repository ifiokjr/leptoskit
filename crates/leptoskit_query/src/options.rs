//! Query options and their defaults.
//!
//! Options layer: client-wide [`QueryDefaults`] under a scope's
//! [`QueryOptions`], under any overrides a single observer passes. Every
//! field is optional so that an unset value inherits from the layer below;
//! [`ResolvedQueryOptions`] is the flattened result with TanStack Query's
//! defaults filling whatever nobody set.

use std::time::Duration;

use crate::retry::Backoff;
use crate::retry::Retry;
use crate::retry::RetryDelay;
use crate::retry::RetryPolicy;
use crate::time::millis;

/// How long fetched data counts as fresh.
///
/// Fresh data is served from the cache without a fetch; stale data is served
/// too, but a background refetch starts whenever a new observer mounts, the
/// window regains focus, or the network reconnects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StaleTime {
	/// Fresh for this long after the last successful fetch.
	Duration(Duration),
	/// Never stale: only an explicit invalidation or refetch fetches again.
	Static,
}

impl StaleTime {
	/// Stale as soon as it is fetched, the TanStack default.
	pub const ZERO: Self = Self::Duration(Duration::ZERO);

	/// Whether data fetched at `data_updated_at` is stale at `now`.
	#[must_use]
	pub fn is_stale_at(&self, data_updated_at: u64, now: u64) -> bool {
		match self {
			Self::Static => false,
			Self::Duration(duration) => now.saturating_sub(data_updated_at) >= millis(*duration),
		}
	}
}

impl From<Duration> for StaleTime {
	fn from(duration: Duration) -> Self {
		Self::Duration(duration)
	}
}

/// How long an unobserved query stays cached.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GcTime {
	/// Dropped this long after its last observer leaves.
	Duration(Duration),
	/// Kept until removed explicitly.
	Never,
}

impl GcTime {
	/// Five minutes, the TanStack default in the browser.
	pub const DEFAULT: Self = Self::Duration(Duration::from_secs(5 * 60));
}

impl From<Duration> for GcTime {
	fn from(duration: Duration) -> Self {
		Self::Duration(duration)
	}
}

/// When an event (mount, window focus, reconnect) triggers a refetch.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RefetchOn {
	/// Refetch only if the data is stale.
	#[default]
	Stale,
	/// Refetch every time.
	Always,
	/// Never refetch on this event.
	Never,
}

/// Periodic background refetching while a query is observed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RefetchInterval {
	/// No polling.
	#[default]
	Off,
	/// Refetch this often.
	Every(Duration),
}

impl From<Duration> for RefetchInterval {
	fn from(duration: Duration) -> Self {
		Self::Every(duration)
	}
}

/// Whether a fetch may run while the device is offline.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NetworkMode {
	/// Pause until the network is back; the default.
	#[default]
	Online,
	/// Always fetch, for example against a local cache or in-memory source.
	Always,
	/// Try once while offline — a service worker may answer — then pause
	/// retries until the network is back.
	OfflineFirst,
}

/// Options for one query scope or one observer; unset fields inherit.
pub struct QueryOptions<E> {
	/// See [`StaleTime`].
	pub stale_time: Option<StaleTime>,
	/// See [`GcTime`].
	pub gc_time: Option<GcTime>,
	/// See [`Retry`].
	pub retry: Option<Retry<E>>,
	/// See [`RetryDelay`].
	pub retry_delay: Option<RetryDelay<E>>,
	/// Whether a query that last failed retries when a new observer mounts.
	pub retry_on_mount: Option<bool>,
	/// See [`RefetchInterval`].
	pub refetch_interval: Option<RefetchInterval>,
	/// Whether interval refetching continues while the window is unfocused.
	pub refetch_interval_in_background: Option<bool>,
	/// Refetch policy when an observer mounts.
	pub refetch_on_mount: Option<RefetchOn>,
	/// Refetch policy when the window regains focus.
	pub refetch_on_window_focus: Option<RefetchOn>,
	/// Refetch policy when the network reconnects.
	pub refetch_on_reconnect: Option<RefetchOn>,
	/// See [`NetworkMode`].
	pub network_mode: Option<NetworkMode>,
}

impl<E> std::fmt::Debug for QueryOptions<E> {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("QueryOptions")
			.field("stale_time", &self.stale_time)
			.field("gc_time", &self.gc_time)
			.field("retry", &self.retry)
			.field("retry_delay", &self.retry_delay)
			.field("retry_on_mount", &self.retry_on_mount)
			.field("refetch_interval", &self.refetch_interval)
			.field(
				"refetch_interval_in_background",
				&self.refetch_interval_in_background,
			)
			.field("refetch_on_mount", &self.refetch_on_mount)
			.field("refetch_on_window_focus", &self.refetch_on_window_focus)
			.field("refetch_on_reconnect", &self.refetch_on_reconnect)
			.field("network_mode", &self.network_mode)
			.finish()
	}
}

impl<E> Clone for QueryOptions<E> {
	fn clone(&self) -> Self {
		Self {
			stale_time: self.stale_time,
			gc_time: self.gc_time,
			retry: self.retry.clone(),
			retry_delay: self.retry_delay.clone(),
			retry_on_mount: self.retry_on_mount,
			refetch_interval: self.refetch_interval,
			refetch_interval_in_background: self.refetch_interval_in_background,
			refetch_on_mount: self.refetch_on_mount,
			refetch_on_window_focus: self.refetch_on_window_focus,
			refetch_on_reconnect: self.refetch_on_reconnect,
			network_mode: self.network_mode,
		}
	}
}

impl<E> Default for QueryOptions<E> {
	fn default() -> Self {
		Self {
			stale_time: None,
			gc_time: None,
			retry: None,
			retry_delay: None,
			retry_on_mount: None,
			refetch_interval: None,
			refetch_interval_in_background: None,
			refetch_on_mount: None,
			refetch_on_window_focus: None,
			refetch_on_reconnect: None,
			network_mode: None,
		}
	}
}

impl<E> QueryOptions<E> {
	/// Options that inherit everything.
	#[must_use]
	pub fn new() -> Self {
		Self::default()
	}

	/// See [`StaleTime`].
	#[must_use]
	pub fn with_stale_time(mut self, stale_time: impl Into<StaleTime>) -> Self {
		self.stale_time = Some(stale_time.into());
		self
	}

	/// See [`GcTime`].
	#[must_use]
	pub fn with_gc_time(mut self, gc_time: impl Into<GcTime>) -> Self {
		self.gc_time = Some(gc_time.into());
		self
	}

	/// See [`Retry`].
	#[must_use]
	pub fn with_retry(mut self, retry: impl Into<Retry<E>>) -> Self {
		self.retry = Some(retry.into());
		self
	}

	/// See [`RetryDelay`].
	#[must_use]
	pub fn with_retry_delay(mut self, retry_delay: impl Into<RetryDelay<E>>) -> Self {
		self.retry_delay = Some(retry_delay.into());
		self
	}

	/// Whether a query that last failed retries when a new observer mounts.
	#[must_use]
	pub fn with_retry_on_mount(mut self, retry_on_mount: bool) -> Self {
		self.retry_on_mount = Some(retry_on_mount);
		self
	}

	/// See [`RefetchInterval`].
	#[must_use]
	pub fn with_refetch_interval(mut self, interval: impl Into<RefetchInterval>) -> Self {
		self.refetch_interval = Some(interval.into());
		self
	}

	/// Whether interval refetching continues while the window is unfocused.
	#[must_use]
	pub fn with_refetch_interval_in_background(mut self, in_background: bool) -> Self {
		self.refetch_interval_in_background = Some(in_background);
		self
	}

	/// Refetch policy when an observer mounts.
	#[must_use]
	pub fn with_refetch_on_mount(mut self, refetch: RefetchOn) -> Self {
		self.refetch_on_mount = Some(refetch);
		self
	}

	/// Refetch policy when the window regains focus.
	#[must_use]
	pub fn with_refetch_on_window_focus(mut self, refetch: RefetchOn) -> Self {
		self.refetch_on_window_focus = Some(refetch);
		self
	}

	/// Refetch policy when the network reconnects.
	#[must_use]
	pub fn with_refetch_on_reconnect(mut self, refetch: RefetchOn) -> Self {
		self.refetch_on_reconnect = Some(refetch);
		self
	}

	/// See [`NetworkMode`].
	#[must_use]
	pub fn with_network_mode(mut self, network_mode: NetworkMode) -> Self {
		self.network_mode = Some(network_mode);
		self
	}

	/// Overlays `other` on `self`: set fields in `other` win.
	#[must_use]
	pub fn merge(mut self, other: &Self) -> Self {
		macro_rules! take {
			($($field:ident),*) => {
				$(if let Some(value) = &other.$field {
					self.$field = Some(value.clone());
				})*
			};
		}
		take!(
			stale_time,
			gc_time,
			retry,
			retry_delay,
			retry_on_mount,
			refetch_interval,
			refetch_interval_in_background,
			refetch_on_mount,
			refetch_on_window_focus,
			refetch_on_reconnect,
			network_mode
		);
		self
	}
}

/// Client-wide defaults for every query.
///
/// Retry settings here cannot inspect the error, since defaults apply to
/// every error type; a scope's [`QueryOptions`] can.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct QueryDefaults {
	/// See [`StaleTime`].
	pub stale_time: Option<StaleTime>,
	/// See [`GcTime`].
	pub gc_time: Option<GcTime>,
	/// See [`RetryPolicy`].
	pub retry: Option<RetryPolicy>,
	/// See [`Backoff`].
	pub retry_delay: Option<Backoff>,
	/// Whether a query that last failed retries when a new observer mounts.
	pub retry_on_mount: Option<bool>,
	/// See [`RefetchInterval`].
	pub refetch_interval: Option<RefetchInterval>,
	/// Whether interval refetching continues while the window is unfocused.
	pub refetch_interval_in_background: Option<bool>,
	/// Refetch policy when an observer mounts.
	pub refetch_on_mount: Option<RefetchOn>,
	/// Refetch policy when the window regains focus.
	pub refetch_on_window_focus: Option<RefetchOn>,
	/// Refetch policy when the network reconnects.
	pub refetch_on_reconnect: Option<RefetchOn>,
	/// See [`NetworkMode`].
	pub network_mode: Option<NetworkMode>,
}

impl QueryDefaults {
	/// Defaults that inherit everything from TanStack's built-ins.
	#[must_use]
	pub fn new() -> Self {
		Self::default()
	}

	/// See [`StaleTime`].
	#[must_use]
	pub fn with_stale_time(mut self, stale_time: impl Into<StaleTime>) -> Self {
		self.stale_time = Some(stale_time.into());
		self
	}

	/// See [`GcTime`].
	#[must_use]
	pub fn with_gc_time(mut self, gc_time: impl Into<GcTime>) -> Self {
		self.gc_time = Some(gc_time.into());
		self
	}

	/// See [`RetryPolicy`].
	#[must_use]
	pub fn with_retry(mut self, retry: RetryPolicy) -> Self {
		self.retry = Some(retry);
		self
	}

	/// See [`Backoff`].
	#[must_use]
	pub fn with_retry_delay(mut self, backoff: Backoff) -> Self {
		self.retry_delay = Some(backoff);
		self
	}

	/// Whether a query that last failed retries when a new observer mounts.
	#[must_use]
	pub fn with_retry_on_mount(mut self, retry_on_mount: bool) -> Self {
		self.retry_on_mount = Some(retry_on_mount);
		self
	}

	/// See [`RefetchInterval`].
	#[must_use]
	pub fn with_refetch_interval(mut self, interval: impl Into<RefetchInterval>) -> Self {
		self.refetch_interval = Some(interval.into());
		self
	}

	/// Whether interval refetching continues while the window is unfocused.
	#[must_use]
	pub fn with_refetch_interval_in_background(mut self, in_background: bool) -> Self {
		self.refetch_interval_in_background = Some(in_background);
		self
	}

	/// Refetch policy when an observer mounts.
	#[must_use]
	pub fn with_refetch_on_mount(mut self, refetch: RefetchOn) -> Self {
		self.refetch_on_mount = Some(refetch);
		self
	}

	/// Refetch policy when the window regains focus.
	#[must_use]
	pub fn with_refetch_on_window_focus(mut self, refetch: RefetchOn) -> Self {
		self.refetch_on_window_focus = Some(refetch);
		self
	}

	/// Refetch policy when the network reconnects.
	#[must_use]
	pub fn with_refetch_on_reconnect(mut self, refetch: RefetchOn) -> Self {
		self.refetch_on_reconnect = Some(refetch);
		self
	}

	/// See [`NetworkMode`].
	#[must_use]
	pub fn with_network_mode(mut self, network_mode: NetworkMode) -> Self {
		self.network_mode = Some(network_mode);
		self
	}
}

/// Every option with a concrete value.
pub struct ResolvedQueryOptions<E> {
	/// See [`StaleTime`].
	pub stale_time: StaleTime,
	/// See [`GcTime`].
	pub gc_time: GcTime,
	/// See [`Retry`].
	pub retry: Retry<E>,
	/// See [`RetryDelay`].
	pub retry_delay: RetryDelay<E>,
	/// Whether a query that last failed retries when a new observer mounts.
	pub retry_on_mount: bool,
	/// See [`RefetchInterval`].
	pub refetch_interval: RefetchInterval,
	/// Whether interval refetching continues while the window is unfocused.
	pub refetch_interval_in_background: bool,
	/// Refetch policy when an observer mounts.
	pub refetch_on_mount: RefetchOn,
	/// Refetch policy when the window regains focus.
	pub refetch_on_window_focus: RefetchOn,
	/// Refetch policy when the network reconnects.
	pub refetch_on_reconnect: RefetchOn,
	/// See [`NetworkMode`].
	pub network_mode: NetworkMode,
}

impl<E> std::fmt::Debug for ResolvedQueryOptions<E> {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("ResolvedQueryOptions")
			.field("stale_time", &self.stale_time)
			.field("gc_time", &self.gc_time)
			.field("retry", &self.retry)
			.field("retry_delay", &self.retry_delay)
			.field("retry_on_mount", &self.retry_on_mount)
			.field("refetch_interval", &self.refetch_interval)
			.field(
				"refetch_interval_in_background",
				&self.refetch_interval_in_background,
			)
			.field("refetch_on_mount", &self.refetch_on_mount)
			.field("refetch_on_window_focus", &self.refetch_on_window_focus)
			.field("refetch_on_reconnect", &self.refetch_on_reconnect)
			.field("network_mode", &self.network_mode)
			.finish()
	}
}

impl<E> Clone for ResolvedQueryOptions<E> {
	fn clone(&self) -> Self {
		Self {
			stale_time: self.stale_time,
			gc_time: self.gc_time,
			retry: self.retry.clone(),
			retry_delay: self.retry_delay.clone(),
			retry_on_mount: self.retry_on_mount,
			refetch_interval: self.refetch_interval,
			refetch_interval_in_background: self.refetch_interval_in_background,
			refetch_on_mount: self.refetch_on_mount,
			refetch_on_window_focus: self.refetch_on_window_focus,
			refetch_on_reconnect: self.refetch_on_reconnect,
			network_mode: self.network_mode,
		}
	}
}

impl<E> ResolvedQueryOptions<E> {
	/// Flattens the layers, with TanStack's defaults underneath.
	///
	/// On the server a query never retries and is never garbage collected:
	/// the client lives for one request and is dropped with it.
	#[must_use]
	pub fn resolve(defaults: &QueryDefaults, options: &QueryOptions<E>, on_server: bool) -> Self {
		Self {
			stale_time: options
				.stale_time
				.or(defaults.stale_time)
				.unwrap_or(StaleTime::ZERO),
			gc_time: options
				.gc_time
				.or(defaults.gc_time)
				.unwrap_or(if on_server {
					GcTime::Never
				} else {
					GcTime::DEFAULT
				}),
			retry: options
				.retry
				.clone()
				.or(defaults.retry.map(Retry::Policy))
				.unwrap_or(if on_server {
					Retry::never()
				} else {
					Retry::count(3)
				}),
			retry_delay: options
				.retry_delay
				.clone()
				.or(defaults.retry_delay.map(RetryDelay::Backoff))
				.unwrap_or_default(),
			retry_on_mount: options
				.retry_on_mount
				.or(defaults.retry_on_mount)
				.unwrap_or(true),
			refetch_interval: options
				.refetch_interval
				.or(defaults.refetch_interval)
				.unwrap_or_default(),
			refetch_interval_in_background: options
				.refetch_interval_in_background
				.or(defaults.refetch_interval_in_background)
				.unwrap_or(false),
			refetch_on_mount: options
				.refetch_on_mount
				.or(defaults.refetch_on_mount)
				.unwrap_or_default(),
			refetch_on_window_focus: options
				.refetch_on_window_focus
				.or(defaults.refetch_on_window_focus)
				.unwrap_or_default(),
			refetch_on_reconnect: options
				.refetch_on_reconnect
				.or(defaults.refetch_on_reconnect)
				.unwrap_or_default(),
			network_mode: options
				.network_mode
				.or(defaults.network_mode)
				.unwrap_or_default(),
		}
	}

	/// Applies one observer's overrides on top of the query's options.
	#[must_use]
	pub fn overlay(&self, options: &QueryOptions<E>) -> Self {
		Self {
			stale_time: options.stale_time.unwrap_or(self.stale_time),
			gc_time: options.gc_time.unwrap_or(self.gc_time),
			retry: options.retry.clone().unwrap_or_else(|| self.retry.clone()),
			retry_delay: options
				.retry_delay
				.clone()
				.unwrap_or_else(|| self.retry_delay.clone()),
			retry_on_mount: options.retry_on_mount.unwrap_or(self.retry_on_mount),
			refetch_interval: options.refetch_interval.unwrap_or(self.refetch_interval),
			refetch_interval_in_background: options
				.refetch_interval_in_background
				.unwrap_or(self.refetch_interval_in_background),
			refetch_on_mount: options.refetch_on_mount.unwrap_or(self.refetch_on_mount),
			refetch_on_window_focus: options
				.refetch_on_window_focus
				.unwrap_or(self.refetch_on_window_focus),
			refetch_on_reconnect: options
				.refetch_on_reconnect
				.unwrap_or(self.refetch_on_reconnect),
			network_mode: options.network_mode.unwrap_or(self.network_mode),
		}
	}
}
