#!/usr/bin/env bash
set -euo pipefail

source_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)"
test_root="$(mktemp -d)"
trap 'rm -rf "${test_root}"' EXIT

init_checkout() {
  local checkout="${1}"
  local remote="${2}"

  git init -q -b main "${checkout}"
  git -C "${checkout}" config user.name "Maki CI"
  git -C "${checkout}" config user.email "ci@example.invalid"
  git init -q --bare "${remote}"
  git -C "${checkout}" remote add origin "${remote}"
}

maki_checkout="${test_root}/workspace/maki/maki"
maki_remote="${test_root}/maki.git"
dotfiles_checkout="${test_root}/workspace/dotfiles"
dotfiles_remote="${test_root}/dotfiles.git"

mkdir -p "${maki_checkout}/scripts" "${dotfiles_checkout}"
init_checkout "${maki_checkout}" "${maki_remote}"
cp "${source_root}/scripts/deploy-dogfood.sh" "${maki_checkout}/scripts/deploy-dogfood.sh"
git -C "${maki_checkout}" add scripts/deploy-dogfood.sh
git -C "${maki_checkout}" commit -q -m "Add deployment script"
git -C "${maki_checkout}" push -q -u origin main

init_checkout "${dotfiles_checkout}" "${dotfiles_remote}"
printf '{ outputs = _: {}; }\n' > "${dotfiles_checkout}/flake.nix"
git -C "${dotfiles_checkout}" add flake.nix
git -C "${dotfiles_checkout}" commit -q -m "Add deployment flake"
git -C "${dotfiles_checkout}" push -q -u origin main

output="$({
  DOTFILES_DIR="${dotfiles_checkout}" \
    MAKI_DEPLOY_SKIP_SSH_CHECK=1 \
    bash "${maki_checkout}/scripts/deploy-dogfood.sh" --check
} 2>&1)"

if [[ "${output}" != *"Dogfood deployment preflight passed."* ]]; then
  echo "Expected a successful deployment preflight, got:" >&2
  echo "${output}" >&2
  exit 1
fi

git -C "${dotfiles_checkout}" switch -q -c feature
if DOTFILES_DIR="${dotfiles_checkout}" \
  MAKI_DEPLOY_SKIP_SSH_CHECK=1 \
  bash "${maki_checkout}/scripts/deploy-dogfood.sh" --check >/dev/null 2>&1; then
  echo "Expected preflight to reject a non-main dotfiles checkout." >&2
  exit 1
fi

echo "Dogfood deployment preflight tests passed."
