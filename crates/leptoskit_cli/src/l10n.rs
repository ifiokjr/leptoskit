//! `leptoskit l10n`: writes the typed translations module from the ARB files
//! in the locales directory. `build` and `dev` run the same step first, so
//! the module is current before cargo compiles it.

use std::path::Path;

use anyhow::Context;
use anyhow::Result;
use leptoskit_config::L10nConfig;
use leptoskit_config::LeptoskitConfig;
use leptoskit_l10n_codegen::Generated;
use leptoskit_l10n_codegen::Status;

/// Runs `leptoskit l10n`: writes the module, or with `check`, fails when the
/// committed one is out of date.
pub fn command(check: bool) -> Result<()> {
	let (config, root) = LeptoskitConfig::load()?;
	let l10n = config.l10n.as_ref().with_context(|| {
		format!(
			"translations are off; add an `[l10n]` table to {}",
			leptoskit_config::CONFIG_FILE
		)
	})?;
	let generated = generate(&root, l10n)?;
	let output = &l10n.output;
	let shown = output.display();

	if check {
		let status = leptoskit_l10n_codegen::status(&root.join(output), &generated)?;
		anyhow::ensure!(
			status == Status::Current,
			"{shown} is out of date with {}; run `{} l10n` and commit the result",
			l10n.dir.display(),
			crate::cli::invoked_name()
		);

		crate::ui::skipped(format_args!("{shown} is up to date"));
	} else if leptoskit_l10n_codegen::write(&root.join(output), &generated)? {
		crate::ui::success(format_args!("wrote {shown}"));
	} else {
		crate::ui::skipped(format_args!("{shown} is up to date"));
	}

	print_summary(&generated);
	Ok(())
}

/// Brings the generated module up to date when the project has translations,
/// before a build compiles it. Writes only when the messages changed, so the
/// dev watcher never sees a rewrite of identical content.
pub fn sync(root: &Path, config: &LeptoskitConfig) -> Result<()> {
	let Some(l10n) = &config.l10n else {
		return Ok(());
	};
	let generated = generate(root, l10n)?;

	if leptoskit_l10n_codegen::write(&root.join(&l10n.output), &generated)? {
		crate::ui::success(format_args!(
			"wrote {} ({} messages in {} locales)",
			l10n.output.display(),
			generated.messages.len(),
			generated.locales.len()
		));
	}

	Ok(())
}

fn generate(root: &Path, l10n: &L10nConfig) -> Result<Generated> {
	leptoskit_l10n_codegen::generate(&root.join(&l10n.dir), &l10n.template).map_err(|diagnostics| {
		let lines = diagnostics
			.0
			.iter()
			.map(|diagnostic| format!("  {}", diagnostic.relative_to(root)))
			.collect::<Vec<_>>();
		anyhow::anyhow!(
			"{} cannot become translations:\n{}",
			l10n.dir.display(),
			lines.join("\n")
		)
	})
}

/// Lists every locale with how complete it is, then every message with the
/// method it became.
fn print_summary(generated: &Generated) {
	let total = generated.messages.len();
	let locales: Vec<[String; 3]> = generated
		.locales
		.iter()
		.map(|locale| {
			let completeness = if locale.template {
				format!("template, {total} messages")
			} else if locale.untranslated.is_empty() {
				"complete".to_owned()
			} else {
				format!(
					"{} of {total} translated; missing {}",
					total - locale.untranslated.len(),
					locale.untranslated.join(", ")
				)
			};

			[locale.tag.clone(), locale.file.clone(), completeness]
		})
		.collect();
	print_rows(&locales);
	let messages: Vec<[String; 2]> = generated
		.messages
		.iter()
		.map(|message| [message.key.clone(), message.signature.clone()])
		.collect();
	print_rows(&messages);
}

/// Prints rows with every column but the last padded to the column's width.
fn print_rows<const N: usize>(rows: &[[String; N]]) {
	let widths: Vec<usize> = (0..N.saturating_sub(1))
		.map(|column| {
			rows.iter()
				.map(|row| row[column].chars().count())
				.max()
				.unwrap_or(0)
		})
		.collect();

	for row in rows {
		let mut line = String::from(" ");

		for (column, cell) in row.iter().enumerate() {
			line.push_str("  ");
			line.push_str(cell);

			if let Some(width) = widths.get(column) {
				line.extend(std::iter::repeat_n(' ', width - cell.chars().count()));
			}
		}

		println!("{}", line.trim_end());
	}
}
