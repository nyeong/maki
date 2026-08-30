#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"

usage() {
  cat <<'EOF'
Usage: check-stack.sh [options]

Verify one immutable Maki, tree-sitter-maki, and maki-zed revision tuple.

Required options:
  --maki-url URL          Credential-free HTTPS Git URL for Maki
  --maki-rev SHA          Full 40-hex Maki commit
  --grammar-url URL       Credential-free HTTPS Git URL for tree-sitter-maki
  --grammar-rev SHA       Full 40-hex tree-sitter-maki commit
  --extension-url URL     Credential-free HTTPS Git URL for maki-zed
  --extension-rev SHA     Full 40-hex maki-zed commit

Other options:
  --validate-only         Validate the six inputs without fetching or executing code
  --keep-checkouts        Preserve the temporary checkout root for debugging
  -h, --help              Show this help

The selected revisions are trusted code: their Nix expressions and repository
verification scripts run on the current machine. URLs must not contain embedded
credentials, query strings, or fragments.
EOF
}

die() {
  printf 'check-stack: %s\n' "$1" >&2
  exit 1
}

require_value() {
  local option="$1"
  local value="${2-}"
  [[ -n "$value" ]] || die "${option} requires a value"
}

validate_revision() {
  local name="$1"
  local revision="$2"
  [[ "$revision" =~ ^[0-9a-fA-F]{40}$ ]] \
    || die "${name} revision must be exactly 40 hexadecimal characters"
}

validate_url() {
  local name="$1"
  local url="$2"
  local remainder authority path

  [[ "$url" == https://* ]] \
    || die "${name} URL must use HTTPS"
  [[ ! "$url" =~ [[:space:][:cntrl:]] ]] \
    || die "${name} URL must not contain whitespace or control characters"
  [[ "$url" != *'?'* && "$url" != *'#'* ]] \
    || die "${name} URL must not contain a query string or fragment"

  remainder="${url#https://}"
  [[ "$remainder" == */* ]] \
    || die "${name} URL must include a host and repository path"
  authority="${remainder%%/*}"
  path="${remainder#*/}"

  [[ -n "$authority" && -n "$path" ]] \
    || die "${name} URL must include a host and repository path"
  [[ "$authority" != *'@'* && "$authority" != *'%'* ]] \
    || die "${name} URL must not contain embedded credentials"
  [[ "$path" == *.git ]] \
    || die "${name} URL repository path must end in .git"
  case "/${path}/" in
    *'/../'*|*'/./'*|*'//'*)
      die "${name} URL must not contain relative or empty path segments"
      ;;
  esac
  case "$path" in
    *\\*) die "${name} URL must use forward-slash path separators" ;;
  esac
}

require_command() {
  command -v "$1" >/dev/null 2>&1 || die "required command not found: $1"
}

run_stage() {
  local label="$1"
  local started status
  shift

  started=$SECONDS
  printf 'check-stack: START %s\n' "$label"
  if "$@"; then
    printf 'check-stack: PASS  %s (%ss)\n' "$label" "$((SECONDS - started))"
  else
    status=$?
    printf 'check-stack: FAIL  %s (%ss, exit %s)\n' \
      "$label" "$((SECONDS - started))" "$status" >&2
    return "$status"
  fi
}

maki_url=
maki_rev=
grammar_url=
grammar_rev=
extension_url=
extension_rev=
validate_only=false
keep_checkouts=false

while (( $# > 0 )); do
  case "$1" in
    --maki-url)
      require_value "$1" "${2-}"
      [[ -z "$maki_url" ]] || die "--maki-url may only be provided once"
      maki_url="$2"
      shift 2
      ;;
    --maki-rev)
      require_value "$1" "${2-}"
      [[ -z "$maki_rev" ]] || die "--maki-rev may only be provided once"
      maki_rev="$2"
      shift 2
      ;;
    --grammar-url)
      require_value "$1" "${2-}"
      [[ -z "$grammar_url" ]] || die "--grammar-url may only be provided once"
      grammar_url="$2"
      shift 2
      ;;
    --grammar-rev)
      require_value "$1" "${2-}"
      [[ -z "$grammar_rev" ]] || die "--grammar-rev may only be provided once"
      grammar_rev="$2"
      shift 2
      ;;
    --extension-url)
      require_value "$1" "${2-}"
      [[ -z "$extension_url" ]] || die "--extension-url may only be provided once"
      extension_url="$2"
      shift 2
      ;;
    --extension-rev)
      require_value "$1" "${2-}"
      [[ -z "$extension_rev" ]] || die "--extension-rev may only be provided once"
      extension_rev="$2"
      shift 2
      ;;
    --validate-only)
      validate_only=true
      shift
      ;;
    --keep-checkouts)
      keep_checkouts=true
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      die "unknown argument; see --help"
      ;;
  esac
done

[[ -n "$maki_url" ]] || die "--maki-url is required"
[[ -n "$maki_rev" ]] || die "--maki-rev is required"
[[ -n "$grammar_url" ]] || die "--grammar-url is required"
[[ -n "$grammar_rev" ]] || die "--grammar-rev is required"
[[ -n "$extension_url" ]] || die "--extension-url is required"
[[ -n "$extension_rev" ]] || die "--extension-rev is required"

validate_url "Maki" "$maki_url"
validate_revision "Maki" "$maki_rev"
validate_url "tree-sitter-maki" "$grammar_url"
validate_revision "tree-sitter-maki" "$grammar_rev"
validate_url "maki-zed" "$extension_url"
validate_revision "maki-zed" "$extension_rev"

maki_rev="${maki_rev,,}"
grammar_rev="${grammar_rev,,}"
extension_rev="${extension_rev,,}"

if [[ "$validate_only" == true ]]; then
  printf 'Revision tuple inputs are valid.\n'
  exit 0
fi

for command in git grep nix python3; do
  require_command "$command"
done

umask 077
checkout_root="$(mktemp -d "${TMPDIR:-/tmp}/maki-stack.XXXXXXXX")"
git_home="${checkout_root}/git-home"
git_config_home="${checkout_root}/git-config"
mkdir -p "$git_home" "$git_config_home"

cleanup() {
  local status=$?
  trap - EXIT INT TERM
  if [[ "$keep_checkouts" == true ]]; then
    printf 'Preserved checkouts at %s\n' "$checkout_root" >&2
  elif [[ -d "$checkout_root" ]]; then
    rm -rf -- "$checkout_root"
  fi
  exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

public_git() {
  env \
    -u GIT_CONFIG_PARAMETERS \
    -u GIT_SSH \
    -u GIT_SSH_COMMAND \
    -u GIT_SSL_NO_VERIFY \
    -u GIT_TRACE \
    -u GIT_TRACE_CURL \
    -u GIT_TRACE_CURL_NO_DATA \
    -u GIT_TRACE_PACKET \
    -u GIT_TRACE2 \
    -u GIT_TRACE2_EVENT \
    -u GIT_TRACE2_PERF \
    HOME="$git_home" \
    XDG_CONFIG_HOME="$git_config_home" \
    GIT_CONFIG_NOSYSTEM=1 \
    GIT_CONFIG_GLOBAL=/dev/null \
    GIT_CONFIG_COUNT=0 \
    GIT_TERMINAL_PROMPT=0 \
    GIT_ASKPASS=false \
    SSH_ASKPASS=false \
    GCM_INTERACTIVE=Never \
    git -c credential.helper= "$@"
}

checkout_revision() {
  local name="$1"
  local url="$2"
  local revision="$3"
  local destination="$4"
  local actual

  printf 'Checking out %s at %s\n' "$name" "$revision"
  public_git -c init.defaultBranch=main init --quiet "$destination"
  public_git -C "$destination" remote add origin "$url"
  # Nix derives sourceInfo.revCount from local Git history. Fetch the selected
  # commit with its reachable history instead of creating a shallow checkout.
  if ! public_git -C "$destination" fetch --quiet --no-tags origin "$revision"; then
    die "failed to fetch ${name} revision ${revision}; ensure it is reachable from a public ref"
  fi
  public_git -C "$destination" cat-file -e "${revision}^{commit}" \
    || die "${name} object ${revision} is not a commit"
  public_git -C "$destination" checkout --quiet --detach "$revision"
  actual="$(public_git -C "$destination" rev-parse HEAD)"
  [[ "$actual" == "$revision" ]] \
    || die "${name} checkout mismatch: expected ${revision}, found ${actual}"
}

assert_file() {
  local name="$1"
  local path="$2"
  [[ -f "$path" ]] || die "${name} revision is missing ${path##*/}"
}

assert_clean_checkout() {
  local name="$1"
  local directory="$2"
  local status
  status="$(public_git -C "$directory" status --porcelain --untracked-files=all)"
  [[ -z "$status" ]] \
    || die "${name} verification mutated its checkout"
}

git_flake_url() {
  python3 -c \
    'from pathlib import Path; import sys; print("git+" + Path(sys.argv[1]).resolve().as_uri() + "?rev=" + sys.argv[2])' \
    "$1" "$2"
}

maki_dir="${checkout_root}/maki"
grammar_dir="${checkout_root}/tree-sitter-maki"
extension_dir="${checkout_root}/maki-zed"
overall_started=$SECONDS

run_stage "checkout Maki" \
  checkout_revision "Maki" "$maki_url" "$maki_rev" "$maki_dir"
run_stage "checkout tree-sitter-maki" \
  checkout_revision "tree-sitter-maki" "$grammar_url" "$grammar_rev" "$grammar_dir"
run_stage "checkout maki-zed" \
  checkout_revision "maki-zed" "$extension_url" "$extension_rev" "$extension_dir"

assert_file "Maki" "${maki_dir}/scripts/ci/check-maki.sh"
assert_file "tree-sitter-maki" "${grammar_dir}/scripts/verify.sh"
assert_file "tree-sitter-maki" "${grammar_dir}/test/fixtures/stable.maki"
assert_file "maki-zed" "${extension_dir}/scripts/verify.sh"
assert_file "maki-zed" "${extension_dir}/scripts/update-tree-sitter-queries.py"

run_stage "validate revision metadata" \
  env PYTHONDONTWRITEBYTECODE=1 python3 \
    "${script_dir}/check-stack-metadata.py" \
    "$extension_dir" "$maki_rev" "$grammar_rev"

run_stage "validate generated Zed queries" \
  env PYTHONDONTWRITEBYTECODE=1 python3 \
    "${extension_dir}/scripts/update-tree-sitter-queries.py" \
    --check \
    --target-dir "$extension_dir" \
    --source-dir "$grammar_dir" \
    --revision "$grammar_rev"

maki_input="$(git_flake_url "$maki_dir" "$maki_rev")"
grammar_input="$(git_flake_url "$grammar_dir" "$grammar_rev")"
extension_input="$(git_flake_url "$extension_dir" "$extension_rev")"
shared_fixture="${grammar_dir}/test/fixtures/stable.maki"
shared_output="${checkout_root}/stable.html"
maki_package=
extension_nix_options=(
  --no-write-lock-file
  --override-input maki "$maki_input"
  --override-input tree-sitter-maki "$grammar_input"
)
extension_environment=(
  MAKI_ROOT="$maki_dir"
  TREE_SITTER_MAKI_DIR="$grammar_dir"
  TREE_SITTER_MAKI_PINNED_DIR="$grammar_dir"
  TREE_SITTER_MAKI_PINNED_REV="$grammar_rev"
)

maki_repository_gate() (
  cd "$maki_dir"
  bash scripts/ci/check-maki.sh
)

resolve_maki_package() {
  maki_package="$(nix build --no-link --print-out-paths "${maki_input}#default")"
  [[ "$maki_package" != *$'\n'* ]] \
    || die "Maki default package produced more than one output"
  [[ -x "${maki_package}/bin/maki" ]] \
    || die "Maki default package does not contain an executable bin/maki"
}

build_shared_fixture() {
  "${maki_package}/bin/maki" build "$shared_fixture" >"$shared_output"
  grep -qi '<!doctype html>' "$shared_output" \
    || die "Maki did not render the shared syntax fixture as HTML"
}

legacy_grammar_repository_gate() (
  cd "$grammar_dir"
  MAKI_DOCS_DIR="${maki_dir}/docs" \
    nix develop --no-write-lock-file -c ./scripts/verify.sh
)

cached_grammar_repository_gate() {
  nix build --no-link --no-write-lock-file "${grammar_input}#verify"
}

grammar_maki_docs_gate() (
  cd "$grammar_dir"
  nix develop --no-write-lock-file "${grammar_input}#default" \
    -c tree-sitter parse --quiet \
      "${maki_dir}/docs"/*.maki \
      "${maki_dir}/docs"/milestones/*.maki
)

legacy_extension_flake_checks() (
  cd "$extension_dir"
  nix flake check "${extension_nix_options[@]}"
)

legacy_extension_repository_gate() (
  cd "$extension_dir"
  nix develop \
    "${extension_nix_options[@]}" \
    -c env \
      "${extension_environment[@]}" \
      ./scripts/verify.sh
)

cached_extension_repository_gate() {
  nix build \
    --no-link \
    "${extension_nix_options[@]}" \
    "${extension_input}#verify"
}

extension_stack_gate() (
  cd "$extension_dir"
  nix develop \
    "${extension_nix_options[@]}" \
    "${extension_input}#stack-ci" \
    -c env \
      MAKI_BIN="${maki_package}/bin/maki" \
      "${extension_environment[@]}" \
      ./scripts/verify-stack.sh
)

run_stage "Maki repository gate" maki_repository_gate
run_stage "resolve canonical Maki package" resolve_maki_package
run_stage "render shared syntax fixture" build_shared_fixture
assert_clean_checkout "Maki" "$maki_dir"

if [[ -f "${grammar_dir}/nix/verify.nix" ]]; then
  run_stage "tree-sitter-maki cached repository gate" \
    cached_grammar_repository_gate
  run_stage "tree-sitter-maki Maki docs integration" grammar_maki_docs_gate
else
  run_stage "tree-sitter-maki legacy repository gate" \
    legacy_grammar_repository_gate
fi
assert_clean_checkout "tree-sitter-maki" "$grammar_dir"

if [[ -f "${extension_dir}/scripts/verify-stack.sh" ]]; then
  run_stage "maki-zed cached repository gate" \
    cached_extension_repository_gate
  run_stage "maki-zed canonical Maki LSP integration" extension_stack_gate
else
  run_stage "maki-zed legacy flake checks" legacy_extension_flake_checks
  run_stage "maki-zed legacy extension, query, and LSP gate" \
    legacy_extension_repository_gate
fi
assert_clean_checkout "maki-zed" "$extension_dir"

printf 'check-stack: completed all stages in %ss\n' \
  "$((SECONDS - overall_started))"
printf 'Verified revision tuple:\n'
printf '  maki              %s\n' "$maki_rev"
printf '  tree-sitter-maki  %s\n' "$grammar_rev"
printf '  maki-zed          %s\n' "$extension_rev"
