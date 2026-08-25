#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
dotfiles_dir="${DOTFILES_DIR:-${HOME}/.dotfiles}"
remote="${MAKI_GIT_REMOTE:-nixbox}"
target="${MAKI_DEPLOY_TARGET:-nixbox}"

cd "${repo_root}"

branch="$(git branch --show-current)"
if [[ "${branch}" != "main" ]]; then
  echo "Refusing to deploy Maki from branch '${branch:-detached HEAD}'; switch to main first." >&2
  exit 1
fi

if [[ -n "$(git status --porcelain)" ]]; then
  echo "Refusing to deploy from a dirty Maki worktree." >&2
  exit 1
fi

git fetch "${remote}" main

local_revision="$(git rev-parse HEAD)"
remote_revision="$(git rev-parse "${remote}/main")"
if [[ "${local_revision}" != "${remote_revision}" ]]; then
  echo "Refusing to deploy: local main does not match ${remote}/main." >&2
  exit 1
fi

nix flake update maki --flake "${dotfiles_dir}"
nix run nixpkgs#deploy-rs -- "${dotfiles_dir}/#${target}" -s --remote-build
