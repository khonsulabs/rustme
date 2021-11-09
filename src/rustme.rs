use std::{
    collections::{BTreeMap, HashMap},
    fs::File,
    io::{ErrorKind, Write},
    path::{Path, PathBuf},
    str::Utf8Error,
    string::FromUtf8Error,
};

use serde::{Deserialize, Serialize};

/// A configuration of how to generate one or more READMEs.
#[derive(Deserialize, Serialize, Debug)]
pub struct Configuration {
    /// The location that paths should be resolved relative to.
    #[serde(skip)]
    pub relative_to: PathBuf,
    /// The collection of files (key) and sections (values).
    pub files: HashMap<String, Vec<String>>,
    /// A list of glossaries that act as a source of snippets.
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
    pub fn generate(&self) -> Result<(), Error> {
        let mut snippets = HashMap::new();
        let glossary = self.load_glossaries()?;
        for (name, sections) in &self.files {
            let output_path = self.relative_to.join(name);
            if output_path.exists() {
                std::fs::remove_file(&output_path)?;
            }

            let mut output = File::create(&output_path)?;
            for (index, section) in sections.iter().enumerate() {
                if index > 0 {
                    output.write_all(b"\n")?;
                }
                let markdown = if section.starts_with("http://") || section.starts_with("https://")
                {
                    ureq::get(section)
                        .set("User-Agent", "RustMe")
                        .call()?
                        .into_string()?
                } else {
                    std::fs::read_to_string(self.relative_to.join(section))?
                };
                let processed =
                    process_markdown(&markdown, &self.relative_to, &mut snippets, &glossary)?;
                output.write_all(processed.as_bytes())?;
            }
        }

        Ok(())
    }

    fn load_glossaries(&self) -> Result<HashMap<String, String>, Error> {
        let mut combined = HashMap::new();

        for glossary in &self.glossaries {
            match glossary {
                Glossary::External(url) => {
                    let glossary_text = ureq::get(url)
                        .set("User-Agent", "RustMe")
                        .call()?
                        .into_string()?;
                    let glossary = ron::from_str::<BTreeMap<String, String>>(&glossary_text)?;
                    for (key, value) in glossary {
                        combined.insert(key, value);
                    }
                }
                Glossary::Inline(glossary) => {
                    for (key, value) in glossary {
                        combined.insert(key.to_string(), value.to_string());
                    }
                }
            }
        }

        Ok(combined)
    }
}

fn replace_references(
    markdown: &str,
    base_dir: &Path,
    snippets: &mut HashMap<String, String>,
    glossary: &HashMap<String, String>,
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
        } else if let Some(value) = glossary.get(snippet_ref) {
            processed.extend(value.bytes());
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
    glossary: &HashMap<String, String>,
) -> Result<String, Error> {
    let expanded = replace_references(markdown, base_dir, snippets, glossary)?;
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
    let contents = std::fs::read_to_string(disk_path)?;
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
#[derive(Deserialize, Serialize, Debug)]
pub enum Glossary {
    /// An external glossary. The contained value should be a valid Url to a
    /// Ron-encoded `HashMap<String, String>`.
    External(String),
    /// An inline glossary.
    Inline(HashMap<String, String>),
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
                Inline({
                    "TEST": "SUCCESS",
                })
            ]
        )
        "#,
    )
    .unwrap();
    println!("Parsed: {:?}", configuration);
}

/// All errors that `rustme` can return.
#[derive(thiserror::Error, Debug)]
pub enum Error {
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
