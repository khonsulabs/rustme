# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## Unreleased

### Breaking Changes

- `Glossary` is now defined as an untagged enum in Serde. This introduces a
  breaking change for existing stored configurations. The change is easy in Ron:
  remove the word `Inline` or `External` and the surrounding parentheses. E.g.:

  ```ron
  glossaries: [
      External("https://github.com/khonsulabs/.github/raw/main/snippets/glossary.ron"),
  ],
  ```

  Becomes:

  ```ron
  glossaries: [
      "https://github.com/khonsulabs/.github/raw/main/snippets/glossary.ron",
  ],
  ```

### Changes

- `Configuration::files` can now be configured with a `File`, allowing more
  options for each file.
- Introduced a `Term`, allowing a value in a `Glossary` to be customized based
  on context.
- Added `File::for_docs`, which enables rendering glossary terms with different
  values when the output is for `cargo doc`. A reason to use this feature would
  be to link to `docs.rs` in a README, but use the internal docs links when
  rendering a file destined for use in a doc attribute.
- Added `File::glossaries`, the primary purpose being the ability to override
  glossary term values on a per-file basis. A reason to use this feature might
  be in a workspace repository where there are many crates with multiple READMEs
  being generated. A documentation link in one crate might be able to be an
  inline reference, while in most crates, the default behavior should link to
  `docs.rs`.
- `Configuration::generate` now takes a boolean parameter denoting whether the
  files are being generated for release. The command-line option is `--release`.
  Terms will use the `release` value instead of the `default` value.
- `rustme` now scans the current directory for all RustMe configurations at all
  depths.

## v0.1.1

### Fixed

- Unicode characters are no longer broken during encoding.
- Common prefix stripping on snippets now enforces it only strips whitespace.
- `#[derive]` lines are no longer stripped as part of the RustDoc compatibility
  process. Now lines that begin with "# " are stripped -- the space is required.

## v0.1.0

Initial release.
