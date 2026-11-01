//! The client: calls queries and mutations by their argument structs, the
//! same way on every platform.
//!
//! ```ignore
//! rpc::client::configure(ClientConfig::new().origin("https://myapp.com"));   // once, at startup
//!
//! let latest = rpc::client::call(Messages { channel, limit: 50 }).await?;
//!
//! // Cached, shared, and refetched like any `leptoskit_query` query:
//! let messages = use_rpc_query(move || Messages { channel: channel.get(), limit: 50 });
//! let send = use_mutation(mutation_scope::<SendMessage>());
//! send.mutate(SendMessage { channel, body });
//! ```
//!
//! Every target speaks the same protocol, `POST /api/<name>` with the
//! arguments as JSON, and the client picks how to send it:
//!
//! - **In the browser** (`web`), a same-origin `fetch`, which carries the
//!   session cookie.
//! - **In a native app** (`guest`), the HTTP plugin, with the credentials
//!   [`ClientConfig::credentials`] supplies, such as `leptoskit_auth`'s bearer
//!   tokens.
//! - **While the server renders** (`server`), in process: the request being
//!   rendered is identified as the routes would identify it, with no HTTP hop.
//!
//! A mutation is sent with an idempotency key, the same one for every
//! attempt, so when its answer is lost and the client sends it again, it
//! still runs once.

use std::sync::Arc;
use std::sync::PoisonError;
use std::sync::RwLock;

use leptoskit_query::BoxFuture;
use leptoskit_query::MutationHandle;
use leptoskit_query::MutationScope;
use leptoskit_query::QueryHandle;
use leptoskit_query::QueryScope;
use leptoskit_query::use_mutation;
use leptoskit_query::use_query;
use serde_json::Value;

use crate::FnError;
use crate::Operation;

#[cfg_attr(
	all(feature = "guest", target_arch = "wasm32"),
	allow(dead_code, reason = "a native app sends through the HTTP plugin")
)]
mod browser;
mod keys;
pub mod live;
#[cfg(all(feature = "guest", target_arch = "wasm32"))]
mod native;
mod protocol;
#[cfg(all(
	feature = "server",
	any(feature = "postgres", feature = "sqlite"),
	not(target_arch = "wasm32")
))]
mod server;
#[cfg(test)]
mod tests;

pub use live::ChannelEvent;
pub use live::ChannelHandle;
pub use live::LiveClient;
pub use live::use_channel;
pub use live::use_live;
pub use protocol::Credentials;

/// Where the client finds the server, and how this device proves who it
/// is. Pass it to [`configure`] once, when the app starts; the same call
/// works on every platform.
#[derive(Clone, Default)]
pub struct ClientConfig {
	/// The server's origin, such as `https://myapp.com`. A native app needs
	/// it. In the browser, leave it unset: the page's own origin is the one
	/// its session cookie belongs to.
	pub origin: Option<String>,
	/// What signs each request, such as a native app's bearer token. In the
	/// browser the session cookie needs none.
	pub credentials: Option<Arc<dyn Credentials>>,
}

impl ClientConfig {
	/// No origin and no credentials: the page's own origin, in the browser.
	pub fn new() -> Self {
		Self::default()
	}

	/// The server's origin, such as `https://myapp.com`.
	#[must_use]
	pub fn origin(mut self, origin: impl Into<String>) -> Self {
		self.origin = Some(origin.into());
		self
	}

	/// Signs each request with `credentials`. `leptoskit_auth`'s client
	/// sets this itself when its `rpc` feature is on.
	#[must_use]
	pub fn credentials(mut self, credentials: impl Credentials) -> Self {
		self.credentials = Some(Arc::new(credentials));
		self
	}
}

impl std::fmt::Debug for ClientConfig {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("ClientConfig")
			.field("origin", &self.origin)
			.field("credentials", &self.credentials.is_some())
			.finish()
	}
}

/// Options for one call.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CallOptions {
	/// A mutation's idempotency key: a repeat with the same key and
	/// arguments returns the first call's result instead of running again.
	/// [`call`] makes a fresh one for every mutation.
	pub idempotency_key: Option<String>,
}

impl CallOptions {
	/// No options.
	pub fn new() -> Self {
		Self::default()
	}

	/// Sends `key` as the idempotency key.
	#[must_use]
	pub fn idempotency_key(mut self, key: impl Into<String>) -> Self {
		self.idempotency_key = Some(key.into());
		self
	}
}

/// The idempotency key a call goes with.
pub(crate) enum Key {
	/// None: a query, or a call the app sends without one.
	None,
	/// A fresh one, made where the call is sent.
	Fresh,
	/// The app's own.
	Given(String),
}

impl Key {
	/// The key itself, made now if it is to be fresh.
	pub(crate) async fn resolve(self) -> Option<String> {
		match self {
			Self::None => None,
			Self::Fresh => Some(keys::new_key().await),
			Self::Given(key) => Some(key),
		}
	}
}

/// What every target's client can do: call an operation by name, with its
/// arguments as JSON. Whatever the call needs from where it was made, such
/// as the request being rendered, is read before this returns.
pub(crate) trait Api: Send + Sync + 'static {
	fn call(
		&self,
		name: &'static str,
		arguments: Value,
		key: Key,
	) -> BoxFuture<'static, Result<Value, FnError>>;
}

static CLIENT: RwLock<Option<Arc<dyn Api>>> = RwLock::new(None);

/// Tells the client where the server is, and how to sign requests. Call it
/// once, when the app starts; calling it again starts a fresh client.
/// Without it, the client uses the page's own origin, which works in the
/// browser; a native app answers each call with a configuration error until
/// it has an origin.
pub fn configure(config: ClientConfig) {
	live::install(&config);
	install(backend(config));
}

pub(crate) fn install(api: Arc<dyn Api>) {
	*CLIENT.write().unwrap_or_else(PoisonError::into_inner) = Some(api);
}

fn api() -> Arc<dyn Api> {
	if let Some(api) = CLIENT
		.read()
		.unwrap_or_else(PoisonError::into_inner)
		.as_ref()
	{
		return Arc::clone(api);
	}

	let mut client = CLIENT.write().unwrap_or_else(PoisonError::into_inner);
	Arc::clone(client.get_or_insert_with(|| backend(ClientConfig::default())))
}

/// While the server renders: in process.
#[cfg(all(
	feature = "server",
	any(feature = "postgres", feature = "sqlite"),
	not(target_arch = "wasm32")
))]
fn backend(_config: ClientConfig) -> Arc<dyn Api> {
	Arc::new(server::InProcess)
}

/// In a native app: the HTTP plugin.
#[cfg(all(feature = "guest", target_arch = "wasm32"))]
fn backend(config: ClientConfig) -> Arc<dyn Api> {
	Arc::new(protocol::Remote::new(config, native::Plugin))
}

/// In the browser: `fetch`.
#[cfg(all(feature = "web", not(feature = "guest"), target_arch = "wasm32"))]
fn backend(config: ClientConfig) -> Arc<dyn Api> {
	Arc::new(protocol::Remote::new(config, browser::Fetch))
}

/// Anywhere else there's no way to reach the server, and each call says so.
#[cfg(not(any(
	all(
		feature = "server",
		any(feature = "postgres", feature = "sqlite"),
		not(target_arch = "wasm32")
	),
	all(any(feature = "guest", feature = "web"), target_arch = "wasm32"),
)))]
fn backend(config: ClientConfig) -> Arc<dyn Api> {
	Arc::new(protocol::Remote::new(config, protocol::Unreachable))
}

/// Calls an operation and returns its result.
///
/// A mutation goes with a fresh idempotency key, so the client may send it
/// again when the network drops its answer, and it still runs once.
///
/// While the server renders, the call runs in process as the request being
/// rendered, which it reads when `call` is called, not when the future is
/// first polled.
///
/// # Errors
///
/// The operation's own failure, classified as [`FnError`]: an
/// `rpc::Error::Invalid` as [`FnError::Invalid`] and the others as
/// [`FnError::Server`]; [`FnError::Network`] when the server couldn't be
/// reached; [`FnError::Decode`] when its answer didn't fit `O::Output`.
#[expect(
	clippy::needless_pass_by_value,
	reason = "a call is written `call(Op { … })`: the arguments are built for it"
)]
pub fn call<O: Operation>(
	op: O,
) -> impl Future<Output = Result<O::Output, FnError>> + Send + 'static {
	let key = if O::KIND == crate::Kind::Mutation {
		Key::Fresh
	} else {
		Key::None
	};
	send(&op, key)
}

/// Calls an operation with explicit options, such as an idempotency key the
/// app keeps itself, for example across restarts.
///
/// # Errors
///
/// As [`call`].
#[expect(
	clippy::needless_pass_by_value,
	reason = "a call is written `call(Op { … })`: the arguments are built for it"
)]
pub fn call_with<O: Operation>(
	op: O,
	options: CallOptions,
) -> impl Future<Output = Result<O::Output, FnError>> + Send + 'static {
	send(&op, options.idempotency_key.map_or(Key::None, Key::Given))
}

fn send<O: Operation>(
	op: &O,
	key: Key,
) -> impl Future<Output = Result<O::Output, FnError>> + Send + 'static + use<O> {
	let pending = serde_json::to_value(op)
		.map(|arguments| api().call(O::NAME, arguments, key))
		.map_err(|error| FnError::Invalid(format!("the arguments aren't JSON: {error}")));
	async move {
		let value = pending?.await?;
		serde_json::from_value(value)
			.map_err(|error| FnError::Decode(format!("`{}` answered with {error}", O::NAME)))
	}
}

/// A query's arguments as a cache key: two keys are equal when their
/// arguments serialize the same, so the argument types needn't implement
/// `Hash` or `Eq`.
pub struct Args<O> {
	op: O,
	json: Arc<str>,
}

impl<O: Operation> Args<O> {
	/// The key for `op`.
	///
	/// # Panics
	///
	/// If `op` doesn't serialize, which the derived `Serialize` of an
	/// argument struct only does for a map with keys that aren't strings.
	pub fn new(op: O) -> Self {
		let json = serde_json::to_string(&op)
			.expect("an operation's arguments serialize, as they must to be sent");
		Self {
			op,
			json: Arc::from(json),
		}
	}

	/// The arguments.
	pub fn get(&self) -> &O {
		&self.op
	}

	/// The arguments, by value.
	pub fn into_inner(self) -> O {
		self.op
	}
}

impl<O: Operation> From<O> for Args<O> {
	fn from(op: O) -> Self {
		Self::new(op)
	}
}

impl<O: Clone> Clone for Args<O> {
	fn clone(&self) -> Self {
		Self {
			op: self.op.clone(),
			json: Arc::clone(&self.json),
		}
	}
}

impl<O> PartialEq for Args<O> {
	fn eq(&self, other: &Self) -> bool {
		self.json == other.json
	}
}

impl<O> Eq for Args<O> {}

impl<O> std::hash::Hash for Args<O> {
	fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
		self.json.hash(state);
	}
}

impl<O> std::fmt::Debug for Args<O> {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.write_str(&self.json)
	}
}

/// The `leptoskit_query` scope of a query, named `rpc:<name>`: its results
/// are cached by arguments and shared by every screen that reads them, and
/// travel from the server's render to the browser.
///
/// ```ignore
/// client.invalidate_scope(&query_scope::<Messages>()).await;
/// client.set_query_data(&query_scope::<Messages>(), Args::new(Messages { channel, limit: 50 }), rows);
/// ```
pub fn query_scope<Q>() -> QueryScope<Args<Q>, Q::Output, FnError>
where
	Q: Operation,
	Q::Output: Clone,
{
	QueryScope::new(|args: Args<Q>| call(args.into_inner()))
		.with_name(&format!("rpc:{}", Q::NAME))
		.with_ssr()
}

/// A background job's state: live over the app's socket when it has one,
/// so each step and progress update arrives as it's written, and otherwise
/// polled every second while the component is mounted.
///
/// ```ignore
/// let invite = use_job(move || invite_id.get().to_string());
/// invite.data().map(|job| job.state);
/// ```
#[cfg(any(not(feature = "server"), feature = "postgres", feature = "sqlite"))]
#[track_caller]
pub fn use_job(
	id: impl Fn() -> String + Send + Sync + 'static,
) -> QueryHandle<crate::jobs::JobStatus, FnError> {
	if live::available() {
		return use_live(move || crate::jobs::JobStatusArgs { id: id() });
	}
	let scope = QueryScope::new(|args: Args<crate::jobs::JobStatusArgs>| call(args.into_inner()))
		.with_name("rpc:job_status")
		.with_refetch_interval(std::time::Duration::from_secs(1));
	use_query(scope, move || {
		Args::new(crate::jobs::JobStatusArgs { id: id() })
	})
}

/// The signed-in person's inbox, newest first, live on every device they
/// have open, with the actions that clear it.
///
/// ```ignore
/// let inbox = use_inbox();
/// let unread = move || inbox.notices.data().map_or(0, |inbox| inbox.unread);
/// inbox.mark_read(notice.id.clone());
/// inbox.mark_all_read();
/// ```
#[cfg(any(not(feature = "server"), feature = "postgres", feature = "sqlite"))]
#[track_caller]
pub fn use_inbox() -> InboxHandle {
	InboxHandle {
		notices: use_live(|| crate::inbox::InboxArgs { limit: 50 }),
		read: use_mutation(mutation_scope::<crate::inbox::MarkRead>()),
		all_read: use_mutation(mutation_scope::<crate::inbox::MarkAllRead>()),
	}
}

/// What [`use_inbox`] returns.
#[cfg(any(not(feature = "server"), feature = "postgres", feature = "sqlite"))]
pub struct InboxHandle {
	/// The newest fifty notices and the unread count, as one query, so the
	/// badge never disagrees with the list.
	pub notices: QueryHandle<crate::inbox::Inbox, FnError>,
	read: MutationHandle<crate::inbox::MarkRead, (), FnError>,
	all_read: MutationHandle<crate::inbox::MarkAllRead, u64, FnError>,
}

#[cfg(any(not(feature = "server"), feature = "postgres", feature = "sqlite"))]
impl InboxHandle {
	/// How many notices are unread, once the inbox has loaded.
	pub fn unread(&self) -> u64 {
		self.notices.data().map_or(0, |inbox| inbox.unread)
	}

	/// Marks notice `id` read.
	pub fn mark_read(&self, id: impl Into<String>) {
		self.read.mutate(crate::inbox::MarkRead { id: id.into() });
	}

	/// Marks every notice read.
	pub fn mark_all_read(&self) {
		self.all_read.mutate(crate::inbox::MarkAllRead {});
	}
}

/// A query's result, as a query handle that refetches when the arguments
/// `args` reads change.
///
/// ```ignore
/// let messages = use_rpc_query(move || Messages { channel: channel.get(), limit: 50 });
/// ```
#[track_caller]
pub fn use_rpc_query<Q>(
	args: impl Fn() -> Q + Send + Sync + 'static,
) -> QueryHandle<Q::Output, FnError>
where
	Q: Operation,
	Q::Output: Clone,
{
	use_query(query_scope::<Q>(), move || Args::new(args()))
}

/// The `leptoskit_query` scope of a mutation, named `rpc:<name>`. Add an
/// optimistic update with `with_on_mutate`, and refresh the queries it
/// changes in `with_on_success`:
///
/// ```ignore
/// let send = use_mutation(
/// 	mutation_scope::<SendMessage>().with_on_success_async(|args| async move {
/// 		args.client.invalidate_scope(&query_scope::<Messages>()).await;
/// 	}),
/// );
/// ```
///
/// Each run sends a fresh idempotency key, and the client resends it with
/// the same key when the network drops the answer, so the mutation runs
/// once.
pub fn mutation_scope<M>() -> MutationScope<M, M::Output, FnError>
where
	M: Operation,
	M::Output: Clone,
{
	MutationScope::new(|op: M| call(op)).with_name(&format!("rpc:{}", M::NAME))
}

/// The `leptoskit_query` scope of an action, named `rpc:<name>`: run it with
/// `mutate`, and read whether it's pending and what it returned. An action
/// is never sent twice, since it isn't safe to repeat.
pub fn action_scope<A>() -> MutationScope<A, A::Output, FnError>
where
	A: Operation,
	A::Output: Clone,
{
	MutationScope::new(|op: A| call(op)).with_name(&format!("rpc:{}", A::NAME))
}

/// An action, ready to run from a screen: `.mutate(args)` runs it once, and
/// `.is_pending()` and `.data()` follow it.
///
/// ```ignore
/// let summarize = use_action::<SummarizeChannel>();
/// summarize.mutate(SummarizeChannel { channel });
/// let summary = move || summarize.data();
/// ```
#[track_caller]
pub fn use_action<A>() -> MutationHandle<A, A::Output, FnError>
where
	A: Operation,
	A::Output: Clone,
{
	use_mutation(action_scope::<A>())
}
