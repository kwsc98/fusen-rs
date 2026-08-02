# fusen-register

`fusen-register` defines the cancellation-safe service registration and
discovery SPI used by `fusen-rs`. Provider implementations own their remote
workers while callers interact through prepared lifecycle handles.

`Registry`, its request and handle types, `RegistryError`, the `directory`
publication API, and the safe `provider` constructors are stable extension
contracts across compatible 0.9.x releases. Tokio channels, provider SDK
objects, lifecycle workers, and cleanup coordinators remain private.

The SPI is independent from HTTP bindings and transport versions. A registration
publishes one service endpoint and its capabilities, while a subscription watches
one `ServiceSelector`; registry providers decide how that identity maps to their
native service names, groups, and metadata.

The lifecycle is explicit:

```text
Registry::prepare_registration / prepare_subscription
    -> track the returned handle
    -> handle.activate().await
    -> handle.close().await
```

`prepare_*` must only construct owned state. Remote side effects begin when
`activate()` starts the provider worker. Cancelling an activation or close
waiter does not cancel that worker; clones share one terminal result, and
`close()` is idempotent. Dropping the final handle requests cleanup without
blocking, so compensation requires its Tokio runtime to remain alive.

Subscriptions expose a latest-wins `Directory`. Each immutable
`DirectorySnapshot` contains a strictly increasing provider revision,
observation time, `DirectoryState`, and shared service instances. The states
are `Initializing`, `Ready`, `Stale`, `Unavailable`, and `Closed`. Tokio
channels and provider SDK values do not cross the SPI boundary.

Registry authors normally use `provider::registration`,
`provider::subscription`, and `directory::directory()` to construct conforming
handles. Errors are classified by
`error::RegistryError`, `RegistryOperation`, and `RegistryErrorKind`.

This crate is provider-neutral; use `fusen-nacos` for the Nacos adapter.
Version 0.9 defines the first compatibility baseline. Requires Rust 1.97 or
newer. Licensed under Apache-2.0.
