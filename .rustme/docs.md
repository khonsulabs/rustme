Generate your Rust project's README-like files.

[![crate version](https://img.shields.io/crates/v/rustme.svg)](https://crates.io/crates/rustme)
[![Live Build Status](https://img.shields.io/github/workflow/status/khonsulabs/rustme/Tests/main)](https://github.com/khonsulabs/rustme/actions?query=workflow:Tests)
[![Documentation for `main` branch](https://img.shields.io/badge/docs-main-informational)](https://khonsulabs.github.io/rustme/main/rustme/)

RustMe generates files by concatenating multiple sections into a new file. It has specific features that are useful for Rust projects:

- Rust-annotated markdown code blocks are processed to remove lines that start with `#`, making the blocks render the same as when used with `#![doc = include_str!("...)]`. This crate uses this functionality with the code snippet below.
- Include snippets from other files. Annotate a file with special comments, and import them using the `$$snippet-path:name$$` pattern. The [basic example](./examples/basic/main.rs) demonstrates this functionality.
  - Snippets are automatically trimmed to remove equal whitespace at the beginning of each line.
- Include sections that are remote URLs.
  - [We](https://khonsulabs.com) manage a lot of repositories, and wanted to standardize specific sections of our README files across all repositories. This README's footer is loaded from another repository.

## `rustme` command line interface

Currently `rustme` ignores all command line arguments. It looks for a
[Ron](https://github.com/ron-rs/ron)-formatted `Configuration` located in either
`./rustme.ron` or `./.rustme/config.ron`, and generates the files relative to
the configuration file.

## `rustme` as a library

```rust
# use std::path::Path;
let config = rustme::Configuration::load("examples/basic/.rustme.ron").unwrap();
config.generate(Path::new("examples/basic/")).unwrap();
```
