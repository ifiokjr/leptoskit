//! The `#[api]` attribute: publishes a leptos server function to generated
//! clients.
//!
//! This crate is an implementation detail of `leptoskit_rpc`; use the
//! attribute through `leptoskit_rpc::api`.
//!
//! The RPC itself is untouched: the function keeps its `#[server]` route,
//! encoding, and transport. `#[api]` sits above `#[server]` and does three
//! things:
//!
//! - pins the route to a stable `endpoint` — `#[api(name = "...")]` when given,
//!   the function name otherwise — when the `#[server]` attribute does not set
//!   one, because the default endpoint embeds a hash of the absolute manifest
//!   path and changes between checkouts, which no generated client could
//!   follow;
//! - rejects what no generated client could call: websocket protocols,
//!   streaming and multipart encodings, the positional `#[server]` form without
//!   an endpoint, and `#[server(flatten)]` arguments;
//! - with the `ssr` feature, registers a description of the function — the
//!   route, encodings, doc comment (including an `# Arguments` section), and a
//!   JSON Schema thunk for every argument and the return type — so
//!   `leptoskit_rpc::spec::collect` can export it and `leptoskit rpc generate`
//!   can write typed clients for any language.

use std::collections::HashMap;

use convert_case::Case;
use convert_case::Converter;
use proc_macro2::Span;
use proc_macro2::TokenStream;
use proc_macro2::TokenTree;
use quote::ToTokens;
use quote::quote;
use syn::Attribute;
use syn::Expr;
use syn::ExprLit;
use syn::FnArg;
use syn::Ident;
use syn::ItemFn;
use syn::Lit;
use syn::LitStr;
use syn::Meta;
use syn::Pat;
use syn::PatType;
use syn::Path;
use syn::Token;
use syn::Type;
use syn::ext::IdentExt;
use syn::parse::ParseStream;
use syn::parse::Parser;
use syn::punctuated::Punctuated;

mod channel;
mod cron;
mod operation;

use operation::OperationArgs;
use operation::OperationKind;

/// Declares a query: an `async fn` that reads, published under its name.
///
/// ```ignore
/// /// The latest messages in a channel, oldest first.
/// #[rpc::query]
/// pub async fn messages(cx: &rpc::QueryCx, channel: Id<Channel>, limit: u32) -> rpc::Result<Vec<PublicMessage>> {
/// 	let rows = Message::query().filter(|m| m.channel_id.eq(channel)).all(cx.db()).await?;
/// 	Ok(rows.into_iter().map(Into::into).collect())
/// }
/// ```
///
/// On every target it declares `Messages { channel, limit }`, the arguments
/// as one value that serializes and has a JSON Schema, and implements
/// `rpc::Operation` for it, so any client can send it with `rpc::call`. In a
/// server build (the `server` feature) it also keeps the function, runs it as
/// `Operation::run`, and registers it for `POST /api/messages`. A client
/// build has no body at all.
///
/// The first argument is the context, `cx: &rpc::QueryCx`; the rest are the
/// arguments, which must be owned and implement `Serialize`, `Deserialize`,
/// and `JsonSchema`. Doc comments, `#[serde(...)]`, and `#[schemars(...)]`
/// on an argument move onto its field.
///
/// Options: `name = "..."`, the operation's wire name when the function name
/// is taken, and `crate = "..."`, the path to `leptoskit_rpc`, such as `crate
/// = "leptoskit::rpc"`. Put `#[api]` above it to publish it to generated
/// clients.
#[proc_macro_attribute]
pub fn query(
	args: proc_macro::TokenStream,
	item: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
	operation_macro(OperationKind::Query, args, item)
}

/// Declares a mutation: an `async fn` that writes in one transaction,
/// published under its name.
///
/// ```ignore
/// /// Posts a message as the signed-in person.
/// #[rpc::mutation]
/// pub async fn send_message(cx: &rpc::MutationCx, channel: Id<Channel>, body: String) -> rpc::Result<Id<Message>> {
/// 	let message = NewMessage { channel_id: channel, body }.insert(cx.tx()).await?;
/// 	Ok(message.id)
/// }
/// ```
///
/// It expands like [`query`](macro@query), with `cx: &rpc::MutationCx` as
/// the context. The body runs in one transaction, `SERIALIZABLE` on
/// Postgres, and runs again when another transaction gets in its way: up to
/// five more times by default, or `retries = N`. So it must not reach the
/// outside world itself; `cx.after_commit(…)` runs work once the
/// transaction has committed.
/// Declares a channel: a join argument struct, the messages members send,
/// the presence they show, and the check that decides who may join.
///
/// ```ignore
/// #[rpc::channel(message = Typing, presence = Presence, authorize = may_join)]
/// pub struct Chat {
/// 	pub channel: Id<Channel>,
/// }
///
/// async fn may_join(cx: &rpc::QueryCx, chat: &Chat) -> rpc::Result<()> {
/// 	// read the membership as the caller
/// 	Ok(())
/// }
/// ```
///
/// The join arguments serialize and implement `rpc::Channel` on every
/// target, so `rpc::use_channel::<Chat>(…)` works in the browser and in a
/// native guest. In a server build it also registers who may join, which
/// runs as the connection's identity and again when that identity changes.
///
/// Options: `message = Type` (required), `authorize = path` (required),
/// `presence = Type` (default `()`), `name = "..."`, and `crate = "..."`.
#[proc_macro_attribute]
pub fn channel(
	args: proc_macro::TokenStream,
	item: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
	let item = TokenStream::from(item);
	let server = cfg!(feature = "server");

	match channel::expand(args.into(), item.clone(), server) {
		Ok(tokens) => tokens.into(),
		// Only the error: the struct's field types may not compile on a
		// client, and the server check certainly won't.
		Err(error) => error.to_compile_error().into(),
	}
}

#[proc_macro_attribute]
pub fn mutation(
	args: proc_macro::TokenStream,
	item: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
	operation_macro(OperationKind::Mutation, args, item)
}

/// Declares an action: an `async fn` that may call anything, published
/// under its name.
///
/// ```ignore
/// #[rpc::action]
/// pub async fn summarize_channel(cx: &rpc::ActionCx, channel: Id<Channel>) -> rpc::Result<String> {
/// 	let messages = cx.query(Messages { channel, limit: 200 }).await?;
/// 	let summary = my_llm::summarize(&messages).await?;
/// 	cx.mutate(PinSummary { channel, summary: summary.clone() }).await?;
/// 	Ok(summary)
/// }
/// ```
///
/// It expands like [`query`](macro@query), with `cx: &rpc::ActionCx` as the
/// context. An action isn't a transaction and never runs again by itself,
/// so it may reach the outside world; it reaches the database through
/// `cx.query(…)` and `cx.mutate(…)`, which run as the caller.
///
/// Options: `name = "..."` and `crate = "..."`.
#[proc_macro_attribute]
pub fn action(
	args: proc_macro::TokenStream,
	item: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
	operation_macro(OperationKind::Action, args, item)
}

/// Declares a job: work that runs later, in the background.
///
/// ```ignore
/// #[rpc::job(queue = "chain", serialize_by = payment, retries = 20,
///            backoff = exponential(2s, 5m), deadline = 15m, on_failure = payment_gave_up)]
/// pub async fn settle_payment(cx: &rpc::JobCx, payment: Id<Payment>) -> rpc::Result<()> {
/// 	// ...
/// 	Ok(())
/// }
/// ```
///
/// On every target it declares the argument struct, so a mutation enqueues it
/// with `cx.enqueue(SettlePayment { payment })`. In a server build it keeps
/// the body, registers it with the job runner, and runs it with
/// `rpc::JobCx`.
///
/// Options, besides `name = "..."` and `crate = "..."`:
///
/// - `queue = "mail"`: its queue, whose concurrency the server sets;
/// - `retries = N`: how many more attempts after a failure, four by default;
/// - `backoff = exponential(min, max)`: the wait between attempts;
/// - `deadline = 15m`: how long after enqueueing an attempt may still start;
/// - `timeout = 30s`: how long one attempt may run;
/// - `serialize_by = argument`: at most one running job per value of it;
/// - `visible_to = signed_in`, `everyone`, or an argument naming an account:
///   who may read it besides whoever enqueued it;
/// - `on_failure = mutation`: a `#[rpc::mutation]` taking `job:
///   rpc::Failed<ThisJob>`, run once when the job gives up.
///
/// Durations are a whole number and a unit: `ms`, `s`, `m`, `h`, or `d`.
#[proc_macro_attribute]
pub fn job(
	args: proc_macro::TokenStream,
	item: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
	operation_macro(OperationKind::Job, args, item)
}

/// Enqueues an argument-less job on a schedule: a five-field cron
/// expression in UTC, or `@hourly`, `@daily`, `@weekly`, `@monthly`, or
/// `@yearly`.
///
/// ```ignore
/// #[rpc::cron("*/5 * * * *")]
/// #[rpc::job]
/// pub async fn expire_trials(cx: &rpc::JobCx) -> rpc::Result<()> { … }
/// ```
///
/// Each instance running workers enqueues each tick, keyed by the tick, so
/// the job is enqueued once per tick however many instances there are.
/// Put it above `#[rpc::job]`. The expression is checked when it compiles.
#[proc_macro_attribute]
pub fn cron(
	args: proc_macro::TokenStream,
	item: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
	match cron::expand(args.into(), item.into(), cfg!(feature = "server")) {
		Ok(tokens) => tokens.into(),
		Err(error) => error.to_compile_error().into(),
	}
}

fn operation_macro(
	kind: OperationKind,
	args: proc_macro::TokenStream,
	item: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
	let item = TokenStream::from(item);

	match operation::expand(kind, args.into(), item.clone(), cfg!(feature = "server")) {
		Ok(tokens) => tokens.into(),
		// Only the error: the function's body may not compile on a client.
		Err(error) => error.to_compile_error().into(),
	}
}

/// Publishes a server function to generated clients.
///
/// Place it above `#[server]`:
///
/// ```ignore
/// /// Lists every todo.
/// ///
/// /// # Arguments
/// ///
/// /// * `limit` - How many to return.
/// #[api]
/// #[server]
/// pub async fn list_todos(limit: Option<u32>) -> Result<Vec<Todo>, ServerFnError> {
/// 	// ...
/// }
/// ```
///
/// It publishes `#[rpc::query]` and `#[rpc::mutation]` operations the same
/// way: place it above them.
///
/// Options:
///
/// - `internal` — the function stays callable, it is just not publicized:
///   generated clients leave it out unless `--include-internal` is passed;
/// - `name = "..."` — the operation name, when the function name is taken; it
///   also becomes the route, `/api/<name>`, unless `#[server]` sets an
///   `endpoint`;
/// - `tag = "..."` — a grouping tag (repeatable);
/// - `scope = Scope::PostsRead` — an OAuth scope a third-party app's access
///   token must carry to call the function (repeatable: every one is required).
///   The value is a variant of an enum declared with `leptoskit_rpc::scopes!`,
///   or a string literal. Without one, only the app's own sessions may call the
///   function;
/// - `crate = "..."` — the path to `leptoskit_rpc` when it is not a direct
///   dependency, such as `crate = "leptoskit::rpc"`.
///
/// Every argument type and the `Ok` type of the return value must implement
/// `schemars::JsonSchema`. Argument attributes `#[server(default)]`,
/// `#[server(rename = "...")]`, and `#[server(skip)]` are honored, and doc
/// comments on the arguments — or the `# Arguments` section of the function's
/// doc comment — document individual arguments in every generated client.
#[proc_macro_attribute]
pub fn api(
	args: proc_macro::TokenStream,
	item: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
	let item = TokenStream::from(item);

	match expand(args.into(), item.clone(), cfg!(feature = "ssr")) {
		Ok(tokens) => tokens.into(),
		// The function is kept as written, so the error is the only one:
		// callers do not also fail to find it.
		Err(error) => {
			let mut tokens = error.to_compile_error();
			tokens.extend(item);
			tokens.into()
		}
	}
}

/// The options `#[api(...)]` accepts.
#[derive(Default)]
struct ApiArgs {
	internal: bool,
	name: Option<LitStr>,
	tags: Vec<LitStr>,
	scopes: Vec<Expr>,
	krate: Option<Path>,
}

impl ApiArgs {
	fn parse(tokens: TokenStream) -> syn::Result<Self> {
		let metas = Punctuated::<Meta, Token![,]>::parse_terminated.parse2(tokens)?;
		let mut args = Self::default();

		for meta in metas {
			api_option(&mut args, &meta)?;
		}

		Ok(args)
	}
}

/// One `#[api(...)]` option.
fn api_option(args: &mut ApiArgs, meta: &Meta) -> syn::Result<()> {
	match meta {
		Meta::Path(path) if path.is_ident("internal") => args.internal = true,

		Meta::NameValue(pair) if pair.path.is_ident("name") => {
			if args.name.is_some() {
				return Err(syn::Error::new_spanned(pair, "`name` is given twice"));
			}

			args.name = Some(operation_name(pair)?);
		}

		Meta::NameValue(pair) if pair.path.is_ident("tag") => {
			args.tags.push(string_value(&pair.value, "tag")?);
		}

		Meta::NameValue(pair) if pair.path.is_ident("scope") => {
			args.scopes.push(scope_value(pair)?);
		}

		Meta::NameValue(pair) if pair.path.is_ident("crate") => {
			if args.krate.is_some() {
				return Err(syn::Error::new_spanned(pair, "`crate` is given twice"));
			}

			args.krate = Some(crate_path(pair)?);
		}

		other => {
			return Err(syn::Error::new_spanned(
				other,
				"unknown #[api] option; expected `internal`, `name = \"...\"`, `tag = \"...\"`, \
				 `scope = ...`, or `crate = \"...\"`",
			));
		}
	}

	Ok(())
}

/// The `name` option: the route segment and generated method names.
fn operation_name(pair: &syn::MetaNameValue) -> syn::Result<LitStr> {
	let name = string_value(&pair.value, "name")?;

	if !is_route_segment(&name.value()) {
		return Err(syn::Error::new_spanned(
			&name,
			"`name` becomes the route `/api/<name>` and the generated method names, so it must be \
			 non-empty and use ASCII letters, digits, `_`, `-`, and `.`, as in `chat.messages`",
		));
	}

	Ok(name)
}

/// The `scope` option, checked as an OAuth scope.
fn scope_value(pair: &syn::MetaNameValue) -> syn::Result<Expr> {
	if let Expr::Lit(ExprLit {
		lit: Lit::Str(name),
		..
	}) = &pair.value
		&& !is_scope_name(&name.value())
	{
		return Err(syn::Error::new_spanned(
			name,
			"a scope is one or more printable ASCII characters other than space, `\"`, and `\\`, \
			 such as \"posts:read\"",
		));
	}

	Ok(pair.value.clone())
}

/// The `crate` option: the path to `leptoskit_rpc`.
fn crate_path(pair: &syn::MetaNameValue) -> syn::Result<Path> {
	let path = string_value(&pair.value, "crate")?;
	path.parse::<Path>().map_err(|_| {
		syn::Error::new_spanned(
			&path,
			"`crate` expects the path to `leptoskit_rpc`, such as `crate = \"leptoskit::rpc\"`",
		)
	})
}

/// Whether `name` can be an OAuth scope (RFC 6749 §3.3), as
/// `leptoskit_rpc::is_scope_name` checks it.
fn is_scope_name(name: &str) -> bool {
	!name.is_empty()
		&& name
			.bytes()
			.all(|byte| matches!(byte, 0x21 | 0x23..=0x5b | 0x5d..=0x7e))
}

/// Whether `name` can stand alone as one path segment of a route: letters,
/// digits, `_`, `-`, and `.` for a namespaced operation such as
/// `chat.messages`.
fn is_route_segment(name: &str) -> bool {
	!name.is_empty()
		&& name.chars().all(|character| {
			character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.')
		})
}

/// One argument of the `#[server(...)]` attribute, kept as written so the
/// attribute can be rebuilt with an endpoint added.
enum ServerArg {
	/// `key = value`.
	Named { key: Ident, value: TokenStream },
	/// A legacy positional argument: the struct name or a string.
	Positional(TokenStream),
}

impl ToTokens for ServerArg {
	fn to_tokens(&self, tokens: &mut TokenStream) {
		match self {
			Self::Named { key, value } => tokens.extend(quote!(#key = #value)),
			Self::Positional(value) => tokens.extend(value.clone()),
		}
	}
}

/// The `#[server(...)]` arguments, and the parts of them `#[api]` reads.
#[derive(Default)]
struct ServerArgs {
	/// Every argument, in order, with its separators.
	args: Punctuated<ServerArg, Token![,]>,
	struct_name: Option<Ident>,
	has_endpoint: bool,
	positional: bool,
	input: Option<Type>,
	output: Option<Type>,
	protocol: Option<Type>,
	/// `custom = Wrapper`: leptos implements `ServerFn` for `Wrapper<Struct>`.
	custom: Option<Path>,
}

impl ServerArgs {
	/// A lenient parse of the server macro's arguments: just enough to learn
	/// the struct name, the codecs, the custom wrapper, whether an endpoint is
	/// set, and whether the legacy positional form is in use. The server macro
	/// itself reports anything malformed.
	fn parse(tokens: TokenStream) -> syn::Result<Self> {
		let parser = |input: ParseStream| {
			let mut args = Self::default();

			while !input.is_empty() {
				let position = args.args.len() + 1;
				let arg = if input.peek(Ident) && input.peek2(Token![=]) {
					let key: Ident = input.parse()?;
					input.parse::<Token![=]>()?;
					let value = args.named_arg(&key, input)?;
					ServerArg::Named { key, value }
				} else {
					args.positional_arg(input, position)?
				};

				args.args.push_value(arg);

				if input.is_empty() {
					break;
				}

				args.args.push_punct(input.parse()?);
			}

			Ok(args)
		};
		parser.parse2(tokens)
	}

	/// One `key = value` argument, recording what it says about the struct.
	fn named_arg(&mut self, key: &Ident, input: ParseStream<'_>) -> syn::Result<TokenStream> {
		let value = match key.to_string().as_str() {
			"name" => {
				let name: Ident = input.parse()?;
				self.struct_name = Some(name.clone());
				name.into_token_stream()
			}
			"endpoint" => {
				self.has_endpoint = true;
				input.parse::<LitStr>()?.into_token_stream()
			}
			"input" | "output" | "protocol" => {
				let codec: Type = input.parse()?;
				let slot = match key.to_string().as_str() {
					"input" => &mut self.input,
					"output" => &mut self.output,
					_ => &mut self.protocol,
				};
				*slot = Some(codec.clone());
				codec.into_token_stream()
			}
			"server" | "client" => input.parse::<Type>()?.into_token_stream(),
			"custom" => {
				let wrapper: Path = input.parse()?;
				self.custom = Some(wrapper.clone());
				wrapper.into_token_stream()
			}
			"prefix" | "encoding" => input.parse::<LitStr>()?.into_token_stream(),
			"impl_from" | "impl_deref" => input.parse::<syn::LitBool>()?.into_token_stream(),
			"input_derive" => input.parse::<syn::ExprTuple>()?.into_token_stream(),
			_ => input.parse::<Expr>()?.into_token_stream(),
		};

		Ok(value)
	}

	/// One positional argument, which the legacy form uses.
	fn positional_arg(
		&mut self,
		input: ParseStream<'_>,
		position: usize,
	) -> syn::Result<ServerArg> {
		if input.peek(Ident) {
			let ident: Ident = input.parse()?;
			self.positional = true;

			if position == 1 {
				self.struct_name = Some(ident.clone());
			}

			Ok(ServerArg::Positional(ident.into_token_stream()))
		} else if input.peek(LitStr) {
			let text: LitStr = input.parse()?;
			self.positional = true;

			if position == 4 {
				self.has_endpoint = true;
			}

			Ok(ServerArg::Positional(text.into_token_stream()))
		} else {
			Err(input.error("unexpected token in #[server] arguments"))
		}
	}

	/// The first codec no generated client can speak, with the reason.
	fn unsupported_codec(&self) -> Option<(Ident, &'static str)> {
		[&self.input, &self.output, &self.protocol]
			.into_iter()
			.flatten()
			.find_map(|codec| unsupported_codec(codec.to_token_stream()))
	}
}

/// Codecs and protocols with no HTTP request/response shape a generated
/// client could speak, by type name, with the reason.
const UNSUPPORTED_CODECS: &[(&str, &str)] = &[
	(
		"Websocket",
		"websocket protocols keep a connection open instead of answering one request",
	),
	(
		"Streaming",
		"streaming encodings send their body in chunks instead of one JSON value",
	),
	(
		"StreamingText",
		"streaming encodings send their body in chunks instead of one JSON value",
	),
	(
		"MultipartFormData",
		"multipart forms carry files, which have no JSON Schema",
	),
];

/// Finds an unsupported codec named anywhere in a codec type, including
/// inside `Http<Input, Output>`.
fn unsupported_codec(tokens: TokenStream) -> Option<(Ident, &'static str)> {
	for token in tokens {
		match token {
			TokenTree::Ident(ident) => {
				if let Some((_, reason)) =
					UNSUPPORTED_CODECS.iter().find(|(name, _)| ident == *name)
				{
					return Some((ident, reason));
				}
			}
			TokenTree::Group(group) => {
				if let Some(found) = unsupported_codec(group.stream()) {
					return Some(found);
				}
			}
			TokenTree::Punct(_) | TokenTree::Literal(_) => {}
		}
	}

	None
}

/// One argument of the server function, as it travels on the wire.
struct Param {
	wire_name: String,
	ty: Type,
	required: bool,
	docs: String,
}

/// Expands `#[api]`. `register` decides whether the description is
/// registered: the `ssr` feature in the real macro, either way in tests.
fn expand(args: TokenStream, item: TokenStream, register: bool) -> syn::Result<TokenStream> {
	let args = ApiArgs::parse(args)?;

	if let Ok(channel) = syn::parse2::<syn::ItemStruct>(item.clone()) {
		return expand_channel(&args, &channel, register);
	}

	let function: ItemFn = syn::parse2(item)?;

	match function
		.attrs
		.iter()
		.enumerate()
		.find_map(|(index, attr)| operation_attribute(attr).map(|kind| (index, kind)))
	{
		Some((index, kind)) => expand_operation(&args, function, index, kind, register),
		None => expand_server(&args, function, register),
	}
}

/// Publishes a leptos `#[server]` function.
fn expand_server(args: &ApiArgs, mut function: ItemFn, register: bool) -> syn::Result<TokenStream> {
	let server_index = function
		.attrs
		.iter()
		.position(is_server_attribute)
		.ok_or_else(|| {
			syn::Error::new(
				function.sig.ident.span(),
				"#[api] must be placed above #[server], #[rpc::query], #[rpc::mutation], or \
				 #[rpc::action]: it publishes a leptos server function or an operation",
			)
		})?;
	let server_attr = &function.attrs[server_index];

	let server_tokens = match &server_attr.meta {
		Meta::Path(_) => TokenStream::new(),
		Meta::List(list) => list.tokens.clone(),
		Meta::NameValue(pair) => {
			return Err(syn::Error::new_spanned(
				pair,
				"expected #[server] or #[server(...)]",
			));
		}
	};

	let mut server_args = ServerArgs::parse(server_tokens)?;

	if let Some((codec, reason)) = server_args.unsupported_codec() {
		return Err(syn::Error::new(
			codec.span(),
			format!(
				"#[api] cannot publish a `{codec}` server function: {reason}, so no generated \
				 client could call it. Remove #[api] from this function, or use an HTTP encoding \
				 such as `PostUrl`, `GetUrl`, or `Json`."
			),
		));
	}

	if server_args.positional && !server_args.has_endpoint {
		return Err(syn::Error::new_spanned(
			server_attr,
			"the positional #[server] form needs its fourth argument, the endpoint, under #[api]: \
			 without it the route embeds a hash of this crate's path and changes between \
			 checkouts. Add one, as in `#[server(Name, \"/api\", \"Url\", \"name\")]`, or use the \
			 named form, such as `#[server(prefix = \"/api\")]`, which #[api] gives a stable \
			 endpoint.",
		));
	}

	let fn_ident = function.sig.ident.clone();
	let fn_name = fn_ident.to_string();

	if let Some(unraw) = fn_name.strip_prefix("r#") {
		return Err(syn::Error::new(
			fn_ident.span(),
			format!(
				"leptos cannot generate a server function named with a raw identifier: its helper \
				 items would be named `__server_{fn_name}`, which is not an identifier. Rename \
				 the function and publish it as `{unraw}` with #[api(name = \"{unraw}\")]."
			),
		));
	}

	let name = args
		.name
		.clone()
		.unwrap_or_else(|| LitStr::new(&fn_name, fn_ident.span()));
	let krate = args
		.krate
		.clone()
		.unwrap_or_else(|| syn::parse_quote!(::leptoskit_rpc));

	let struct_ident = server_args.struct_name.clone().unwrap_or_else(|| {
		let struct_name = Converter::new()
			.from_case(Case::Snake)
			.to_case(Case::UpperCamel)
			.convert(&fn_name);
		Ident::new(&struct_name, fn_ident.span())
	});

	// The named form gets a stable endpoint; the positional form already has
	// one (checked above).
	if !server_args.positional && !server_args.has_endpoint {
		server_args.args.push(ServerArg::Named {
			key: Ident::new("endpoint", Span::call_site()),
			value: name.to_token_stream(),
		});
		let path = server_attr.path().clone();
		let tokens = &server_args.args;
		function.attrs[server_index] = syn::parse_quote!(#[#path(#tokens)]);
	}

	let raw_docs = docs_of(&function.attrs);
	let params = params_of(&function, &raw_docs)?;
	// The `# Arguments` section lands on the arguments themselves, so it is
	// stripped from the function docs to avoid saying everything twice.
	let docs = strip_arguments_section(&raw_docs);
	let deprecated = function
		.attrs
		.iter()
		.any(|attr| attr.path().is_ident("deprecated"));

	let registration = if register {
		// The type leptos implements `ServerFn` for.
		let server_fn: Type = match &server_args.custom {
			Some(wrapper) => syn::parse_quote!(#wrapper<#struct_ident>),
			None => syn::parse_quote!(#struct_ident),
		};

		let struct_name = LitStr::new(&struct_ident.to_string(), struct_ident.span());
		let docs = LitStr::new(&docs, Span::call_site());
		let internal = args.internal;
		let tags = &args.tags;
		// A literal is the scope itself; anything else is a variant of a
		// `scopes!` enum, whose `as_str` is a `const fn`.
		let scopes = args.scopes.iter().map(|scope| {
			if let Expr::Lit(_) = scope {
				scope.to_token_stream()
			} else {
				quote!((#scope).as_str())
			}
		});
		let param_descriptors = params.iter().map(|param| {
			let wire_name = LitStr::new(&param.wire_name, Span::call_site());
			let docs = LitStr::new(&param.docs, Span::call_site());
			let ty = &param.ty;
			let required = param.required;
			quote! {
				#krate::spec::ApiParamDescriptor {
					name: #wire_name,
					docs: #docs,
					schema: |generator| generator.subschema_for::<#ty>(),
					required: #required,
				}
			}
		});

		quote! {
			#krate::__private::inventory::submit! {
				#krate::spec::ApiFnDescriptor {
					name: #name,
					kind: #krate::spec::ApiKind::Call,
					module_path: ::core::module_path!(),
					struct_name: #struct_name,
					docs: #docs,
					path: <#server_fn as #krate::__private::ServerFn>::PATH,
					method: #krate::spec::method_of::<#server_fn>(),
					input_content_type: #krate::spec::input_content_type_of::<#server_fn>(),
					output_content_type: #krate::spec::output_content_type_of::<#server_fn>(),
					error_content_type: #krate::spec::error_content_type_of::<#server_fn>(),
					params: &[#(#param_descriptors),*],
					output_schema: |generator| {
						generator.subschema_for::<
							<#server_fn as #krate::__private::ServerFn>::Output,
						>()
					},
					internal: #internal,
					deprecated: #deprecated,
					tags: &[#(#tags),*],
					scopes: &[#(#scopes),*],
				}
			}
		}
	} else {
		TokenStream::new()
	};

	Ok(quote! {
		#function
		#registration
	})
}

/// Publishes an `#[rpc::query]` or `#[rpc::mutation]`: hands `#[api]`'s name
/// and crate path on to the operation macro, and registers the description
/// the spec is built from. The operation macro below declares the struct the
/// description names.
fn expand_operation(
	args: &ApiArgs,
	mut function: ItemFn,
	index: usize,
	kind: OperationKind,
	register: bool,
) -> syn::Result<TokenStream> {
	let tokens = operation_tokens(&function.attrs[index], kind)?;
	let operation_args = OperationArgs::parse(kind, tokens.clone())?;
	check_duplicates(args, &operation_args, kind)?;

	let signature = operation::signature(kind, &function)?;
	let fn_ident = function.sig.ident.clone();
	let name = args
		.name
		.clone()
		.or_else(|| operation_args.name.clone())
		.unwrap_or_else(|| LitStr::new(&fn_ident.unraw().to_string(), fn_ident.span()));
	let krate = args
		.krate
		.clone()
		.or_else(|| operation_args.krate.clone())
		.unwrap_or_else(|| syn::parse_quote!(::leptoskit_rpc));

	let forwarded = forward_options(args, tokens);
	let path = function.attrs[index].path().clone();
	function.attrs[index] = syn::parse_quote!(#[#path(#forwarded)]);

	let registration = if register {
		registration(args, &function, &signature, kind, &name, &krate)?
	} else {
		TokenStream::new()
	};

	Ok(quote! {
		#function
		#registration
	})
}

/// The arguments written on the operation attribute itself.
fn operation_tokens(attr: &Attribute, kind: OperationKind) -> syn::Result<TokenStream> {
	match &attr.meta {
		Meta::Path(_) => Ok(TokenStream::new()),
		Meta::List(list) => Ok(list.tokens.clone()),
		Meta::NameValue(pair) => {
			Err(syn::Error::new_spanned(
				pair,
				format!("expected #[rpc::{0}] or #[rpc::{0}(...)]", kind.attribute()),
			))
		}
	}
}

/// Rejects a name or crate path given on both `#[api]` and the operation.
fn check_duplicates(
	args: &ApiArgs,
	operation_args: &OperationArgs,
	kind: OperationKind,
) -> syn::Result<()> {
	if let (Some(api_name), Some(_)) = (&args.name, &operation_args.name) {
		return Err(syn::Error::new_spanned(
			api_name,
			format!(
				"the operation's name is given twice; keep it on #[rpc::{}(name = \"...\")], \
				 which names it for every client, and drop it from #[api]",
				kind.attribute()
			),
		));
	}

	if let (Some(api_crate), Some(_)) = (&args.krate, &operation_args.krate) {
		return Err(syn::Error::new_spanned(
			api_crate,
			format!(
				"the crate path is given twice; keep it on #[rpc::{}(crate = \"...\")] and drop \
				 it from #[api]",
				kind.attribute()
			),
		));
	}

	Ok(())
}

/// The operation macro names the operation and finds `leptoskit_rpc` itself,
/// so what `#[api]` was told goes on to it.
fn forward_options(args: &ApiArgs, tokens: TokenStream) -> Punctuated<TokenStream, Token![,]> {
	let mut forwarded: Punctuated<TokenStream, Token![,]> = Punctuated::new();

	if !tokens.is_empty() {
		forwarded.push(tokens);
	}

	if let Some(api_name) = &args.name {
		forwarded.push(quote!(name = #api_name));
	}

	if let Some(api_crate) = &args.krate {
		let text = LitStr::new(&api_crate.to_token_stream().to_string(), Span::call_site());
		forwarded.push(quote!(crate = #text));
	}

	forwarded
}

/// The `ApiFnDescriptor` a registered operation submits.
fn registration(
	args: &ApiArgs,
	function: &ItemFn,
	signature: &operation::Signature,
	kind: OperationKind,
	name: &LitStr,
	krate: &Path,
) -> syn::Result<TokenStream> {
	let struct_ident = operation::struct_ident(function);
	let struct_name = LitStr::new(&struct_ident.to_string(), struct_ident.span());
	let path = LitStr::new(&format!("/api/{}", name.value()), name.span());
	let docs = LitStr::new(
		&strip_arguments_section(&docs_of(&function.attrs)),
		Span::call_site(),
	);
	let deprecated = function
		.attrs
		.iter()
		.any(|attr| attr.path().is_ident("deprecated"));
	let internal = args.internal;
	let tags = &args.tags;
	let api_kind = kind.api_variant();
	let params = param_descriptors(signature, function, krate)?;

	Ok(quote! {
		#[allow(deprecated)]
		const _: () = {
			#krate::__private::inventory::submit! {
				#krate::spec::ApiFnDescriptor {
					name: #name,
					kind: #krate::spec::ApiKind::#api_kind,
					module_path: ::core::module_path!(),
					struct_name: #struct_name,
					docs: #docs,
					path: #path,
					method: #krate::spec::OPERATION_METHOD,
					input_content_type: "application/json",
					output_content_type: "application/json",
					error_content_type: "text/plain",
					params: &[#(#params),*],
					output_schema: |generator| {
						generator.subschema_for::<
							<#struct_ident as #krate::Operation>::Output,
						>()
					},
					internal: #internal,
					deprecated: #deprecated,
					tags: &[#(#tags),*],
					scopes: &[],
				}
			}
		};
	})
}

/// One `ApiParamDescriptor` per argument, with the docs from the function.
fn param_descriptors(
	signature: &operation::Signature,
	function: &ItemFn,
	krate: &Path,
) -> syn::Result<Vec<TokenStream>> {
	let raw_docs = docs_of(&function.attrs);
	let arguments = argument_docs(&raw_docs);
	let mut params = Vec::new();

	for argument in &signature.arguments {
		let ident = argument.ident.unraw().to_string();
		let (wire_name, required) = serde_field(&argument.field_attrs, &ident)?;
		let own_docs = docs_of(&argument.doc_attrs);
		let param_docs = if own_docs.is_empty() {
			arguments.get(&ident).cloned().unwrap_or_default()
		} else {
			own_docs
				.lines()
				.map(str::trim)
				.filter(|line| !line.is_empty())
				.collect::<Vec<_>>()
				.join(" ")
		};

		let wire_name = LitStr::new(&wire_name, Span::call_site());
		let param_docs = LitStr::new(&param_docs, Span::call_site());
		let ty = &argument.ty;
		params.push(quote! {
			#krate::spec::ApiParamDescriptor {
				name: #wire_name,
				docs: #param_docs,
				schema: |generator| generator.subschema_for::<#ty>(),
				required: #required,
			}
		});
	}

	Ok(params)
}

/// Publishes an `#[rpc::channel]`: its join arguments, messages and
/// presence, with the arguments' wire names in declaration order.
fn expand_channel(
	args: &ApiArgs,
	item: &syn::ItemStruct,
	register: bool,
) -> syn::Result<TokenStream> {
	let Some(channel_attr) = item.attrs.iter().find(|attr| {
		attr.path()
			.segments
			.last()
			.is_some_and(|segment| segment.ident == "channel")
	}) else {
		return Err(syn::Error::new(
			item.ident.span(),
			"#[api] on a struct publishes a channel, so it must be placed above \
			 #[rpc::channel(...)]",
		));
	};

	if args.name.is_some() {
		return Err(syn::Error::new(
			item.ident.span(),
			"a channel is named by #[rpc::channel(name = \"...\")], not by #[api]",
		));
	}

	if !args.scopes.is_empty() {
		return Err(syn::Error::new(
			item.ident.span(),
			"a channel decides who may join with its `authorize` function; #[api(scope = ...)] \
			 applies to calls only",
		));
	}

	let channel_args = match &channel_attr.meta {
		Meta::List(list) => channel::krate(list.tokens.clone())?,
		_ => None,
	};
	let krate = args
		.krate
		.clone()
		.or(channel_args)
		.unwrap_or_else(|| syn::parse_quote!(::leptoskit_rpc));

	if !register {
		return Ok(item.to_token_stream());
	}

	let rename_all = serde_rename_all(&item.attrs)?;
	let syn::Fields::Named(named) = &item.fields else {
		return Err(syn::Error::new_spanned(
			&item.fields,
			"a published channel's join arguments are a struct with named fields",
		));
	};
	let mut fields = Vec::new();

	for field in &named.named {
		let Some(ident) = &field.ident else {
			continue;
		};
		let ident = ident.unraw().to_string();
		let default_name = match &rename_all {
			Some(case) => {
				Converter::new()
					.from_case(Case::Snake)
					.to_case(*case)
					.convert(&ident)
			}
			None => ident.clone(),
		};
		let (wire_name, _) = serde_field(&field.attrs, &default_name)?;
		fields.push(LitStr::new(&wire_name, Span::call_site()));
	}

	let ident = &item.ident;
	let struct_name = LitStr::new(&ident.to_string(), ident.span());
	let docs = LitStr::new(&docs_of(&item.attrs), Span::call_site());
	let internal = args.internal;
	let tags = &args.tags;
	let deprecated = item
		.attrs
		.iter()
		.any(|attr| attr.path().is_ident("deprecated"));

	Ok(quote! {
		#item

		#[allow(deprecated)]
		const _: () = {
			#krate::__private::inventory::submit! {
				#krate::spec::ApiChannelDescriptor {
					name: <#ident as #krate::Channel>::NAME,
					module_path: ::core::module_path!(),
					struct_name: #struct_name,
					docs: #docs,
					fields: &[#(#fields),*],
					args_schema: |generator| generator.subschema_for::<#ident>(),
					message_schema: |generator| {
						generator.subschema_for::<<#ident as #krate::Channel>::Message>()
					},
					presence_schema: |generator| {
						generator.subschema_for::<<#ident as #krate::Channel>::Presence>()
					},
					internal: #internal,
					deprecated: #deprecated,
					tags: &[#(#tags),*],
				}
			}
		};
	})
}

/// The case `#[serde(rename_all = "...")]` puts field names in, when a
/// struct says.
fn serde_rename_all(attrs: &[Attribute]) -> syn::Result<Option<Case<'static>>> {
	for attr in attrs {
		if !attr.path().is_ident("serde") {
			continue;
		}

		let Meta::List(list) = &attr.meta else {
			continue;
		};
		let metas = Punctuated::<Meta, Token![,]>::parse_terminated.parse2(list.tokens.clone())?;

		for meta in metas {
			let Meta::NameValue(pair) = &meta else {
				continue;
			};

			if !pair.path.is_ident("rename_all") {
				continue;
			}

			let value = string_value(&pair.value, "rename_all")?;
			let case = match value.value().as_str() {
				"lowercase" => Case::Flat,
				"UPPERCASE" => Case::UpperFlat,
				"PascalCase" => Case::Pascal,
				"camelCase" => Case::Camel,
				"snake_case" => Case::Snake,
				"SCREAMING_SNAKE_CASE" => Case::UpperSnake,
				"kebab-case" => Case::Kebab,
				"SCREAMING-KEBAB-CASE" => Case::UpperKebab,
				_ => {
					return Err(syn::Error::new_spanned(
						value,
						"#[api] can't tell how this `rename_all` names a channel's fields; rename \
						 each field instead",
					));
				}
			};
			return Ok(Some(case));
		}
	}

	Ok(None)
}

/// An argument's name on the wire, and whether it must be sent, from the
/// `#[serde(rename = "...")]` and `#[serde(default)]` it carries.
fn serde_field(attrs: &[Attribute], ident: &str) -> syn::Result<(String, bool)> {
	let mut wire_name = ident.to_owned();
	let mut required = true;

	for attr in attrs {
		if !attr.path().is_ident("serde") {
			continue;
		}

		let Meta::List(list) = &attr.meta else {
			continue;
		};
		let metas = Punctuated::<Meta, Token![,]>::parse_terminated.parse2(list.tokens.clone())?;

		for meta in metas {
			serde_option(&meta, &mut wire_name, &mut required)?;
		}
	}

	Ok((wire_name, required))
}

/// One `#[serde(...)]` option on a field.
fn serde_option(meta: &Meta, wire_name: &mut String, required: &mut bool) -> syn::Result<()> {
	match meta {
		Meta::Path(path) if path.is_ident("default") => *required = false,

		Meta::NameValue(pair) if pair.path.is_ident("default") => *required = false,

		Meta::NameValue(pair) if pair.path.is_ident("rename") => {
			*wire_name = string_value(&pair.value, "rename")?.value();
		}

		// serde reports anything malformed.
		_ => {}
	}

	Ok(())
}
/// Whether an attribute is `#[rpc::query]` or `#[rpc::mutation]`, and which.
fn operation_attribute(attr: &Attribute) -> Option<OperationKind> {
	let segment = attr.path().segments.last()?;

	if segment.ident == "query" {
		Some(OperationKind::Query)
	} else if segment.ident == "mutation" {
		Some(OperationKind::Mutation)
	} else if segment.ident == "action" {
		Some(OperationKind::Action)
	} else {
		None
	}
}

/// Whether an attribute is `#[api]`, however it was imported.
fn is_api_attribute(attr: &Attribute) -> bool {
	attr.path()
		.segments
		.last()
		.is_some_and(|segment| segment.ident == "api")
}

/// Whether an attribute is the `#[server]` macro, however it was imported.
fn is_server_attribute(attr: &Attribute) -> bool {
	attr.path()
		.segments
		.last()
		.is_some_and(|segment| segment.ident == "server")
}

/// The arguments with their wire names, types, requirements, and docs,
/// leaving out `#[server(skip)]` ones, which the server never reads.
fn params_of(function: &ItemFn, docs: &str) -> syn::Result<Vec<Param>> {
	let arguments = argument_docs(docs);
	let mut params = Vec::new();

	for input in &function.sig.inputs {
		let typed = match input {
			FnArg::Receiver(receiver) => {
				return Err(syn::Error::new_spanned(
					receiver,
					"server functions cannot take `self`",
				));
			}
			FnArg::Typed(typed) => typed,
		};

		let ident = param_ident(typed)?;
		let (wire_name, required, skipped) = server_param(typed, &ident)?;

		if skipped {
			continue;
		}

		let own_docs = docs_of(&typed.attrs);
		params.push(Param {
			docs: if own_docs.is_empty() {
				arguments
					.get(&ident.to_string())
					.cloned()
					.unwrap_or_default()
			} else {
				own_docs
					.lines()
					.map(str::trim)
					.filter(|line| !line.is_empty())
					.collect::<Vec<_>>()
					.join(" ")
			},

			wire_name,
			ty: (*typed.ty).clone(),
			required,
		});
	}

	Ok(params)
}

/// The argument's name, which is its key on the wire.
fn param_ident(typed: &PatType) -> syn::Result<Ident> {
	match &*typed.pat {
		Pat::Ident(pattern) => Ok(pattern.ident.unraw()),
		Pat::Wild(wild) => {
			Err(syn::Error::new_spanned(
				wild,
				"#[api] needs a name for every argument: it is the argument's key on the wire",
			))
		}
		other => {
			Err(syn::Error::new_spanned(
				other,
				"#[api] needs a plain argument name here; destructuring patterns are not supported",
			))
		}
	}
}

/// The `#[server(...)]` options on one argument: its wire name, whether it
/// has a default, and whether the server skips it.
fn server_param(typed: &PatType, ident: &Ident) -> syn::Result<(String, bool, bool)> {
	let mut wire_name = ident.to_string();
	let mut required = true;
	let mut skipped = false;

	for attr in &typed.attrs {
		if !attr.path().is_ident("server") {
			continue;
		}

		let Meta::List(list) = &attr.meta else {
			continue;
		};
		let metas = Punctuated::<Meta, Token![,]>::parse_terminated.parse2(list.tokens.clone())?;

		for meta in metas {
			server_option(&meta, &mut wire_name, &mut required, &mut skipped)?;
		}
	}

	Ok((wire_name, required, skipped))
}

/// One `#[server(...)]` option on an argument.
fn server_option(
	meta: &Meta,
	wire_name: &mut String,
	required: &mut bool,
	skipped: &mut bool,
) -> syn::Result<()> {
	match meta {
		Meta::Path(path) if path.is_ident("default") => *required = false,

		Meta::NameValue(pair) if pair.path.is_ident("default") => {
			*required = false;
		}

		Meta::NameValue(pair) if pair.path.is_ident("rename") => {
			*wire_name = string_value(&pair.value, "rename")?.value();
		}

		Meta::Path(path) if path.is_ident("skip") => *skipped = true,

		Meta::Path(path) if path.is_ident("flatten") => {
			return Err(syn::Error::new_spanned(
				path,
				"#[api] cannot describe `#[server(flatten)]` arguments: their fields travel at \
				 the top level of the request, which generated clients would nest under the \
				 argument's name. Pass the fields as separate arguments, or the struct as one \
				 named argument.",
			));
		}

		// The server macro reports anything else.
		_ => {}
	}

	Ok(())
}
/// Removes the `# Arguments` section from a doc comment, leaving the rest.
fn strip_arguments_section(docs: &str) -> String {
	let mut out = String::new();
	let mut in_section = false;

	for line in docs.lines() {
		if let Some(heading) = line.trim().strip_prefix("# ") {
			in_section = heading.trim() == "Arguments";

			if !in_section {
				out.push_str(line);
				out.push('\n');
			}

			continue;
		}

		if !in_section {
			out.push_str(line);
			out.push('\n');
		}
	}

	out.trim().to_owned()
}

/// The `# Arguments` section of a doc comment: rustdoc's convention of
/// `` * `name` - description `` lines, keyed by argument name. An entry may
/// wrap onto further lines, up to the next entry or a blank line.
fn argument_docs(docs: &str) -> HashMap<String, String> {
	let mut arguments: HashMap<String, String> = HashMap::new();
	let mut in_section = false;
	// The argument a wrapped line continues.
	let mut current: Option<String> = None;

	for line in docs.lines() {
		let line = line.trim();

		if let Some(heading) = line.strip_prefix("# ") {
			in_section = heading.trim() == "Arguments";
			current = None;

			continue;
		}

		if !in_section {
			continue;
		}

		if line.is_empty() {
			current = None;
			continue;
		}

		let bullet = line.strip_prefix("* ").or_else(|| line.strip_prefix("- "));
		let entry = bullet.or_else(|| line.starts_with('`').then_some(line));
		let Some(entry) = entry else {
			if let Some(description) = current.as_ref().and_then(|name| arguments.get_mut(name)) {
				description.push(' ');
				description.push_str(line);
			}

			continue;
		};
		current = None;
		let Some((name, description)) = entry.split_once(" - ") else {
			continue;
		};
		let name = name.trim().trim_matches('`');
		let name = name.strip_prefix("r#").unwrap_or(name);

		if name.is_empty() || description.trim().is_empty() {
			continue;
		}

		arguments.insert(name.to_owned(), description.trim().to_owned());
		current = Some(name.to_owned());
	}

	arguments
}

/// The doc comment in `attrs`, `///` markers stripped and lines joined.
fn docs_of(attrs: &[Attribute]) -> String {
	let lines: Vec<String> = attrs
		.iter()
		.filter_map(|attr| {
			let Meta::NameValue(pair) = &attr.meta else {
				return None;
			};
			if !pair.path.is_ident("doc") {
				return None;
			}

			let Expr::Lit(ExprLit {
				lit: Lit::Str(text),
				..
			}) = &pair.value
			else {
				return None;
			};
			Some(text.value())
		})
		.flat_map(|text| {
			if text.is_empty() {
				// A bare `///` is a paragraph break; `lines()` would drop it.
				return vec![String::new()];
			}

			text.lines()
				.map(|line| line.strip_prefix(' ').unwrap_or(line).to_owned())
				.collect::<Vec<_>>()
		})
		.collect();
	lines.join("\n").trim().to_owned()
}

/// The string literal in `key = "..."`.
fn string_value(value: &Expr, key: &str) -> syn::Result<LitStr> {
	if let Expr::Lit(ExprLit {
		lit: Lit::Str(text),
		..
	}) = value
	{
		return Ok(text.clone());
	}

	Err(syn::Error::new_spanned(
		value,
		format!("`{key}` expects a string literal"),
	))
}

#[cfg(test)]
mod tests;
