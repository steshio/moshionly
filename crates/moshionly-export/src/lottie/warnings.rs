//! What could not be exported exactly, in plain words, naming the layer.
//!
//! A Lottie export never fails because one layer cannot be expressed. The
//! layer is drawn as closely as Lottie allows, or left out, and a warning
//! says which layer and what is different, for the Export dialog to show.

use moshionly_core::Node;

/// The warnings of one export, each said once, in the order they came up.
#[derive(Debug, Default)]
pub(crate) struct Warnings {
    list: Vec<String>,
}

impl Warnings {
    /// Notes that `node` is not exported exactly: `what` says how, as the
    /// rest of a sentence that starts with the layer's name.
    pub fn layer(&mut self, node: &Node, what: &str) {
        self.add(format!("{} {what}", name(node)));
    }

    fn add(&mut self, text: String) {
        if !self.list.contains(&text) {
            self.list.push(text);
        }
    }

    /// The warnings, in order.
    pub fn into_vec(self) -> Vec<String> {
        self.list
    }
}

/// How a warning names a layer: its name in quotes, or its kind and id
/// when it has none, with a capital letter to start a sentence.
pub(crate) fn name(node: &Node) -> String {
    match node.name() {
        "" => format!("The {} layer \"{}\"", node.node_type(), node.id),
        name => format!("\"{name}\""),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moshionly_core::model::{LayerProps, RectLayer};

    #[test]
    fn warnings_name_the_layer_and_come_once() {
        let mut node = Node::new(RectLayer {
            layer: LayerProps::new("Card", 0.0, 0.0, 10.0, 10.0),
            fill: None,
            stroke: None,
            corner_radius: Default::default(),
        })
        .with_id("ab12");
        let mut warnings = Warnings::default();
        warnings.layer(&node, "has a blur that is left out.");
        warnings.layer(&node, "has a blur that is left out.");
        node.data.layer_mut().unwrap().name.clear();
        warnings.layer(&node, "has a blur that is left out.");
        assert_eq!(
            warnings.into_vec(),
            [
                "\"Card\" has a blur that is left out.",
                "The rect layer \"ab12\" has a blur that is left out."
            ]
        );
    }
}
