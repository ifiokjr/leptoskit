//! The `#[db::table]` attribute, and `#[derive(db::Enum)]` and
//! `#[derive(db::Scalar)]`, behind leptoskit's
//! database layer.
//!
//! Apps use these through `leptoskit::db`; the documentation lives there.
#![doc(
	html_logo_url = "https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/icon.svg"
)]
#![doc(
	html_favicon_url = "https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/mark.svg"
)]

mod attrs;
mod enums;
mod migrations;
mod scalar;
mod table;

use proc_macro::TokenStream;
use proc_macro_crate::FoundCrate;
use quote::quote;
use syn::DeriveInput;
use syn::ItemStruct;

/// Declares a database table. See `leptoskit::db::table`.
#[proc_macro_attribute]
pub fn table(args: TokenStream, item: TokenStream) -> TokenStream {
	let result = attrs::TableArgs::parse(args.into()).and_then(|args| {
		let item: ItemStruct = syn::parse(item.clone())?;
		let krate = runtime(args.krate.as_ref());
		table::expand(&args, item, &krate)
	});

	match result {
		Ok(tokens) => tokens.into(),
		Err(error) => error.to_compile_error().into(),
	}
}

/// Embeds the migrations in a directory. See `leptoskit::db::migrations`.
#[proc_macro]
pub fn migrations(input: TokenStream) -> TokenStream {
	let result = syn::parse::<syn::LitStr>(input).and_then(|path| {
		let manifest_dir = std::env::var_os("CARGO_MANIFEST_DIR")
			.map(std::path::PathBuf::from)
			.ok_or_else(|| syn::Error::new(path.span(), "`migrations!` runs only under Cargo"))?;
		migrations::expand(&path, &manifest_dir, &runtime(None))
	});

	match result {
		Ok(tokens) => tokens.into(),
		Err(error) => error.to_compile_error().into(),
	}
}

/// Stores a unit enum as text. See `leptoskit::db::Enum`.
#[proc_macro_derive(Enum, attributes(db))]
pub fn derive_enum(input: TokenStream) -> TokenStream {
	let result =
		syn::parse::<DeriveInput>(input).and_then(|input| enums::expand(&input, &runtime(None)));

	match result {
		Ok(tokens) => tokens.into(),
		Err(error) => error.to_compile_error().into(),
	}
}

/// A newtype that is valid by construction. See `leptoskit::db::Scalar`.
#[proc_macro_derive(Scalar, attributes(db))]
pub fn derive_scalar(input: TokenStream) -> TokenStream {
	let result =
		syn::parse::<DeriveInput>(input).and_then(|input| scalar::expand(&input, &runtime(None)));

	match result {
		Ok(tokens) => tokens.into(),
		Err(error) => error.to_compile_error().into(),
	}
}

/// The path generated code uses for the runtime: `leptoskit::db` in an app,
/// or `leptoskit_db` where that is the dependency.
fn runtime(explicit: Option<&syn::Path>) -> proc_macro2::TokenStream {
	if let Some(path) = explicit {
		return quote!(#path);
	}

	let ident = |name: &str| syn::Ident::new(name, proc_macro2::Span::call_site());

	if let Ok(found) = proc_macro_crate::crate_name("leptoskit") {
		return match found {
			FoundCrate::Itself => quote!(crate::db),
			FoundCrate::Name(name) => {
				let name = ident(&name);
				quote!(::#name::db)
			}
		};
	}

	match proc_macro_crate::crate_name("leptoskit_db") {
		Ok(FoundCrate::Name(name)) => {
			let name = ident(&name);
			quote!(::#name)
		}
		// The runtime's own tests and examples.
		Ok(FoundCrate::Itself) | Err(_) => quote!(::leptoskit_db),
	}
}

#[cfg(test)]
mod tests;
