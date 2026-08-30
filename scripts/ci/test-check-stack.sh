#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
check_stack="${script_dir}/check-stack.sh"
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

credential_test_root="$(mktemp -d "${TMPDIR:-/tmp}/maki-stack-credentials.XXXXXXXX")"
trap 'rm -rf -- "$credential_test_root"' EXIT
fake_git_dir="${credential_test_root}/bin"
leak_marker="${credential_test_root}/ambient-credential-leaked"
mkdir -p "$fake_git_dir"
# These variables intentionally expand when the generated fake Git runs.
# shellcheck disable=SC2016
printf '%s\n' \
  '#!/usr/bin/env bash' \
  'if [[ -n "${GIT_CONFIG_PARAMETERS-}" || -n "${GIT_SSL_NO_VERIFY-}" ]]; then' \
  '  : >"$CHECK_STACK_LEAK_MARKER"' \
  'fi' \
  'exit 97' >"${fake_git_dir}/git"
chmod +x "${fake_git_dir}/git"
if env \
  PATH="${fake_git_dir}:${PATH}" \
  CHECK_STACK_LEAK_MARKER="$leak_marker" \
  GIT_CONFIG_PARAMETERS="'http.extraHeader=Authorization: sentinel'" \
  GIT_SSL_NO_VERIFY=1 \
  bash "$check_stack" "${runtime_args[@]}" >/dev/null 2>&1; then
  fail "fake Git checkout unexpectedly succeeded"
fi
[[ ! -e "$leak_marker" ]] \
  || fail "ambient Git credential or TLS override reached the public fetch"
rm -rf -- "$credential_test_root"
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
