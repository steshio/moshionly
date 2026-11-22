//! Keeps `docs/file-format.md` in step with the code.
//!
//! Two parts of that document are not written by hand: the reference tables,
//! which come from the node definitions, and the worked example, which is
//! the demo project. When the code changes, refresh them with
//!
//! ```text
//! MOSHIONLY_UPDATE_DOCS=1 cargo test -p moshionly-core --test docs
//! ```
//!
//! and commit the document together with the change.

use moshionly_core::schema;

const DOCUMENT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/file-format.md");
const DEMO: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/examples/demo.moshi");

/// Replaces what is between the start and end markers of a generated block.
fn set_block(document: &mut String, name: &str, content: &str) {
    let start = format!("<!-- {name}:start -->");
    let end = format!("<!-- {name}:end -->");
    let from = document
        .find(&start)
        .unwrap_or_else(|| panic!("the document has no \"{start}\" marker"))
        + start.len();
    let to = document
        .find(&end)
        .unwrap_or_else(|| panic!("the document has no \"{end}\" marker"));
    assert!(from <= to, "the \"{name}\" markers are in the wrong order");
    let content = content.trim_end_matches('\n');
    document.replace_range(from..to, &format!("\n\n{content}\n\n"));
}

/// The anchor GitHub gives a heading.
fn anchor(heading: &str) -> String {
    heading
        .trim_start_matches('#')
        .trim()
        .to_lowercase()
        .chars()
        .filter_map(|c| match c {
            ' ' => Some('-'),
            c if c.is_alphanumeric() || c == '-' || c == '_' => Some(c),
            _ => None,
        })
        .collect()
}

#[test]
fn the_generated_parts_match_the_code() {
    let original = std::fs::read_to_string(DOCUMENT).expect("docs/file-format.md exists");
    let demo = std::fs::read_to_string(DEMO).expect("the demo project exists");
    let mut document = original.clone();
    set_block(&mut document, "reference", &schema::reference());
    set_block(&mut document, "example", &format!("```json\n{demo}```"));
    if document == original {
        return;
    }
    if std::env::var_os("MOSHIONLY_UPDATE_DOCS").is_some() {
        std::fs::write(DOCUMENT, document).expect("docs/file-format.md can be written");
        return;
    }
    panic!(
        "docs/file-format.md no longer matches the code. Refresh it with\n\n    \
         MOSHIONLY_UPDATE_DOCS=1 cargo test -p moshionly-core --test docs\n\n\
         and commit the result."
    );
}

#[test]
fn every_link_inside_the_document_leads_somewhere() {
    let document = std::fs::read_to_string(DOCUMENT).expect("docs/file-format.md exists");
    let anchors: Vec<String> = document
        .lines()
        .filter(|line| line.starts_with('#'))
        .map(anchor)
        .collect();
    let mut links = 0;
    for (_, rest) in document
        .match_indices("](#")
        .map(|(at, _)| document.split_at(at + 3))
    {
        let target = rest.split(')').next().unwrap_or_default();
        assert!(
            anchors.iter().any(|anchor| anchor == target),
            "a link points at \"#{target}\", but no heading has that anchor"
        );
        links += 1;
    }
    assert!(links > 20, "the reference links its value types");
}

#[test]
fn the_document_names_every_node_type_and_rule() {
    let document = std::fs::read_to_string(DOCUMENT).expect("docs/file-format.md exists");
    for info in schema::node_types() {
        assert!(
            document.contains(&format!("#### `{}`", info.name)),
            "{} has no section",
            info.name
        );
    }
    for heading in [
        "## The file",
        "## Nodes",
        "## Units and conventions",
        "## Reference",
        "### The additive rule",
        "### The replacing rule",
        "### Presets",
        "### Text animations",
        "### Counters",
        "## Images",
        "## A worked example",
    ] {
        assert!(
            document.contains(heading),
            "the section \"{heading}\" is missing"
        );
    }
}
