use std::{collections::HashMap, fs::File, io::Write, path::Path};

use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize, Debug)]
pub struct Configuration {
    pub files: HashMap<String, RustMe>,
    #[serde(default)]
    pub glossaries: Vec<Glossary>,
}

impl Configuration {
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self, Error> {
        let contents = std::fs::read_to_string(path)?;
        let configuration = ron::from_str(&contents)?;
        Ok(configuration)
    }

    pub fn generate(&self, base_dir: &Path) -> Result<(), Error> {
        let mut snippets = HashMap::new();
        for (name, file) in &self.files {
            let output_path = base_dir.join(name);
            if output_path.exists() {
                std::fs::remove_file(&output_path)?;
            }

            let mut output = File::create(&output_path)?;
            for (index, section) in file.sections.iter().enumerate() {
                if index > 0 {
                    output.write_all(b"\n")?;
                }
                if section.starts_with("http://") || section.starts_with("https://") {
                    let body: String = ureq::get(section)
                        .set("User-Agent", "RustMe")
                        .call()?
                        .into_string()?;
                    output.write_all(body.as_bytes())?;
                } else {
                    let markdown = std::fs::read_to_string(base_dir.join(section))?;
                    let processed = process_markdown(&markdown, base_dir, &mut snippets)?;
                    output.write_all(processed.as_bytes())?;
                }
            }
        }

        Ok(())
    }
}

fn include_snippets(
    markdown: &str,
    base_dir: &Path,
    snippets: &mut HashMap<String, String>,
) -> Result<String, Error> {
    let mut processed = String::with_capacity(markdown.len());
    let mut chars = StrByteIterator::new(markdown);
    loop {
        let skipped = chars.read_until_char(b'$');
        if !skipped.is_empty() {
            processed.push_str(skipped);
        }
        // Skip the $, or exit if one wasn't found.
        if chars.next().is_none() {
            break;
        }

        let snippet_ref = chars.read_until_char(b'$');
        // Skip the $
        if chars.next().is_none() {
            return Err(Error::MalformedCodeBlock);
        }
        if snippet_ref.is_empty() {
            // An escaped dollar sign
            processed.push('$');
        } else {
            let snippet = load_snippet(snippet_ref, base_dir, snippets)?;
            processed.push_str(snippet);
        }
    }
    Ok(processed)
}

fn preprocess_rust_codeblocks(markdown: &str) -> Result<String, Error> {
    let mut processed = String::with_capacity(markdown.len());
    let mut chars = StrByteIterator::new(markdown);
    while let Some(ch) = chars.next() {
        match ch {
            b'`' => {
                if chars.try_read("``rust") {
                    // Preprocess rust blocks in the same way that rustdoc does.
                    processed.push_str("```rust");
                    let rest_of_line = chars.read_line();
                    processed.push_str(rest_of_line);

                    loop {
                        let line = chars.read_line();
                        if line.is_empty() {
                            return Err(Error::MalformedCodeBlock);
                        }
                        let trimmed_start = line.trim_start();
                        if trimmed_start.starts_with("```") {
                            // Ends the code block
                            processed.push_str(line);
                            break;
                        } else if trimmed_start.starts_with('#') {
                            // A rust-doc comment
                        } else {
                            processed.push_str(line);
                        }
                    }
                } else {
                    processed.push(ch as char);
                }
            }
            ch => {
                processed.push(ch as char);
            }
        }
    }
    Ok(processed)
}

fn process_markdown(
    markdown: &str,
    base_dir: &Path,
    snippets: &mut HashMap<String, String>,
) -> Result<String, Error> {
    let expanded = include_snippets(markdown, base_dir, snippets)?;
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
    if strings.is_empty() {
        return;
    }

    loop {
        if strings[1..]
            .iter()
            .all(|string| !string.is_empty() && string[0..1] == strings[0][0..1])
        {
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
    remaining: &'a str,
}

impl<'a> StrByteIterator<'a> {
    pub fn new(value: &'a str) -> Self {
        Self { remaining: value }
    }

    pub fn try_read(&mut self, compare_against: &str) -> bool {
        if self.remaining.starts_with(compare_against) {
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
    ) -> &'a str {
        for (index, byte) in self.remaining.bytes().enumerate() {
            if cb(byte) {
                let (read, tail) = if include_last_byte {
                    self.remaining.split_at(index + 1)
                } else {
                    self.remaining.split_at(index)
                };
                self.remaining = tail;
                return read;
            }
        }

        let result = self.remaining;
        self.remaining = "";
        result
    }

    pub fn read_until_char(&mut self, ch: u8) -> &'a str {
        self.read_until(|byte| byte == ch, false)
    }

    pub fn read_line(&mut self) -> &'a str {
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
            next.as_bytes().get(0).copied()
        }
    }
}

#[derive(Deserialize, Serialize, Debug)]
pub struct RustMe {
    pub sections: Vec<String>,
}

#[derive(Deserialize, Serialize, Debug)]
pub enum Glossary {
    External(String),
    Inline(HashMap<String, String>),
}

#[test]
fn test_no_glossary() {
    let configuration: Configuration = ron::from_str(
        r#"
        Configuration(
            files: {
                "README.md": (
                    sections: ["a", "b"]
                ),
                "OTHERREADME.md": (
                    sections: ["a", "b"]
                )
            }
        )
        "#,
    )
    .unwrap();
    println!("Parsed: {:?}", configuration);
}

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("A snippet reference is missing its closing $")]
    MalformedSnippetReference,
    #[error("A mismatch of snippet begins and ends")]
    MalformedSnippet,
    #[error("A rust code block was not able to be parsed")]
    MalformedCodeBlock,
    #[error("snippet already defined: {0}")]
    SnippetAlreadyDefined(String),
    #[error("snippet not found: {0}")]
    SnippetNotFound(String),
    #[error("snippet end not missing")]
    SnippetEndNotFound,
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("ron error: {0}")]
    Ron(#[from] ron::Error),
    #[error("http error: {0}")]
    Http(#[from] ureq::Error),
}
