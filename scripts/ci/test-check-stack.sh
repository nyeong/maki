#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
check_stack="${script_dir}/check-stack.sh"
check_stack_tree_sitter="${script_dir}/check-stack-tree-sitter.sh"
revision_a="1111111111111111111111111111111111111111"
revision_b="2222222222222222222222222222222222222222"
revision_c="3333333333333333333333333333333333333333"

runtime_args=(
  --maki-url https://example.invalid/owner/maki.git
  --maki-rev "$revision_a"
  --grammar-url https://example.invalid/owner/tree-sitter-maki.git
  --grammar-rev "$revision_b"
  --extension-url https://example.invalid/owner/maki-zed.git
  --extension-rev "$revision_c"
)
valid_args=(
  "${runtime_args[@]}"
  --validate-only
)

fail() {
  printf 'test-check-stack: %s\n' "$1" >&2
  exit 1
}

expect_failure() {
  local expected="$1"
  shift
  local output
  if output="$(bash "$check_stack" "$@" 2>&1)"; then
    fail "command unexpectedly succeeded"
  fi
  [[ "$output" == *"$expected"* ]] \
    || fail "failure did not contain expected diagnostic: ${expected}"
}

bash -n "$check_stack"
bash -n "$check_stack_tree_sitter"
bash "$check_stack" --help >/dev/null
bash "$check_stack" "${valid_args[@]}" >/dev/null

expect_failure \
  "Maki revision must be exactly 40 hexadecimal characters" \
  "${valid_args[@]/$revision_a/1234}"

marker="${TMPDIR:-/tmp}/maki-stack-argument-marker"
rm -f -- "$marker"
expect_failure \
  "Maki revision must be exactly 40 hexadecimal characters" \
  "${valid_args[@]/$revision_a/\$(touch ${marker})}"
[[ ! -e "$marker" ]] || fail "revision input was evaluated as shell code"

expect_failure \
  "tree-sitter-maki URL must use HTTPS" \
  "${valid_args[@]/https:\/\/example.invalid\/owner\/tree-sitter-maki.git/ssh:\/\/example.invalid\/owner\/tree-sitter-maki.git}"

secret="stack-ci-secret-sentinel"
output=
if output="$(bash "$check_stack" \
  "${valid_args[@]/https:\/\/example.invalid\/owner\/maki.git/https:\/\/${secret}@example.invalid\/owner\/maki.git}" \
  2>&1)"; then
  fail "credential-bearing URL unexpectedly succeeded"
fi
[[ "$output" == *"Maki URL must not contain embedded credentials"* ]] \
  || fail "credential-bearing URL did not produce the expected diagnostic"
[[ "$output" != *"$secret"* ]] \
  || fail "credential-bearing URL leaked its userinfo"

runtime_test_root="$(mktemp -d "${TMPDIR:-/tmp}/maki-stack-runtime.XXXXXXXX")"
trap 'rm -rf -- "$runtime_test_root"' EXIT
fake_bin_dir="${runtime_test_root}/bin"
leak_marker="${runtime_test_root}/ambient-credential-leaked"
mkdir -p "$fake_bin_dir"
# These variables intentionally expand when the generated fake Git runs.
# shellcheck disable=SC2016
{
  printf '#!%s\n' "$BASH"
  printf '%s\n' \
    'if [[ -n "${GIT_CONFIG_PARAMETERS-}" || -n "${GIT_SSL_NO_VERIFY-}" ]]; then' \
    '  : >"$CHECK_STACK_LEAK_MARKER"' \
    'fi' \
    'exit 97'
} >"${fake_bin_dir}/git"
chmod +x "${fake_bin_dir}/git"
if env \
  PATH="${fake_bin_dir}:${PATH}" \
  CHECK_STACK_LEAK_MARKER="$leak_marker" \
  GIT_CONFIG_PARAMETERS="'http.extraHeader=Authorization: sentinel'" \
  GIT_SSL_NO_VERIFY=1 \
  bash "$check_stack" "${runtime_args[@]}" >/dev/null 2>&1; then
  fail "fake Git checkout unexpectedly succeeded"
fi
[[ ! -e "$leak_marker" ]] \
  || fail "ambient Git credential or TLS override reached the public fetch"

shared_tree_sitter_home="${runtime_test_root}/shared-home"
shared_parser="${shared_tree_sitter_home}/.cache/tree-sitter/lib/maki.so"
mkdir -p -- "${shared_parser%/*}"
printf 'cancelled build\n' >"$shared_parser"
# These variables intentionally expand when the generated fake parser runs.
# shellcheck disable=SC2016
{
  printf '#!%s\n' "$BASH"
  printf '%s\n' \
    'set -euo pipefail' \
    '[[ "${1-}" == parse && "${2-}" == --quiet ]] || exit 89' \
    'cache_home="${XDG_CACHE_HOME:-${HOME}/.cache}"' \
    'library="${cache_home}/tree-sitter/lib/maki.so"' \
    'if [[ -e "$library" ]] && grep -qx "cancelled build" "$library"; then' \
    '  printf "dlopen failed\n" >&2' \
    '  exit 88' \
    'fi' \
    'mkdir -p -- "${library%/*}"' \
    'printf "valid parser\n" >"$library"'
} >"${fake_bin_dir}/tree-sitter"
chmod +x "${fake_bin_dir}/tree-sitter"

for attempt in initial retry; do
  run_tree_sitter_cache="${runtime_test_root}/${attempt}-cache"
  run_parser="${run_tree_sitter_cache}/tree-sitter/lib/maki.so"
  if ! env -u XDG_CACHE_HOME \
    PATH="${fake_bin_dir}:${PATH}" \
    HOME="$shared_tree_sitter_home" \
    bash "$check_stack_tree_sitter" \
      "$run_tree_sitter_cache" test/fixtures/stable.maki; then
    fail "isolated tree-sitter cache ${attempt} run failed"
  fi
  grep -qx 'valid parser' "$run_parser" \
    || fail "${attempt} run did not populate its local tree-sitter cache"
done
grep -qx 'cancelled build' "$shared_parser" \
  || fail "isolated parse changed the poisoned shared tree-sitter cache"
rm -rf -- "$runtime_test_root"
trap - EXIT

expect_failure \
  "maki-zed URL must not contain a query string or fragment" \
  "${valid_args[@]/https:\/\/example.invalid\/owner\/maki-zed.git/https:\/\/example.invalid\/owner\/maki-zed.git?token=sentinel}"

expect_failure "--extension-rev is required" \
  --maki-url https://example.invalid/owner/maki.git \
  --maki-rev "$revision_a" \
  --grammar-url https://example.invalid/owner/tree-sitter-maki.git \
  --grammar-rev "$revision_b" \
  --extension-url https://example.invalid/owner/maki-zed.git \
  --validate-only

printf 'check-stack argument tests passed.\n'
