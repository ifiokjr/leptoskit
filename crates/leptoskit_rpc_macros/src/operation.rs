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
}

impl OperationKind {
	/// The attribute's name, as written: `query` or `mutation`.
	pub(crate) fn attribute(self) -> &'static str {
		match self {
			Self::Query => "query",
			Self::Mutation => "mutation",
		}
	}

	/// The context type its first argument takes.
	fn context(self) -> &'static str {
		match self {
			Self::Query => "QueryCx",
			Self::Mutation => "MutationCx",
		}
	}

	/// The other kind, whose context is a mistake here.
	fn other(self) -> Self {
		match self {
			Self::Query => Self::Mutation,
			Self::Mutation => Self::Query,
		}
	}

	/// The `Kind` variant, as `leptoskit_rpc` names it.
	fn variant(self) -> Ident {
		match self {
			Self::Query => format_ident!("Query"),
			Self::Mutation => format_ident!("Mutation"),
		}
	}

	/// The `ApiKind` variant in the spec.
	pub(crate) fn api_variant(self) -> Ident {
		self.variant()
	}
}

/// The options `#[rpc::query(...)]` and `#[rpc::mutation(...)]` accept.
#[derive(Default)]
pub(crate) struct OperationArgs {
	pub(crate) name: Option<LitStr>,
	pub(crate) krate: Option<Path>,
	retries: Option<LitInt>,
}

impl OperationArgs {
	pub(crate) fn parse(kind: OperationKind, tokens: TokenStream) -> syn::Result<Self> {
		let metas = Punctuated::<Meta, Token![,]>::parse_terminated.parse2(tokens)?;
		let mut args = Self::default();
		for meta in metas {
			match &meta {
				Meta::NameValue(pair) if pair.path.is_ident("name") => {
					if args.name.is_some() {
						return Err(syn::Error::new_spanned(pair, "`name` is given twice"));
					}
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
					if args.krate.is_some() {
						return Err(syn::Error::new_spanned(pair, "`crate` is given twice"));
					}
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
					if kind == OperationKind::Query {
						return Err(syn::Error::new_spanned(
							pair,
							"`retries` only applies to #[rpc::mutation]: a query reads, outside \
							 any transaction, so there is nothing to retry",
						));
					}
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
				other => {
					let expected = match kind {
						OperationKind::Query => "`name = \"...\"` or `crate = \"...\"`",
						OperationKind::Mutation => {
							"`name = \"...\"`, `retries = N`, or `crate = \"...\"`"
						}
					};
					return Err(syn::Error::new_spanned(
						other,
						format!(
							"unknown #[rpc::{}] option; expected {expected}",
							kind.attribute()
						),
					));
				}
			}
		}
		Ok(args)
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
				match kind {
					OperationKind::Query => "reads the database as the caller",
					OperationKind::Mutation => "writes in one transaction as the caller",
				}
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
	if last.as_deref() == Some(kind.other().context()) {
		let message = match kind {
			OperationKind::Query => {
				"a query's context is `&rpc::QueryCx`, which only reads; `&rpc::MutationCx` \
				 writes, and belongs to #[rpc::mutation]"
			}
			OperationKind::Mutation => {
				"a mutation's context is `&rpc::MutationCx`, which writes in one transaction; \
				 `&rpc::QueryCx` only reads, and belongs to #[rpc::query]"
			}
		};
		return Err(syn::Error::new_spanned(&reference.elem, message));
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

/// Expands `#[rpc::query]` or `#[rpc::mutation]`. `server` decides whether the
/// body and its registration are kept: the `server` feature in the real
/// macro, either way in tests.
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
	}

	Ok(out)
}

/// What only a server build keeps: the function, `Operation::run`, and the
/// operation's entry in the registry.
fn server_items(
	kind: OperationKind,
	args: &OperationArgs,
	mut function: ItemFn,
	signature: &Signature,
	krate: &Path,
	struct_ident: &Ident,
	name: &LitStr,
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
	let retries = match &args.retries {
		Some(count) => quote!(const RETRIES: u32 = #count;),
		None => quote!(),
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

			#krate::__require_db!();

			#[allow(deprecated)]
			const _: () = {
				#krate::__private::inventory::submit! {
					#krate::server::Registration {
						name: <#struct_ident as #krate::Operation>::NAME,
						kind: <#struct_ident as #krate::Operation>::KIND,
						module_path: ::core::module_path!(),
						handler: #handler,
					}
				}
			};
	}
}
