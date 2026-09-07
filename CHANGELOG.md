# Changelog

Notable user-visible changes to Maki are recorded here. Release sections use
the package version and release date, and link to the immutable source tag in
the canonical Forgejo repository.

## [Unreleased]

### Changed

- Replaced implicit document-local reference and footnote markers with the
  explicit `[key][]`, `[title][key]`, `[^key][]`, `[^title][key]`, and
  `[^][key]` forms; `[title](path)` now provides local definition-free links,
  `[title]<URL>` provides titled HTTP/HTTPS links, and `[title][[target]]`
  provides titled note links, while legacy bare markers and `[^key]: value`
  definitions are text.
- Existing `[title](URL)` source remains literal and must be migrated manually;
  the LSP intentionally provides no migration diagnostic or action.
- Added a `refactor.extract` LSP Code Action that turns authored URL links into
  document-local references, lazily fetching a bounded, public-web HTML title
  for bare URLs and using deterministic reuse, fallback, and key suffix rules.
- Limited Notes entries and ordinals to caret-prefixed footnotes. Ordinary
  references now render link-capable targets directly without Notes markers;
  Prose and titled Date Range uses stay literal, and each Notes `[n]` marker
  links back to its first footnote occurrence.
- Established public release metadata, source provenance, and automated
  release-candidate checks.
