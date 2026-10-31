//! A tracked connection only reads: every way of writing through one fails
//! to compile, with an error that says why.

#![cfg(feature = "sqlite")]

#[test]
fn a_tracked_connection_cannot_write() {
	trybuild::TestCases::new().compile_fail("tests/ui/*.rs");
}
