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
use syn::Path;
use syn::Token;
use syn::Type;
use syn::ext::IdentExt;
use syn::parse::ParseStream;
use syn::parse::Parser;
use syn::punctuated::Punctuated;

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
/// Options:
///
/// - `internal` — the function stays callable, it is just not publicized:
///   generated clients leave it out unless `--include-internal` is passed;
/// - `name = "..."` — the operation name, when the function name is taken; it
///   also becomes the route, `/api/<name>`, unless `#[server]` sets an
///   `endpoint`;
/// - `tag = "..."` — a grouping tag (repeatable);
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
	krate: Option<Path>,
}

impl ApiArgs {
	fn parse(tokens: TokenStream) -> syn::Result<Self> {
		let metas = Punctuated::<Meta, Token![,]>::parse_terminated.parse2(tokens)?;
		let mut args = Self::default();
		for meta in metas {
			match &meta {
				Meta::Path(path) if path.is_ident("internal") => args.internal = true,
				Meta::NameValue(pair) if pair.path.is_ident("name") => {
					if args.name.is_some() {
						return Err(syn::Error::new_spanned(pair, "`name` is given twice"));
					}
					let name = string_value(&pair.value, "name")?;
					if !is_route_segment(&name.value()) {
						return Err(syn::Error::new_spanned(
							&name,
							"`name` becomes the route `/api/<name>` and the generated method \
							 names, so it must be non-empty and use only ASCII letters, digits, \
							 `_`, and `-`",
						));
					}
					args.name = Some(name);
				}
				Meta::NameValue(pair) if pair.path.is_ident("tag") => {
					args.tags.push(string_value(&pair.value, "tag")?);
				}
				Meta::NameValue(pair) if pair.path.is_ident("crate") => {
					if args.krate.is_some() {
						return Err(syn::Error::new_spanned(pair, "`crate` is given twice"));
					}
					let path = string_value(&pair.value, "crate")?;
					args.krate = Some(path.parse::<Path>().map_err(|_| {
						syn::Error::new_spanned(
							&path,
							"`crate` expects the path to `leptoskit_rpc`, such as `crate = \
							 \"leptoskit::rpc\"`",
						)
					})?);
				}
				other => {
					return Err(syn::Error::new_spanned(
						other,
						"unknown #[api] option; expected `internal`, `name = \"...\"`, `tag = \
						 \"...\"`, or `crate = \"...\"`",
					));
				}
			}
		}
		Ok(args)
	}
}

/// Whether `name` can stand alone as one path segment of a route.
fn is_route_segment(name: &str) -> bool {
	!name.is_empty()
		&& name
			.chars()
			.all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
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
					let value = match key.to_string().as_str() {
						"name" => {
							let name: Ident = input.parse()?;
							args.struct_name = Some(name.clone());
							name.into_token_stream()
						}
						"endpoint" => {
							args.has_endpoint = true;
							input.parse::<LitStr>()?.into_token_stream()
						}
						"input" | "output" | "protocol" => {
							let codec: Type = input.parse()?;
							let slot = match key.to_string().as_str() {
								"input" => &mut args.input,
								"output" => &mut args.output,
								_ => &mut args.protocol,
							};
							*slot = Some(codec.clone());
							codec.into_token_stream()
						}
						"server" | "client" => input.parse::<Type>()?.into_token_stream(),
						"custom" => {
							let wrapper: Path = input.parse()?;
							args.custom = Some(wrapper.clone());
							wrapper.into_token_stream()
						}
						"prefix" | "encoding" => input.parse::<LitStr>()?.into_token_stream(),
						"impl_from" | "impl_deref" => {
							input.parse::<syn::LitBool>()?.into_token_stream()
						}
						"input_derive" => input.parse::<syn::ExprTuple>()?.into_token_stream(),
						_ => input.parse::<Expr>()?.into_token_stream(),
					};
					ServerArg::Named { key, value }
				} else if input.peek(Ident) {
					let ident: Ident = input.parse()?;
					args.positional = true;
					if position == 1 {
						args.struct_name = Some(ident.clone());
					}
					ServerArg::Positional(ident.into_token_stream())
				} else if input.peek(LitStr) {
					let text: LitStr = input.parse()?;
					args.positional = true;
					if position == 4 {
						args.has_endpoint = true;
					}
					ServerArg::Positional(text.into_token_stream())
				} else {
					return Err(input.error("unexpected token in #[server] arguments"));
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
	let mut function: ItemFn = syn::parse2(item)?;

	let server_index = function
		.attrs
		.iter()
		.position(is_server_attribute)
		.ok_or_else(|| {
			syn::Error::new(
				function.sig.ident.span(),
				"#[api] must be placed above #[server]: it publishes a leptos server function",
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
		let ident = match &*typed.pat {
			Pat::Ident(pattern) => pattern.ident.unraw(),
			Pat::Wild(wild) => {
				return Err(syn::Error::new_spanned(
					wild,
					"#[api] needs a name for every argument: it is the argument's key on the wire",
				));
			}
			other => {
				return Err(syn::Error::new_spanned(
					other,
					"#[api] needs a plain argument name here; destructuring patterns are not \
					 supported",
				));
			}
		};
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
			let metas =
				Punctuated::<Meta, Token![,]>::parse_terminated.parse2(list.tokens.clone())?;
			for meta in metas {
				match &meta {
					Meta::Path(path) if path.is_ident("default") => required = false,
					Meta::NameValue(pair) if pair.path.is_ident("default") => {
						required = false;
					}
					Meta::NameValue(pair) if pair.path.is_ident("rename") => {
						wire_name = string_value(&pair.value, "rename")?.value();
					}
					Meta::Path(path) if path.is_ident("skip") => skipped = true,
					Meta::Path(path) if path.is_ident("flatten") => {
						return Err(syn::Error::new_spanned(
							path,
							"#[api] cannot describe `#[server(flatten)]` arguments: their fields \
							 travel at the top level of the request, which generated clients \
							 would nest under the argument's name. Pass the fields as separate \
							 arguments, or the struct as one named argument.",
						));
					}
					// The server macro reports anything else.
					_ => {}
				}
			}
		}
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
