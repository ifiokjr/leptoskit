//! Typed OAuth scopes: what a third-party app may do with a person's
//! account once they let it sign in with yours.
//!
//! Declare them once, with a sentence each for the consent screen:
//!
//! ```
//! leptoskit_rpc::scopes! {
//! 	/// What other apps may do for the people who connect them.
//! 	pub enum Scope {
//! 		/// Read your posts and drafts.
//! 		PostsRead = "posts:read",
//! 		/// Create, edit and delete your posts.
//! 		PostsWrite = "posts:write",
//! 	}
//! }
//!
//! use leptoskit_rpc::Scopes;
//!
//! assert_eq!(Scope::PostsRead.as_str(), "posts:read");
//! assert_eq!(
//! 	Scope::ALL[1].description(),
//! 	"Create, edit and delete your posts."
//! );
//! ```
//!
//! Then require one on a function with `#[api(scope = Scope::PostsRead)]`.
//! A third-party app's token reaches that function only when it carries the
//! scope, and the function runs as the person who granted it.

/// One scope: its name on the wire, and what it lets an app do.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ScopeInfo {
	/// The scope as tokens and `#[api]` carry it, such as `posts:read`.
	pub name: &'static str,
	/// The doc comment on the variant, `///` markers stripped, lines joined
	/// with `\n`.
	pub docs: &'static str,
}

impl ScopeInfo {
	/// The doc comment as one sentence for a consent screen: lines trimmed
	/// and joined with spaces.
	pub fn description(&self) -> String {
		self.docs
			.lines()
			.map(str::trim)
			.filter(|line| !line.is_empty())
			.collect::<Vec<_>>()
			.join(" ")
	}
}

/// A set of scopes, as [`scopes!`](crate::scopes!) declares it.
pub trait Scopes: Copy + 'static {
	/// Every scope in the set, in declaration order.
	const ALL: &'static [ScopeInfo];
}

/// Whether `name` can be a scope: one or more printable ASCII characters
/// other than space, `"` and `\`, as RFC 6749 §3.3 allows.
pub fn is_scope_name(name: &str) -> bool {
	!name.is_empty()
		&& name
			.bytes()
			.all(|byte| matches!(byte, 0x21 | 0x23..=0x5b | 0x5d..=0x7e))
}

/// Declares an enum of OAuth scopes. Each variant names its scope and
/// documents what it allows; the doc comment is what a consent screen
/// shows.
///
/// The enum gets `as_str`, a `const fn`, and implements
/// [`Scopes`](crate::Scopes). See the [module docs](mod@crate::scopes) for an
/// example.
#[macro_export]
macro_rules! scopes {
	(
		$(#[$meta:meta])*
		$vis:vis enum $name:ident {
			$(
				$(#[doc = $doc:literal])*
				$variant:ident = $scope:literal
			),+ $(,)?
		}
	) => {
		$(#[$meta])*
		#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
		$vis enum $name {
			$(
				$(#[doc = $doc])*
				$variant,
			)+
		}

		impl $name {
			/// The scope as tokens and `#[api(scope = …)]` carry it.
			pub const fn as_str(self) -> &'static str {
				match self {
					$(Self::$variant => $scope,)+
				}
			}
		}

		impl $crate::Scopes for $name {
			const ALL: &'static [$crate::ScopeInfo] = &[
				$($crate::ScopeInfo {
					name: $scope,
					docs: ::core::concat!($($doc, "\n",)*),
				},)+
			];
		}
	};
}

#[cfg(test)]
mod tests {
	use super::*;

	crate::scopes! {
		/// Test scopes.
		pub(crate) enum Scope {
			/// Read your posts
			/// and drafts.
			PostsRead = "posts:read",
			Undocumented = "misc",
		}
	}

	#[test]
	fn the_macro_names_and_describes_each_scope() {
		assert_eq!(Scope::PostsRead.as_str(), "posts:read");
		assert_eq!(Scope::Undocumented.as_str(), "misc");
		assert_eq!(Scope::ALL.len(), 2);
		assert_eq!(Scope::ALL[0].description(), "Read your posts and drafts.");
		assert_eq!(Scope::ALL[1].description(), "");
	}

	#[test]
	fn scope_names_follow_rfc_6749() {
		for name in ["openid", "posts:read", "https://api.example/read", "a!#[]~"] {
			assert!(is_scope_name(name), "{name}");
		}
		for name in ["", "two words", "quote\"", "back\\slash", "tab\t", "é"] {
			assert!(!is_scope_name(name), "{name}");
		}
	}
}
