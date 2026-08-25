#!/usr/bin/env bash
set -euo pipefail

# Keep the canonical CI gate in flake.nix so local, Forgejo, and future mirrors
# run the same package, formatting, lint, test, smoke, and module checks.
nix flake check
