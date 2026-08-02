#!/usr/bin/env python3

"""Reject removed 0.9 symbols from clean rustdoc output."""

import argparse
import re
import sys
from pathlib import Path


CRATES = {
    "fusen_config",
    "fusen_contract",
    "fusen_nacos",
    "fusen_observability",
    "fusen_procedural_macro",
    "fusen_register",
    "fusen_rs",
}
SOURCE_DIRECTORIES = (
    "fusen-config/src",
    "fusen-contract/src",
    "fusen-macro/procedural-macro/src",
    "fusen-nacos/src",
    "fusen-observability/src",
    "fusen-register/src",
    "fusen/src",
)

# Exact items removed by the clean-slate contract. The kind is intentionally not
# constrained: changing an old enum into a struct must not bypass the gate.
REMOVED_ITEMS = {
    "ApplicationError",
    "ConfigCloseFuture",
    "ConfigLifecycle",
    "ConfigManager",
    "ConfigResponse",
    "DirectoryWriter",
    "FusenError",
    "FusenHttpCodec",
    "FusenRequest",
    "FusenV1",
    "HotConfigChangeListener",
    "Http1PoolConfig",
    "Http2PoolConfig",
    "Idempotency",
    "InvocationFinish",
    "InvocationObserver",
    "InvocationOutcome",
    "InvocationPhase",
    "InvocationSide",
    "InvocationStart",
    "LogConfig",
    "LogWorkGroup",
    "Middleware",
    "MiddlewareFuture",
    "MiddlewareResult",
    "MiddlewareStage",
    "NacosConfiguration",
    "NacosRegister",
    "ParameterDescriptor",
    "ParameterSource",
    "Register",
    "RegisterError",
    "RegisteredRpcService",
    "RequestBodyCodec",
    "RequestCodec",
    "ResponseBodyCodec",
    "ResponseCodec",
    "ProtocolSet",
    "RpcArguments",
    "RpcBody",
    "RpcCall",
    "RpcCategory",
    "RpcContext",
    "RpcContextParts",
    "RpcError",
    "RpcErrorDetails",
    "RpcErrorInner",
    "RpcField",
    "RpcFieldSource",
    "RpcOrigin",
    "RpcResponse",
    "RpcSide",
    "RpcService",
    "RpcServiceInfo",
    "RpcMessage",
    "RpcRequest",
    "Router",
    "ServiceSnapshot",
    "ServiceSubscription",
    "SpringCloudMethod",
    "SpringCloudParameter",
    "SpringCloudParameterCardinality",
    "SpringCloudParameterSource",
    "SpringCloudV1",
    "StaticBoxFuture",
    "StrategyDebug",
    "SubscriptionCleanup",
    "SubscriptionCloser",
    "__benchmark_middleware",
    "asset",
    "attr_macro",
    "config_build",
    "directory_channel",
    "fusen_service",
    "fusen_trait",
    "fusen_attr",
    "get_config_by_path",
    "get_toml_by_context",
    "get_yaml_by_context",
    "init_log",
    "limit_str",
    "mask_str",
    "parse_ident_or_string",
    "parse_string",
    "rpc_call",
    "subscription_cleanup",
    "WireProtocol",
}
REMOVED_ITEM_PREFIXES = ("SpringCloud",)

# Old public modules and root items that are too generic to reject globally.
# Rustdoc emits an index only for an externally reachable module, so these
# checks do not confuse today's private implementation modules with public API.
REMOVED_PUBLIC_MODULES = {
    "fusen_contract/protocol/index.html",
    "fusen_register/contract/index.html",
    "fusen_rs/client/index.html",
    "fusen_rs/client/cluster/index.html",
    "fusen_rs/error/index.html",
    "fusen_rs/middleware/index.html",
}
REMOVED_ROOT_ITEMS = {
    ("fusen_config", "Error"),
}
REMOVED_CRATE_ITEMS = {
    "fusen_rs": {"Path"},
}
ALLOWED_PUBLIC_TRAITS = {
    "fusen_config": {"ConfigSource"},
    "fusen_contract": {"SensitiveFields"},
    "fusen_observability": {"MetricsRecorder"},
    "fusen_register": {"Registry"},
    "fusen_rs": {
        "ErrorDecoder",
        "InstanceRouter",
        "Interceptor",
        "LoadBalancer",
        "MetricsRecorder",
        "RequestEncoder",
        "Registry",
        "ResponseDecoder",
        "RetryPolicy",
        "Sanitizer",
        "SensitiveFields",
    },
}

REMOVED_METHODS = {
    "AttemptFinishedEvent": {"protocol"},
    "CallInfo": {"protocol"},
    "ClientBuilder": {"attempt_middleware", "middleware", "protocol"},
    "ClientError": {"message_ref"},
    "ClientRuntime": {"__client_builder"},
    "ClientRuntimeBuilder": {
        "attempt_middleware",
        "http1_pool",
        "http2_pool",
        "middleware",
        "observer",
    },
    "Context": {"__take_arguments", "protocol"},
    "Directory": {"fixed"},
    "Error": {"new"},
    "ErrorCategory": {"status"},
    "InvocationFinishedEvent": {"protocol"},
    "InvocationStartedEvent": {"protocol"},
    "MethodDescriptor": {"__new"},
    "MethodId": {"__new"},
    "Response": {"__from_result"},
    "Server": {"bind", "observer", "run", "run_with_shutdown"},
    "RegistrationRequest": {"protocol"},
    "ServerConfig": {"new", "protocols"},
    "ServerConfigBuilder": {"protocols"},
    "ServerError": {"message_ref"},
    "ServerBuilder": {"head_middleware", "middleware"},
    "ServiceDescriptor": {"__from_selector", "__new", "supported_protocols"},
    "ServiceRegistration": {"__new", "head_middleware", "middleware", "protocols"},
    "SubscriptionRequest": {"protocol"},
}
REMOVED_VARIANTS = {
    "ErrorCategory": {"Application"},
    "ErrorOrigin": {"Application"},
}

REMOVED_PACKAGE_PATHS = (
    "fusen-config/procedural-macro/Cargo.toml",
    "fusen-macro/support/Cargo.toml",
)

ITEM_FILE = re.compile(
    r"^(?:constant|enum|fn|macro|primitive|static|struct|trait|type|union)\."
    r"(?P<name>[^.!]+)!?\.html$"
)
INTERNAL_EXTENSION = re.compile(r"(?:Transport|Codec|Acceptor)")
OLD_WIRE_VARIANTS = (
    "FusenV1",
    "SpringCloudV1",
)
REMOVED_MACROS = {"service"}
PUBLIC_DEFINITION = re.compile(
    r"\bpub\s+(?:(?:async|const|unsafe)\s+)*"
    r"(?P<kind>enum|fn|mod|static|struct|trait|type|union)\s+"
    r"(?P<name>[A-Za-z_][A-Za-z0-9_]*)"
)
PUBLIC_USE = re.compile(r"\bpub\s+use\s+(?P<items>[^;]+);", re.DOTALL)
WIRE_VARIANT = re.compile(
    r"^\s*(?P<name>FusenV1|SpringCloudV1)\s*(?:,|\(|\{)", re.MULTILINE
)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "doc_root",
        type=Path,
        help="path to a clean rustdoc output directory (normally target/doc)",
    )
    parser.add_argument(
        "--source-root",
        type=Path,
        help="repository root used to catch doc-hidden public definitions and re-exports",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    doc_root = args.doc_root.resolve()
    missing = sorted(crate for crate in CRATES if not (doc_root / crate).is_dir())
    if missing:
        print(
            "public API denylist cannot audit missing rustdoc crates: " + ", ".join(missing),
            file=sys.stderr,
        )
        return 2

    failures: list[str] = []
    for public_module in sorted(REMOVED_PUBLIC_MODULES):
        if (doc_root / public_module).is_file():
            failures.append(f"{public_module}: removed public module")
    for crate in sorted(CRATES):
        crate_root = doc_root / crate
        for html in sorted(crate_root.rglob("*.html")):
            relative = html.relative_to(doc_root)
            match = ITEM_FILE.match(html.name)
            if match:
                name = match.group("name")
                if html.name.startswith("macro.") and name in REMOVED_MACROS:
                    failures.append(f"{relative}: removed public macro {name}")
                if name in REMOVED_ITEMS or name.startswith(REMOVED_ITEM_PREFIXES):
                    failures.append(f"{relative}: removed public item {name}")
                if html.name.startswith("trait.") and name not in ALLOWED_PUBLIC_TRAITS.get(
                    crate, set()
                ):
                    failures.append(f"{relative}: unexpected public extension trait {name}")
                if html.parent == crate_root and (crate, name) in REMOVED_ROOT_ITEMS:
                    failures.append(f"{relative}: removed root public item {name}")
                if name in REMOVED_CRATE_ITEMS.get(crate, set()):
                    failures.append(f"{relative}: removed crate public item {name}")
                if html.name.startswith("trait.") and INTERNAL_EXTENSION.search(name):
                    failures.append(
                        f"{relative}: transport/codec/acceptor traits are runtime-private"
                    )

            if html.name == "index.html":
                modules = html.relative_to(crate_root).parts[:-1]
                if "v2" in modules:
                    failures.append(f"{relative}: public v2 compatibility modules are forbidden")
                if "__private" in modules:
                    failures.append(f"{relative}: removed __private macro ABI is public")

            if html.name.startswith("enum."):
                contents = html.read_text(encoding="utf-8")
                for variant in OLD_WIRE_VARIANTS:
                    if f'id="variant.{variant}"' in contents:
                        failures.append(
                            f"{relative}: removed wire variant {variant} is public"
                        )

            type_match = re.match(r"^(?:enum|struct|trait)\.(?P<name>[^.]+)\.html$", html.name)
            if type_match:
                type_name = type_match.group("name")
                removed_methods = REMOVED_METHODS.get(type_name, set())
                removed_variants = REMOVED_VARIANTS.get(type_name, set())
                if not removed_methods and not removed_variants:
                    continue
                contents = html.read_text(encoding="utf-8")
                for method in sorted(removed_methods):
                    if f'id="method.{method}"' in contents:
                        failures.append(
                            f"{relative}: removed associated method "
                            f"{type_name}::{method}"
                        )
                for variant in sorted(removed_variants):
                    if f'id="variant.{variant}"' in contents:
                        failures.append(
                            f"{relative}: removed enum variant {type_name}::{variant}"
                        )

    if args.source_root is not None:
        source_root = args.source_root.resolve()
        for package_path in REMOVED_PACKAGE_PATHS:
            if (source_root / package_path).exists():
                failures.append(f"{package_path}: removed package is present")
        for directory in SOURCE_DIRECTORIES:
            for source in sorted((source_root / directory).rglob("*.rs")):
                contents = source.read_text(encoding="utf-8")
                relative = source.relative_to(source_root)
                for match in PUBLIC_DEFINITION.finditer(contents):
                    name = match.group("name")
                    if (
                        name in REMOVED_ITEMS
                        or name.startswith(REMOVED_ITEM_PREFIXES)
                        or name == "__private"
                    ):
                        failures.append(f"{relative}: removed public definition {name}")
                    if name == "v2":
                        failures.append(f"{relative}: public v2 compatibility module is forbidden")
                    if match.group("kind") == "trait" and INTERNAL_EXTENSION.search(name):
                        failures.append(
                            f"{relative}: transport/codec/acceptor traits are runtime-private"
                        )
                for match in PUBLIC_USE.finditer(contents):
                    identifiers = set(re.findall(r"[A-Za-z_][A-Za-z0-9_]*", match.group("items")))
                    removed_identifiers = {
                        name
                        for name in identifiers
                        if name in REMOVED_ITEMS
                        or name.startswith(REMOVED_ITEM_PREFIXES)
                        or name == "__private"
                    }
                    for name in sorted(removed_identifiers):
                        failures.append(f"{relative}: removed public re-export {name}")
                for match in WIRE_VARIANT.finditer(contents):
                    failures.append(
                        f"{relative}: removed wire variant {match.group('name')} is defined"
                    )
    if failures:
        print("\n".join(failures), file=sys.stderr)
        return 1

    print("public API denylist passed: no removed 0.9 symbols are exposed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
