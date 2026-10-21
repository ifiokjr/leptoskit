//! `leptoskit routes`: writes the typed `Route` module from the routes
//! directory. `build`, `dev` and `native` run the same step first, so the
//! module is current before cargo compiles it.

use std::path::Path;

use anyhow::Context;
use anyhow::Result;
use leptoskit_config::LeptoskitConfig;
use leptoskit_config::RouterConfig;
use leptoskit_router_codegen::ExportStyle;
use leptoskit_router_codegen::Generated;
use leptoskit_router_codegen::RouteKind;
use leptoskit_router_codegen::Status;

/// Runs `leptoskit routes`: writes the module, or with `check`, fails when
/// the committed one is out of date.
pub fn command(check: bool) -> Result<()> {
	let (config, root) = LeptoskitConfig::load()?;
	let router = config.router.as_ref().with_context(|| {
		format!(
			"the file router is off; add a `[router]` table to {}",
			leptoskit_config::CONFIG_FILE
		)
	})?;
	let generated = generate(&root, router)?;
	let output = router.output();
	let shown = output.display();
	if check {
		let status = leptoskit_router_codegen::status(&root.join(&output), &generated)?;
		anyhow::ensure!(
			status == Status::Current,
			"{shown} is out of date with {}; run `{} routes` and commit the result",
			router.dir.display(),
			crate::cli::invoked_name()
		);
		println!("{shown} is up to date");
	} else if leptoskit_router_codegen::write(&root.join(&output), &generated)? {
		println!("wrote {shown}");
	} else {
		println!("{shown} is up to date");
	}
	print_table(&generated);
	Ok(())
}

/// Brings the generated module up to date when the project uses the file
/// router, before a build compiles it. Writes only when the routes changed,
/// so the dev watcher never sees a rewrite of identical content.
pub fn sync(root: &Path, config: &LeptoskitConfig) -> Result<()> {
	let Some(router) = &config.router else {
		return Ok(());
	};
	let generated = generate(root, router)?;
	let output = router.output();
	if leptoskit_router_codegen::write(&root.join(&output), &generated)? {
		println!(
			"wrote {} ({} routes)",
			output.display(),
			generated.routes.len()
		);
	}
	Ok(())
}

fn generate(root: &Path, router: &RouterConfig) -> Result<Generated> {
	leptoskit_router_codegen::generate(&root.join(&router.dir)).map_err(|diagnostics| {
		let lines = diagnostics
			.0
			.iter()
			.map(|diagnostic| {
				let file = diagnostic
					.file
					.strip_prefix(root)
					.unwrap_or(&diagnostic.file);
				match diagnostic.line {
					Some(line) => format!("  {}:{line}: {}", file.display(), diagnostic.message),
					None => format!("  {}: {}", file.display(), diagnostic.message),
				}
			})
			.collect::<Vec<_>>();
		anyhow::anyhow!(
			"{} cannot become routes:\n{}",
			router.dir.display(),
			lines.join("\n")
		)
	})
}

fn print_table(generated: &Generated) {
	let rows: Vec<[String; 3]> = generated
		.routes
		.iter()
		.map(|route| {
			let mut name = route.name.clone();
			if route.kind == RouteKind::NotFound {
				name.push_str(" (not found)");
			}
			if route.page == ExportStyle::Component {
				name.push_str(" (component)");
			}
			[
				route.pattern.clone(),
				name,
				route.file.display().to_string(),
			]
		})
		.collect();
	let widths = [0, 1].map(|column| {
		rows.iter()
			.map(|row| row[column].chars().count())
			.max()
			.unwrap_or(0)
	});
	for [pattern, name, file] in rows {
		println!(
			"  {pattern:<pattern_width$}  {name:<name_width$}  {file}",
			pattern_width = widths[0],
			name_width = widths[1]
		);
	}
}
