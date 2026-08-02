#!/usr/bin/env python3

from __future__ import annotations

import collections
import os
import pathlib
import subprocess
import tempfile
import textwrap
import unittest


SCRIPT = pathlib.Path(__file__).with_name("check-package-consumer.sh")
REPO_ROOT = SCRIPT.parents[2]


def write_executable(path: pathlib.Path, contents: str) -> None:
    path.write_text(contents, encoding="utf-8")
    path.chmod(0o755)


class PackageConsumerEnvironmentTests(unittest.TestCase):
    def test_cargo_commands_receive_only_the_sanitized_environment(self) -> None:
        secret = "FUSEN_PACKAGE_CONSUMER_SECRET_SENTINEL"
        with tempfile.TemporaryDirectory() as temporary_directory:
            temporary = pathlib.Path(temporary_directory)
            fake_bin = temporary / "bin"
            fake_bin.mkdir()
            invocation_log = temporary / "cargo-invocations.txt"
            caller_home = temporary / "caller-home"
            caller_cargo_home = temporary / "caller-cargo-home"
            rustup_home = temporary / "rustup-home"
            task_tmp = temporary / "tmp"
            for directory in [
                caller_home / ".cargo",
                caller_cargo_home,
                rustup_home,
                task_tmp,
            ]:
                directory.mkdir(parents=True, exist_ok=True)
            (caller_home / ".cargo" / "config.toml").write_text(
                f'[registry]\ntoken = "{secret}-home"\n',
                encoding="utf-8",
            )
            (caller_cargo_home / "credentials.toml").write_text(
                f'[registry]\ntoken = "{secret}-cargo-home"\n',
                encoding="utf-8",
            )

            fake_cargo = textwrap.dedent(
                """\
                #!/usr/bin/env python3
                import os
                import pathlib
                import sys
                import tarfile

                LOG_PATH = pathlib.Path(__LOG_PATH__)
                CALLER_HOME = pathlib.Path(__CALLER_HOME__)
                CALLER_CARGO_HOME = pathlib.Path(__CALLER_CARGO_HOME__)
                SECRET = __SECRET__
                ALLOWED_CARGO_ENV = {
                    "CARGO_HOME",
                    "CARGO_REGISTRIES_CRATES_IO_PROTOCOL",
                }
                FORBIDDEN_BUILD_ENV = {
                    "RUSTFLAGS",
                    "RUSTDOCFLAGS",
                    "RUSTC_WRAPPER",
                    "RUSTC_WORKSPACE_WRAPPER",
                }
                NETWORK_ENV = {
                    "HTTP_PROXY",
                    "HTTPS_PROXY",
                    "NO_PROXY",
                    "ALL_PROXY",
                    "http_proxy",
                    "https_proxy",
                    "no_proxy",
                    "all_proxy",
                    "SSL_CERT_FILE",
                    "SSL_CERT_DIR",
                }

                arguments = sys.argv[1:]
                if arguments and arguments[0].startswith("+"):
                    arguments = arguments[1:]
                command = arguments[0] if arguments else "missing"
                problems = []
                unexpected_cargo = sorted(
                    name
                    for name in os.environ
                    if name.startswith("CARGO_") and name not in ALLOWED_CARGO_ENV
                )
                if unexpected_cargo:
                    problems.append("cargo=" + ",".join(unexpected_cargo))
                unexpected_build = sorted(FORBIDDEN_BUILD_ENV.intersection(os.environ))
                if unexpected_build:
                    problems.append("build=" + ",".join(unexpected_build))
                if any(SECRET in value for value in os.environ.values()):
                    problems.append("secret-value-present")

                home = pathlib.Path(os.environ.get("HOME", ""))
                cargo_home = pathlib.Path(os.environ.get("CARGO_HOME", ""))
                if not home.is_dir() or home == CALLER_HOME:
                    problems.append("home-not-fresh")
                if not cargo_home.is_dir() or cargo_home == CALLER_CARGO_HOME:
                    problems.append("cargo-home-not-fresh")
                if (home / ".cargo" / "config.toml").exists():
                    problems.append("user-home-config-visible")
                if (cargo_home / "credentials.toml").exists():
                    problems.append("user-cargo-credentials-visible")
                if os.environ.get("CARGO_REGISTRIES_CRATES_IO_PROTOCOL") != "sparse":
                    problems.append("registry-protocol-not-sparse")

                inherited_network = sorted(NETWORK_ENV.intersection(os.environ))
                if command != "fetch" and inherited_network:
                    problems.append("offline-network=" + ",".join(inherited_network))
                if command == "fetch" and "--offline" in arguments:
                    problems.append("fetch-is-offline")
                if command != "fetch" and "--offline" not in arguments:
                    problems.append("offline-flag-missing")

                if problems:
                    print(
                        "sanitized Cargo environment check failed: " + ";".join(problems),
                        file=sys.stderr,
                    )
                    raise SystemExit(86)

                with LOG_PATH.open("a", encoding="utf-8") as log:
                    log.write(command + "\\n")

                if command == "package":
                    def option(name):
                        index = arguments.index(name)
                        return arguments[index + 1]

                    package = option("--package")
                    target_dir = pathlib.Path(option("--target-dir"))
                    archive = target_dir / "package" / f"{package}-0.9.0.crate"
                    archive.parent.mkdir(parents=True, exist_ok=True)
                    root = tarfile.TarInfo(f"{package}-0.9.0")
                    root.type = tarfile.DIRTYPE
                    root.mode = 0o755
                    with tarfile.open(archive, "w:gz") as packaged:
                        packaged.addfile(root)
                """
            )
            fake_cargo = (
                fake_cargo.replace("__LOG_PATH__", repr(str(invocation_log)))
                .replace("__CALLER_HOME__", repr(str(caller_home)))
                .replace("__CALLER_CARGO_HOME__", repr(str(caller_cargo_home)))
                .replace("__SECRET__", repr(secret))
            )
            write_executable(fake_bin / "cargo", fake_cargo)
            write_executable(fake_bin / "git", "#!/bin/sh\nexit 0\n")

            environment = os.environ.copy()
            environment.update(
                {
                    "PATH": f"{fake_bin}{os.pathsep}{environment['PATH']}",
                    "HOME": str(caller_home),
                    "CARGO_HOME": str(caller_cargo_home),
                    "RUSTUP_HOME": str(rustup_home),
                    "TMPDIR": str(task_tmp),
                    "RUST_TOOLCHAIN": "test-toolchain",
                    "CARGO_REGISTRY_TOKEN": f"{secret}-registry-token",
                    "CARGO_REGISTRIES_CRATES_IO_TOKEN": f"{secret}-crates-io-token",
                    "CARGO_REGISTRY_CREDENTIAL_PROVIDER": f"{secret}-provider",
                    "CARGO_REGISTRIES_CRATES_IO_CREDENTIAL_PROVIDER": (
                        f"{secret}-crates-io-provider"
                    ),
                    "CARGO_CONFIG": f"{secret}-config",
                    "CARGO_NET_OFFLINE": "true",
                    "CARGO_HTTP_PROXY": f"http://{secret}.invalid",
                    "CARGO_BUILD_TARGET": f"{secret}-target",
                    "CARGO_TARGET_DIR": f"{secret}-target-dir",
                    "CARGO_ENCODED_RUSTFLAGS": f"--cfg{chr(31)}{secret}",
                    "RUSTFLAGS": f"--cfg {secret}",
                    "RUSTDOCFLAGS": f"--cfg {secret}",
                    "RUSTC_WRAPPER": f"/{secret}/rustc-wrapper",
                    "RUSTC_WORKSPACE_WRAPPER": f"/{secret}/workspace-wrapper",
                }
            )

            completed = subprocess.run(
                ["bash", str(SCRIPT)],
                cwd=REPO_ROOT,
                env=environment,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
                timeout=30,
                check=False,
            )
            combined_output = completed.stdout + completed.stderr
            if secret in combined_output:
                self.fail("package consumer output exposed a sensitive Cargo environment value")
            self.assertEqual(completed.returncode, 0, combined_output)

            commands = invocation_log.read_text(encoding="utf-8").splitlines()
            counts = collections.Counter(commands)
            self.assertEqual(commands[0], "fetch")
            self.assertEqual(counts["fetch"], 1)
            self.assertEqual(counts["package"], 7)
            self.assertEqual(counts["generate-lockfile"], 7)
            self.assertEqual(counts["check"], 11)


if __name__ == "__main__":
    unittest.main()
