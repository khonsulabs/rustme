# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## Unreleased

### Fixed

- Unicode characters are no longer broken during encoding.
- Common prefix stripping on snippets now enforces it only strips whitespace.
- `#[derive]` lines are no longer stripped as part of the RustDoc compatibility
  process. Now lines that begin with "# " are stripped -- the space is required.
  