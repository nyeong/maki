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

## Validation

Run the canonical repository gate before submitting a pull request:

```bash
bash scripts/ci/check-maki.sh
```

For a focused Rust change, `cargo test` is also useful. Syntax or analysis
changes may require coordinated pull requests in `tree-sitter-maki` and
`maki-zed`; cross-link those pull requests and keep grammar pins and generated
queries synchronized.
