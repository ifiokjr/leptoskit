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
//! - pins the route to a stable `endpoint` (the function name) when the
//!   `#[server]` attribute does not set one, because the default endpoint
//!   embeds a hash of the absolute manifest path and changes between checkouts,
//!   which no generated client could follow;
//! - rejects custom protocols (websockets, streams), which have no HTTP
//!   request/response shape a client could speak;
//! - with the `ssr` feature, registers a description of the function — the
//!   route, encodings, doc comment (including an `# Arguments` section), and a
//!   JSON Schema thunk for every argument and the return type — so
//!   `leptoskit_rpc::spec::collect` can export it and `leptoskit rpc generate`
//!   can write typed clients for any language.

use convert_case::Case;
use convert_case::Converter;
use proc_macro2::Span;
use proc_macro2::TokenStream;
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
/// - `name = "..."` — the operation name, when the function name is taken;
/// - `tag = "..."` — a grouping tag (repeatable);
/// - `crate = "..."` — the path to `leptoskit_rpc` when it is not a direct
///   dependency, such as `crate = "leptoskit::rpc"`.
///
/// Every argument type and the `Ok` type of the return value must implement
/// `schemars::JsonSchema`. Argument attributes `#[server(default)]` and
/// `#[server(rename = "...")]` are honored, and the `# Arguments` section of
/// the doc comment documents individual arguments in every generated client.
#[proc_macro_attribute]
pub fn api(
	args: proc_macro::TokenStream,
	item: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
	match expand(args.into(), item.into()) {
		Ok(tokens) => tokens.into(),
		Err(error) => error.to_compile_error().into(),
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
					args.name = Some(string_value(&pair.value, "name")?);
				}
				Meta::NameValue(pair) if pair.path.is_ident("tag") => {
					args.tags.push(string_value(&pair.value, "tag")?);
				}
				Meta::NameValue(pair) if pair.path.is_ident("crate") => {
					let path = string_value(&pair.value, "crate")?;
					args.krate = Some(path.parse::<Path>()?);
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

/// The parts of the `#[server(...)]` arguments `#[api]` cares about.
#[derive(Default)]
struct ServerArgs {
	struct_name: Option<Ident>,
	has_endpoint: bool,
	positional: bool,
	protocol: Option<Type>,
}

impl ServerArgs {
	/// A lenient parse of the server macro's arguments: just enough to learn
	/// the struct name, whether an endpoint or protocol is set, and whether the
	/// legacy positional form is in use.
	fn parse(tokens: TokenStream) -> syn::Result<Self> {
		let parser = |input: ParseStream| {
			let mut args = Self::default();
			let mut position = 0_usize;
			while !input.is_empty() {
				position += 1;
				if input.peek(Ident) && input.peek2(Token![=]) {
					let key: Ident = input.parse()?;
					input.parse::<Token![=]>()?;
					match key.to_string().as_str() {
						"name" => args.struct_name = Some(input.parse()?),
						"endpoint" => {
							input.parse::<LitStr>()?;
							args.has_endpoint = true;
						}
						"protocol" => args.protocol = Some(input.parse::<Type>()?),
						"input" | "output" | "server" | "client" | "req" | "res" => {
							input.parse::<Type>()?;
						}
						"custom" => {
							input.parse::<Path>()?;
						}
						"prefix" | "encoding" => {
							input.parse::<LitStr>()?;
						}
						"impl_from" | "impl_deref" => {
							input.parse::<syn::LitBool>()?;
						}
						"input_derive" => {
							input.parse::<syn::ExprTuple>()?;
						}
						_ => {
							input.parse::<Expr>()?;
						}
					}
				} else if input.peek(Ident) {
					let ident: Ident = input.parse()?;
					args.positional = true;
					if position == 1 {
						args.struct_name = Some(ident);
					}
				} else if input.peek(LitStr) {
					input.parse::<LitStr>()?;
					args.positional = true;
					if position == 4 {
						args.has_endpoint = true;
					}
				} else {
					return Err(input.error("unexpected token in #[server] arguments"));
				}
				if !input.is_empty() {
					input.parse::<Token![,]>()?;
				}
			}
			Ok(args)
		};
		parser.parse2(tokens)
	}
}

/// One argument of the server function, as it travels on the wire.
struct Param {
	wire_name: String,
	ty: Type,
	required: bool,
	docs: String,
}

fn expand(args: TokenStream, item: TokenStream) -> syn::Result<TokenStream> {
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
	let server_args = ServerArgs::parse(server_tokens.clone())?;
	if let Some(protocol) = &server_args.protocol {
		return Err(syn::Error::new_spanned(
			protocol,
			"#[api] only publishes HTTP server functions; websocket and streaming protocols have \
			 no request/response shape a client could speak. Remove #[api] from this function.",
		));
	}

	let fn_ident = function.sig.ident.clone();
	let fn_name = fn_ident.to_string();
	let krate = args
		.krate
		.clone()
		.unwrap_or_else(|| syn::parse_quote!(::leptoskit_rpc));

	// Named-style attributes get a stable endpoint; the legacy positional form
	// has its own slot for one and is left alone.
	if !server_args.positional && !server_args.has_endpoint {
		let endpoint = LitStr::new(&fn_name, fn_ident.span());
		let path = server_attr.path().clone();
		let tokens = if server_tokens.is_empty() {
			quote!(endpoint = #endpoint)
		} else {
			quote!(#server_tokens, endpoint = #endpoint)
		};
		function.attrs[server_index] = syn::parse_quote!(#[#path(#tokens)]);
	}

	let struct_ident = server_args.struct_name.clone().unwrap_or_else(|| {
		let name = Converter::new()
			.from_case(Case::Snake)
			.to_case(Case::UpperCamel)
			.convert(&fn_name);
		Ident::new(&name, fn_ident.span())
	});
	let raw_docs = docs_of(&function.attrs);
	let params = params_of(&function, &raw_docs)?;
	// The `# Arguments` section lands on the arguments themselves, so it is
	// stripped from the function docs to avoid saying everything twice.
	let docs = strip_arguments_section(&raw_docs);
	let deprecated = function
		.attrs
		.iter()
		.any(|attr| attr.path().is_ident("deprecated"));

	let registration = if cfg!(feature = "ssr") {
		let name = args
			.name
			.clone()
			.unwrap_or_else(|| LitStr::new(&fn_name, fn_ident.span()));
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
					path: <#struct_ident as #krate::__private::ServerFn>::PATH,
					method: #krate::spec::method_of::<#struct_ident>(),
					input_content_type: #krate::spec::input_content_type_of::<#struct_ident>(),
					output_content_type: #krate::spec::output_content_type_of::<#struct_ident>(),
					error_content_type: #krate::spec::error_content_type_of::<#struct_ident>(),
					params: &[#(#param_descriptors),*],
					output_schema: |generator| {
						generator.subschema_for::<
							<#struct_ident as #krate::__private::ServerFn>::Output,
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

/// The arguments with their wire names, types, requirements, and docs.
fn params_of(function: &ItemFn, docs: &str) -> syn::Result<Vec<Param>> {
	let arguments = argument_docs(docs);
	function
		.sig
		.inputs
		.iter()
		.map(|input| {
			match input {
				FnArg::Receiver(receiver) => {
					Err(syn::Error::new_spanned(
						receiver,
						"server functions cannot take `self`",
					))
				}
				FnArg::Typed(typed) => {
					let Pat::Ident(pattern) = &*typed.pat else {
						return Err(syn::Error::new_spanned(
							&typed.pat,
							"#[api] needs a plain argument name here; destructuring patterns are \
							 not supported",
						));
					};
					let ident = pattern.ident.clone();
					let mut wire_name = ident.to_string();
					let mut required = true;
					for attr in &typed.attrs {
						if !attr.path().is_ident("server") {
							continue;
						}
						let Meta::List(list) = &attr.meta else {
							continue;
						};
						let metas = Punctuated::<Meta, Token![,]>::parse_terminated
							.parse2(list.tokens.clone())?;
						for meta in metas {
							match &meta {
								Meta::Path(path) if path.is_ident("default") => required = false,
								Meta::NameValue(pair) if pair.path.is_ident("default") => {
									required = false;
								}
								Meta::NameValue(pair) if pair.path.is_ident("rename") => {
									wire_name = string_value(&pair.value, "rename")?.value();
								}
								_ => {}
							}
						}
					}
					Ok(Param {
						docs: arguments
							.get(&ident.to_string())
							.cloned()
							.unwrap_or_default(),
						wire_name,
						ty: (*typed.ty).clone(),
						required,
					})
				}
			}
		})
		.collect()
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
/// `` * `name` - description `` lines, keyed by argument name.
fn argument_docs(docs: &str) -> std::collections::HashMap<String, String> {
	let mut arguments = std::collections::HashMap::new();
	let mut in_section = false;
	for line in docs.lines() {
		let line = line.trim();
		if let Some(heading) = line.strip_prefix("# ") {
			in_section = heading.trim() == "Arguments";
			continue;
		}
		if !in_section {
			continue;
		}
		let entry = line
			.strip_prefix("* ")
			.or_else(|| line.strip_prefix("- "))
			.unwrap_or(line);
		let Some((name, description)) = entry.split_once(" - ") else {
			continue;
		};
		let name = name.trim().trim_matches('`');
		if name.is_empty() || description.trim().is_empty() {
			continue;
		}
		arguments.insert(name.to_owned(), description.trim().to_owned());
	}
	arguments
}

/// The doc comment on the function, `///` markers stripped and lines joined.
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
