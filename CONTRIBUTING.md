# Contributing to Maki

## Work tracking

Use [Forgejo issues](https://git.eska.nyeong.me/nyeong/maki/issues) for bugs,
design proposals, priorities, dependencies, and work that is not implemented.
Keep each issue independently understandable and link prerequisites from both
sides. Pull requests should link the issue they implement when one exists.

## Documentation

Repository documentation describes behavior available in the revision that
contains it.

- Document commands, syntax, configuration, APIs, limitations, and contributor
  procedures that can be verified in the current tree.
- Track roadmaps, milestones, wish lists, incomplete designs, and proposed
  syntax in Forgejo issues rather than under `docs/`.
- Update the relevant reference document in the same pull request that changes
  user-visible behavior.
- Describe an unsupported case as a current boundary. Do not promise a future
  implementation in the reference documentation.
- Keep public examples usable on a clean machine without a maintainer account,
  private host, personal checkout layout, or dotfiles.

`docs/maki-syntax.maki` is the stable syntax source of truth.
`docs/maki-toml.maki`, `docs/web.maki`, and `docs/lsp.maki` describe the current
configuration and runtime surfaces.

## Deployment ownership

This repository owns reusable deployment mechanisms, including the NixOS
module and its checks. The repository that owns a deployment must provide the
exact Maki revision, project inputs, host and target selection, credentials,
and activation workflow.

Do not add scripts or checks here that discover a maintainer checkout, default
to maintainer-specific hosts or users, or locate or hard-code a specific
deployment-owned flake. Reusable deployment tooling must receive every
external input explicitly and remain usable by an unrelated contributor on a
clean machine.

## Validation

Run the canonical repository gate before submitting a pull request:

```bash
bash scripts/ci/check-maki.sh
```

For a focused Rust change, `cargo test` is also useful. Syntax or analysis
changes may require coordinated pull requests in `tree-sitter-maki` and
`maki-zed`; cross-link those pull requests and keep grammar pins and generated
queries synchronized.

## Cross-repository revision tuples

Use `scripts/ci/check-stack.sh` to reproduce one immutable combination of Maki,
the Tree-sitter grammar, and the Zed extension. It creates isolated temporary
checkouts from credential-free HTTPS URLs, verifies the extension manifest,
flake lock, and generated queries against the requested revisions, and then
runs the three repository-owned validation gates. The grammar's
`test/fixtures/stable.maki` is also built by the selected Maki revision and
exercised by both canonical and Zed queries.

The entrypoint requires Bash 4 or newer, Git, Python 3.11 or newer, and Nix with
flakes enabled. The Forgejo workflow only requires Git and a flake-enabled Nix
installation on the runner; it supplies Python through the lightweight
`stack-ci` development shell. The script deliberately ignores sibling
checkouts, direnv state, and ambient Git credentials. Treat every selected
revision as trusted code because the check evaluates its Nix expressions and
runs its repository scripts.
The caller is responsible for choosing publicly reachable, trusted hosts; an
HTTPS scheme alone does not prove that a host is public.

Sibling revisions that expose a `#verify` package run their repository-owned
Cargo and query gates as cacheable Nix derivations. The remaining Maki docs and
LSP integration gates reuse the selected Maki package's canonical binary.
Older tuples without that interface continue to use their legacy verification
scripts. Tree-sitter parser libraries use the temporary checkout root instead
of a shared runner cache, so an interrupted parse cannot poison a later run.
Every major stage logs its elapsed time. Binary-cache endpoints,
credentials, and trust policy belong to the runner environment and are not
configured by this public workflow.

This known-compatible tuple is a complete example:

```bash
bash scripts/ci/check-stack.sh \
  --maki-url https://git.eska.nyeong.me/nyeong/maki.git \
  --maki-rev d055f882a0b27049b3896b52854e59027518c973 \
  --grammar-url https://git.eska.nyeong.me/nyeong/tree-sitter-maki.git \
  --grammar-rev d43977d68bddf3090fc038f17744a833ca42d515 \
  --extension-url https://git.eska.nyeong.me/nyeong/maki-zed.git \
  --extension-rev 8bb3fac02e810370deee541629e23ec1da5c5fc0
```

The same six values are inputs to the `Revision tuple CI` manual Forgejo
workflow. A newer dispatch for the same three revisions cancels an in-progress
duplicate. A fetch fails unless each commit is reachable from its public
repository. Use `--keep-checkouts` to preserve the temporary directory after a
failure, or `--validate-only` to check input shape without fetching or executing
the selected code.
