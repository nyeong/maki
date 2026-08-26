#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
default_dotfiles_dir="${HOME}/.dotfiles"
workspace_dotfiles_dir="$(cd "${repo_root}/../.." && pwd -P)/dotfiles"
if [[ -z "${DOTFILES_DIR:-}" && ! -d "${default_dotfiles_dir}" && -f "${workspace_dotfiles_dir}/flake.nix" ]]; then
  default_dotfiles_dir="${workspace_dotfiles_dir}"
fi

dotfiles_dir="${DOTFILES_DIR:-${default_dotfiles_dir}}"
remote="${MAKI_GIT_REMOTE:-origin}"
dotfiles_remote="${DOTFILES_GIT_REMOTE:-origin}"
target="${MAKI_DEPLOY_TARGET:-nixbox}"
deploy_host="${MAKI_DEPLOY_HOST:-${target}}"
deploy_user="${MAKI_DEPLOY_USER:-deploy}"
mode="deploy"

usage() {
  cat <<'EOF'
Usage: scripts/deploy-dogfood.sh [--check]

  --check  Validate repository and deployment access without updating or deploying.

Environment:
  DOTFILES_DIR              dotfiles checkout (default: ~/.dotfiles or workspace sibling)
  MAKI_GIT_REMOTE           Maki source-of-truth remote (default: origin)
  DOTFILES_GIT_REMOTE       dotfiles source-of-truth remote (default: origin)
  MAKI_DEPLOY_TARGET        deploy-rs target (default: nixbox)
  MAKI_DEPLOY_HOST          SSH preflight host (default: deploy target)
  MAKI_DEPLOY_USER          SSH preflight user (default: deploy)
  MAKI_DEPLOY_SKIP_SSH_CHECK=1
                            Skip only the explicit SSH probe; deploy-rs still requires access.
EOF
}

case "${1:-}" in
  "") ;;
  --check) mode="check" ;;
  -h|--help)
    usage
    exit 0
    ;;
  *)
    usage >&2
    exit 2
    ;;
esac

require_command() {
  local command_name="${1}"
  if ! command -v "${command_name}" >/dev/null 2>&1; then
    echo "Required command not found: ${command_name}" >&2
    exit 1
  fi
}

require_clean_main() {
  local checkout="${1}"
  local label="${2}"

  local branch
  branch="$(git -C "${checkout}" branch --show-current)"
  if [[ "${branch}" != "main" ]]; then
    echo "Refusing to use ${label} from branch '${branch:-detached HEAD}'; switch to main first." >&2
    exit 1
  fi

  if [[ -n "$(git -C "${checkout}" status --porcelain)" ]]; then
    echo "Refusing to use a dirty ${label} worktree: ${checkout}" >&2
    exit 1
  fi
}

require_remote_main() {
  local checkout="${1}"
  local label="${2}"
  local git_remote="${3}"

  if ! git -C "${checkout}" remote get-url "${git_remote}" >/dev/null 2>&1; then
    echo "${label} remote '${git_remote}' does not exist in ${checkout}." >&2
    exit 1
  fi

  git -C "${checkout}" fetch "${git_remote}" main

  local local_revision
  local remote_revision
  local_revision="$(git -C "${checkout}" rev-parse HEAD)"
  remote_revision="$(git -C "${checkout}" rev-parse FETCH_HEAD)"
  if [[ "${local_revision}" != "${remote_revision}" ]]; then
    echo "Refusing to use ${label}: local main does not match fetched ${git_remote}/main." >&2
    exit 1
  fi
}

require_command git
require_command nix
require_command ssh

if ! git -C "${repo_root}" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  echo "Maki checkout is not a Git worktree: ${repo_root}" >&2
  exit 1
fi

if [[ ! -f "${dotfiles_dir}/flake.nix" ]] || ! git -C "${dotfiles_dir}" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  echo "dotfiles checkout with flake.nix not found: ${dotfiles_dir}" >&2
  echo "Set DOTFILES_DIR to the deploy-rs flake checkout." >&2
  exit 1
fi

require_clean_main "${repo_root}" "Maki"
require_remote_main "${repo_root}" "Maki" "${remote}"
require_clean_main "${dotfiles_dir}" "dotfiles"
require_remote_main "${dotfiles_dir}" "dotfiles" "${dotfiles_remote}"

if [[ "${MAKI_DEPLOY_SKIP_SSH_CHECK:-0}" != "1" ]]; then
  if ! ssh -o BatchMode=yes -o ConnectTimeout=10 "${deploy_user}@${deploy_host}" true; then
    echo "Cannot reach ${deploy_user}@${deploy_host} for deploy-rs activation." >&2
    exit 1
  fi
fi

echo "Dogfood deployment preflight passed."
echo "  Maki:     ${repo_root} (${remote}/main)"
echo "  dotfiles: ${dotfiles_dir} (${dotfiles_remote}/main)"
echo "  target:   ${target} via ${deploy_user}@${deploy_host}"

if [[ "${mode}" == "check" ]]; then
  exit 0
fi

nix flake update maki --flake "${dotfiles_dir}"
nix run nixpkgs#deploy-rs -- "${dotfiles_dir}/#${target}" -s --remote-build
