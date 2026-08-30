#!/usr/bin/env bash
set -euo pipefail

if (( $# < 2 )); then
  printf 'Usage: check-stack-tree-sitter.sh CACHE_HOME PATH...\n' >&2
  exit 2
fi

export XDG_CACHE_HOME="$1"
shift

exec tree-sitter parse --quiet "$@"
