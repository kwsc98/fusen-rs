# 0.9 发布流程

`0.9.0` 是第一个 compatibility baseline。发布前不运行相对历史开发提交的 semver 检查；`v0.9.0` tag 完成后以该 tag 配置后续 `cargo-semver-checks`。

## Required Checks

```shell
cargo +1.97.0 fmt --all --check
cargo +1.97.0 fmt --manifest-path fuzz-support/Cargo.toml -- --check
cargo +1.97.0 fmt --manifest-path fuzz/Cargo.toml -- --check
cargo +1.97.0 clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo +1.97.0 test --locked --workspace --all-features
cargo +1.97.0 test --locked --workspace --all-features --doc
RUSTDOCFLAGS="-D warnings" cargo +1.97.0 doc --locked --workspace --all-features --no-deps

cargo +1.97.0 check --locked -p fusen-config --no-default-features
cargo +1.97.0 check --locked -p fusen-observability --no-default-features
cargo +stable test --locked --workspace --all-features
bash .github/scripts/check-security.sh
bash .github/scripts/check-dependency-policy.sh
bash .github/scripts/check-public-api-denylist.sh
bash .github/scripts/check-package-consumer.sh
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s .github/scripts -p 'test_run_benchmark_gate.py' -v
cargo +1.97.0 clippy --locked --offline --manifest-path fuzz-support/Cargo.toml --all-targets -- -D warnings
```

CI 还必须通过 Linux/macOS/Windows、feature matrix、renamed-runtime macro consumer、`nacos-live-container`、HTTP/HTTPS client socket tests、lifecycle 重复测试、Markdown links 与从 `.crate` archive 构建的 package consumer。`repeat-lifecycle-tests.sh` 和 `run-live-nacos-tests.sh` 都先列举预期测试并在匹配数为零时失败，Cargo 的空过滤器不得被当成成功。
定时流水线还必须通过三个真实私有源码 harness 的 `cargo-fuzz` 任务，以及 `runtime_e2e` 和 `wire_v1_contract` 的 100 轮真实 socket 重复测试。fuzz corpus 与运行方法见 [`fuzz/README.md`](../fuzz/README.md)。

## Contract Audit

发布负责人必须确认：

- `check-public-api-denylist.sh` 从干净 rustdoc 输出确认旧入口为零，只有 `service`/`method` 宏和约定的八个扩展 SPI；
- `check-security.sh` 对 root、fuzz-support 和 fuzz 的独立 manifest/lockfile 分别运行 `cargo deny` 的四类检查与 `cargo audit`，前项失败后仍执行并汇总其余结果；三者的解析后 dependency graph 只包含批准的 Rustls Ring/bundled WebPKI client TLS 栈，不含 `hyper-tls`、`native-tls`、OpenSSL TLS backend、AWS-LC、native/system root loader、platform verifier 或 PEM loader；
- Core 不依赖 Nacos、subscriber 或 OTel backend；
- Fusen V1/Spring Cloud V1 golden fixtures、真实明文 H1/h2c sockets、HTTPS H1/ALPN h2 sockets、证书/hostname 拒绝、Problem Details 和 macro trybuild 全部通过；
- HTTPS 测试确认 Rustls Ring、TLS 1.2/1.3、bundled Mozilla WebPKI roots、无明文 fallback；Server listener 仍不加载证书或私钥；
- 永久 pending request/registry/config cleanup 在 deadline 内有界返回；
- lifecycle、retry、breaker 与 byte-budget tests 不依赖 correctness sleep 或预占端口；
- 在绑定参考机器上执行 `Release Benchmark Gate`，四个 Fusen `c1/c100 × small/64 KiB` case 的 p50/p99 相对 committed baseline 回退均不超过 10%；Spring H1 同矩阵、QPS、原始五轮日志与 JSON summary 已归档。

真实 Nacos container tests 是 required release gate。CI 固定启动 Nacos `v2.4.3` standalone service container，测试同时覆盖 registration/discovery 和 config publish/listen，并在主流程失败后继续关闭 handle、注销 instance 和删除 config。资源名由 GitHub run id、进程 id 和时间戳组成，不复用共享名称。

在已经运行并 ready 的 Nacos 上可复现同一个非空 gate：

```shell
NACOS_ADDR=127.0.0.1:8848 \
NACOS_TEST_RUN_ID=manual-$(date +%s) \
bash .github/scripts/run-live-nacos-tests.sh
```

脚本在执行前要求至少发现一个 `live_nacos_` ignored test；零测试、provider error、cleanup error 或 deadline 均使 gate 失败。CI job 是容器启动和 finally cleanup 的规范入口，本地命令不会代替调用方关闭自己启动的 Nacos 容器。

## Performance Gate

普通托管 CI 会运行一轮缩短参数但 case 完整的 benchmark smoke，并从按 `run_id-run_attempt` 隔离的目录上传 artifact，用于确认 Fusen h2c、Spring H1、并发/大 payload 和 schema 可执行；不同托管机器之间的绝对延迟不用于 10% 判定。

正式比较由 [release-benchmark.yml](../.github/workflows/release-benchmark.yml) 在带有 `fusen-benchmark-0-9-reference` label 的固定 self-hosted runner 上执行。该 runner 必须是建立 [committed baseline](../.github/benchmarks/fusen-0.9-reference-macos-arm64.json) 的同一参考机器：

```shell
python3 .github/scripts/run-benchmark-gate.py \
  --host-id fusen-0.9-reference-macos-arm64 \
  --runs 5 \
  --baseline .github/benchmarks/fusen-0.9-reference-macos-arm64.json \
  --output-dir target/release-benchmark-gate/manual-compare-001
```

每次运行必须使用全新或空的 output directory；workflow 使用 `run_id-run_attempt` 唯一路径，防止失败 artifact 混入旧证据。比较器拒绝脏工作树，以及 host id、CPU、OS、toolchain、rustc、schema、参数、五轮原始样本不匹配，并要求 baseline `source_commit` 是当前候选 `HEAD` 的 ancestor；四个 Fusen case 的 p50 或 p99 任一中位数回退严格超过 10% 即非零退出。QPS 只记录，Spring H1 的四个 case 只归档。发布负责人必须保留 workflow artifact，并确认 checkout SHA 是待发布 commit。

首个 schema v2 baseline 必须两阶段生成：先提交 benchmark 实现为干净候选 A，再在固定 runner 上以 workflow 的 `calibrate` mode 对 A 运行五轮；审查 artifact 后将生成的 baseline 单独提交。生成文件的完整 `source_commit` 必须等于 A，且 compare 时 A 必须是当前候选 `HEAD` 的 ancestor。committed baseline 仍为 `calibration-required` 时 compare 必须失败，不能发布。完整命令与字段定义见[性能基线](performance-baseline.md)。

## Final Candidate Freeze

最终候选必须已经包含实际发布日期、committed benchmark baseline 和全部发布文档。替换 CHANGELOG 中的 `YYYY-MM-DD`、提交该变更并完成最后一次代码审查后，才记录候选 SHA；发布日期延后或任何后续修改都不能在原候选上补写。

```shell
set -euo pipefail
export RELEASE_CANDIDATE_SHA="$(git rev-parse HEAD)"
[[ "$RELEASE_CANDIDATE_SHA" =~ ^[0-9a-f]{40}$ ]]
test -z "$(git status --porcelain=v1)"
if rg -n '^## \[0\.9\.0\] - YYYY-MM-DD$' CHANGELOG.md; then
  echo "CHANGELOG release date is still a placeholder" >&2
  exit 1
fi
```

立即把完整 SHA 写入候选外部记录。以后开始 M0.12 发布会话时，必须从该记录重新设置 `RELEASE_CANDIDATE_SHA`，不能重新用当时的 `HEAD` 推导候选。

在同一 `RELEASE_CANDIDATE_SHA` 上依次完成完整 CI、Nightly、真实 Nacos、三 workspace security、package archive consumer 和固定机 benchmark compare。每个 run 必须核对 checkout SHA，不允许本地结果替代 Windows、Nightly、Nacos 或固定机器证据。将下表填入 GitHub Release draft 或 workflow summary；不要为了回填 run URL 再提交仓库：

| Evidence | Run URL | Checkout SHA | Artifact / result |
| --- | --- | --- | --- |
| CI：MSRV、Linux、macOS、Windows、release contracts |  |  | `benchmark-smoke-<os>-<arch>-<sha>` 及 required jobs 全绿 |
| CI：三 workspace security |  |  | `security` job 成功，root/fuzz-support/fuzz 的 deny 与 audit 均执行 |
| CI：真实 Nacos |  |  | `nacos-live-container` job 成功 |
| CI：七个 package archive consumer |  |  | `release-contracts` 中 package consumer step 成功 |
| Nightly：三个 fuzz target、100 轮 core E2E |  |  | 三个 target 的日志/artifact 名称与 `core-e2e-repeat-100` 结果 |
| Release Benchmark Gate |  |  | `release-benchmark-compare-<sha>`、baseline SHA、四个 Fusen case 的 p50/p99 summary |

任意代码、测试、Cargo manifest/lockfile、workflow、CHANGELOG 或发布文档改动都会产生新候选，原候选的全部证据立即失效。新的候选必须从完整 CI 开始重跑所有证据；只修改注释、日期或证据链接也不例外。Release draft、workflow summary 和已有 artifact 是候选外部记录，不提交“证据更新”来改变候选 SHA。

## crates.io Preflight

以下步骤只在 M0.11 的同一 SHA 全绿后执行。本 runbook 要求通过 `CARGO_REGISTRY_TOKEN` 环境变量提供 token，禁止将 token 写入仓库、命令参数、日志或 shell trace；不要执行 `cargo login <token>`。只检查 token 非空并通过 crates.io 身份接口验证，不打印值：

```shell
set -euo pipefail
set +x
test -n "${CARGO_REGISTRY_TOKEN:-}" || {
  echo "CARGO_REGISTRY_TOKEN is not set" >&2
  exit 1
}
test -n "${CRATES_IO_OWNER:-}" || {
  echo "CRATES_IO_OWNER is not set" >&2
  exit 1
}

auth_response="$(
  printf 'header = "Authorization: %s"\n' "$CARGO_REGISTRY_TOKEN" |
    curl --fail --silent --show-error --config - https://crates.io/api/v1/me
)"
authenticated_owner="$(
  printf '%s' "$auth_response" |
    python3 -c 'import json, sys; print(json.load(sys.stdin)["user"]["login"])'
)"
unset auth_response
test "$authenticated_owner" = "$CRATES_IO_OWNER" || {
  echo "crates.io token owner does not match CRATES_IO_OWNER" >&2
  exit 1
}
echo "crates.io token identity and expected owner login passed preflight; token value was not printed"
```

在 crates.io token 设置页确认该 token 允许发布新 crate，并允许更新下列全部已有 crate。然后通过公共 API 检查名称和 `0.9.0` 是否已存在；HTTP error 或无法联网必须阻断，不能被解释为名称可用：

```shell
packages=(
  fusen-contract
  fusen-register
  fusen-config
  fusen-observability
  fusen-procedural-macro
  fusen-nacos
  fusen-rs
)

for package in "${packages[@]}"; do
  name_status="$(curl --silent --show-error --output /dev/null \
    --write-out '%{http_code}' "https://crates.io/api/v1/crates/$package")"
  case "$name_status" in
    200)
      owners="$(cargo +1.97.0 owner --registry crates-io --list "$package")"
      printf '%s\n' "$owners"
      if ! printf '%s\n' "$owners" | rg --fixed-strings -- "$CRATES_IO_OWNER" >/dev/null; then
        echo "$CRATES_IO_OWNER is not an owner of $package" >&2
        exit 1
      fi
      ;;
    404)
      echo "$package is not published yet; re-check token permission to publish a new crate"
      ;;
    *)
      echo "unexpected crates.io status for $package: $name_status" >&2
      exit 1
      ;;
  esac

  version_status="$(curl --silent --show-error --output /dev/null \
    --write-out '%{http_code}' "https://crates.io/api/v1/crates/$package/0.9.0")"
  case "$version_status" in
    404) ;;
    200)
      echo "$package 0.9.0 already exists; stop the initial release procedure" >&2
      exit 1
      ;;
    *)
      echo "unexpected crates.io version status for $package: $version_status" >&2
      exit 1
      ;;
  esac
done
```

名称检查只能确认检查时的状态。开始上传前仍需重新确认工作树、候选 SHA 和全部 archive；若新名称在此期间被占用，停止发布并重新规划 crate 名称和版本，不能切换到未审查的包名继续发布。

```shell
test "$(git rev-parse HEAD)" = "$RELEASE_CANDIDATE_SHA"
test -z "$(git status --porcelain=v1)"
bash .github/scripts/check-package-consumer.sh
```

`check-package-consumer.sh` 对七个发布 crate 执行不带 `--no-verify` 的 `cargo package`，解包每个 `.crate`，并在七个独立外部 workspace 中编译；`fusen-rs` consumer 还会展开服务宏。脚本的 path patches 只用于验证候选 archive，不是 crates.io 传播证据。实际发布命令禁止 `--allow-dirty` 和 `--no-verify`。

## Packaging And Publication Order

按以下四层发布。每层开始前重新核对候选，先对该层全部 crate dry-run，再执行正式 publish；上一层必须全部能从 crates.io 精确解析并下载后，才能 dry-run 下一层。

等待函数使用 `cargo info --registry crates-io <crate>@0.9.0` 同时检查 registry 解析和 archive 下载。超时或返回其他版本均阻断发布：

```shell
assert_release_candidate() {
  test "$(git rev-parse HEAD)" = "$RELEASE_CANDIDATE_SHA"
  test -z "$(git status --porcelain=v1)"
}

wait_for_crates_io() {
  package="$1"
  for attempt in $(seq 1 60); do
    if cargo +1.97.0 info --registry crates-io "${package}@0.9.0"; then
      return 0
    fi
    echo "waiting for $package 0.9.0 registry propagation ($attempt/60)" >&2
    sleep 10
  done
  echo "$package 0.9.0 did not propagate within 10 minutes" >&2
  return 1
}
```

第一层（不依赖其他 workspace 发布 crate）：

```shell
assert_release_candidate
cargo +1.97.0 publish --locked --registry crates-io --dry-run -p fusen-procedural-macro
cargo +1.97.0 publish --locked --registry crates-io --dry-run -p fusen-config
cargo +1.97.0 publish --locked --registry crates-io --dry-run -p fusen-observability

cargo +1.97.0 publish --locked --registry crates-io -p fusen-procedural-macro
cargo +1.97.0 publish --locked --registry crates-io -p fusen-config
cargo +1.97.0 publish --locked --registry crates-io -p fusen-observability

wait_for_crates_io fusen-procedural-macro
wait_for_crates_io fusen-config
wait_for_crates_io fusen-observability
```

第二层（`fusen-contract` 的可选 `derive` feature 依赖第一层的过程宏）：

```shell
assert_release_candidate
cargo +1.97.0 publish --locked --registry crates-io --dry-run -p fusen-contract
cargo +1.97.0 publish --locked --registry crates-io -p fusen-contract
wait_for_crates_io fusen-contract
```

第三层：

```shell
assert_release_candidate
cargo +1.97.0 publish --locked --registry crates-io --dry-run -p fusen-register
cargo +1.97.0 publish --locked --registry crates-io -p fusen-register
wait_for_crates_io fusen-register
```

第四层：

```shell
assert_release_candidate
cargo +1.97.0 publish --locked --registry crates-io --dry-run -p fusen-nacos
cargo +1.97.0 publish --locked --registry crates-io --dry-run -p fusen-rs

cargo +1.97.0 publish --locked --registry crates-io -p fusen-nacos
cargo +1.97.0 publish --locked --registry crates-io -p fusen-rs

wait_for_crates_io fusen-nacos
wait_for_crates_io fusen-rs
```

## Registry-only Consumer

七个 crate 可见后，在 repository 外创建没有 path dependency、没有 `[patch.crates-io]` 的 consumer。依赖必须精确固定为 crates.io 的 `fusen-rs = "=0.9.0"`；成功生成 lockfile、下载依赖并展开宏后，才允许创建 tag：

```shell
registry_consumer_dir="$(mktemp -d "${TMPDIR:-/tmp}/fusen-registry-consumer.XXXXXX")"
cargo +1.97.0 init --lib --edition 2024 \
  --name fusen_registry_consumer "$registry_consumer_dir"
cargo +1.97.0 add --manifest-path "$registry_consumer_dir/Cargo.toml" \
  --registry crates-io 'fusen-rs@=0.9.0'

cat >"$registry_consumer_dir/src/lib.rs" <<'RUST'
use fusen_rs::{RpcError, RpcResponse, interface};

#[interface(name = "registry-consumer")]
pub trait RegistryConsumerApi {
    #[fusen_rs::method(method = "GET", path = "/ping")]
    async fn ping(&self) -> Result<RpcResponse<String>, RpcError>;
}
RUST

cargo +1.97.0 generate-lockfile \
  --manifest-path "$registry_consumer_dir/Cargo.toml"
cargo +1.97.0 check --locked \
  --manifest-path "$registry_consumer_dir/Cargo.toml"
```

审查生成的 consumer manifest 和 lockfile，确认没有本地 path、patch 或非 `0.9.0` 的 fusen crate。将命令结果记录到外部 release evidence，不回填仓库。

## Failure And Recovery

crates.io version 不可覆盖或删除。上传命令因网络中断、超时或 registry 传播延迟失败时，先用 `cargo info --registry crates-io <crate>@0.9.0` 判断该版本是否已经成功上传；如果候选源码没有变化，只重试尚未发布的命令，并继续等待传播，不重复发布已经存在的版本。

第一个 crate 上传后，只要后续步骤需要修改代码、测试、manifest、lockfile、workflow 或发布文档，立即终止整个 `0.9.0` 发布。不得从新 SHA 继续发布剩余 `0.9.0` crate，也不得用本地 patch 掩盖 registry 状态。

Yank 只用于已经发布且存在严重正确性或安全缺陷的版本，不用于普通网络失败、传播延迟或文档瑕疵。Yank 不删除 archive，也不能让同一版本重新上传；逐个记录受影响 crate、原因和执行结果：

```shell
package_to_yank="fusen-contract"
cargo +1.97.0 yank --registry crates-io \
  --version 0.9.0 "$package_to_yank"
```

一旦因源码改动放弃 `0.9.0`，将全部七个发布 crate 的 package version、所有 workspace/path dependency version、renamed-runtime/fuzz-support/fuzz metadata 和相关文档整体提升到 `0.9.1`，重新生成三套 lockfile 与 package archive。即使某些 crate 的 `0.9.0` 从未上传，也不能让 `0.9.1` 与残留的本地 `0.9.0` 混发。随后从 M0.11 重新冻结候选并重跑完整 CI、Nightly、Nacos、security、package consumer 和固定机 benchmark 证据。

## Tag And GitHub Release

只有七个 crate 都已从 crates.io 解析、下载，并且 registry-only consumer 通过后，才能创建 annotated tag。tag 必须显式指向记录的候选 SHA，而不是隐式使用当时的 `HEAD`：

```shell
assert_release_candidate
if git rev-parse --verify --quiet refs/tags/v0.9.0 >/dev/null; then
  echo "local v0.9.0 tag already exists" >&2
  exit 1
fi
if ! remote_tag="$(git ls-remote --tags origin refs/tags/v0.9.0)"; then
  echo "could not check the remote v0.9.0 tag" >&2
  exit 1
fi
test -z "$remote_tag" || {
  echo "remote v0.9.0 tag already exists" >&2
  exit 1
}
git tag --annotate v0.9.0 "$RELEASE_CANDIDATE_SHA" \
  --message "fusen-rs v0.9.0"
test "$(git rev-parse 'v0.9.0^{commit}')" = "$RELEASE_CANDIDATE_SHA"
git push origin refs/tags/v0.9.0
remote_commit="$(
  git ls-remote --tags origin 'refs/tags/v0.9.0^{}' | awk '{print $1}'
)"
test "$remote_commit" = "$RELEASE_CANDIDATE_SHA"
```

推送 tag 后再次确认远端 `v0.9.0` 指向候选 SHA。发布已有的 GitHub Release draft，或使用包含 M0.11 证据表的外部 notes file 创建 Release；命令路径要求预先安装并认证 GitHub CLI，`--verify-tag` 防止 GitHub 代建其他 tag：

```shell
command -v gh >/dev/null
gh auth status --hostname github.com
test -n "${RELEASE_NOTES_FILE:-}" || {
  echo "RELEASE_NOTES_FILE is not set" >&2
  exit 1
}
gh release create v0.9.0 \
  --repo kwsc98/fusen-rs \
  --verify-tag \
  --title "fusen-rs v0.9.0" \
  --notes-file "$RELEASE_NOTES_FILE"
```

GitHub Release 的 target、annotated `v0.9.0` tag 和所有候选证据必须指向同一个 `RELEASE_CANDIDATE_SHA`。如果已经建立 draft，不要再执行第二次 `gh release create`；核对 draft target 和 notes 后通过 GitHub UI 发布它。
