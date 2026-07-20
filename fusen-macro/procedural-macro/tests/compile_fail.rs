#[test]
fn rejects_inherent_service_impl() {
    let tests = trybuild::TestCases::new();
    tests.compile_fail("tests/ui/*.rs");
}
