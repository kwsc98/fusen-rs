# Nightly fuzzing

The fuzz workspace exercises the actual private wire, route, and Problem Details implementation
without adding a public runtime test API. `fusen-fuzz-support` compiles those source modules into an
isolated, non-published harness; required CI checks that harness with Rust 1.97 so private source
changes cannot silently make nightly fuzzing stale.

Run one target with a nightly toolchain and `cargo-fuzz`:

```shell
cd fuzz
cargo +nightly fuzz run wire_codec
cargo +nightly fuzz run spring_path
cargo +nightly fuzz run problem_details
```

Seed corpora are committed under `corpus/`. Crashing and timeout inputs are written under
`artifacts/`, ignored by Git, and uploaded by the nightly workflow when a target fails. Promote a
minimized regression input into a deterministic unit or integration test before closing the issue.
