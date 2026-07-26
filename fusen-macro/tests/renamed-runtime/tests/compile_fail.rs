//! Negative contract tests that need a complete, renamed runtime dependency.

#[test]
fn rejects_lookalike_rpc_results() {
    let tests = trybuild::TestCases::new();
    tests.compile_fail("tests/ui/*.rs");
}
