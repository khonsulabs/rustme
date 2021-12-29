pub fn main() {
    // begin rustme snippet: example
    let config = rustme::Configuration::load("./examples/basic/.rustme.ron").unwrap();
    config.generate(false).unwrap();
    // end rustme snippet
}

#[test]
fn check_readme() {
    main();
    let generated =
        std::fs::read_to_string("./examples/basic/README.md").expect("generated file missing");
    assert!(generated.contains("# Basic Example"));
    assert!(generated.contains("## Content"));
    assert!(generated.contains("## Open-source Licenses"));
    // Test snippet loading, as well as common-prefix stripping
    assert!(generated.contains("```rust\nlet config"));
    // Test glossary replacements.
    assert!(generated.contains("https://khonsulabs.com"));
}
