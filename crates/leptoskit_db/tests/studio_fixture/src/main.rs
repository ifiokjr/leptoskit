//! A program with the studio feature on, as an app's db crate has it while
//! `leptoskit db studio` runs: `export_schema!`'s test serves the studio
//! when asked to. `tests/studio_release.rs` builds it in release and checks
//! that none of the studio is in it.

fn main() {
	leptoskit_db::__private::export_schema_if_requested();
}
