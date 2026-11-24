//! Designs sent from Figma: what arrived, and where the person puts it.
//!
//! A design waits as a [`PendingImport`] until the person answers the
//! dialog. Their choice of [`ImportChoice`] becomes the agent crate's
//! [`ImportTarget`], and accepting applies the whole design as one undo
//! step made by an import.

use std::collections::HashSet;

use moshionly_agent::import::{ImportTarget, PendingImport, RequestedMode};
use moshionly_core::{Document, NodeId, NodeType};

/// Where the person puts a design.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ImportChoice {
    /// Add it as new artboards, next to the others.
    NewArtboards,
    /// Put it in place of the layers it replaces, keeping their animations.
    ReplaceMatching,
    /// Put it into the artboard being worked on.
    IntoArtboard,
}

impl ImportChoice {
    /// Every choice, in the order the dialog lists them.
    pub const ALL: [Self; 3] = [
        Self::NewArtboards,
        Self::ReplaceMatching,
        Self::IntoArtboard,
    ];

    /// The choice as the dialog words it.
    pub fn label(self) -> &'static str {
        match self {
            Self::NewArtboards => "Add as new artboards",
            Self::ReplaceMatching => "Replace the matching layers and keep their animations",
            Self::IntoArtboard => "Put it into the current artboard",
        }
    }
}

/// The import call for `choice`. `artboard` is the artboard being worked
/// on; without one, there is nothing to put the design into.
pub fn import_target(choice: ImportChoice, artboard: Option<&NodeId>) -> Option<ImportTarget> {
    match choice {
        ImportChoice::NewArtboards => Some(ImportTarget::NewArtboards),
        ImportChoice::ReplaceMatching => Some(ImportTarget::ReplaceMatching),
        ImportChoice::IntoArtboard => artboard.cloned().map(ImportTarget::Into),
    }
}

/// How many top-level layers of the design came over before and are in
/// `document`: what "replace" would put new versions in place of.
pub fn matching_layers(pending: &PendingImport, document: &Document) -> usize {
    let known: HashSet<&str> = document
        .nodes()
        .filter_map(|node| node.layer()?.source_id.as_deref())
        .filter(|source| !source.is_empty())
        .collect();
    let ids: HashSet<&NodeId> = pending.nodes.iter().map(|node| &node.id).collect();
    pending
        .nodes
        .iter()
        .filter(|node| {
            node.parent
                .as_ref()
                .is_none_or(|parent| !ids.contains(parent))
        })
        .filter_map(|node| node.layer()?.source_id.as_deref())
        .filter(|source| known.contains(source))
        .count()
}

/// The choice the dialog starts on: replacing when the sender asked for it
/// and something matches, otherwise new artboards.
pub fn suggested_choice(pending: &PendingImport, document: &Document) -> ImportChoice {
    if pending.mode == Some(RequestedMode::Replace) && matching_layers(pending, document) > 0 {
        ImportChoice::ReplaceMatching
    } else {
        ImportChoice::NewArtboards
    }
}

/// Whether `choice` can be taken: replacing needs something to replace,
/// and putting into an artboard needs an artboard.
pub fn is_possible(
    choice: ImportChoice,
    pending: &PendingImport,
    document: &Document,
    artboard: Option<&NodeId>,
) -> bool {
    match choice {
        ImportChoice::NewArtboards => true,
        ImportChoice::ReplaceMatching => matching_layers(pending, document) > 0,
        ImportChoice::IntoArtboard => artboard.is_some_and(|id| document.contains(id)),
    }
}

/// What arrived, in words for the dialog: "15 layers, 2 artboards".
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
    use moshionly_agent::{Origin, SessionHandle};
    use moshionly_core::io;

    use super::*;

    const ENVELOPE: &str = include_str!("../../tests/fixtures/figma-hero.json");

    fn pending() -> PendingImport {
        read_envelope(ENVELOPE.as_bytes()).expect("the fixture is a valid envelope")
    }

    fn demo() -> Document {
        io::load(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../moshionly-core/examples/demo.moshi"
        ))
        .unwrap()
    }

    #[test]
    fn each_choice_becomes_its_import_call() {
        let artboard = NodeId::from("scene");
        assert_eq!(
            import_target(ImportChoice::NewArtboards, Some(&artboard)),
            Some(ImportTarget::NewArtboards)
        );
        assert_eq!(
            import_target(ImportChoice::ReplaceMatching, None),
            Some(ImportTarget::ReplaceMatching)
        );
        assert_eq!(
            import_target(ImportChoice::IntoArtboard, Some(&artboard)),
            Some(ImportTarget::Into(artboard))
        );
        assert_eq!(import_target(ImportChoice::IntoArtboard, None), None);
    }

    #[test]
    fn the_dialog_says_what_arrived_and_from_where() {
        let pending = pending();
        assert!(
            contents(&pending).contains("artboard"),
            "{}",
            contents(&pending)
        );
        assert!(origin(&pending).contains("Figma"));
        assert!(
            ImportChoice::ALL
                .iter()
                .all(|choice| !choice.label().is_empty())
        );
    }

    #[test]
    fn replacing_is_offered_once_the_design_came_over_before() {
        let mut pending = pending();
        pending.mode = Some(RequestedMode::Replace);
        let session = SessionHandle::from_document(demo());
        let document = session.document();
        assert_eq!(matching_layers(&pending, &document), 0);
        assert_eq!(
            suggested_choice(&pending, &document),
            ImportChoice::NewArtboards
        );
        assert!(!is_possible(
            ImportChoice::ReplaceMatching,
            &pending,
            &document,
            None
        ));

        pending
            .apply(&session, &ImportTarget::NewArtboards)
            .unwrap();
        let document = session.document();
        assert!(matching_layers(&pending, &document) > 0);
        assert_eq!(
            suggested_choice(&pending, &document),
            ImportChoice::ReplaceMatching
        );
        assert!(is_possible(
            ImportChoice::IntoArtboard,
            &pending,
            &document,
            Some(&NodeId::from("scene"))
        ));
        assert!(!is_possible(
            ImportChoice::IntoArtboard,
            &pending,
            &document,
            None
        ));
        assert_eq!(
            session.read(|s| s.undo_steps().last().map(|step| step.origin)),
            Some(Origin::Import)
        );
    }
}
