#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    fusen_fuzz_support::fuzz_http_binding(data);
});
