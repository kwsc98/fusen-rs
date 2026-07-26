//! Compile-time contract coverage for the clean-slate service macro.

#[test]
fn validates_macro_contracts() {
    let tests = trybuild::TestCases::new();
    tests.compile_fail("tests/ui/fail/*.rs");
    tests.pass("tests/ui/pass/*.rs");
}
