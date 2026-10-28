//! Expands `migrations!("../migrations")`: every migration listed in
//! `registry.txt`, embedded with its checksum.

use std::fmt::Write;
use std::fs;
use std::path::Path;

use proc_macro2::TokenStream;
use quote::quote;
use sha2::Digest;
use sha2::Sha256;
use syn::LitStr;

pub(crate) fn expand(
	path: &LitStr,
	manifest_dir: &Path,
	krate: &TokenStream,
) -> syn::Result<TokenStream> {
	let directory = manifest_dir.join(path.value());
	let error = |message: String| syn::Error::new(path.span(), message);
	let registry_path = directory.join("registry.txt");
	let registry = fs::read_to_string(&registry_path).map_err(|source| {
		error(format!(
			"couldn't read {}: {source}. Create the first migration with `leptoskit db migration \
			 create`",
			registry_path.display()
		))
	})?;
	let names: Vec<&str> = registry
		.lines()
		.map(str::trim)
		.filter(|line| !line.is_empty() && !line.starts_with('#'))
		.collect();

	// Every migration directory must be listed, so none is silently skipped.
	let entries = fs::read_dir(&directory)
		.map_err(|source| error(format!("couldn't list {}: {source}", directory.display())))?;

	for entry in entries.flatten() {
		let name = entry.file_name().to_string_lossy().to_string();

		if entry.path().is_dir() && !names.contains(&name.as_str()) {
			return Err(error(format!(
				"migration `{name}` isn't listed in registry.txt; regenerate it with `leptoskit \
				 db migration create`, or add the line by hand"
			)));
		}
	}

	let mut migrations = Vec::new();

	for name in &names {
		let file = directory.join(name).join("migration.sql");
		let sql = fs::read_to_string(&file).map_err(|source| {
			error(format!(
				"registry.txt lists `{name}`, but {} can't be read: {source}",
				file.display()
			))
		})?;
		let checksum = checksum(&sql);
		let file = file.to_string_lossy().to_string();
		migrations.push(
			quote! { #krate::Migration::embedded(#name, ::core::include_str!(#file), #checksum) },
		);
	}

	let registry_path = registry_path.to_string_lossy().to_string();

	Ok(quote! {
		{
			// Rebuilds when a migration is added to the registry.
			const _: &str = ::core::include_str!(#registry_path);
			&[#(#migrations),*]
		}
	})
}

/// The same checksum as `leptoskit_db::checksum`.
fn checksum(sql: &str) -> String {
	Sha256::digest(sql.as_bytes())
		.iter()
		.fold(String::with_capacity(64), |mut hex, byte| {
			let _ = write!(hex, "{byte:02x}");
			hex
		})
}
