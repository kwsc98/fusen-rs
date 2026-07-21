#[test]
fn rejects_multiple_services_for_one_type() {
    let tests = trybuild::TestCases::new();
    tests.compile_fail("tests/ui/*.rs");
}
