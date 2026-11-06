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
mod action;
#[cfg(feature = "live")]
pub mod channel;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
mod cx;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
mod http;

#[cfg(feature = "db")]
mod identity;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub mod inbox;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub mod jobs;
#[cfg(feature = "live")]
pub(crate) mod live;

#[cfg(feature = "db")]
pub mod receipts;
#[cfg(feature = "relay")]
pub mod relay;
mod roles;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
mod run;
#[cfg(feature = "db")]
mod services;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub mod stream;

#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use action::ActionCx;
#[cfg(feature = "live")]
pub use channel::ChannelRegistration;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use cx::MutationCx;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use cx::QueryCx;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use cx::ReadSink;
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
#[cfg(feature = "db")]
pub use identity::IdentityLease;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use jobs::JobCx;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use jobs::JobRegistration;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use jobs::JobRun;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use jobs::Jobs;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use jobs::JobsHandle;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use jobs::cancel;
#[cfg(feature = "live")]
pub use live::Live;
#[cfg(feature = "live")]
pub use live::LiveStats;
#[cfg(feature = "relay")]
pub use relay::PublisherStats;
#[cfg(feature = "relay")]
pub use relay::Relay;
#[cfg(feature = "relay")]
pub use relay::RelayError;
#[cfg(feature = "relay")]
pub use relay::RelayPublisher;
#[cfg(feature = "relay")]
pub use relay::RelaySettings;
pub use roles::ROLE_ENV;
pub use roles::Role;
pub use roles::Roles;
pub use roles::RolesError;
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
#[cfg(feature = "db")]
pub use services::APP_CALLER;
#[cfg(feature = "db")]
pub use services::SERVICE_CALLER_PREFIX;
#[cfg(feature = "db")]
pub use services::ServiceCallers;
#[cfg(feature = "live")]
pub(crate) use services::is_service;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use stream::StreamRegistration;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use stream::StreamSink;

/// What an operation's body receives: [`QueryCx`], [`MutationCx`],
/// [`ActionCx`], or [`JobCx`]. Sealed:
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

	/// Runs `op` as [`dispatch`](Self::dispatch) does, and adds what a query
	/// read to `reads`.
	#[cfg(any(feature = "postgres", feature = "sqlite"))]
	#[doc(hidden)]
	fn dispatch_recording<O: crate::Operation<Context = Self>>(
		db: &leptoskit_db::Db,
		identity: Identity,
		request_id: Option<&str>,
		reads: Option<ReadSink>,
		op: O,
	) -> impl Future<Output = crate::Result<O::Output>> + Send {
		let _ = reads;
		Self::dispatch(db, identity, request_id, op)
	}
}

pub(crate) mod sealed {
	pub trait Sealed {}
}

/// The context of an operation that runs in another service: what
/// `#[rpc::query(service = "...")]` gives the operation everywhere except
/// in the service itself. Its body isn't here, so nothing dispatches it; the
/// client sends it to the service instead.
#[doc(hidden)]
#[derive(Debug)]
pub struct RemoteCx {
	_private: (),
}

impl sealed::Sealed for RemoteCx {}

impl OperationContext for RemoteCx {
	#[cfg(any(feature = "postgres", feature = "sqlite"))]
	fn dispatch<O: crate::Operation<Context = Self>>(
		_db: &leptoskit_db::Db,
		_identity: Identity,
		_request_id: Option<&str>,
		_op: O,
	) -> impl Future<Output = crate::Result<O::Output>> + Send {
		std::future::ready(Err(remote_error::<O>()))
	}
}

/// What running a service's operation outside that service answers.
#[doc(hidden)]
#[must_use]
pub fn remote_error<O: crate::Operation>() -> crate::Error {
	remote_named_error(O::NAME, O::SERVICE)
}

/// What running a service's operation, job, or stream called `name`
/// outside that service answers.
#[doc(hidden)]
#[must_use]
pub fn remote_named_error(name: &str, service: Option<&str>) -> crate::Error {
	crate::Error::server(format!(
		"`{name}` runs in the `{}` service, not here: turn on the `service` feature of the crate \
		 that declares it in that service's build",
		service.unwrap_or("other")
	))
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
	/// The service that runs it, for `#[rpc::query(service = "...")]` and
	/// the like; `None` for the app's own. A [`Server`] serves only its own
	/// service's operations (see [`Server::service`]), so a service's
	/// operation linked into the app's build is never served by the app.
	pub service: Option<&'static str>,
	/// Decodes the arguments, runs the operation, and encodes its result.
	#[doc(hidden)]
	pub handler: run::Handler,
}

#[cfg(any(feature = "postgres", feature = "sqlite"))]
impl Registration {
	/// Whether a server for `service` (`None` for the app's own) serves it:
	/// its own service's operations, and the toolkit's built-in `rpc.*`
	/// operations, which every server has for its own jobs and inbox.
	#[must_use]
	pub fn served_by(&self, service: Option<&str>) -> bool {
		self.service == service || self.is_builtin()
	}

	/// Whether it is one of the toolkit's own `rpc.*` operations, which
	/// `leptoskit_rpc` itself registers: an app's operation named `rpc.…`
	/// isn't, so a service never serves it.
	#[must_use]
	pub fn is_builtin(&self) -> bool {
		self.service.is_none()
			&& self.name.starts_with("rpc.")
			&& (self.module_path == "leptoskit_rpc"
				|| self.module_path.starts_with("leptoskit_rpc::"))
	}
}

#[cfg(any(feature = "postgres", feature = "sqlite"))]
inventory::collect!(Registration);

/// Every registered operation, the app's and every linked service's, sorted
/// by service, then name.
///
/// # Errors
///
/// [`RegistryError`] when two operations of one service (or two of the
/// app's) share a name: the server couldn't tell which one a call means.
/// An app operation and a service operation may share a name, since each
/// is served only by its own server.
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub fn registrations() -> Result<Vec<&'static Registration>, RegistryError> {
	registrations_from(inventory::iter::<Registration>().collect())
}

/// The operations a server for `service` serves (`None` for the app's
/// own), sorted by name: see [`Registration::served_by`].
///
/// # Errors
///
/// As [`registrations`].
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub fn registrations_in(
	service: Option<&str>,
) -> Result<Vec<&'static Registration>, RegistryError> {
	let mut served: Vec<_> = registrations()?
		.into_iter()
		.filter(|registration| registration.served_by(service))
		.collect();
	served.sort_by(|a, b| a.name.cmp(b.name));
	Ok(served)
}

#[cfg(any(feature = "postgres", feature = "sqlite"))]
fn registrations_from(
	mut registrations: Vec<&'static Registration>,
) -> Result<Vec<&'static Registration>, RegistryError> {
	registrations.sort_by(|a, b| {
		a.service
			.cmp(&b.service)
			.then_with(|| a.name.cmp(b.name))
			.then_with(|| a.module_path.cmp(b.module_path))
	});

	for pair in registrations.windows(2) {
		if pair[0].service == pair[1].service && pair[0].name == pair[1].name {
			return Err(RegistryError {
				name: pair[0].name,
				first: pair[0].module_path,
				second: pair[1].module_path,
			});
		}
	}

	Ok(registrations)
}

/// The app's registered operation called `name`, or a built-in `rpc.*`
/// one: never a service's. See [`registration_in`].
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub fn registration(name: &str) -> Option<&'static Registration> {
	registration_in(None, name)
}

/// The operation called `name` that a server for `service` serves (`None`
/// for the app's own): see [`Registration::served_by`].
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub fn registration_in(service: Option<&str>, name: &str) -> Option<&'static Registration> {
	inventory::iter::<Registration>()
		.find(|registration| registration.name == name && registration.served_by(service))
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
		service: None,
		handler,
	};
	static B: Registration = Registration {
		name: "b",
		kind: crate::Kind::Mutation,
		module_path: "app::one",
		service: None,
		handler,
	};
	static A_AGAIN: Registration = Registration {
		name: "a",
		kind: crate::Kind::Mutation,
		module_path: "app::two",
		service: None,
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

	static A_IN_CARDS: Registration = Registration {
		name: "a",
		kind: crate::Kind::Query,
		module_path: "cards::one",
		service: Some("cards"),
		handler,
	};
	static A_IN_CARDS_AGAIN: Registration = Registration {
		name: "a",
		kind: crate::Kind::Query,
		module_path: "cards::two",
		service: Some("cards"),
		handler,
	};
	static BUILT_IN: Registration = Registration {
		name: "rpc.inbox",
		kind: crate::Kind::Query,
		module_path: "leptoskit_rpc",
		service: None,
		handler,
	};

	#[test]
	fn a_name_belongs_to_one_operation_per_service() {
		// The app's `a` and the cards service's `a` are served apart.
		let sorted = registrations_from(vec![&A_IN_CARDS, &B, &A]).expect("one per service");
		assert_eq!(sorted.len(), 3);

		let error =
			registrations_from(vec![&A_IN_CARDS_AGAIN, &A, &A_IN_CARDS]).expect_err("a clash");
		assert_eq!((error.first, error.second), ("cards::one", "cards::two"));
	}

	#[test]
	fn a_server_serves_only_its_own_services_operations_and_the_built_ins() {
		assert!(A.served_by(None));
		assert!(!A.served_by(Some("cards")));
		assert!(A_IN_CARDS.served_by(Some("cards")));
		assert!(!A_IN_CARDS.served_by(None));
		assert!(!A_IN_CARDS.served_by(Some("analytics")));
		assert!(BUILT_IN.served_by(None) && BUILT_IN.served_by(Some("cards")));
	}

	static APP_NAMED_LIKE_A_BUILT_IN: Registration = Registration {
		name: "rpc.audit",
		kind: crate::Kind::Query,
		module_path: "app::audit",
		service: None,
		handler,
	};

	#[test]
	fn only_the_toolkits_own_rpc_operations_are_built_in() {
		assert!(BUILT_IN.is_builtin());
		// An app's operation named `rpc.…` is the app's: no service serves it.
		assert!(!APP_NAMED_LIKE_A_BUILT_IN.is_builtin());
		assert!(APP_NAMED_LIKE_A_BUILT_IN.served_by(None));
		assert!(!APP_NAMED_LIKE_A_BUILT_IN.served_by(Some("cards")));
	}
}
