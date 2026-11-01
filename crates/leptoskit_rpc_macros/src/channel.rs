//! `#[rpc::channel]`: a join argument struct with a message type, a
//! presence type, and the server-side check that decides who may join.

use convert_case::Case;
use convert_case::Converter;
use proc_macro2::TokenStream;
use quote::ToTokens;
use quote::quote;
use syn::ItemStruct;
use syn::LitStr;
use syn::Meta;
use syn::Path;
use syn::Token;
use syn::Type;
use syn::parse::Parser;
use syn::punctuated::Punctuated;

/// What the attribute was given.
pub(crate) struct ChannelArgs {
	pub(crate) message: Type,
	pub(crate) presence: Option<Type>,
	pub(crate) authorize: Path,
	pub(crate) name: Option<LitStr>,
	pub(crate) krate: Option<Path>,
}

/// Expands `#[rpc::channel(...)]`: adds the derives, implements
/// `rpc::Channel`, and, in a server build with a driver, registers who may
/// join.
pub(crate) fn expand(
	args: TokenStream,
	item: TokenStream,
	server: bool,
) -> syn::Result<TokenStream> {
	let args = parse(args)?;
	let mut item: ItemStruct = syn::parse2(item)?;

	if !item.generics.params.is_empty() {
		return Err(syn::Error::new_spanned(
			&item.generics,
			"a channel's join arguments can't be generic: the server has to name them",
		));
	}

	let ident = &item.ident;
	let krate: Path = args
		.krate
		.clone()
		.unwrap_or_else(|| syn::parse_quote!(::leptoskit_rpc));
	let name = args.name.clone().unwrap_or_else(|| {
		let name = Converter::new()
			.from_case(Case::Pascal)
			.to_case(Case::Snake)
			.convert(ident.to_string());
		LitStr::new(&name, ident.span())
	});
	let message = &args.message;
	let presence = args
		.presence
		.clone()
		.unwrap_or_else(|| syn::parse_quote!(()));
	let authorize = &args.authorize;

	item.attrs.retain(|attr| !attr.path().is_ident("rpc"));
	let attrs = &item.attrs;
	let vis = &item.vis;
	let fields = &item.fields;

	let registration = server.then(|| {
		quote! {
			#[allow(deprecated)]
			const _: () = {
				#krate::__private::inventory::submit! {
					#krate::server::ChannelRegistration {
						name: #name,
						authorize: |args: #krate::__private::serde_json::Value,
									  cx: #krate::server::QueryCx|
						 -> #krate::__private::BoxFuture<'static, #krate::Result<()>> {
							#krate::__private::Box::pin(async move {
								let channel: #ident = #krate::__private::serde_json::from_value(args)
									.map_err(|error| #krate::Error::invalid(format!(
										"the arguments of `{}` don't fit: {error}",
										#name
									)))?;
								#authorize(&cx, &channel).await
							})
						},
					}
				}
			};
		}
	});

	Ok(quote! {
		#[derive(
			::core::clone::Clone,
			#krate::__private::serde::Serialize,
			#krate::__private::serde::Deserialize,
			#krate::__private::schemars::JsonSchema,
		)]
		#(#attrs)*
		#vis struct #ident #fields

		impl #krate::Channel for #ident {
			const NAME: &'static str = #name;
			type Message = #message;
			type Presence = #presence;
		}

		const _: () = {
			#[allow(dead_code)]
			fn __check() {
				#krate::__private::assert_channel_argument::<#ident>();
				#krate::__private::assert_channel_message::<#message>();
				#krate::__private::assert_channel_presence::<#presence>();
			}
		};

		#registration
	})
}

/// The `crate = "..."` a `#[rpc::channel(...)]` was given, for `#[api]`
/// above it.
pub(crate) fn krate(args: TokenStream) -> syn::Result<Option<Path>> {
	Ok(parse(args)?.krate)
}

fn parse(args: TokenStream) -> syn::Result<ChannelArgs> {
	let parser = Punctuated::<Meta, Token![,]>::parse_terminated;
	let pairs = parser.parse2(args)?;
	let mut options = ChannelOptions::default();

	for pair in pairs {
		channel_option(&mut options, pair)?;
	}

	let Some(message) = options.message else {
		return Err(syn::Error::new(
			proc_macro2::Span::call_site(),
			"a channel needs its message type: `#[rpc::channel(message = Typing, authorize = \
			 may_join)]`",
		));
	};
	let Some(authorize) = options.authorize else {
		return Err(syn::Error::new(
			proc_macro2::Span::call_site(),
			"a channel needs `authorize = path`, the function that decides who may join",
		));
	};

	Ok(ChannelArgs {
		message,
		presence: options.presence,
		authorize,
		name: options.name,
		krate: options.krate,
	})
}

/// The options a channel may name, before the required ones are checked.
#[derive(Default)]
struct ChannelOptions {
	message: Option<Type>,
	presence: Option<Type>,
	authorize: Option<Path>,
	name: Option<LitStr>,
	krate: Option<Path>,
}

/// One `#[rpc::channel(...)]` option.
fn channel_option(options: &mut ChannelOptions, pair: Meta) -> syn::Result<()> {
	match pair {
		Meta::NameValue(entry) if entry.path.is_ident("message") => {
			options.message = Some(syn::parse2(entry.value.to_token_stream())?);
		}

		Meta::NameValue(entry) if entry.path.is_ident("presence") => {
			options.presence = Some(syn::parse2(entry.value.to_token_stream())?);
		}

		Meta::NameValue(entry) if entry.path.is_ident("authorize") => {
			options.authorize = Some(syn::parse2(entry.value.to_token_stream())?);
		}

		Meta::NameValue(entry) if entry.path.is_ident("name") => {
			options.name = Some(syn::parse2(entry.value.to_token_stream())?);
		}

		Meta::NameValue(entry) if entry.path.is_ident("crate") => {
			let value: LitStr = syn::parse2(entry.value.to_token_stream())?;
			options.krate = Some(value.parse()?);
		}

		other => {
			return Err(syn::Error::new_spanned(
				other,
				"a channel takes `message = Type`, `presence = Type`, `authorize = path`, `name = \
				 \"...\"`, and `crate = \"...\"`",
			));
		}
	}

	Ok(())
}
