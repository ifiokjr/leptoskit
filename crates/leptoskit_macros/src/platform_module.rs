//! Selects one app-owned module without copying or rewriting its source.

use std::path::Component;
use std::path::Path;
use std::path::PathBuf;

use proc_macro2::TokenStream;
use quote::quote;
use syn::Attribute;
use syn::Ident;
use syn::LitStr;
use syn::Token;
use syn::Visibility;
use syn::parse::Parse;
use syn::parse::ParseStream;

/// A module declaration with a crate-source-relative shared file stem.
struct Module {
	attributes: Vec<Attribute>,
	visibility: Visibility,
	name: Ident,
	path: LitStr,
}

impl Parse for Module {
	fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
		let attributes = input.call(Attribute::parse_outer)?;
		let visibility = input.parse()?;
		input.parse::<Token![mod]>()?;
		let name = input.parse()?;
		input.parse::<Token![=]>()?;
		let path = input.parse()?;
		input.parse::<Option<Token![;]>>()?;

		Ok(Self {
			attributes,
			visibility,
			name,
			path,
		})
	}
}

/// Expands the selected file into a normal Rust module declaration.
pub fn expand(tokens: TokenStream) -> TokenStream {
	let result = syn::parse2::<Module>(tokens).and_then(|module| {
		let manifest = std::env::var_os("CARGO_MANIFEST_DIR").ok_or_else(|| {
			syn::Error::new(
				module.path.span(),
				"platform_module! requires a Cargo application",
			)
		})?;
		let platform = std::env::var("LEPTOSKIT_PLATFORM").map_err(|_| {
			syn::Error::new(
				module.path.span(),
				"platform_module! requires leptoskit_build::platforms() in the application's \
				 build.rs",
			)
		})?;

		expand_in(&module, Path::new(&manifest), &platform)
	});

	result.unwrap_or_else(|error| error.to_compile_error())
}

/// Resolves the shared fallback and optional platform override below `src`.
fn expand_in(module: &Module, manifest: &Path, platform: &str) -> syn::Result<TokenStream> {
	let path = module.path.value();
	let stem = Path::new(&path);
	let source = manifest.join("src");
	let attributes = &module.attributes;
	let visibility = &module.visibility;
	let name = &module.name;
	let error = |message: &str| syn::Error::new(module.path.span(), message);

	if let Some(attribute) = attributes
		.iter()
		.find(|attribute| attribute.path().is_ident("path"))
	{
		return Err(syn::Error::new_spanned(
			attribute,
			"platform_module! selects the module path; remove the #[path] attribute",
		));
	}

	if stem.as_os_str().is_empty()
		|| stem
			.components()
			.any(|component| !matches!(component, Component::Normal(_)))
		|| stem.extension().is_some()
		|| path.contains('\\')
	{
		return Err(error(
			"platform module paths must be relative to src without an extension, `..`, or \
			 backslashes; use \"routes/profile\"",
		));
	}

	if platform.is_empty() || !platform.bytes().all(|byte| byte.is_ascii_lowercase()) {
		return Err(error(
			"invalid LEPTOSKIT_PLATFORM; select the application platform with \
			 leptoskit_build::platforms()",
		));
	}

	let source = source.canonicalize().map_err(|source_error| {
		error(&format!(
			"could not resolve {}: {source_error}",
			source.display()
		))
	})?;
	let shared = checked_source(&source, &stem.with_extension("rs"), &module.path)?;
	let platform_source = source.join(stem.with_extension(format!("{platform}.rs")));
	let selected = match std::fs::symlink_metadata(&platform_source) {
		Ok(_) if platform != "shared" => {
			checked_source(
				&source,
				&stem.with_extension(format!("{platform}.rs")),
				&module.path,
			)?
		}
		Ok(_) => shared,
		Err(source_error) if source_error.kind() == std::io::ErrorKind::NotFound => shared,
		Err(source_error) => {
			return Err(error(&format!(
				"could not inspect {}: {source_error}",
				platform_source.display(),
			)));
		}
	};
	let selected = selected
		.to_str()
		.ok_or_else(|| error("platform module paths must be UTF-8"))?;

	Ok(quote! {
		#(#attributes)*
		#[path = #selected]
		#visibility mod #name;
	})
}

/// Canonicalizes a module source and refuses symlinks outside the app source.
fn checked_source(source: &Path, relative: &Path, literal: &LitStr) -> syn::Result<PathBuf> {
	let path = source.join(relative);
	let canonical = path.canonicalize().map_err(|source_error| {
		syn::Error::new(
			literal.span(),
			format!(
				"could not resolve shared/platform module {}: {source_error}",
				path.display()
			),
		)
	})?;

	if !canonical.starts_with(source) || !canonical.is_file() {
		return Err(syn::Error::new(
			literal.span(),
			format!(
				"platform module {} must be a file inside {}",
				path.display(),
				source.display()
			),
		));
	}

	Ok(canonical)
}

#[cfg(test)]
mod tests {
	use super::*;

	/// Makes a source directory with one portable module for resolution tests.
	fn fixture() -> tempfile::TempDir {
		let directory = tempfile::tempdir().unwrap();

		std::fs::create_dir_all(directory.path().join("src/routes")).unwrap();
		std::fs::write(
			directory.path().join("src/routes/profile.rs"),
			"pub fn shared() {}",
		)
		.unwrap();
		directory
	}

	#[test]
	fn rejects_paths_outside_the_source_tree_and_explicit_extensions() {
		let directory = fixture();

		for path in [
			"../outside",
			"/outside",
			"routes/profile.rs",
			"routes\\profile",
			"",
		] {
			let literal = LitStr::new(path, proc_macro2::Span::call_site());
			let module = syn::parse2::<Module>(quote!(pub mod profile = #literal;)).unwrap();
			let error = expand_in(&module, directory.path(), "ios")
				.unwrap_err()
				.to_string();

			assert!(error.contains("relative to src"), "{path}: {error}");
		}
	}

	#[test]
	fn requires_a_shared_fallback_even_when_an_override_exists() {
		let directory = fixture();
		let module = syn::parse2::<Module>(quote!(mod missing = "routes/missing";)).unwrap();

		std::fs::write(directory.path().join("src/routes/missing.ios.rs"), "").unwrap();
		let error = expand_in(&module, directory.path(), "ios")
			.unwrap_err()
			.to_string();

		assert!(error.contains("missing.rs"), "{error}");
	}

	#[test]
	fn preserves_module_visibility_and_documentation() {
		let directory = fixture();
		let module = syn::parse2::<Module>(quote!(
			/// The portable profile module.
			pub(crate) mod profile = "routes/profile";
		))
		.unwrap();
		let expanded = expand_in(&module, directory.path(), "ios")
			.unwrap()
			.to_string();

		assert!(expanded.contains("pub (crate) mod profile"), "{expanded}");
		assert!(
			expanded.contains("The portable profile module."),
			"{expanded}"
		);
		assert!(expanded.contains("profile.rs"), "{expanded}");
	}

	#[test]
	fn rejects_a_second_module_path_attribute() {
		let directory = fixture();
		let module = syn::parse2::<Module>(quote!(
			#[path = "somewhere.rs"]
			mod profile = "routes/profile";
		))
		.unwrap();
		let error = expand_in(&module, directory.path(), "ios")
			.unwrap_err()
			.to_string();

		assert!(error.contains("remove the #[path] attribute"), "{error}");
	}

	#[cfg(unix)]
	#[test]
	fn rejects_override_symlinks_outside_source() {
		let directory = fixture();
		let outside = directory.path().join("outside.rs");
		let module = syn::parse2::<Module>(quote!(mod profile = "routes/profile";)).unwrap();

		std::fs::write(&outside, "pub fn outside() {}").unwrap();
		std::os::unix::fs::symlink(&outside, directory.path().join("src/routes/profile.ios.rs"))
			.unwrap();
		let error = expand_in(&module, directory.path(), "ios")
			.unwrap_err()
			.to_string();

		assert!(error.contains("must be a file inside"), "{error}");
	}

	#[cfg(unix)]
	#[test]
	fn reports_a_broken_override_instead_of_silently_using_shared_source() {
		let directory = fixture();
		let module = syn::parse2::<Module>(quote!(mod profile = "routes/profile";)).unwrap();

		std::os::unix::fs::symlink(
			directory.path().join("src/routes/missing.rs"),
			directory.path().join("src/routes/profile.ios.rs"),
		)
		.unwrap();
		let error = expand_in(&module, directory.path(), "ios")
			.unwrap_err()
			.to_string();

		assert!(error.contains("profile.ios.rs"), "{error}");
	}
}
