# Changelog

Notable user-visible changes to Maki are recorded here. Release sections use
the package version and release date, and link to the immutable source tag in
the canonical Forgejo repository.

## [Unreleased]

### Changed

- Replaced implicit document-local reference and footnote markers with the
  explicit `[key][]`, `[title][key]`, `[^key][]`, `[^title][key]`, and
  `[^][key]` forms; `[title](target)` now provides definition-free direct
  links, while legacy bare markers and `[^key]: value` definitions are text.
- Limited Notes entries and ordinals to caret-prefixed footnotes. Ordinary
  references now render link-capable targets directly without Notes markers;
  Prose and titled Date Range uses stay literal, and each Notes `[n]` marker
  links back to its first footnote occurrence.
- Established public release metadata, source provenance, and automated
  release-candidate checks.
