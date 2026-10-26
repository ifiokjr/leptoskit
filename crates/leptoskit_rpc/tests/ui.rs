//! Compile-fail tests: what `#[api]` cannot publish must fail the server
//! build with an error that names the problem and points at its cause.
#![cfg(feature = "ssr")]

#[test]
fn compile_errors() {
	let cases = trybuild::TestCases::new();
	cases.compile_fail("tests/ui/*.rs");
}
