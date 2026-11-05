//! Compile-fail tests: misuse of `view!` and `#[component]` must produce an
//! error that names the problem and points at the markup that caused it.

#[test]
fn compile_errors() {
	let cases = trybuild::TestCases::new();
	cases.compile_fail("tests/ui/*.rs");
}
