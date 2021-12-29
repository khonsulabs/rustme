use std::{
    borrow::Cow,
    collections::{BTreeMap, HashMap},
    fs,
    io::{ErrorKind, Write},
    path::{Path, PathBuf},
    str::Utf8Error,
    string::FromUtf8Error,
};

use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

/// A configuration of how to generate one or more READMEs.
#[derive(Deserialize, Serialize, Debug)]
pub struct Configuration {
    /// The location that paths should be resolved relative to.
    #[serde(skip)]
    pub relative_to: PathBuf,
    /// The collection of files (key) and sections (values).
    pub files: HashMap<String, FileConfiguration>,
    /// A list of glossaries that act as a source of snippets.
    #[serde(default)]
    pub glossaries: Vec<Glossary>,
}

/// A configuration for a [`File`].
#[derive(Deserialize, Serialize, Debug)]
#[serde(untagged)]
pub enum FileConfiguration {
    /// An inline file configuration, which is just a list of sections.
    Sections(Vec<String>),
    /// A full file configuration.
    File(File),
}

impl<'a> From<&'a FileConfiguration> for File {
    fn from(config: &'a FileConfiguration) -> Self {
        match config {
            FileConfiguration::Sections(sections) => Self {
                sections: sections.clone(),
                ..Self::default()
            },
            FileConfiguration::File(file) => file.clone(),
        }
    }
}

/// A `RustMe` file configuration.
#[derive(Deserialize, Serialize, Debug, Default, Clone)]
pub struct File {
    /// If true, the output is considered for `cargo doc`.
    #[serde(default)]
    pub for_docs: bool,
    /// A list of sections that compose this file.
    pub sections: Vec<String>,
    /// A list of glossaries that are used for this file. Any [`Term`]s defined
    /// in these glossaries will have a higher precedence than the ones defined
    /// at the [`Configuration`] level.
    #[serde(default)]
    pub glossaries: Vec<Glossary>,
}

impl Configuration {
    /// Attempts to load a configuration from `path`.
    ///
    /// # Errors
    ///
    /// - [`Error::Io`]: Returned if an error occurs interacting with the
    ///   filesystem.
    /// - [`Error::Ron`]: Returned if an error occurs while parsing the
    ///   configuration file.
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self, Error> {
        let contents = std::fs::read_to_string(path.as_ref())?;
        let mut configuration = ron::from_str::<Self>(&contents)?;
        configuration.relative_to = path
            .as_ref()
            .parent()
            .ok_or_else(|| Error::Io(std::io::Error::from(ErrorKind::NotFound)))?
            .to_path_buf();
        Ok(configuration)
    }

    /// Generates the README files.
    ///
    /// # Errors
    ///
    /// Can return various errors that are encountred with files that could not
    /// be parsed.
    pub fn generate(&self, release: bool) -> Result<(), Error> {
        self.generate_with_cache(release, &mut Cache::default())
    }

    /// Generates the README files using `cache` to load glossaries and snippets.
    ///
    /// # Errors
    ///
    /// Can return various errors that are encountred with files that could not
    /// be parsed.
    pub fn generate_with_cache(&self, release: bool, cache: &mut Cache) -> Result<(), Error> {
        let mut snippets = HashMap::new();
        let glossary = self.load_glossaries(cache)?;
        for (name, file_config) in &self.files {
            let output_path = self.relative_to.join(name);
            if output_path.exists() {
                std::fs::remove_file(&output_path)?;
            }

            let file = File::from(file_config);

            let glossary = if file.glossaries.is_empty() {
                Cow::Borrowed(&glossary)
            } else {
                let mut combined_glossary = glossary.clone();
                self.load_glossaries_into(&file.glossaries, &mut combined_glossary, cache)?;
                Cow::Owned(combined_glossary)
            };

            let mut output = fs::File::create(&output_path)?;
            for (index, section) in file.sections.iter().enumerate() {
                if index > 0 {
                    output.write_all(b"\n")?;
                }
                let markdown = cache.get(section, &self.relative_to, || {
                    Error::SnippetNotFound(section.to_string())
                })?;
                let processed = process_markdown(
                    &markdown,
                    &self.relative_to,
                    &mut snippets,
                    &glossary,
                    Context {
                        release,
                        for_docs: file.for_docs,
                    },
                )?;
                output.write_all(processed.as_bytes())?;
            }
        }

        Ok(())
    }

    fn load_glossaries(&self, cache: &mut Cache) -> Result<HashMap<String, Term>, Error> {
        let mut combined = HashMap::new();

        self.load_glossaries_into(&self.glossaries, &mut combined, cache)?;

        Ok(combined)
    }

    fn load_glossaries_into(
        &self,
        glossaries: &[Glossary],
        combined: &mut HashMap<String, Term>,
        cache: &mut Cache,
    ) -> Result<(), Error> {
        for glossary in glossaries {
            self.load_glossary_terms(glossary, combined, cache)
                .map_err(|err| Error::Glossary {
                    location: glossary.location().to_string(),
                    error: err.to_string(),
                })?;
        }

        Ok(())
    }

    fn load_glossary_terms(
        &self,
        glossary: &Glossary,
        combined: &mut HashMap<String, Term>,
        cache: &mut Cache,
    ) -> Result<(), Error> {
        match glossary {
            Glossary::External(reference) => {
                let glossary_text =
                    cache.get(reference, &self.relative_to, || Error::Glossary {
                        location: reference.to_string(),
                        error: String::from("not found"),
                    })?;

                let glossary = ron::from_str::<BTreeMap<String, Term>>(&glossary_text)?;
                for (key, value) in glossary {
                    merge_term(combined, key, value);
                }
            }
            Glossary::Inline(glossary) => {
                for (key, term) in glossary {
                    merge_term(combined, key.to_string(), term.clone());
                }
            }
        }

        Ok(())
    }
}

fn merge_term(combined: &mut HashMap<String, Term>, key: String, term: Term) {
    if let Some(original_term) = combined.get_mut(&key) {
        original_term.update_with(term);
    } else {
        combined.insert(key, term);
    }
}

/// A cache for loading snippets and glossaries.
#[derive(Default)]
pub struct Cache(HashMap<CacheKey, String>);

#[derive(Hash, Eq, PartialEq)]
enum CacheKey {
    Path(PathBuf),
    Url(String),
}

impl Cache {
    fn get(
        &mut self,
        resource: &str,
        relative_to: &Path,
        not_found: impl FnOnce() -> Error,
    ) -> Result<String, Error> {
        let cache_key = if resource.starts_with("http://") || resource.starts_with("https://") {
            CacheKey::Url(resource.to_string())
        } else {
            CacheKey::Path(relative_to.join(resource))
        };
        if let Some(existing_value) = self.0.get(&cache_key) {
            Ok(existing_value.clone())
        } else {
            let contents = match &cache_key {
                CacheKey::Path(resource_path) => match std::fs::read_to_string(&resource_path) {
                    Ok(contents) => contents,
                    Err(err) => {
                        if err.kind() == ErrorKind::NotFound {
                            return Err(not_found());
                        }
                        return Err(Error::from(err));
                    }
                },
                CacheKey::Url(url) => {
                    println!("Requesting {}", url);
                    match ureq::get(url).set("User-Agent", "RustMe").call() {
                        Ok(response) => response.into_string()?,
                        Err(ureq::Error::Status(code, _)) if code == 404 => return Err(not_found()),
                        Err(err) => return Err(Error::from(err)),
                    }
                }
            };
            self.0.insert(cache_key, contents.clone());
            Ok(contents)
        }
    }
}

#[derive(Copy, Clone)]
struct Context {
    for_docs: bool,
    release: bool,
}

fn replace_references(
    markdown: &str,
    base_dir: &Path,
    snippets: &mut HashMap<String, String>,
    glossary: &HashMap<String, Term>,
    context: Context,
) -> Result<String, Error> {
    let mut processed = Vec::with_capacity(markdown.len());
    let mut chars = StrByteIterator::new(markdown);
    loop {
        let skipped = chars.read_until_char(b'$')?;
        if !skipped.is_empty() {
            processed.extend(skipped.bytes());
        }
        // Skip the $, or exit if one wasn't found.
        if chars.next().is_none() {
            break;
        }

        let snippet_ref = chars.read_until_char(b'$')?;
        // Skip the trailing $
        if chars.next().is_none() {
            return Err(Error::MalformedCodeBlock);
        }
        if snippet_ref.is_empty() {
            // An escaped dollar sign
            processed.push(b'$');
        } else if let Some(term) = glossary.get(snippet_ref) {
            processed.extend(term.to_string(context).bytes());
        } else {
            let snippet = load_snippet(snippet_ref, base_dir, snippets)?;
            processed.extend(snippet.bytes());
        }
    }
    Ok(String::from_utf8(processed)?)
}

fn preprocess_rust_codeblocks(markdown: &str) -> Result<String, Error> {
    let mut processed = Vec::with_capacity(markdown.len());
    let mut chars = StrByteIterator::new(markdown);
    while let Some(ch) = chars.next() {
        match ch {
            b'`' => {
                if chars.try_read("``rust") {
                    // Preprocess rust blocks in the same way that rustdoc does.
                    processed.extend(b"```rust");
                    let rest_of_line = chars.read_line()?;
                    processed.extend(rest_of_line.bytes());

                    loop {
                        let line = chars.read_line()?;
                        if line.is_empty() {
                            return Err(Error::MalformedCodeBlock);
                        }
                        let trimmed_start = line.trim_start();
                        if trimmed_start.starts_with("```") {
                            // Ends the code block
                            processed.extend(line.bytes());
                            break;
                        } else if trimmed_start.starts_with("# ") {
                            // A rust-doc comment
                        } else {
                            processed.extend(line.bytes());
                        }
                    }
                } else {
                    processed.push(ch);
                }
            }
            ch => {
                processed.push(ch);
            }
        }
    }
    Ok(String::from_utf8(processed)?)
}

fn process_markdown(
    markdown: &str,
    base_dir: &Path,
    snippets: &mut HashMap<String, String>,
    glossary: &HashMap<String, Term>,
    context: Context,
) -> Result<String, Error> {
    let expanded = replace_references(markdown, base_dir, snippets, glossary, context)?;
    preprocess_rust_codeblocks(&expanded)
}

fn load_snippet<'a>(
    snippet_ref: &str,
    base_dir: &Path,
    snippets: &'a mut HashMap<String, String>,
) -> Result<&'a String, Error> {
    if !snippets.contains_key(snippet_ref) {
        let path = snippet_ref.split(':').next().unwrap();
        load_snippets(path, &base_dir.join(path), snippets)?;
    }

    if let Some(snippet) = snippets.get(snippet_ref) {
        Ok(snippet)
    } else {
        Err(Error::SnippetNotFound(snippet_ref.to_string()))
    }
}

fn remove_shared_prefix(strings: &mut [&str]) {
    if strings.is_empty() || strings[0].is_empty() {
        return;
    }

    loop {
        if strings[1..].iter().all(|string| {
            !string.is_empty()
                && string.as_bytes()[0].is_ascii_whitespace()
                && string[0..1] == strings[0][0..1]
        }) {
            for string in strings.iter_mut() {
                *string = &string[1..];
            }
        } else {
            break;
        }
    }
}

fn load_snippets(
    ref_path: &str,
    disk_path: &Path,
    snippets: &mut HashMap<String, String>,
) -> Result<(), Error> {
    const SNIPPET_START: &str = "begin rustme snippet:";
    const SNIPPET_END: &str = "end rustme snippet";
    let contents = match std::fs::read_to_string(disk_path) {
        Ok(contents) => contents,
        Err(err) => {
            if err.kind() == ErrorKind::NotFound {
                return Err(Error::SnippetNotFound(ref_path.to_string()));
            }
            return Err(Error::from(err));
        }
    };
    let mut current_snippet = Vec::new();
    let mut current_snippet_name = None;
    for line in contents.lines() {
        if let Some(phrase_start) = line.find(SNIPPET_START) {
            current_snippet_name = Some(
                line[phrase_start + SNIPPET_START.len()..]
                    .trim()
                    .split(' ')
                    .next()
                    .unwrap(),
            );
            current_snippet = Vec::default();
        } else if line.contains(SNIPPET_END) {
            if let Some(name) = current_snippet_name.take() {
                remove_shared_prefix(&mut current_snippet);
                let contents = current_snippet.join("\n");
                if snippets
                    .insert(format!("{}:{}", ref_path, name), contents)
                    .is_some()
                {
                    return Err(Error::SnippetAlreadyDefined(name.to_string()));
                }
            } else {
                return Err(Error::MalformedSnippet);
            }
        } else if current_snippet_name.is_some() {
            current_snippet.push(line);
        }
    }

    // Allow referring to an entire file as a snippet.
    snippets.insert(ref_path.to_string(), contents);

    Ok(())
}

struct StrByteIterator<'a> {
    remaining: &'a [u8],
}

impl<'a> StrByteIterator<'a> {
    pub const fn new(value: &'a str) -> Self {
        Self {
            remaining: value.as_bytes(),
        }
    }

    pub fn try_read(&mut self, compare_against: &str) -> bool {
        if self.remaining.starts_with(compare_against.as_bytes()) {
            let (_, tail) = self.remaining.split_at(compare_against.len());
            self.remaining = tail;
            true
        } else {
            false
        }
    }

    pub fn read_until(
        &mut self,
        mut cb: impl FnMut(u8) -> bool,
        include_last_byte: bool,
    ) -> Result<&'a str, Error> {
        for (index, byte) in self.remaining.iter().copied().enumerate() {
            // Do not offer non-ascii characters to the callback. This could
            // allow splitting inside of a unicode code point.
            if byte < 128 && cb(byte) {
                let (read, tail) = if include_last_byte {
                    self.remaining.split_at(index + 1)
                } else {
                    self.remaining.split_at(index)
                };
                self.remaining = tail;
                return Ok(std::str::from_utf8(read)?);
            }
        }

        let result = self.remaining;
        self.remaining = b"";
        Ok(std::str::from_utf8(result)?)
    }

    pub fn read_until_char(&mut self, ch: u8) -> Result<&'a str, Error> {
        self.read_until(|byte| byte == ch, false)
    }

    pub fn read_line(&mut self) -> Result<&'a str, Error> {
        self.read_until(|ch| ch == b'\n', true)
    }
}

impl<'a> Iterator for StrByteIterator<'a> {
    type Item = u8;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining.is_empty() {
            None
        } else {
            let (next, tail) = self.remaining.split_at(1);
            self.remaining = tail;
            next.get(0).copied()
        }
    }
}

/// A mapping of replacements that can be used within the files using `$name$`
/// syntax.
#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(untagged)]
pub enum Glossary {
    /// An external glossary. The contained value should be a valid Url to a
    /// Ron-encoded `HashMap<String, String>`.
    External(String),
    /// An inline glossary.
    Inline(HashMap<String, Term>),
}

impl Glossary {
    fn location(&self) -> &str {
        match self {
            Glossary::External(location) => location,
            Glossary::Inline(_) => "(inline)",
        }
    }
}

/// A [`Glossary`] value.
#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(untagged)]
pub enum Term {
    /// A term that is the same value in all contexts.
    Static(String),
    /// A term that has different values based on the output context.
    Conditional {
        /// The value to be used when [`File::for_docs`] is true.
        #[serde(default)]
        for_docs: Option<String>,
        /// The value to be used when outputting in release mode.
        #[serde(default)]
        release: Option<String>,
        /// The value to be used basic output operations.
        #[serde(default)]
        default: Option<String>,
    },
}

impl Term {
    fn update_with(&mut self, other: Self) {
        *self = match (&self, other) {
            (_, Term::Static(value)) => Self::Static(value),
            (
                Term::Static(value),
                Term::Conditional {
                    for_docs,
                    release,
                    default,
                },
            ) => Self::Conditional {
                for_docs,
                release,
                default: default.or_else(|| Some(value.to_string())),
            },
            (
                Term::Conditional {
                    for_docs,
                    release,
                    default,
                },
                Term::Conditional {
                    for_docs: other_for_docs,
                    release: other_release,
                    default: other_default,
                },
            ) => Self::Conditional {
                for_docs: other_for_docs.or_else(|| for_docs.clone()),
                release: other_release.or_else(|| release.clone()),
                default: other_default.or_else(|| default.clone()),
            },
        }
    }
    fn to_string(&self, context: Context) -> String {
        match self {
            Term::Static(value) => value.clone(),
            Term::Conditional {
                default,
                release,
                for_docs: for_docs_value,
            } => {
                if let (true, Some(value)) = (context.for_docs, for_docs_value) {
                    value.clone()
                } else if let (true, Some(value)) = (context.release, release) {
                    value.clone()
                } else {
                    default.clone().unwrap_or_default()
                }
            }
        }
    }
}

#[test]
fn test_no_glossary() {
    let configuration: Configuration = ron::from_str(
        r#"
        Configuration(
            files: {
                "README.md": ["a", "b"],
                "OTHERREADME.md": ["a", "b"],
            }
        )
        "#,
    )
    .unwrap();
    println!("Parsed: {:?}", configuration);
}

#[test]
fn test_glossary() {
    let configuration: Configuration = ron::from_str(
        r#"
        Configuration(
            files: {
                "README.md": ["a", "b"],
                "OTHERREADME.md": ["a", "b"],
            },
            glossaries: [
                {
                    "TEST": "SUCCESS",
                }
            ]
        )
        "#,
    )
    .unwrap();
    println!("Parsed: {:?}", configuration);
}

/// All errors that `rustme` can return.
#[derive(thiserror::Error, Debug)]
#[non_exhaustive]
pub enum Error {
    /// No configuration was found.
    #[error("no configuration found")]
    NoConfiguration,
    /// A snippet reference is missing its closing `$`.
    #[error("A snippet reference is missing its closing $")]
    MalformedSnippetReference,
    /// A mismatch of snippet begins and ends.
    #[error("A mismatch of snippet begins and ends")]
    MalformedSnippet,
    /// A rust code block was not able to be parsed.
    #[error("A rust code block was not able to be parsed")]
    MalformedCodeBlock,
    /// A snippet was already defined.
    #[error("snippet already defined: {0}")]
    SnippetAlreadyDefined(String),
    /// A snippet was not found.
    #[error("snippet not found: {0}")]
    SnippetNotFound(String),
    /// A snippet was begun but not ended.
    #[error("snippet end not missing")]
    SnippetEndNotFound,
    /// An I/O error occurred.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    /// A [Ron](https://github.com/ron-rs/ron) error.
    #[error("ron error: {0}")]
    Ron(#[from] ron::Error),
    /// An error requesting an Http resource.
    #[error("http error: {0}")]
    Http(#[from] ureq::Error),
    /// An invalid Unicode byte sequence was encountered.
    #[error("unicode error: {0}")]
    Unicode(String),
    /// An error loading a glossary.
    #[error("glossary {location} error: {error}")]
    Glossary {
        /// The location of the glossary.
        location: String,
        /// The error encountered.
        error: String,
    },
}

impl From<Utf8Error> for Error {
    fn from(err: Utf8Error) -> Self {
        Self::Unicode(err.to_string())
    }
}

impl From<FromUtf8Error> for Error {
    fn from(err: FromUtf8Error) -> Self {
        Self::Unicode(err.to_string())
    }
}

/// Generates all `RustMe` configurations found within the current directory.
///
/// ## Errors
///
/// - Returns any errors occurred processing an individual configuration.
/// - Returns [`Error::NoConfiguration`] if no configurations were found.
pub fn generate(release: bool) -> Result<(), Error> {
    generate_in_directory(Path::new("."), release)
}

/// Generates all `RustMe` configurations found within `directory`.
///
/// ## Errors
///
/// - Returns any errors occurred processing an individual configuration.
/// - Returns [`Error::NoConfiguration`] if no configurations were found.
pub fn generate_in_directory(directory: &Path, release: bool) -> Result<(), Error> {
    let mut cache = Cache::default();
    let mut found_a_config = false;
    for entry in WalkDir::new(directory).into_iter().filter_map(Result::ok) {
        let config_path = if entry.file_name() == ".rustme.ron" {
            entry.into_path()
        } else if entry.file_type().is_dir() && entry.file_name() == ".rustme" {
            entry.path().join("config.ron")
        } else {
            continue;
        };
        found_a_config = true;

        println!("Processing {:?}", config_path);
        let config = Configuration::load(config_path)?;
        config.generate_with_cache(release, &mut cache)?;
    }

    if found_a_config {
        Ok(())
    } else {
        Err(Error::NoConfiguration)
    }
}
