//! `#[rpc::query]` and `#[rpc::mutation]`: an operation declared as an
//! `async fn`, turned into an arguments struct every target can build and
//! send, and, in a server build, the body that runs it.
//!
//! ```ignore
//! /// The latest messages in a channel.
//! #[rpc::query]
//! pub async fn messages(cx: &rpc::QueryCx, channel: Id<Channel>, limit: u32) -> rpc::Result<Vec<PublicMessage>> {
//! 	// …
//! }
//! ```
//!
//! expands, on every target, to
//!
//! ```ignore
//! /// The latest messages in a channel.
//! #[derive(Clone, Serialize, Deserialize, JsonSchema)]
//! pub struct Messages { pub channel: Id<Channel>, pub limit: u32 }
//!
//! impl rpc::Operation for Messages {
//! 	const NAME: &'static str = "messages";
//! 	const KIND: rpc::Kind = rpc::Kind::Query;
//! 	type Output = Vec<PublicMessage>;
//! }
//! ```
//!
//! and, in a server build, adds the function itself, `Operation::run`, and
//! the operation's entry in the registry the HTTP routes serve.

use convert_case::Case;
use convert_case::Converter;
use proc_macro2::Span;
use proc_macro2::TokenStream;
use quote::ToTokens;
use quote::format_ident;
use quote::quote;
use quote::quote_spanned;
use syn::Attribute;
use syn::FnArg;
use syn::GenericArgument;
use syn::Ident;
use syn::ItemFn;
use syn::LitInt;
use syn::LitStr;
use syn::Meta;
use syn::Pat;
use syn::Path;
use syn::PathArguments;
use syn::ReturnType;
use syn::Token;
use syn::Type;
use syn::ext::IdentExt;
use syn::parse::Parser;
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;

/// Which operation macro is expanding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OperationKind {
	Query,
	Mutation,
	Action,
	Job,
}

impl OperationKind {
	/// Every kind.
	const ALL: [Self; 4] = [Self::Query, Self::Mutation, Self::Action, Self::Job];

	/// The attribute's name, as written: `query` or `mutation`.
	pub(crate) fn attribute(self) -> &'static str {
		match self {
			Self::Query => "query",
			Self::Mutation => "mutation",
			Self::Action => "action",
			Self::Job => "job",
		}
	}

	/// The context type its first argument takes.
	fn context(self) -> &'static str {
		match self {
			Self::Query => "QueryCx",
			Self::Mutation => "MutationCx",
			Self::Action => "ActionCx",
			Self::Job => "JobCx",
		}
	}

	/// What its context does, for errors about a missing or wrong one.
	fn context_does(self) -> &'static str {
		match self {
			Self::Query => "reads the database as the caller",
			Self::Mutation => "writes in one transaction as the caller",
			Self::Action => "calls queries and mutations as the caller, and anything else",
			Self::Job => "has its own database handle and runs later",
		}
	}

	/// The `Kind` variant, as `leptoskit_rpc` names it.
	fn variant(self) -> Ident {
		match self {
			Self::Query => format_ident!("Query"),
			Self::Mutation => format_ident!("Mutation"),
			Self::Action => format_ident!("Action"),
			Self::Job => format_ident!("Job"),
		}
	}

	/// The `ApiKind` variant in the spec.
	pub(crate) fn api_variant(self) -> Ident {
		self.variant()
	}
}

/// The options `#[rpc::query(...)]`, `#[rpc::mutation(...)]`,
/// `#[rpc::action(...)]`, and `#[rpc::job(...)]` accept.
#[derive(Default)]
pub(crate) struct OperationArgs {
	pub(crate) name: Option<LitStr>,
	pub(crate) krate: Option<Path>,
	retries: Option<LitInt>,
	job: JobArgs,
}

/// The options only `#[rpc::job(...)]` accepts.
#[derive(Default)]
struct JobArgs {
	queue: Option<LitStr>,
	backoff: Option<(TokenStream, TokenStream)>,
	deadline: Option<TokenStream>,
	timeout: Option<TokenStream>,
	serialize_by: Option<Ident>,
	visible_to: Option<Ident>,
	on_failure: Option<Path>,
}

/// The options each kind takes, for the error about one it doesn't.
fn expected_options(kind: OperationKind) -> &'static str {
	match kind {
		OperationKind::Query | OperationKind::Action => "`name = \"...\"` or `crate = \"...\"`",
		OperationKind::Mutation => "`name = \"...\"`, `retries = N`, or `crate = \"...\"`",
		OperationKind::Job => {
			"`name`, `crate`, `queue`, `retries`, `backoff`, `deadline`, `timeout`, \
			 `serialize_by`, `visible_to`, or `on_failure`"
		}
	}
}

/// A duration written as a number with a unit, `500ms`, `30s`, `15m`, `2h`,
/// or `7d`, as tokens that build a `Duration`.
fn duration_value(value: &syn::Expr, option: &str, example: &str) -> syn::Result<TokenStream> {
	let error = || {
		syn::Error::new_spanned(
			value,
			format!(
				"`{option}` expects a duration: a whole number and a unit, `ms`, `s`, `m`, `h`, \
				 or `d`, such as `{example}`"
			),
		)
	};
	let syn::Expr::Lit(syn::ExprLit {
		lit: syn::Lit::Int(literal),
		..
	}) = value
	else {
		return Err(error());
	};
	let amount: u64 = literal.base10_parse().map_err(|_| error())?;

	let millis = match literal.suffix() {
		"ms" => Some(amount),
		"s" => amount.checked_mul(1_000),
		"m" => amount.checked_mul(60_000),
		"h" => amount.checked_mul(3_600_000),
		"d" => amount.checked_mul(86_400_000),
		_ => return Err(error()),
	}
	.ok_or_else(error)?;
	Ok(quote!(::core::time::Duration::from_millis(#millis)))
}

/// A plain name, such as an argument's.
fn ident_value(value: &syn::Expr, option: &str, example: &str) -> syn::Result<Ident> {
	match value {
		syn::Expr::Path(path) if path.qself.is_none() => {
			path.path.get_ident().cloned().ok_or_else(|| {
				syn::Error::new_spanned(
					value,
					format!("`{option}` expects a name, such as {example}"),
				)
			})
		}

		_ => {
			Err(syn::Error::new_spanned(
				value,
				format!("`{option}` expects a name, such as {example}"),
			))
		}
	}
}

impl OperationArgs {
	pub(crate) fn parse(kind: OperationKind, tokens: TokenStream) -> syn::Result<Self> {
		let metas = Punctuated::<Meta, Token![,]>::parse_terminated.parse2(tokens)?;
		let mut args = Self::default();

		for meta in metas {
			match &meta {
				Meta::NameValue(pair) if pair.path.is_ident("name") => {
					Self::once(args.name.is_some(), pair)?;
					let name = crate::string_value(&pair.value, "name")?;
					if !crate::is_route_segment(&name.value()) {
						return Err(syn::Error::new_spanned(
							&name,
							"`name` becomes the route `/api/<name>` and the generated method \
							 names, so it must be non-empty and use ASCII letters, digits, `_`, \
							 `-`, and `.`, as in `chat.messages`",
						));
					}

					args.name = Some(name);
				}

				Meta::NameValue(pair) if pair.path.is_ident("crate") => {
					Self::once(args.krate.is_some(), pair)?;
					let path = crate::string_value(&pair.value, "crate")?;
					args.krate = Some(path.parse::<Path>().map_err(|_| {
						syn::Error::new_spanned(
							&path,
							"`crate` expects the path to `leptoskit_rpc`, such as `crate = \
							 \"leptoskit::rpc\"`",
						)
					})?);
				}

				Meta::NameValue(pair) if pair.path.is_ident("retries") => {
					match kind {
						OperationKind::Query => {
							return Err(syn::Error::new_spanned(
								pair,
								"`retries` only applies to #[rpc::mutation] and #[rpc::job]: a \
								 query reads, outside any transaction, so there is nothing to \
								 retry",
							));
						}
						OperationKind::Action => {
							return Err(syn::Error::new_spanned(
								pair,
								"an action never runs again by itself, since it may have reached \
								 the outside world; enqueue a #[rpc::job] for work that retries",
							));
						}
						OperationKind::Mutation | OperationKind::Job => {}
					}

					Self::once(args.retries.is_some(), pair)?;
					let syn::Expr::Lit(syn::ExprLit {
						lit: syn::Lit::Int(count),
						..
					}) = &pair.value
					else {
						return Err(syn::Error::new_spanned(
							&pair.value,
							"`retries` expects a whole number, such as `retries = 0`",
						));
					};
					count.base10_parse::<u32>()?;
					args.retries = Some(count.clone());
				}

				Meta::NameValue(pair) if pair.path.is_ident("queue") => {
					Self::job_only(kind, pair)?;
					Self::once(args.job.queue.is_some(), pair)?;
					let queue = crate::string_value(&pair.value, "queue")?;
					if queue.value().is_empty() {
						return Err(syn::Error::new_spanned(&queue, "a queue needs a name"));
					}

					args.job.queue = Some(queue);
				}

				Meta::NameValue(pair) if pair.path.is_ident("backoff") => {
					Self::job_only(kind, pair)?;
					Self::once(args.job.backoff.is_some(), pair)?;
					let error = || {
						syn::Error::new_spanned(
							&pair.value,
							"`backoff` expects `exponential(min, max)`, such as `backoff = \
							 exponential(2s, 5m)`",
						)
					};
					let syn::Expr::Call(call) = &pair.value else {
						return Err(error());
					};
					let is_exponential = matches!(&*call.func, syn::Expr::Path(path) if path.path.is_ident("exponential"));
					if !is_exponential || call.args.len() != 2 {
						return Err(error());
					}

					args.job.backoff = Some((
						duration_value(&call.args[0], "backoff", "exponential(2s, 5m)")?,
						duration_value(&call.args[1], "backoff", "exponential(2s, 5m)")?,
					));
				}

				Meta::NameValue(pair) if pair.path.is_ident("deadline") => {
					Self::job_only(kind, pair)?;
					Self::once(args.job.deadline.is_some(), pair)?;
					args.job.deadline =
						Some(duration_value(&pair.value, "deadline", "deadline = 15m")?);
				}

				Meta::NameValue(pair) if pair.path.is_ident("timeout") => {
					Self::job_only(kind, pair)?;
					Self::once(args.job.timeout.is_some(), pair)?;
					args.job.timeout =
						Some(duration_value(&pair.value, "timeout", "timeout = 30s")?);
				}

				Meta::NameValue(pair) if pair.path.is_ident("serialize_by") => {
					Self::job_only(kind, pair)?;
					Self::once(args.job.serialize_by.is_some(), pair)?;
					args.job.serialize_by = Some(ident_value(
						&pair.value,
						"serialize_by",
						"`serialize_by = wallet`, one of the job's arguments",
					)?);
				}

				Meta::NameValue(pair) if pair.path.is_ident("visible_to") => {
					Self::job_only(kind, pair)?;
					Self::once(args.job.visible_to.is_some(), pair)?;
					args.job.visible_to = Some(ident_value(
						&pair.value,
						"visible_to",
						"`visible_to = signed_in`, `everyone`, or an argument naming an account",
					)?);
				}

				Meta::NameValue(pair) if pair.path.is_ident("on_failure") => {
					Self::job_only(kind, pair)?;
					Self::once(args.job.on_failure.is_some(), pair)?;
					let syn::Expr::Path(path) = &pair.value else {
						return Err(syn::Error::new_spanned(
							&pair.value,
							"`on_failure` names a #[rpc::mutation], such as `on_failure = \
							 payment_gave_up`",
						));
					};
					args.job.on_failure = Some(path.path.clone());
				}

				other => {
					return Err(syn::Error::new_spanned(
						other,
						format!(
							"unknown #[rpc::{}] option; expected {}",
							kind.attribute(),
							expected_options(kind)
						),
					));
				}
			}
		}

		Ok(args)
	}

	/// Rejects an option only a job takes.
	fn job_only(kind: OperationKind, pair: &syn::MetaNameValue) -> syn::Result<()> {
		if kind == OperationKind::Job {
			return Ok(());
		}

		Err(syn::Error::new_spanned(
			pair,
			format!(
				"`{}` only applies to #[rpc::job]; for #[rpc::{}], expected {}",
				pair.path.to_token_stream(),
				kind.attribute(),
				expected_options(kind)
			),
		))
	}

	/// Rejects an option given twice.
	fn once(given: bool, pair: &syn::MetaNameValue) -> syn::Result<()> {
		if given {
			return Err(syn::Error::new_spanned(
				pair,
				format!("`{}` is given twice", pair.path.to_token_stream()),
			));
		}

		Ok(())
	}
}

/// One argument of the operation, after the context.
pub(crate) struct Argument {
	/// The field and binding name, as written (raw identifiers kept).
	pub(crate) ident: Ident,
	pub(crate) ty: Type,
	/// `serde` and `schemars` attributes, moved onto the field.
	pub(crate) field_attrs: Vec<Attribute>,
	/// The argument's own doc comment.
	pub(crate) doc_attrs: Vec<Attribute>,
}

/// What the function says about the operation.
pub(crate) struct Signature {
	pub(crate) arguments: Vec<Argument>,
	/// `T` in `-> rpc::Result<T>`.
	pub(crate) output: Type,
	/// The span of the return type, for errors about the body's result.
	output_span: Span,
}

/// Checks a function is shaped like an operation of `kind`, and reads its
/// arguments and result.
pub(crate) fn signature(kind: OperationKind, function: &ItemFn) -> syn::Result<Signature> {
	let attribute = kind.attribute();
	let context = kind.context();

	if function.sig.asyncness.is_none() {
		return Err(syn::Error::new_spanned(
			function.sig.fn_token,
			format!(
				"#[rpc::{attribute}] needs an `async fn`: the operation runs on the server and \
				 answers over the network"
			),
		));
	}

	if !function.sig.generics.params.is_empty() || function.sig.generics.where_clause.is_some() {
		return Err(syn::Error::new_spanned(
			&function.sig.generics,
			format!(
				"#[rpc::{attribute}] can't be generic: its arguments travel as one JSON object of \
				 a known type"
			),
		));
	}

	if let Some(variadic) = &function.sig.variadic {
		return Err(syn::Error::new_spanned(
			variadic,
			"an operation can't be variadic",
		));
	}

	let mut inputs = function.sig.inputs.iter();
	let Some(first) = inputs.next() else {
		return Err(syn::Error::new(
			function.sig.paren_token.span.join(),
			format!(
				"#[rpc::{attribute}] takes its context first: `cx: &rpc::{context}`, which {}",
				kind.context_does()
			),
		));
	};
	check_context(kind, first)?;

	let mut arguments = Vec::new();

	for input in inputs {
		let typed = match input {
			FnArg::Receiver(receiver) => {
				return Err(syn::Error::new_spanned(
					receiver,
					"an operation can't take `self`",
				));
			}
			FnArg::Typed(typed) => typed,
		};

		let ident = match &*typed.pat {
			Pat::Ident(pattern) if pattern.by_ref.is_none() && pattern.subpat.is_none() => {
				pattern.ident.clone()
			}

			other => {
				return Err(syn::Error::new_spanned(
					other,
					"an operation needs a plain name for every argument: it is the argument's key \
					 on the wire",
				));
			}
		};

		if let Type::Reference(reference) = &*typed.ty {
			return Err(syn::Error::new_spanned(
				reference,
				"an operation's arguments arrive over the network, so they are owned: take \
				 `String` rather than `&str`, or `Vec<T>` rather than `&[T]`",
			));
		}

		if let Type::ImplTrait(implementation) = &*typed.ty {
			return Err(syn::Error::new_spanned(
				implementation,
				"an operation's arguments need a concrete type to be read from JSON",
			));
		}

		let mut field_attrs = Vec::new();
		let mut doc_attrs = Vec::new();

		for attr in &typed.attrs {
			if attr.path().is_ident("doc") {
				doc_attrs.push(attr.clone());
			} else if attr.path().is_ident("serde") || attr.path().is_ident("schemars") {
				field_attrs.push(attr.clone());
			} else {
				return Err(syn::Error::new_spanned(
					attr,
					"an operation's arguments take doc comments, `#[serde(...)]`, and \
					 `#[schemars(...)]`; this attribute has nowhere to go",
				));
			}
		}

		arguments.push(Argument {
			ident,
			ty: (*typed.ty).clone(),
			field_attrs,
			doc_attrs,
		});
	}

	let (output, output_span) = output_of(kind, &function.sig.output)?;
	Ok(Signature {
		arguments,
		output,
		output_span,
	})
}

/// Checks the first argument is `cx: &rpc::QueryCx` or `&rpc::MutationCx`,
/// whichever `kind` needs.
fn check_context(kind: OperationKind, first: &FnArg) -> syn::Result<()> {
	let attribute = kind.attribute();
	let context = kind.context();
	let FnArg::Typed(typed) = first else {
		return Err(syn::Error::new_spanned(
			first,
			"an operation can't take `self`",
		));
	};
	let wrong = |span: &dyn ToTokens, extra: &str| {
		syn::Error::new_spanned(
			span,
			format!("#[rpc::{attribute}] takes its context first, as `cx: &rpc::{context}`{extra}"),
		)
	};
	let Type::Reference(reference) = &*typed.ty else {
		return Err(wrong(&typed.ty, ", by reference"));
	};

	if reference.mutability.is_some() {
		return Err(wrong(&typed.ty, ", by shared reference"));
	}

	let Type::Path(path) = &*reference.elem else {
		return Err(wrong(&typed.ty, ""));
	};
	let last = path
		.path
		.segments
		.last()
		.map(|segment| segment.ident.to_string());

	if last.as_deref() == Some(context) {
		return Ok(());
	}

	if let Some(other) = OperationKind::ALL
		.into_iter()
		.find(|other| last.as_deref() == Some(other.context()))
	{
		return Err(syn::Error::new_spanned(
			&reference.elem,
			format!(
				"#[rpc::{attribute}]'s context is `&rpc::{context}`, which {}; `&rpc::{}` belongs \
				 to #[rpc::{}]",
				kind.context_does(),
				other.context(),
				other.attribute()
			),
		));
	}
	Err(wrong(&reference.elem, ""))
}

/// `T` in `-> rpc::Result<T>`, or `-> Result<T, rpc::Error>`.
fn output_of(kind: OperationKind, output: &ReturnType) -> syn::Result<(Type, Span)> {
	let attribute = kind.attribute();
	let error = |span: Span| {
		syn::Error::new(
			span,
			format!(
				"#[rpc::{attribute}] returns `rpc::Result<T>`: the value the caller receives, or \
				 the `rpc::Error` it fails with"
			),
		)
	};
	let ReturnType::Type(_, ty) = output else {
		return Err(error(output.span()));
	};
	let Type::Path(path) = &**ty else {
		return Err(error(ty.span()));
	};
	let Some(last) = path.path.segments.last() else {
		return Err(error(ty.span()));
	};

	if last.ident != "Result" {
		return Err(error(ty.span()));
	}

	let PathArguments::AngleBracketed(arguments) = &last.arguments else {
		return Err(error(ty.span()));
	};

	match arguments.args.first() {
		Some(GenericArgument::Type(value)) => Ok((value.clone(), ty.span())),
		_ => Err(error(ty.span())),
	}
}

/// The struct an operation's arguments travel in: the function name in
/// `PascalCase`.
pub(crate) fn struct_ident(function: &ItemFn) -> Ident {
	let name = function.sig.ident.unraw().to_string();
	let struct_name = Converter::new()
		.from_case(Case::Snake)
		.to_case(Case::UpperCamel)
		.convert(&name);
	Ident::new(&struct_name, function.sig.ident.span())
}

/// `const OPTIONS` for a job's `Job` impl, from its options, after checking
/// the arguments they name exist.
fn job_options(
	args: &OperationArgs,
	signature: &Signature,
	krate: &Path,
) -> syn::Result<TokenStream> {
	let job = &args.job;
	let argument = |ident: &Ident, option: &str| -> syn::Result<LitStr> {
		let found = signature
			.arguments
			.iter()
			.any(|argument| argument.ident.unraw() == ident.unraw());

		if !found {
			return Err(syn::Error::new_spanned(
				ident,
				format!("`{option}` names one of the job's arguments, and `{ident}` isn't one"),
			));
		}

		Ok(LitStr::new(&ident.unraw().to_string(), ident.span()))
	};
	let queue = job
		.queue
		.clone()
		.unwrap_or_else(|| LitStr::new("default", Span::call_site()));
	let retries = if let Some(count) = &args.retries {
		quote!(#count)
	} else {
		quote!(#krate::JobOptions::DEFAULT.retries)
	};
	let backoff = if let Some((min, max)) = &job.backoff {
		quote!(#krate::Backoff::exponential(#min, #max))
	} else {
		quote!(#krate::Backoff::DEFAULT)
	};

	let some = |value: &Option<TokenStream>| {
		if let Some(value) = value {
			quote!(::core::option::Option::Some(#value))
		} else {
			quote!(::core::option::Option::None)
		}
	};
	let deadline = some(&job.deadline);
	let timeout = some(&job.timeout);
	let serialize_by = if let Some(ident) = &job.serialize_by {
		let name = argument(ident, "serialize_by")?;
		quote!(::core::option::Option::Some(#name))
	} else {
		quote!(::core::option::Option::None)
	};

	let visible_to = match &job.visible_to {
		None => quote!(#krate::Visibility::Owner),

		Some(ident) if ident == "owner" => quote!(#krate::Visibility::Owner),

		Some(ident) if ident == "signed_in" => quote!(#krate::Visibility::SignedIn),

		Some(ident) if ident == "everyone" => quote!(#krate::Visibility::Everyone),
		Some(ident) => {
			let name = argument(ident, "visible_to")?;
			quote!(#krate::Visibility::Argument(#name))
		}
	};

	let on_failure = job.on_failure.is_some();
	Ok(quote! {
		const OPTIONS: #krate::JobOptions = #krate::JobOptions {
			queue: #queue,
			retries: #retries,
			backoff: #backoff,
			deadline: #deadline,
			timeout: #timeout,
			serialize_by: #serialize_by,
			visible_to: #visible_to,
			on_failure: #on_failure,
		};
	})
}

/// The struct a function path's operation travels in: `payment_gave_up` is
/// `PaymentGaveUp`, `crate::pay::gave_up` is `crate::pay::GaveUp`.
fn struct_path(path: &Path) -> Path {
	let mut path = path.clone();

	if let Some(last) = path.segments.last_mut() {
		let name = Converter::new()
			.from_case(Case::Snake)
			.to_case(Case::UpperCamel)
			.convert(last.ident.unraw().to_string());
		last.ident = Ident::new(&name, last.ident.span());
	}

	path
}

/// Whether `ty` is `rpc::Failed<…>`: a mutation taking only one is a job's
/// `on_failure`, and converts from it.
fn is_failed(ty: &Type) -> bool {
	matches!(ty, Type::Path(path) if path.qself.is_none()
		&& path.path.segments.last().is_some_and(|segment| segment.ident == "Failed"))
}

/// Expands `#[rpc::query]` or `#[rpc::mutation]`. `server` decides whether the
/// body and its registration are kept: the `server` feature in the real
/// macro, either way in tests.
#[expect(
	clippy::too_many_lines,
	reason = "the struct, its checks, and each kind's impls are one expansion"
)]
pub(crate) fn expand(
	kind: OperationKind,
	args: TokenStream,
	item: TokenStream,
	server: bool,
) -> syn::Result<TokenStream> {
	let args = OperationArgs::parse(kind, args)?;
	let function: ItemFn = syn::parse2(item)?;
	let attribute = kind.attribute();

	if let Some(api) = function
		.attrs
		.iter()
		.find(|attr| crate::is_api_attribute(attr))
	{
		return Err(syn::Error::new_spanned(
			api,
			format!("put #[api] above #[rpc::{attribute}], so it sees the operation it publishes"),
		));
	}

	if let Some(server_attr) = function
		.attrs
		.iter()
		.find(|attr| crate::is_server_attribute(attr))
	{
		return Err(syn::Error::new_spanned(
			server_attr,
			format!(
				"a function is either a leptos #[server] function or an #[rpc::{attribute}], not \
				 both"
			),
		));
	}
	let fn_name = function.sig.ident.to_string();

	if fn_name.starts_with("r#") {
		return Err(syn::Error::new_spanned(
			&function.sig.ident,
			"an operation can't be named with a raw identifier; rename the function, and give it \
			 its wire name with `name = \"...\"`",
		));
	}

	let signature = signature(kind, &function)?;
	let krate = args
		.krate
		.clone()
		.unwrap_or_else(|| syn::parse_quote!(::leptoskit_rpc));
	let name = args
		.name
		.clone()
		.unwrap_or_else(|| LitStr::new(&fn_name, function.sig.ident.span()));
	let struct_ident = struct_ident(&function);
	let vis = &function.vis;
	let field_vis = vis;
	let docs: Vec<&Attribute> = function
		.attrs
		.iter()
		.filter(|attr| attr.path().is_ident("doc"))
		.collect();
	let deprecated: Vec<&Attribute> = function
		.attrs
		.iter()
		.filter(|attr| attr.path().is_ident("deprecated"))
		.collect();
	let serde_path = LitStr::new(
		&quote!(#krate::__private::serde).to_string(),
		Span::call_site(),
	);
	let schemars_path = LitStr::new(
		&quote!(#krate::__private::schemars).to_string(),
		Span::call_site(),
	);
	let fields = signature.arguments.iter().map(|argument| {
		let ident = &argument.ident;
		let ty = &argument.ty;
		let doc_attrs = &argument.doc_attrs;
		let field_attrs = &argument.field_attrs;
		quote! {
			#(#doc_attrs)*
			#(#field_attrs)*
			#field_vis #ident: #ty,
		}
	});
	// Each argument type must cross the wire; this names the type, so the
	// error points at the argument rather than at the derive.
	let argument_checks = signature.arguments.iter().map(|argument| {
		let ty = &argument.ty;
		quote_spanned! {ty.span()=>
			#krate::__private::assert_argument::<#ty>();
		}
	});
	let output = &signature.output;
	let output_check = quote_spanned! {signature.output_span=>
		#krate::__private::assert_output::<#output>();
	};
	let variant = kind.variant();
	let options = if kind == OperationKind::Job {
		job_options(&args, &signature, &krate)?
	} else {
		TokenStream::new()
	};

	let from_failed = match signature.arguments.as_slice() {
		[argument] if kind == OperationKind::Mutation && is_failed(&argument.ty) => {
			let ident = &argument.ident;
			let ty = &argument.ty;
			quote! {
				impl ::core::convert::From<#ty> for #struct_ident {
					fn from(#ident: #ty) -> Self {
						Self { #ident }
					}
				}
			}
		}

		_ => TokenStream::new(),
	};

	let mut out = quote! {
		#(#docs)*
		#(#deprecated)*
		#[derive(
			::core::clone::Clone,
			#krate::__private::serde::Serialize,
			#krate::__private::serde::Deserialize,
			#krate::__private::schemars::JsonSchema,
		)]
		#[serde(crate = #serde_path)]
		#[schemars(crate = #schemars_path)]
		#vis struct #struct_ident {
			#(#fields)*
		}

		const _: () = {
			#[allow(dead_code)]
			fn __check() {
				#(#argument_checks)*
				#output_check
			}
		};

		#from_failed
	};

	if server {
		out.extend(server_items(
			kind,
			&args,
			function,
			&signature,
			&krate,
			&struct_ident,
			&name,
			&options,
		));
	} else {
		out.extend(quote! {
			#[allow(deprecated)]
			impl #krate::Operation for #struct_ident {
				const NAME: &'static str = #name;
				const KIND: #krate::Kind = #krate::Kind::#variant;
				type Output = #output;
			}
		});

		if kind == OperationKind::Job {
			out.extend(quote! {
				#[allow(deprecated)]
				impl #krate::Job for #struct_ident {
					const NAME: &'static str = #name;
					#options
					type Output = #output;
				}
			});
		}
	}

	Ok(out)
}

/// What only a server build keeps: the function, `Operation::run`, and the
/// operation's entry in the registry.
#[expect(
	clippy::too_many_arguments,
	reason = "what `expand` already worked out, handed on rather than worked out again"
)]
fn server_items(
	kind: OperationKind,
	args: &OperationArgs,
	mut function: ItemFn,
	signature: &Signature,
	krate: &Path,
	struct_ident: &Ident,
	name: &LitStr,
	options: &TokenStream,
) -> TokenStream {
	let variant = kind.variant();
	let output = &signature.output;
	let context = Ident::new(kind.context(), Span::call_site());
	let bindings: Vec<&Ident> = signature
		.arguments
		.iter()
		.map(|argument| &argument.ident)
		.collect();
	let fn_ident = &function.sig.ident;
	// A job's retries are its attempts, not a transaction's.
	let retries = match &args.retries {
		Some(count) if kind == OperationKind::Mutation => quote!(const RETRIES: u32 = #count;),
		_ => quote!(),
	};

	let handler = quote!(#krate::__private::handle::<#struct_ident>);
	let call = quote_spanned! {signature.output_span=>
		#fn_ident(cx, #(#bindings),*)
	};
	// Every operation is async, whether or not its body awaits.
	function
		.attrs
		.push(syn::parse_quote!(#[allow(clippy::unused_async)]));
	// Argument attributes moved onto the struct have no place on the
	// function.
	for input in &mut function.sig.inputs {
		if let FnArg::Typed(typed) = input {
			typed.attrs.retain(|attr| attr.path().is_ident("doc"));
		}
	}

	let dos = quote! { #krate::__private::inventory::submit! {
		#krate::server::Registration {
			name: <#struct_ident as #krate::Operation>::NAME,
			kind: <#struct_ident as #krate::Operation>::KIND,
			module_path: ::core::module_path!(),
			handler: #handler,
		}
	} };
	let job_impl = if kind == OperationKind::Job {
		quote! {
			#[allow(deprecated)]
			impl #krate::Job for #struct_ident {
				const NAME: &'static str = #name;
				#options
				type Output = #output;

				fn run(
					self,
					cx: &#krate::JobCx,
				) -> impl ::core::future::Future<Output = #krate::Result<Self::Output>>
				+ ::core::marker::Send {
					let Self { #(#bindings),* } = self;
					#call
				}
			}
		}
	} else {
		quote!()
	};
	let registry = if kind == OperationKind::Job {
		let on_failure = if let Some(path) = &args.job.on_failure {
			let handler = struct_path(path);
			quote_spanned! {path.span()=>
				::core::option::Option::Some(
					#krate::__private::fail_job::<#struct_ident, #handler>
				)
			}
		} else {
			quote!(::core::option::Option::None)
		};

		quote! {
			#krate::__private::inventory::submit! {
				#krate::server::jobs::JobRegistration {
					name: <#struct_ident as #krate::Job>::NAME,
					options: <#struct_ident as #krate::Job>::OPTIONS,
					handler: #krate::__private::run_job::<#struct_ident>,
					on_failure: #on_failure,
				}
			}
		}
	} else {
		dos
	};

	quote! {
			#function

			#[allow(deprecated)]
			impl #krate::Operation for #struct_ident {
				const NAME: &'static str = #name;
				const KIND: #krate::Kind = #krate::Kind::#variant;
				type Output = #output;
				type Context = #krate::#context;
				#retries

				fn run(
					self,
					cx: &#krate::#context,
				) -> impl ::core::future::Future<Output = #krate::Result<Self::Output>>
				+ ::core::marker::Send {
					let Self { #(#bindings),* } = self;
					#call
				}
			}

			#job_impl

			#krate::__require_db!();

			#[allow(deprecated)]
			const _: () = {
				#registry
			};
	}
}
