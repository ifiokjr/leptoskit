//! Running operations on the server: the registry `#[rpc::query]` and
//! `#[rpc::mutation]` fill, who is calling, the contexts bodies receive, and
//! the `POST /api/<name>` routes.
//!
//! ```ignore
//! let rpc = rpc::server::Server::new(db.clone()).identify(auth.clone());
//! let app = Router::new()
//! 	.merge(rpc.router())          // POST /api/<name> for every query and mutation
//! 	.merge(auth.router())
//! 	.layer(auth.layer());         // resolves the session first
//! ```
//!
//! Queries and mutations run on a database, so everything past the registry
//! needs a driver: the `postgres` or `sqlite` feature.

#[cfg(any(feature = "postgres", feature = "sqlite"))]
mod cx;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
mod http;
#[cfg(feature = "db")]
mod identity;
#[cfg(feature = "live")]
mod live;
#[cfg(feature = "db")]
pub mod receipts;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
mod run;

#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use cx::MutationCx;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use cx::QueryCx;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use http::IDEMPOTENCY_KEY;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use http::Server;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub(crate) use http::installed;
#[cfg(feature = "db")]
pub use identity::Anonymous;
#[cfg(feature = "db")]
#[doc(hidden)]
pub use identity::HasContext;
#[cfg(feature = "db")]
pub use identity::Identify;
#[cfg(feature = "db")]
pub use identity::Identity;
#[cfg(feature = "live")]
pub use live::Live;
#[cfg(feature = "live")]
pub use live::LiveStats;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use run::Invocation;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use run::Target;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use run::call;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use run::call_as;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
#[doc(hidden)]
pub use run::handle;

/// What an operation's body receives: [`QueryCx`] or [`MutationCx`]. Sealed:
/// a new kind of operation brings its own.
pub trait OperationContext: Send + Sync + Sized + 'static + sealed::Sealed {
	/// Runs `op` as `identity`, on `db`, with a mutation's retries and
	/// receipt.
	#[cfg(any(feature = "postgres", feature = "sqlite"))]
	#[doc(hidden)]
	fn dispatch<O: crate::Operation<Context = Self>>(
		db: &leptoskit_db::Db,
		identity: Identity,
		request_id: Option<&str>,
		op: O,
	) -> impl Future<Output = crate::Result<O::Output>> + Send;
}

pub(crate) mod sealed {
	pub trait Sealed {}
}

/// One registered operation, as `#[rpc::query]` and `#[rpc::mutation]`
/// submit it in a server build.
#[cfg(any(feature = "postgres", feature = "sqlite"))]
#[derive(Debug)]
pub struct Registration {
	/// The operation's name: its route is `/api/<name>`.
	pub name: &'static str,
	/// Its kind.
	pub kind: crate::Kind,
	/// `module_path!()` where it was declared.
	pub module_path: &'static str,
	/// Decodes the arguments, runs the operation, and encodes its result.
	#[doc(hidden)]
	pub handler: run::Handler,
}

#[cfg(any(feature = "postgres", feature = "sqlite"))]
inventory::collect!(Registration);

/// Every registered operation, sorted by name.
///
/// # Errors
///
/// [`RegistryError`] when two operations share a name: the server couldn't
/// tell which one a call means.
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub fn registrations() -> Result<Vec<&'static Registration>, RegistryError> {
	registrations_from(inventory::iter::<Registration>().collect())
}

#[cfg(any(feature = "postgres", feature = "sqlite"))]
fn registrations_from(
	mut registrations: Vec<&'static Registration>,
) -> Result<Vec<&'static Registration>, RegistryError> {
	registrations.sort_by(|a, b| {
		a.name
			.cmp(b.name)
			.then_with(|| a.module_path.cmp(b.module_path))
	});
	for pair in registrations.windows(2) {
		if pair[0].name == pair[1].name {
			return Err(RegistryError {
				name: pair[0].name,
				first: pair[0].module_path,
				second: pair[1].module_path,
			});
		}
	}
	Ok(registrations)
}

/// The registered operation called `name`.
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub fn registration(name: &str) -> Option<&'static Registration> {
	inventory::iter::<Registration>().find(|registration| registration.name == name)
}

/// Two operations share a name.
#[cfg(any(feature = "postgres", feature = "sqlite"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error(
	"two operations are named `{name}` (in `{first}` and `{second}`); rename one, or give one its \
	 own `name = \"...\"`"
)]
pub struct RegistryError {
	/// The shared name.
	pub name: &'static str,
	/// Where the first is declared.
	pub first: &'static str,
	/// Where the second is declared.
	pub second: &'static str,
}

#[cfg(all(test, any(feature = "postgres", feature = "sqlite")))]
mod tests {
	use super::*;

	fn handler(
		_: Invocation,
	) -> leptoskit_query::BoxFuture<'static, crate::Result<serde_json::Value>> {
		Box::pin(std::future::ready(Ok(serde_json::Value::Null)))
	}

	static A: Registration = Registration {
		name: "a",
		kind: crate::Kind::Query,
		module_path: "app::one",
		handler,
	};
	static B: Registration = Registration {
		name: "b",
		kind: crate::Kind::Mutation,
		module_path: "app::one",
		handler,
	};
	static A_AGAIN: Registration = Registration {
		name: "a",
		kind: crate::Kind::Mutation,
		module_path: "app::two",
		handler,
	};

	#[test]
	fn a_name_belongs_to_one_operation() {
		let sorted = registrations_from(vec![&B, &A]).expect("distinct names");
		assert_eq!(
			sorted.iter().map(|r| r.name).collect::<Vec<_>>(),
			["a", "b"]
		);

		let error = registrations_from(vec![&A_AGAIN, &B, &A]).expect_err("a clash");
		assert_eq!(error.name, "a");
		assert_eq!((error.first, error.second), ("app::one", "app::two"));
		assert!(error.to_string().contains("two operations are named `a`"));
	}
}
