//! The `#[derive(Route)]` macro for leptoskit file routes.
//!
//! This crate is an implementation detail of `leptoskit_router`; use the
//! derive through that crate.
#![doc(
	html_logo_url = "https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/icon.svg"
)]
#![doc(
	html_favicon_url = "https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/mark.svg"
)]

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::Data;
use syn::DeriveInput;
use syn::Fields;
use syn::GenericArgument;
use syn::PathArguments;
use syn::Type;
use syn::ext::IdentExt;

/// Implements `leptoskit_router::RouteParams` for a route struct.
///
/// Each field is read from the URL by name. The generated `routes.rs` decides
/// which names are path segments; every other field comes from the query
/// string. A field's type decides how it is read:
///
/// - `Option<T>` may be absent, and is left out of the URL when it is `None`;
/// - `Vec<T>` takes every value: the segments of a catch-all like `[...topic]`,
///   or each repeat of a query key;
/// - any other `T` is required, unless the field is marked `#[route(default)]`,
///   which reads `T::default()` when the value is absent and leaves it out of
///   the URL when it equals the default.
///
/// Every `T` needs `FromStr` (with an error that implements `Display`) and
/// `Display`. Unit structs are allowed: they only name a route.
#[proc_macro_derive(Route, attributes(route))]
pub fn derive_route(input: TokenStream) -> TokenStream {
	let input = syn::parse_macro_input!(input as DeriveInput);
	expand(&input)
		.unwrap_or_else(syn::Error::into_compile_error)
		.into()
}

/// How a field is read from and written to the URL.
enum FieldKind {
	One,
	OneOrDefault,
	Optional,
	Many,
}

fn expand(input: &DeriveInput) -> syn::Result<TokenStream2> {
	if !input.generics.params.is_empty() {
		return Err(syn::Error::new_spanned(
			&input.generics,
			"a route struct cannot be generic: its fields are read from a URL",
		));
	}
	for attribute in &input.attrs {
		if attribute.path().is_ident("route") {
			return Err(syn::Error::new_spanned(
				attribute,
				"`#[route(...)]` belongs on a field, as `#[route(default)]`",
			));
		}
	}
	let Data::Struct(data) = &input.data else {
		return Err(syn::Error::new_spanned(
			&input.ident,
			"`#[derive(Route)]` needs a struct with named fields, or a unit struct",
		));
	};
	let name = &input.ident;
	let fields = match &data.fields {
		Fields::Named(fields) => fields.named.iter().collect::<Vec<_>>(),
		Fields::Unit => Vec::new(),
		Fields::Unnamed(fields) => {
			return Err(syn::Error::new_spanned(
				fields,
				"`#[derive(Route)]` needs named fields: each field is read from the URL by its \
				 name",
			));
		}
	};

	let mut names = Vec::new();
	let mut decodes = Vec::new();
	let mut encodes = Vec::new();
	for field in &fields {
		let ident = field.ident.as_ref().expect("named fields have names");
		let key = ident.unraw().to_string();
		let kind = field_kind(field)?;
		let (decode, encode) = match kind {
			FieldKind::One => {
				(
					quote!(params.one(#key)?),
					quote!(sink.one(#key, &self.#ident);),
				)
			}
			FieldKind::OneOrDefault => {
				(
					quote!(params.one_or_default(#key)?),
					quote!(sink.unless_default(#key, &self.#ident);),
				)
			}
			FieldKind::Optional => {
				(
					quote!(params.optional(#key)?),
					quote!(sink.optional(#key, ::core::option::Option::as_ref(&self.#ident));),
				)
			}
			FieldKind::Many => {
				(
					quote!(params.many(#key)?),
					quote!(sink.many(#key, &self.#ident);),
				)
			}
		};
		names.push(key);
		decodes.push(quote!(#ident: #decode));
		encodes.push(encode);
	}

	let body = if matches!(data.fields, Fields::Unit) {
		quote!(Self)
	} else {
		quote!(Self { #(#decodes),* })
	};
	let params = if fields.is_empty() {
		quote!(_params)
	} else {
		quote!(params)
	};
	let sink = if fields.is_empty() {
		quote!(_sink)
	} else {
		quote!(sink)
	};
	Ok(quote! {
		#[automatically_derived]
		impl ::leptoskit_router::RouteParams for #name {
			const FIELDS: &'static [&'static str] = &[#(#names),*];

			fn decode(
				#params: &::leptoskit_router::Params<'_>,
			) -> ::core::result::Result<Self, ::leptoskit_router::ParamError> {
				::core::result::Result::Ok(#body)
			}

			fn encode(&self, #sink: &mut ::leptoskit_router::ParamSink) {
				#(#encodes)*
			}
		}
	})
}

fn field_kind(field: &syn::Field) -> syn::Result<FieldKind> {
	let mut default = false;
	for attribute in &field.attrs {
		if !attribute.path().is_ident("route") {
			continue;
		}
		attribute.parse_nested_meta(|meta| {
			if meta.path.is_ident("default") {
				default = true;
				Ok(())
			} else {
				Err(meta.error("unknown route option; the only one is `#[route(default)]`"))
			}
		})?;
	}
	let wrapper = wrapper_name(&field.ty);
	match (wrapper.as_deref(), default) {
		(Some("Option"), false) => Ok(FieldKind::Optional),
		(Some("Vec"), false) => Ok(FieldKind::Many),
		(Some(wrapper), true) => {
			Err(syn::Error::new_spanned(
				&field.ty,
				format!(
					"`#[route(default)]` is for required values; an empty `{wrapper}` is already \
					 the default"
				),
			))
		}
		(_, true) => Ok(FieldKind::OneOrDefault),
		(_, false) => Ok(FieldKind::One),
	}
}

/// `Option` or `Vec` when the type is written as one of them with a single
/// type argument.
fn wrapper_name(ty: &Type) -> Option<String> {
	let Type::Path(path) = ty else {
		return None;
	};
	let last = path.path.segments.last()?;
	let name = last.ident.to_string();
	if name != "Option" && name != "Vec" {
		return None;
	}
	let PathArguments::AngleBracketed(arguments) = &last.arguments else {
		return None;
	};
	matches!(arguments.args.first(), Some(GenericArgument::Type(_))).then_some(name)
}

#[cfg(test)]
mod tests {
	use syn::parse_quote;

	use super::*;

	fn error(input: &DeriveInput) -> String {
		expand(input).expect_err("rejected").to_string()
	}

	#[test]
	fn rejects_shapes_that_cannot_come_from_a_url() {
		assert!(
			error(&parse_quote!(
				struct Note<T> {
					note: T,
				}
			))
			.contains("cannot be generic")
		);
		assert!(
			error(&parse_quote!(
				#[route(default)]
				struct Note {
					note: u64,
				}
			))
			.contains("belongs on a field")
		);
		assert!(
			error(&parse_quote!(
				enum Note {
					A,
				}
			))
			.contains("needs a struct")
		);
		assert!(
			error(&parse_quote!(
				struct Note(u64);
			))
			.contains("needs named fields")
		);
	}

	#[test]
	fn rejects_unknown_and_pointless_field_options() {
		assert!(
			error(&parse_quote!(
				struct Note {
					#[route(skip)]
					note: u64,
				}
			))
			.contains("unknown route option")
		);
		assert!(
			error(&parse_quote!(
				struct Note {
					#[route(default)]
					tags: Vec<String>,
				}
			))
			.contains("an empty `Vec` is already the default")
		);
	}

	#[test]
	fn reads_only_wrappers_written_with_a_type_argument() {
		assert_eq!(
			wrapper_name(&parse_quote!(Option<u32>)).as_deref(),
			Some("Option")
		);
		assert_eq!(
			wrapper_name(&parse_quote!(std::vec::Vec<u32>)).as_deref(),
			Some("Vec")
		);
		assert_eq!(wrapper_name(&parse_quote!(&'static str)), None);
		assert_eq!(wrapper_name(&parse_quote!(Option)), None);
		assert_eq!(wrapper_name(&parse_quote!(Vec<'static>)), None);
		assert_eq!(wrapper_name(&parse_quote!(String)), None);
	}

	#[test]
	fn expands_a_unit_struct_without_reading_anything() {
		let tokens = expand(&parse_quote!(
			struct Today;
		))
		.expect("expands")
		.to_string();
		assert!(tokens.contains("_params"), "{tokens}");
		assert!(tokens.contains("_sink"), "{tokens}");
	}
}
