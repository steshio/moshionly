//! Designs sent from Figma, in words for the person.
//!
//! A design arrives as a [`PendingImport`]; the shell adds it at once, as
//! one undo step made by an import, and says what arrived with these.

use moshionly_agent::import::PendingImport;
use moshionly_core::NodeType;

/// What arrived, in words: "15 layers, 2 artboards".
pub fn contents(pending: &PendingImport) -> String {
    let artboards = pending
        .nodes
        .iter()
        .filter(|node| node.node_type() == NodeType::Artboard)
        .count();
    let layers = pending.nodes.len() - artboards;
    let mut text = plural(layers, "layer");
    if artboards > 0 {
        text.push_str(", ");
        text.push_str(&plural(artboards, "artboard"));
    }
    text
}

/// Where the design came from: `“Launch” in Figma`.
pub fn origin(pending: &PendingImport) -> String {
    let app = match pending.source.app.as_str() {
        "figma" => "Figma".to_owned(),
        other => other.to_owned(),
    };
    match &pending.source.file_name {
        Some(name) if !name.trim().is_empty() => format!("“{}” in {app}", name.trim()),
        _ => app,
    }
}

/// `1 layer`, `2 layers`.
fn plural(count: usize, noun: &str) -> String {
    if count == 1 {
        format!("1 {noun}")
    } else {
        format!("{count} {noun}s")
    }
}

#[cfg(test)]
mod tests {
    use moshionly_agent::import::read_envelope;

    use super::*;

    const ENVELOPE: &str = include_str!("../../tests/fixtures/figma-hero.json");

    #[test]
    fn it_says_what_arrived_and_from_where() {
        let pending = read_envelope(ENVELOPE.as_bytes()).expect("the fixture is a valid envelope");
        assert!(
            contents(&pending).contains("artboard"),
            "{}",
            contents(&pending)
        );
        assert!(origin(&pending).contains("Figma"));
        assert_eq!(plural(1, "layer"), "1 layer");
        assert_eq!(plural(3, "layer"), "3 layers");
    }
}
