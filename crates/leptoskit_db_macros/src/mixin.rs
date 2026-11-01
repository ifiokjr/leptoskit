//! Expands `#[db::mixin]`: a reusable set of columns that tables flatten in
//! with `#[db(flatten)]`.
//!
//! Besides the struct, a mixin declares a hidden macro under the same name.
//! A table that flattens the mixin calls it, and it hands the mixin's fields
//! back to `table_continue!`: that is how the table learns columns declared
//! in another struct, or another crate.

use std::hash::Hash;
use std::hash::Hasher;

use proc_macro2::TokenStream;
use quote::format_ident;
use quote::quote;
use syn::Fields;
use syn::Ident;
use syn::ItemStruct;

use crate::attrs::FieldArgs;
use crate::attrs::MixinArgs;
use crate::attrs::Visibility;
use crate::attrs::is_db_attr;
use crate::attrs::is_vector_type;
use crate::table::check_derives;
use crate::table::is_provided_args;
use crate::table::json_schema_rules;

pub(crate) fn expand(raw_args: &TokenStream, mut item: ItemStruct) -> syn::Result<TokenStream> {
	let args = MixinArgs::parse(raw_args.clone())?;
	let krate = crate::runtime(args.krate.as_ref());

	if !item.generics.params.is_empty() {
		return Err(syn::Error::new_spanned(
			&item.generics,
			"a mixin can't be generic",
		));
	}

	check_derives(&item.attrs, "mixin")?;

	let Fields::Named(named) = &mut item.fields else {
		return Err(syn::Error::new_spanned(
			&item.fields,
			"a mixin needs named fields: one per column",
		));
	};

	let mut fields = Vec::new();

	for field in &named.named {
		let ident = field.ident.clone().expect("named fields have names");
		let args = FieldArgs::parse(&field.attrs)?;

		for (set, option) in [
			(args.key, "key"),
			(args.generate.is_some(), "generate"),
			(args.flatten, "flatten"),
			(args.prefix.is_some(), "prefix"),
		] {
			if set {
				return Err(syn::Error::new_spanned(
					&field.ident,
					format!(
						"a mixin's field can't be `{option}`: mixins have no key and don't nest"
					),
				));
			}
		}

		if args.vector.is_some() || args.public.is_some() || is_vector_type(&field.ty) {
			return Err(syn::Error::new_spanned(
				&field.ident,
				"a mixin can't hold a vector: declare vector columns on the table itself",
			));
		}

		if let Some(projection) = args.projections.first() {
			return Err(syn::Error::new_spanned(
				projection,
				"a mixin's field can't be `in(...)` a projection: projections belong to tables",
			));
		}

		fields.push((ident, field.clone(), args));
	}

	for columns in args.indexes.iter().chain(&args.uniques) {
		for ident in columns {
			if !fields.iter().any(|(field, ..)| field == ident) {
				return Err(syn::Error::new_spanned(
					ident,
					format!("`{}` has no field `{ident}`", item.ident),
				));
			}
		}
	}

	let name = &item.ident;
	let vis = &item.vis;
	let attrs = &item.attrs;
	let label = name.to_string();
	let declarations = fields.iter().map(|(_, field, _)| {
		let ident = &field.ident;
		let ty = &field.ty;
		let vis = &field.vis;
		let attrs = field.attrs.iter().filter(|attr| !is_db_attr(attr));
		quote! { #(#attrs)* #vis #ident: #ty }
	});
	let debug = fields.iter().map(|(ident, _, args)| {
		let label = ident.to_string();

		if args.visibility == Visibility::Secret {
			quote! { .field(#label, &::core::format_args!("<secret>")) }
		} else {
			quote! { .field(#label, &self.#ident) }
		}
	});
	let field_types = fields.iter().enumerate().map(|(position, (_, field, _))| {
		let ty = &field.ty;
		quote! {
			impl #krate::MixinField<#position> for #name {
				type Type = #ty;
			}
		}
	});
	let (public_type, public) = public_type(&krate, &item, &fields);
	let (new_type, new) = new_type(&item, &fields);
	let callback = callback(&item, raw_args, &fields);
	let doc = format!(
		" Columns that tables flatten in with `#[db(flatten)] pub {}: {name}`.",
		label.to_lowercase()
	);

	Ok(quote! {
		#(#attrs)*
		#[doc = ""]
		#[doc = #doc]
		#vis struct #name {
			#(#declarations,)*
		}

		impl ::core::fmt::Debug for #name {
			fn fmt(&self, formatter: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
				formatter.debug_struct(#label) #(#debug)* .finish()
			}
		}

		impl #krate::Mixin for #name {
			type Public = #public;
			type New = #new;
		}

		#(#field_types)*
		#public_type
		#new_type
		#callback
	})
}

type MixinField = (Ident, syn::Field, FieldArgs);

/// What a client sees of the mixin: its public fields, or `()` without any.
fn public_type(
	krate: &TokenStream,
	item: &ItemStruct,
	fields: &[MixinField],
) -> (TokenStream, TokenStream) {
	let visible: Vec<&MixinField> = fields
		.iter()
		.filter(|(.., args)| args.visibility == Visibility::Public)
		.collect();

	if visible.is_empty() {
		return (quote! {}, quote! { () });
	}

	let name = &item.ident;
	let vis = &item.vis;
	let public = format_ident!("Public{name}");
	let serde_path = format!("{krate}::__private::serde").replace(' ', "");
	let schemars_path = format!("{krate}::__private::schemars").replace(' ', "");
	let declarations = visible.iter().map(|(ident, field, args)| {
		let ty = &field.ty;
		let docs = field
			.attrs
			.iter()
			.filter(|attr| attr.path().is_ident("doc"));
		let schema = json_schema_rules(args);
		quote! { #(#docs)* #schema pub #ident: #ty }
	});
	let conversions = visible
		.iter()
		.map(|(ident, ..)| quote! { #ident: mixin.#ident });
	let doc = format!(
		" What a client may see of a [`{name}`]: every field except the `private` and `secret` \
		 ones."
	);

	(
		quote! {
			#[doc = #doc]
			#[derive(
				::core::clone::Clone,
				::core::fmt::Debug,
				::core::cmp::PartialEq,
				#krate::__private::serde::Serialize,
				#krate::__private::serde::Deserialize,
				#krate::__private::schemars::JsonSchema,
			)]
			#[serde(crate = #serde_path)]
			#[schemars(crate = #schemars_path)]
			#vis struct #public {
				#(#declarations,)*
			}

			impl ::core::convert::From<#name> for #public {
				fn from(mixin: #name) -> Self {
					Self { #(#conversions),* }
				}
			}
		},
		quote! { #public },
	)
}

/// What an insert provides: the whole mixin when it provides every field,
/// `()` when the ORM fills them all, and otherwise a `New…` struct of the
/// fields it provides.
fn new_type(item: &ItemStruct, fields: &[MixinField]) -> (TokenStream, TokenStream) {
	let provided: Vec<&MixinField> = fields
		.iter()
		.filter(|(.., args)| is_provided_args(args))
		.collect();

	if provided.is_empty() {
		return (quote! {}, quote! { () });
	}

	if provided.len() == fields.len() {
		return (quote! {}, quote! { Self });
	}

	let name = &item.ident;
	let vis = &item.vis;
	let new = format_ident!("New{name}");
	let declarations = provided.iter().map(|(ident, field, _)| {
		let ty = &field.ty;
		let vis = &field.vis;
		let docs = field
			.attrs
			.iter()
			.filter(|attr| attr.path().is_ident("doc"));
		quote! { #(#docs)* #vis #ident: #ty }
	});
	let doc = format!(
		" The fields of a [`{name}`] an insert provides; the database layer fills in the rest."
	);

	(
		quote! {
			#[doc = #doc]
			#vis struct #new {
				#(#declarations,)*
			}
		},
		quote! { #new },
	)
}

/// The hidden macro a table calls to learn the mixin's fields, exported
/// under the mixin's own name.
fn callback(item: &ItemStruct, raw_args: &TokenStream, fields: &[MixinField]) -> TokenStream {
	let name = &item.ident;
	let vis = &item.vis;
	// Exported macros share one namespace per crate, so the name carries a
	// hash of the declaration.
	let mut hasher = std::collections::hash_map::DefaultHasher::new();
	quote!(#item).to_string().hash(&mut hasher);
	let macro_name = format_ident!("__leptoskit_db_mixin_{}_{:016x}", name, hasher.finish());
	let options: Vec<TokenStream> = syn::parse::Parser::parse2(
		syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated,
		raw_args.clone(),
	)
	.map(|options| {
		options
			.into_iter()
			.filter(|option| !option.path().is_ident("crate"))
			.map(|option| quote!(#option))
			.collect()
	})
	.unwrap_or_default();
	let described = fields.iter().map(|(ident, field, _)| {
		let attrs = &field.attrs;
		quote! { #(#attrs)* #ident; }
	});

	quote! {
		#[doc(hidden)]
		#[macro_export]
		macro_rules! #macro_name {
			(($($cont:tt)*) $($state:tt)*) => {
				$($cont)*! { $($state)* { [#(#options),*] [#(#described)*] } }
			};
		}

		#[doc(hidden)]
		#[allow(unused_imports, reason = "tables call it by the mixin's name")]
		#vis use #macro_name as #name;
	}
}
