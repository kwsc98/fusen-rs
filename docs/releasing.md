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
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s .github/scripts -p 'test_*.py' -v
cargo +1.97.0 clippy --locked --offline --manifest-path fuzz-support/Cargo.toml --all-targets -- -D warnings
```

CI 还必须通过 Linux/macOS/Windows、feature matrix、renamed-runtime macro consumer、`nacos-live-container`、HTTP/HTTPS client socket tests、lifecycle 重复测试、Markdown links 与从 `.crate` archive 构建的 package consumer。三种 OS 的 workspace test 都将完整输出写入 `target/ci-logs`；失败时上传 `workspace-tests-<os>-<arch>-<sha>-<attempt>`，不能只保留 Windows 诊断。第三方 Actions 固定到完整 commit SHA，Nacos `v2.4.3` 固定到 OCI index digest；升级只能通过单独审查的候选提交，不能在已经冻结的 A 或 B 上移动。`check-package-consumer.sh` 要求干净工作树，在不继承用户 Cargo 配置或 credentials 的空 `CARGO_HOME` 中通过 crates.io sparse index 先联网 fetch，随后离线完成 archive 与 default/optional-feature consumer 检查。`repeat-lifecycle-tests.sh` 和 `run-live-nacos-tests.sh` 都先列举预期测试并在匹配数为零时失败，Cargo 的空过滤器不得被当成成功。
定时流水线还必须通过三个真实私有源码 harness 的 `cargo-fuzz` 任务，以及 `runtime_e2e` 和 `wire_v1_contract` 的 100 轮真实 socket 重复测试。fuzz corpus 与运行方法见 [`fuzz/README.md`](../fuzz/README.md)。

## Contract Audit

发布负责人必须确认：

- `check-public-api-denylist.sh` 从干净 rustdoc 输出确认旧入口为零，只有 `interface`/`method` 宏和约定的 `Interceptor`、`Registry`、`ConfigSource`、`InstanceRouter`、`LoadBalancer`、`RetryPolicy`、`MetricsRecorder`、`Sanitizer` 与 client binding codec 扩展 SPI；
- `check-security.sh` 对 root、fuzz-support 和 fuzz 的独立 manifest/lockfile 分别运行 `cargo deny` 的四类检查与 `cargo audit`，前项失败后仍执行并汇总其余结果；三者的解析后 dependency graph 只包含批准的 Rustls Ring/bundled WebPKI client TLS 栈，不含 `hyper-tls`、`native-tls`、OpenSSL TLS backend、AWS-LC、native/system root loader、platform verifier 或 PEM loader；
- Core 不依赖 Nacos、subscriber 或 OTel backend；
- `ConfigSource`、公开 key/handle/error 和 `fusen-config::provider` 安全构造器可由第三方实现，并由 archive consumer 实际编译；provider channel、worker 与 SDK 类型仍保持私有；
- 生成代码只使用 doc-hidden、版本化的 `fusen_rs::__macro::v1` ABI；renamed-runtime consumer 验证 crate rename，且 Cargo 允许组合的任意 0.9.x macro/runtime 必须保持编译兼容；
- `http-json-v1` golden fixtures、capability/discovery filtering、真实明文 H1/h2c sockets、HTTPS H1/ALPN h2 sockets、证书/hostname 拒绝、Problem Details 和 macro trybuild 全部通过；
- HTTPS 测试确认 Rustls Ring、TLS 1.2/1.3、bundled Mozilla WebPKI roots、无明文 fallback；Server listener 仍不加载证书或私钥；
- 永久 pending request/registry/config cleanup 在 deadline 内有界返回；
- lifecycle、retry、breaker 与 byte-budget tests 不依赖 correctness sleep 或预占端口；
- 在绑定参考机器上执行 `Release Benchmark Gate`，`http-json-v1` 的 HTTP/1.1 与 h2c `c1/c100 × small/64 KiB` 共 8 个 case 的 p50/p99 相对 committed baseline 回退均不超过 10%；QPS、原始五轮日志与 JSON summary 已归档。

真实 Nacos container tests 是 required release gate。CI 固定启动 Nacos `v2.4.3` standalone service container，测试同时覆盖 registration/discovery 和 config publish/listen，并在主流程失败后继续关闭 handle、注销 instance 和删除 config。资源名由 GitHub run id、进程 id 和时间戳组成，不复用共享名称。

在已经运行并 ready 的 Nacos 上可复现同一个非空 gate：

```shell
NACOS_ADDR=127.0.0.1:8848 \
NACOS_TEST_RUN_ID=manual-$(date +%s) \
bash .github/scripts/run-live-nacos-tests.sh
```

脚本在执行前要求至少发现一个 `live_nacos_` ignored test；零测试、provider error、cleanup error 或 deadline 均使 gate 失败。CI job 是容器启动和 finally cleanup 的规范入口，本地命令不会代替调用方关闭自己启动的 Nacos 容器。

## Performance Gate

普通托管 CI 会运行一轮缩短参数但 case 完整的 benchmark smoke，并从按 `run_id-run_attempt` 隔离的目录上传 artifact，用于确认 `http-json-v1` 的 h2c/H1、并发/大 payload 和 schema 可执行；不同托管机器之间的绝对延迟不用于 10% 判定。

正式比较由 [release-benchmark.yml](../.github/workflows/release-benchmark.yml) 在带有 `fusen-benchmark-0-9-reference` label 的固定 self-hosted runner 上执行。该 runner 必须是建立 [committed baseline](../.github/benchmarks/fusen-0.9-reference-macos-arm64.json) 的同一参考机器：

```shell
python3 .github/scripts/run-benchmark-gate.py \
  --host-id fusen-0.9-reference-macos-arm64 \
  --runs 5 \
  --baseline .github/benchmarks/fusen-0.9-reference-macos-arm64.json \
  --output-dir target/release-benchmark-gate/manual-compare-001
```

每次运行必须使用全新或空的 output directory；workflow 使用 `run_id-run_attempt` 唯一路径，防止失败 artifact 混入旧证据。比较器拒绝脏工作树，以及 host id、CPU、OS、toolchain、rustc、schema、suite、参数、五轮原始样本不匹配。Compare 还要求 B 是 baseline `source_commit` 所指 A 的唯一直接子提交、baseline 在两端都已提交，并且 `A..B` tree diff 恰好只包含该 baseline JSON；HTTP/1.1 与 h2c 的 8 个 case 中，p50 或 p99 任一中位数回退严格超过 10% 即非零退出。QPS 只记录。发布负责人必须保留 workflow artifact，并确认 checkout SHA 是待发布 commit。

首个 schema v3、suite `http-json-transport-matrix-v1` baseline 必须两阶段生成。先把代码、测试、工具、workflow、实际发布日期和发布文档提交为干净候选 A，再创建固定且不可移动的 calibration ref：

```shell
set -euo pipefail
export BENCHMARK_BASELINE=".github/benchmarks/fusen-0.9-reference-macos-arm64.json"
export CALIBRATION_REF="release/v0.9.0-calibration"
export CALIBRATION_SHA="$(git rev-parse HEAD)"
[[ "$CALIBRATION_SHA" =~ ^[0-9a-f]{40}$ ]]
test -z "$(git status --porcelain=v1)"
rg -n '"status"[[:space:]]*:[[:space:]]*"calibration-required"' \
  "$BENCHMARK_BASELINE"
test "$(git remote get-url origin)" = \
  "https://github.com/kwsc98/fusen-rs.git" || {
  echo "origin must be the canonical fusen-rs repository" >&2
  exit 1
}

remote_calibration="$(
  git ls-remote --heads origin "refs/heads/$CALIBRATION_REF" |
    awk '{print $1}'
)"
test -z "$remote_calibration" || test "$remote_calibration" = "$CALIBRATION_SHA"
if test -z "$remote_calibration"; then
  git push origin "$CALIBRATION_SHA:refs/heads/$CALIBRATION_REF"
fi
test "$(
  git ls-remote --heads origin "refs/heads/$CALIBRATION_REF" |
    awk '{print $1}'
)" = "$CALIBRATION_SHA"

command -v gh >/dev/null
gh auth status --hostname github.com
test "$(
  gh repo view --repo kwsc98/fusen-rs --json nameWithOwner --jq .nameWithOwner
)" = "kwsc98/fusen-rs"
gh workflow run release-benchmark.yml \
  --repo kwsc98/fusen-rs \
  --ref "$CALIBRATION_REF" \
  --field mode=calibrate
```

等待 calibration workflow 成功，确认 checkout SHA 恰为 A，并审查 artifact 中的五轮日志、机器指纹、summary 和 baseline。将下载的 baseline 路径放入 `CALIBRATED_BASELINE`，然后只提交该 JSON，形成候选 B：

```shell
test -n "${CALIBRATED_BASELINE:-}"
test -f "$CALIBRATED_BASELINE"
cp "$CALIBRATED_BASELINE" "$BENCHMARK_BASELINE"
test "$(
  python3 -c 'import json,sys; print(json.load(open(sys.argv[1], encoding="utf-8"))["source_commit"])' \
    "$BENCHMARK_BASELINE"
)" = "$CALIBRATION_SHA"
git add "$BENCHMARK_BASELINE"
test "$(git diff --cached --name-only --no-renames)" = "$BENCHMARK_BASELINE"
test -z "$(git diff --name-only)"
test -z "$(git ls-files --others --exclude-standard)"
git commit -m "perf: calibrate v0.9 benchmark baseline"

export RELEASE_CANDIDATE_SHA="$(git rev-parse HEAD)"
test "$(git rev-list --count "$CALIBRATION_SHA..$RELEASE_CANDIDATE_SHA")" = 1
test "$(
  git diff --name-only --no-renames "$CALIBRATION_SHA..$RELEASE_CANDIDATE_SHA"
)" = "$BENCHMARK_BASELINE"
```

生成文件的完整 `source_commit` 必须等于 A；A 到 B 之间不能夹带任何其他变更。Committed baseline 仍为 `calibration-required` 时 compare 必须失败，不能发布。完整字段定义见[性能基线](performance-baseline.md)。

## Final Candidate Freeze

最终候选 B 必须已经包含实际发布日期、committed benchmark baseline 和全部发布文档。除 baseline JSON 外的所有内容都必须先进入 A；B 只能是 A 的唯一直接子提交，且只修改该 baseline。`0.9.0` 的日期固定为 `2026-08-02`，本次变更全部归入该 section，`[Unreleased]` 在冻结时必须为空。完成最后一次代码审查后才记录 B 的 SHA；发布日期延后或任何后续修改都不能在 B 上补写。

```shell
set -euo pipefail
: "${CALIBRATION_SHA:?restore the recorded calibration SHA}"
: "${CALIBRATION_REF:=release/v0.9.0-calibration}"
: "${BENCHMARK_BASELINE:=.github/benchmarks/fusen-0.9-reference-macos-arm64.json}"
test "$CALIBRATION_REF" = "release/v0.9.0-calibration"
test "$(git remote get-url origin)" = \
  "https://github.com/kwsc98/fusen-rs.git" || {
  echo "origin must be the canonical fusen-rs repository" >&2
  exit 1
}
test "$(
  git ls-remote --heads origin "refs/heads/$CALIBRATION_REF" |
    awk '{print $1}'
)" = "$CALIBRATION_SHA"

export RELEASE_CANDIDATE_SHA="$(git rev-parse HEAD)"
[[ "$RELEASE_CANDIDATE_SHA" =~ ^[0-9a-f]{40}$ ]]
test -z "$(git status --porcelain=v1)"
test "$(git rev-list --count "$CALIBRATION_SHA..$RELEASE_CANDIDATE_SHA")" = 1
test "$(
  git diff --name-only --no-renames "$CALIBRATION_SHA..$RELEASE_CANDIDATE_SHA"
)" = "$BENCHMARK_BASELINE"
test "$(
  python3 -c 'import json,sys; print(json.load(open(sys.argv[1], encoding="utf-8"))["source_commit"])' \
    "$BENCHMARK_BASELINE"
)" = "$CALIBRATION_SHA"
release_heading_count="$(
  rg -c '^## \[0\.9\.0\] - 2026-08-02$' CHANGELOG.md || true
)"
test "$release_heading_count" = 1 || {
  echo "CHANGELOG must contain exactly one dated 0.9.0 release heading" >&2
  exit 1
}
if rg -n 'YYYY-MM-DD' CHANGELOG.md; then
  echo "CHANGELOG still contains a release-date placeholder" >&2
  exit 1
fi
unreleased_content="$(
  awk '
    /^## \[Unreleased\]$/ { inside = 1; next }
    inside && /^## / { exit }
    inside && NF { print }
  ' CHANGELOG.md
)"
test -z "$unreleased_content" || {
  echo "CHANGELOG Unreleased section must be empty for the frozen candidate" >&2
  exit 1
}
if rg -n '"status"[[:space:]]*:[[:space:]]*"calibration-required"' \
  "$BENCHMARK_BASELINE"; then
  echo "committed benchmark baseline still requires calibration" >&2
  exit 1
fi

export RELEASE_CANDIDATE_REF="release/v0.9.0-rc"
remote_candidate="$(
  git ls-remote --heads origin "refs/heads/$RELEASE_CANDIDATE_REF" |
    awk '{print $1}'
)"
test -z "$remote_candidate" || test "$remote_candidate" = "$RELEASE_CANDIDATE_SHA"
if test -z "$remote_candidate"; then
  git push origin \
    "$RELEASE_CANDIDATE_SHA:refs/heads/$RELEASE_CANDIDATE_REF"
fi
test "$(
  git ls-remote --heads origin "refs/heads/$RELEASE_CANDIDATE_REF" |
    awk '{print $1}'
)" = "$RELEASE_CANDIDATE_SHA"
```

立即把 A/B 的完整 SHA 和两个固定 ref 写入候选外部记录。以后开始 M0.12 发布会话时，必须从该记录重新设置 `CALIBRATION_SHA`、`CALIBRATION_REF`、`RELEASE_CANDIDATE_SHA` 与 `RELEASE_CANDIDATE_REF`，不能重新用当时的 `HEAD` 或 `main` 推导候选，也不能 force-move 已经产生证据的固定 ref。

使用该 ref 显式触发三条 required workflow。先等待完整 CI 全绿，再运行 Nightly 与固定机 compare，避免已知失败时占用稀缺 runner：

```shell
command -v gh >/dev/null
gh auth status --hostname github.com
test "$(
  gh repo view --repo kwsc98/fusen-rs --json nameWithOwner --jq .nameWithOwner
)" = "kwsc98/fusen-rs"
gh workflow run ci.yml \
  --repo kwsc98/fusen-rs \
  --ref "$RELEASE_CANDIDATE_REF"
# CI 全绿并核对 head SHA 后：
gh workflow run nightly.yml \
  --repo kwsc98/fusen-rs \
  --ref "$RELEASE_CANDIDATE_REF"
gh workflow run release-benchmark.yml \
  --repo kwsc98/fusen-rs \
  --ref "$RELEASE_CANDIDATE_REF" \
  --field mode=compare
```

在同一 `RELEASE_CANDIDATE_SHA` 上依次完成完整 CI、Nightly、真实 Nacos、三 workspace security、package archive consumer 和固定机 benchmark compare。每个 run 必须核对 checkout SHA，不允许本地结果替代 Windows、Nightly、Nacos 或固定机器证据。将下表填入 GitHub Release draft 或 workflow summary；不要为了回填 run URL 再提交仓库：

| Evidence | Run URL | Checkout SHA | Artifact / result |
| --- | --- | --- | --- |
| CI：MSRV、Linux、macOS、Windows、release contracts |  |  | `benchmark-smoke-<os>-<arch>-<sha>-<attempt>` 及 required jobs 全绿；失败重跑前已保留跨 OS workspace 日志 |
| CI：三 workspace security |  |  | `security` job 成功，root/fuzz-support/fuzz 的 deny 与 audit 均执行 |
| CI：真实 Nacos |  |  | `nacos-live-container` job 成功 |
| CI：七个 package archive consumer |  |  | `release-contracts` 中 package consumer step 成功 |
| Nightly：三个 fuzz target、100 轮 core E2E |  |  | 三个 target 的日志/artifact 名称与 `core-e2e-repeat-100` 结果 |
| Release Benchmark Gate |  |  | `release-benchmark-compare-<sha>-<attempt>`、baseline SHA、HTTP/1.1 与 h2c 共 8 个 case 的 p50/p99 summary |

候选 B 之后禁止再修改代码、测试、Cargo manifest/lockfile、workflow、CHANGELOG 或发布文档；只修改注释、日期或证据链接也会破坏 `A..B` 只能包含 baseline JSON 的 provenance。出现任何这类变更时立即停止当前发布，不移动两个固定 evidence ref，也不把新提交补到 B 上；先重新规划新的 calibration/release evidence chain。Release draft、workflow summary 和已有 artifact 是候选外部记录，不提交“证据更新”来改变候选 SHA。

## crates.io Preflight

以下步骤只在 M0.11 的同一 SHA 全绿后执行，并且必须在同一个 Bash 发布会话中完成。token 只从终端 stdin 读取，先经 curl stdin config 验证，再写入权限为 `0600` 的隔离 Cargo credential 文件；禁止把 token 放入环境、仓库、命令参数、日志或 shell trace，也不要执行 `cargo login <token>`。token-free 的 package、dry-run 和 consumer 构建使用另一份 Cargo home，只有真正的上传命令能看到 credential 配置：

```shell
set -euo pipefail
set +x
export CRATES_IO_USER_AGENT="fusen-rs-release/0.9 (+https://github.com/kwsc98/fusen-rs)"
test "$(git remote get-url origin)" = \
  "https://github.com/kwsc98/fusen-rs.git" || {
  echo "origin must be the canonical fusen-rs repository" >&2
  exit 1
}
test -n "${CRATES_IO_OWNER:-}" || {
  echo "CRATES_IO_OWNER is not set" >&2
  exit 1
}
test -t 0 || {
  echo "crates.io token must be read from an interactive terminal" >&2
  exit 1
}
printf 'crates.io token: ' >&2
IFS= read -r -s crates_io_token
printf '\n' >&2
test -n "$crates_io_token" || {
  echo "crates.io token is empty" >&2
  exit 1
}

auth_response="$(
  printf 'header = "Authorization: %s"\n' "$crates_io_token" |
    curl --disable --fail --silent --show-error \
      --user-agent "$CRATES_IO_USER_AGENT" \
      --config - https://crates.io/api/v1/me
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

validation_cargo_home="$(
  mktemp -d "${TMPDIR:-/tmp}/fusen-release-validation-cargo.XXXXXX"
)"
release_cargo_home="$(
  mktemp -d "${TMPDIR:-/tmp}/fusen-release-upload-cargo.XXXXXX"
)"
chmod 700 "$validation_cargo_home" "$release_cargo_home"
cleanup_release_cargo_homes() {
  rm -rf -- "$validation_cargo_home" "$release_cargo_home"
}
trap cleanup_release_cargo_homes EXIT HUP INT TERM
umask 077
printf '%s\n' "$crates_io_token" |
  CARGO_HOME="$release_cargo_home" \
    cargo +1.97.0 login --registry crates-io
chmod 600 "$release_cargo_home/credentials.toml"
unset crates_io_token CARGO_REGISTRY_TOKEN CARGO_REGISTRIES_CRATES_IO_TOKEN
echo "crates.io identity passed preflight; the token is stored only in the isolated upload Cargo home"
```

在 crates.io token 设置页确认该 token 允许发布新 crate，并允许更新下列全部已有 crate。然后通过公共 API 检查名称和 `0.9.0` 是否已存在；HTTP error 或无法联网必须阻断，不能被解释为名称可用：

```shell
: "${CRATES_IO_USER_AGENT:=fusen-rs-release/0.9 (+https://github.com/kwsc98/fusen-rs)}"
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
  name_status="$(curl --disable --silent --show-error --output /dev/null \
    --user-agent "$CRATES_IO_USER_AGENT" \
    --write-out '%{http_code}' "https://crates.io/api/v1/crates/$package")"
  case "$name_status" in
    200)
      owners_response="$(curl --disable --fail --silent --show-error \
        --user-agent "$CRATES_IO_USER_AGENT" \
        "https://crates.io/api/v1/crates/$package/owners")"
      if ! printf '%s' "$owners_response" |
        python3 -c '
import json
import sys

payload = json.load(sys.stdin)
expected = sys.argv[1]
owners = {
    owner.get("login")
    for group in ("users", "teams")
    for owner in payload.get(group, [])
}
raise SystemExit(0 if expected in owners else 1)
' "$CRATES_IO_OWNER"; then
        echo "$CRATES_IO_OWNER is not an owner of $package" >&2
        exit 1
      fi
      unset owners_response
      ;;
    404)
      echo "$package is not published yet; re-check token permission to publish a new crate"
      ;;
    *)
      echo "unexpected crates.io status for $package: $name_status" >&2
      exit 1
      ;;
  esac

  version_status="$(curl --disable --silent --show-error --output /dev/null \
    --user-agent "$CRATES_IO_USER_AGENT" \
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
set -euo pipefail
test "$(git rev-parse HEAD)" = "$RELEASE_CANDIDATE_SHA"
test -z "$(git status --porcelain=v1)"
test "$(
  git ls-remote --heads origin "refs/heads/$RELEASE_CANDIDATE_REF" |
    awk '{print $1}'
)" = "$RELEASE_CANDIDATE_SHA"
test "$(
  git ls-remote --heads origin "refs/heads/$CALIBRATION_REF" |
    awk '{print $1}'
)" = "$CALIBRATION_SHA"
test "$(
  git diff --name-only --no-renames "$CALIBRATION_SHA..$RELEASE_CANDIDATE_SHA"
)" = "$BENCHMARK_BASELINE"
test "$(
  python3 -c 'import json,sys; print(json.load(open(sys.argv[1], encoding="utf-8"))["source_commit"])' \
    "$BENCHMARK_BASELINE"
)" = "$CALIBRATION_SHA"
bash .github/scripts/check-package-consumer.sh
```

`check-package-consumer.sh` 首先拒绝脏工作树，主动清除继承的 `CARGO_*` 配置和 credential 环境，再在空 `CARGO_HOME` 中通过 crates.io sparse index 执行一次在线 `cargo fetch --locked`；随后所有 package、lockfile 和 consumer 命令都使用 `--offline`。它对七个发布 crate 执行既不带 `--allow-dirty` 也不带 `--no-verify` 的 `cargo package`，解包每个 `.crate`，并在七个独立外部 workspace 中实际引用该 crate 拥有的公共 API/SPI；`fusen-contract/derive`、`fusen-config/yaml`、`fusen-observability/otel` 与 `fusen-nacos/yaml` 还必须从 archive 以 optional feature 编译，Nacos adapters 会被消费为 `Registry`/`ConfigSource` trait object，`fusen-rs` consumer 会展开服务宏并以 `Arc<dyn ...>` 注册全部 owned SPI。脚本的 path patches 只用于验证候选 archive，不是 crates.io 传播证据。实际发布命令同样禁止 `--allow-dirty` 和 `--no-verify`。

## Packaging And Publication Order

按以下四层发布。每层开始前重新核对候选，先对该层全部 crate dry-run，再执行正式 publish；上一层必须全部能从 crates.io 精确解析并下载后，才能 dry-run 下一层。

等待函数使用 `cargo info --registry crates-io <crate>@0.9.0` 同时检查 registry 解析和 archive 下载。超时或返回其他版本均阻断发布：

```shell
set -euo pipefail

assert_release_candidate() {
  local required actual remote_output
  for required in \
    CALIBRATION_SHA CALIBRATION_REF RELEASE_CANDIDATE_SHA \
    RELEASE_CANDIDATE_REF BENCHMARK_BASELINE; do
    if test -z "${!required:-}"; then
      echo "$required is not set" >&2
      return 1
    fi
  done
  actual="$(git remote get-url origin)" || return 1
  test "$actual" = "https://github.com/kwsc98/fusen-rs.git" || {
    echo "origin is not the canonical fusen-rs repository" >&2
    return 1
  }
  if [[ ! "$CALIBRATION_SHA" =~ ^[0-9a-f]{40}$ ]] ||
    [[ ! "$RELEASE_CANDIDATE_SHA" =~ ^[0-9a-f]{40}$ ]]; then
    echo "candidate SHAs must be full lowercase commit IDs" >&2
    return 1
  fi
  if test "$CALIBRATION_REF" != "release/v0.9.0-calibration" ||
    test "$RELEASE_CANDIDATE_REF" != "release/v0.9.0-rc" ||
    test "$BENCHMARK_BASELINE" != \
      ".github/benchmarks/fusen-0.9-reference-macos-arm64.json"; then
    echo "release refs or benchmark baseline path differ from the frozen values" >&2
    return 1
  fi

  actual="$(git rev-parse HEAD)" || return 1
  test "$actual" = "$RELEASE_CANDIDATE_SHA" || {
    echo "HEAD is not the recorded release candidate" >&2
    return 1
  }
  actual="$(git status --porcelain=v1)" || return 1
  test -z "$actual" || {
    echo "release candidate worktree is dirty" >&2
    return 1
  }
  remote_output="$(
    git ls-remote --heads origin "refs/heads/$RELEASE_CANDIDATE_REF"
  )" || return 1
  actual="${remote_output%%[[:space:]]*}"
  test "$actual" = "$RELEASE_CANDIDATE_SHA" || {
    echo "remote release candidate ref does not match the recorded SHA" >&2
    return 1
  }
  remote_output="$(
    git ls-remote --heads origin "refs/heads/$CALIBRATION_REF"
  )" || return 1
  actual="${remote_output%%[[:space:]]*}"
  test "$actual" = "$CALIBRATION_SHA" || {
    echo "remote calibration ref does not match the recorded SHA" >&2
    return 1
  }
  actual="$(
    git rev-list --count "$CALIBRATION_SHA..$RELEASE_CANDIDATE_SHA"
  )" || return 1
  test "$actual" = 1 || {
    echo "release candidate must be the calibration commit's only child" >&2
    return 1
  }
  actual="$(
    git diff --name-only --no-renames \
      "$CALIBRATION_SHA..$RELEASE_CANDIDATE_SHA"
  )" || return 1
  test "$actual" = "$BENCHMARK_BASELINE" || {
    echo "A..B contains changes other than the benchmark baseline" >&2
    return 1
  }
  actual="$(
    python3 -c \
      'import json,sys; print(json.load(open(sys.argv[1], encoding="utf-8"))["source_commit"])' \
      "$BENCHMARK_BASELINE"
  )" || return 1
  test "$actual" = "$CALIBRATION_SHA" || {
    echo "benchmark baseline does not identify the calibration commit" >&2
    return 1
  }
}

wait_for_crates_io() {
  local package="${1:?crate name is required}"
  CARGO_HOME="$validation_cargo_home" python3 - "$package" <<'PY'
import subprocess
import sys
import time

package = sys.argv[1]
deadline = time.monotonic() + 600.0
attempt = 0

while True:
    remaining = deadline - time.monotonic()
    if remaining <= 0:
        break

    attempt += 1
    attempt_timeout = min(30.0, remaining)
    try:
        completed = subprocess.run(
            [
                "cargo",
                "+1.97.0",
                "info",
                "--registry",
                "crates-io",
                f"{package}@0.9.0",
            ],
            check=False,
            timeout=attempt_timeout,
        )
        return_code = completed.returncode
    except subprocess.TimeoutExpired:
        return_code = 124
        print(
            f"cargo info for {package} exceeded its "
            f"{attempt_timeout:.0f}-second attempt timeout",
            file=sys.stderr,
        )

    if return_code == 0:
        raise SystemExit(0)

    remaining = deadline - time.monotonic()
    if remaining <= 0:
        break
    print(
        f"waiting for {package} 0.9.0 registry propagation "
        f"(attempt {attempt}; {int(remaining)} seconds remain)",
        file=sys.stderr,
    )
    time.sleep(min(10.0, remaining))

print(f"{package} 0.9.0 did not propagate within 10 minutes", file=sys.stderr)
raise SystemExit(1)
PY
}
```

第一层（不依赖其他 workspace 发布 crate）：

```shell
set -euo pipefail
assert_release_candidate || exit 1
CARGO_HOME="$validation_cargo_home" cargo +1.97.0 publish \
  --locked --registry crates-io --dry-run -p fusen-procedural-macro
CARGO_HOME="$validation_cargo_home" cargo +1.97.0 publish \
  --locked --registry crates-io --dry-run -p fusen-config
CARGO_HOME="$validation_cargo_home" cargo +1.97.0 publish \
  --locked --registry crates-io --dry-run -p fusen-observability

CARGO_HOME="$release_cargo_home" cargo +1.97.0 publish \
  --locked --registry crates-io -p fusen-procedural-macro
CARGO_HOME="$release_cargo_home" cargo +1.97.0 publish \
  --locked --registry crates-io -p fusen-config
CARGO_HOME="$release_cargo_home" cargo +1.97.0 publish \
  --locked --registry crates-io -p fusen-observability

wait_for_crates_io fusen-procedural-macro
wait_for_crates_io fusen-config
wait_for_crates_io fusen-observability
```

第二层（`fusen-contract` 的可选 `derive` feature 依赖第一层的过程宏）：

```shell
set -euo pipefail
assert_release_candidate || exit 1
CARGO_HOME="$validation_cargo_home" cargo +1.97.0 publish \
  --locked --registry crates-io --dry-run -p fusen-contract
CARGO_HOME="$release_cargo_home" cargo +1.97.0 publish \
  --locked --registry crates-io -p fusen-contract
wait_for_crates_io fusen-contract
```

第三层：

```shell
set -euo pipefail
assert_release_candidate || exit 1
CARGO_HOME="$validation_cargo_home" cargo +1.97.0 publish \
  --locked --registry crates-io --dry-run -p fusen-register
CARGO_HOME="$release_cargo_home" cargo +1.97.0 publish \
  --locked --registry crates-io -p fusen-register
wait_for_crates_io fusen-register
```

第四层：

```shell
set -euo pipefail
assert_release_candidate || exit 1
CARGO_HOME="$validation_cargo_home" cargo +1.97.0 publish \
  --locked --registry crates-io --dry-run -p fusen-nacos
CARGO_HOME="$validation_cargo_home" cargo +1.97.0 publish \
  --locked --registry crates-io --dry-run -p fusen-rs

CARGO_HOME="$release_cargo_home" cargo +1.97.0 publish \
  --locked --registry crates-io -p fusen-nacos
CARGO_HOME="$release_cargo_home" cargo +1.97.0 publish \
  --locked --registry crates-io -p fusen-rs

wait_for_crates_io fusen-nacos
wait_for_crates_io fusen-rs
```

## Registry-only Consumer

七个 crate 可见后，在 repository 外创建没有 path dependency、没有 `[patch.crates-io]` 的 consumer。依赖必须精确固定为 crates.io 的 `fusen-rs = "=0.9.0"`；成功生成 lockfile、下载依赖并展开宏后，才允许创建 tag：

```shell
set -euo pipefail
unset CARGO_REGISTRY_TOKEN CARGO_REGISTRIES_CRATES_IO_TOKEN
rm -f -- "$release_cargo_home/credentials.toml"
test ! -e "$release_cargo_home/credentials.toml"
registry_consumer_dir="$(mktemp -d "${TMPDIR:-/tmp}/fusen-registry-consumer.XXXXXX")"
CARGO_HOME="$validation_cargo_home" cargo +1.97.0 init --lib --edition 2024 \
  --name fusen_registry_consumer "$registry_consumer_dir"
CARGO_HOME="$validation_cargo_home" cargo +1.97.0 add \
  --manifest-path "$registry_consumer_dir/Cargo.toml" \
  --registry crates-io 'fusen-rs@=0.9.0'

cat >"$registry_consumer_dir/src/lib.rs" <<'RUST'
use fusen_rs::{Error, Response, interface};

#[interface(name = "registry-consumer")]
pub trait RegistryConsumerApi {
    #[fusen_rs::method(method = "GET", path = "/ping")]
    async fn ping(&self) -> Result<Response<String>, Error>;
}
RUST

CARGO_HOME="$validation_cargo_home" cargo +1.97.0 generate-lockfile \
  --manifest-path "$registry_consumer_dir/Cargo.toml"
CARGO_HOME="$validation_cargo_home" cargo +1.97.0 check --locked \
  --manifest-path "$registry_consumer_dir/Cargo.toml"
```

审查生成的 consumer manifest 和 lockfile，确认没有本地 path、patch 或非 `0.9.0` 的 fusen crate。将命令结果记录到外部 release evidence，不回填仓库。

## Failure And Recovery

crates.io version 不可覆盖或删除。上传命令因网络中断、超时或 registry 传播延迟失败时，先用 `cargo info --registry crates-io <crate>@0.9.0` 判断该版本是否已经成功上传；如果候选源码没有变化，只重试尚未发布的命令，并继续等待传播，不重复发布已经存在的版本。

第一个 crate 上传后，只要后续步骤需要修改代码、测试、manifest、lockfile、workflow 或发布文档，立即终止整个 `0.9.0` 发布。不得从新 SHA 继续发布剩余 `0.9.0` crate，也不得用本地 patch 掩盖 registry 状态。

Yank 只用于已经发布且存在严重正确性或安全缺陷的版本，不用于普通网络失败、传播延迟或文档瑕疵。Yank 不删除 archive，也不能让同一版本重新上传；逐个记录受影响 crate、原因和执行结果：

```shell
(
  set -euo pipefail
  set +x
  unset CARGO_REGISTRY_TOKEN CARGO_REGISTRIES_CRATES_IO_TOKEN
  test -t 0 || {
    echo "crates.io token must be read from an interactive terminal" >&2
    exit 1
  }

  package_to_yank="fusen-contract"
  yank_upload_cargo_home="$(
    mktemp -d "${TMPDIR:-/tmp}/fusen-yank-upload-cargo.XXXXXX"
  )"
  chmod 700 "$yank_upload_cargo_home"
  cleanup_yank_upload_cargo_home() {
    rm -rf -- "$yank_upload_cargo_home"
  }
  trap cleanup_yank_upload_cargo_home EXIT
  trap 'exit 130' HUP INT TERM

  umask 077
  printf 'crates.io token for yank: ' >&2
  IFS= read -r -s crates_io_token
  printf '\n' >&2
  test -n "$crates_io_token" || {
    echo "crates.io token is empty" >&2
    exit 1
  }
  printf '%s\n' "$crates_io_token" |
    CARGO_HOME="$yank_upload_cargo_home" \
      cargo +1.97.0 login --registry crates-io
  chmod 600 "$yank_upload_cargo_home/credentials.toml"
  unset crates_io_token

  CARGO_HOME="$yank_upload_cargo_home" cargo +1.97.0 yank \
    --registry crates-io --version 0.9.0 "$package_to_yank"
)
```

一旦因源码改动放弃 `0.9.0`，将全部七个发布 crate 的 package version、所有 workspace/path dependency version、renamed-runtime/fuzz-support/fuzz metadata 和相关文档整体提升到 `0.9.1`，重新生成三套 lockfile 与 package archive。即使某些 crate 的 `0.9.0` 从未上传，也不能让 `0.9.1` 与残留的本地 `0.9.0` 混发。随后从 M0.11 重新冻结候选并重跑完整 CI、Nightly、Nacos、security、package consumer 和固定机 benchmark 证据。

## Tag And GitHub Release

只有七个 crate 都已从 crates.io 解析、下载，并且 registry-only consumer 通过后，才能创建 annotated tag。tag 必须显式指向记录的候选 SHA，而不是隐式使用当时的 `HEAD`：

```shell
set -euo pipefail
assert_release_candidate || exit 1
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
