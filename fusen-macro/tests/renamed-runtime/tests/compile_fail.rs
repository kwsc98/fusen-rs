//! Negative contract tests that need a complete, renamed runtime dependency.

#[test]
fn rejects_invalid_external_contracts() {
    let tests = trybuild::TestCases::new();
    tests.compile_fail("tests/ui/*.rs");
}
