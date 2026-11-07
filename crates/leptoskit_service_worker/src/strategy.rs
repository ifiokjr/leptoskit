//! Caching strategies a service worker can follow for a request.
//!
//! The page describes the policy in Rust; the worker script (or a small
//! runtime that reads these values over [`post_message`](crate::post_message))
//! decides whether to answer from the cache, the network, or both.

use serde::Deserialize;
use serde::Serialize;

/// How a service worker should answer a matching request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Strategy {
	/// Try the cache; if nothing matches, go to the network and store the
	/// answer.
	CacheFirst,
	/// Try the network; if it fails (or times out), use the cache.
	NetworkFirst {
		/// Optional network budget in milliseconds. `None` waits until the
		/// browser gives up.
		#[serde(default, skip_serializing_if = "Option::is_none")]
		timeout_ms: Option<u32>,
	},
	/// Answer from the cache immediately when present, and refresh it from
	/// the network in the background.
	StaleWhileRevalidate,
	/// Always go to the network; never read or write the cache.
	NetworkOnly,
	/// Only the cache; never the network.
	CacheOnly,
}

impl Strategy {
	/// Cache first, writing through on a miss.
	#[must_use]
	pub const fn cache_first() -> Self {
		Self::CacheFirst
	}

	/// Network first, with no timeout of our own.
	#[must_use]
	pub const fn network_first() -> Self {
		Self::NetworkFirst { timeout_ms: None }
	}

	/// Network first, falling back to the cache after `timeout_ms`.
	#[must_use]
	pub const fn network_first_within(timeout_ms: u32) -> Self {
		Self::NetworkFirst {
			timeout_ms: Some(timeout_ms),
		}
	}

	/// Stale while revalidate.
	#[must_use]
	pub const fn stale_while_revalidate() -> Self {
		Self::StaleWhileRevalidate
	}
}

/// Where a strategy looks first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
	/// The Cache Storage entry for the request.
	Cache,
	/// The network.
	Network,
}

impl Strategy {
	/// The source to try first.
	#[must_use]
	pub const fn prefer(&self) -> Source {
		match self {
			Self::CacheFirst | Self::StaleWhileRevalidate | Self::CacheOnly => Source::Cache,
			Self::NetworkFirst { .. } | Self::NetworkOnly => Source::Network,
		}
	}

	/// Whether a failed preferred source should fall back to the other.
	#[must_use]
	pub const fn fallback(&self) -> bool {
		matches!(self, Self::CacheFirst | Self::NetworkFirst { .. })
	}

	/// Whether a successful network response should be written to the cache.
	#[must_use]
	pub const fn write_through(&self) -> bool {
		matches!(
			self,
			Self::CacheFirst | Self::NetworkFirst { .. } | Self::StaleWhileRevalidate
		)
	}

	/// Whether a cache hit should still trigger a background network refresh.
	#[must_use]
	pub const fn revalidate(&self) -> bool {
		matches!(self, Self::StaleWhileRevalidate)
	}
}

/// One cache rule: a Cache Storage name, a strategy, and the URL prefixes it
/// covers. Post it to the worker so the worker script can apply it without
/// hard-coding routes in two places.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheRule {
	/// The Cache Storage name, such as `static-v1`.
	pub cache_name: String,
	/// How matching requests are answered.
	pub strategy: Strategy,
	/// URL prefixes (path or absolute) this rule covers. An empty list means
	/// the whole origin the worker controls.
	#[serde(default)]
	pub url_prefixes: Vec<String>,
}

impl CacheRule {
	/// A rule for `cache_name` using `strategy`.
	#[must_use]
	pub fn new(cache_name: impl Into<String>, strategy: Strategy) -> Self {
		Self {
			cache_name: cache_name.into(),
			strategy,
			url_prefixes: Vec::new(),
		}
	}

	/// Limits the rule to URLs that start with any of `prefixes`.
	#[must_use]
	pub fn prefixes(mut self, prefixes: impl IntoIterator<Item = impl Into<String>>) -> Self {
		self.url_prefixes = prefixes.into_iter().map(Into::into).collect();
		self
	}

	/// Whether `url` matches this rule's prefixes.
	#[must_use]
	pub fn matches_url(&self, url: &str) -> bool {
		self.url_prefixes.is_empty()
			|| self
				.url_prefixes
				.iter()
				.any(|prefix| url.starts_with(prefix))
	}
}

/// A message the page can send so a companion worker script installs or
/// replaces its cache rules.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WorkerMessage {
	/// Replace the worker's cache rules with `rules`.
	SetCacheRules { rules: Vec<CacheRule> },
	/// Ask the worker to skip waiting and activate immediately.
	SkipWaiting,
	/// An application-defined payload, as JSON.
	App { value: serde_json::Value },
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn strategies_describe_their_behaviour() {
		assert_eq!(Strategy::cache_first().prefer(), Source::Cache);
		assert!(Strategy::cache_first().fallback());
		assert!(Strategy::cache_first().write_through());
		assert!(!Strategy::cache_first().revalidate());

		let network = Strategy::network_first_within(2_000);
		assert_eq!(network.prefer(), Source::Network);
		assert!(network.fallback());
		assert!(network.write_through());

		assert!(Strategy::stale_while_revalidate().revalidate());
		assert!(!Strategy::NetworkOnly.fallback());
		assert!(!Strategy::CacheOnly.write_through());
	}

	#[test]
	fn cache_rules_match_prefixes() {
		let rule = CacheRule::new("static-v1", Strategy::cache_first())
			.prefixes(["/assets/", "https://cdn.example/"]);
		assert!(rule.matches_url("/assets/app.js"));
		assert!(rule.matches_url("https://cdn.example/a.png"));
		assert!(!rule.matches_url("/api/items"));
		assert!(CacheRule::new("all", Strategy::NetworkOnly).matches_url("/anything"));
	}

	#[test]
	fn worker_messages_round_trip() {
		let message = WorkerMessage::SetCacheRules {
			rules: vec![CacheRule::new("pages", Strategy::network_first())],
		};
		let json = serde_json::to_value(&message).unwrap();
		assert_eq!(json["kind"], "set_cache_rules");
		assert_eq!(
			serde_json::from_value::<WorkerMessage>(json).unwrap(),
			message
		);
	}
}
