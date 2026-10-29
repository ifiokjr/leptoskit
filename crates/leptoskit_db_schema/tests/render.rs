//! The SQL each scenario generates, for both dialects. Review these snapshots
//! as you would a migration: they are what apps will run.

mod support;

use std::fmt::Write;

use leptoskit_db_schema::Dialect;

#[test]
fn renders_every_scenario_for_both_dialects() {
	for scenario in support::scenarios() {
		for dialect in [Dialect::Postgres, Dialect::Sqlite] {
			if scenario.only.is_some_and(|only| only != dialect) {
				continue;
			}

			let migration = scenario.migration(dialect);
			insta::assert_snapshot!(format!("{}__{dialect}", scenario.name), migration.sql());
		}
	}
}

#[test]
fn every_scenario_lists_its_steps() {
	let mut summary = String::new();

	for scenario in support::scenarios() {
		summary.push_str(scenario.name);
		summary.push('\n');

		for step in &scenario.plan(Dialect::Postgres).steps {
			let _ = writeln!(summary, "  {step}");
		}
	}

	insta::assert_snapshot!(summary);
}
