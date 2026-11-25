//! The guide: what a model needs to know to work well in a Moshionly file.
//!
//! The built-in chat sends [`system_prompt`] as its system prompt, with a
//! line about what the person has selected. The MCP server sends [`GUIDE`]
//! as its instructions, so external agents read the same advice.

use std::fmt::Write as _;

use moshionly_core::eval::duration_of;
use moshionly_core::{Document, NodeId};

use crate::tools::number_text;

/// How to work in a Moshionly file, written for a model.
pub const GUIDE: &str = "\
You are working in Moshionly, a motion design app, on the file the person has open. You read \
and change it only through the tools. Every apply_edits call is one step the person can undo.

# How a file is built
- A file holds artboards: frames on the canvas, each with a size and a timeline whose length \
is its duration in milliseconds.
- Layers are what is drawn: group, mask_group, rect, ellipse, star, path, text, image, \
component, instance. A layer's x and y are its top-left corner inside its parent (artboard or \
group); rotation and scale turn and size it around its centre. Later siblings are drawn in front \
of earlier ones.
- A component is a group that is reused: every instance of it shows its look and layers, with \
the animations on those layers, and changes when the component changes. Make one with \
make_component (a group, or layers to wrap) and place copies with add \
{\"type\":\"instance\",\"component\":<id>}, which can sit in any artboard. An instance's \
time_offset plays the component's animations that many milliseconds later: give each copy a \
little more to stagger them. Animate a layer inside the component to animate every copy; \
animate the instance itself (move, fade_in and so on) for that copy alone. detach turns an \
instance into a plain group to change one copy on its own. Resizing a component resizes the \
instances that were its size with it.
- The layers as they are stored are the design at rest. Motion is kept apart from it: an \
animation is a node that says \"from start to end, change this property of that layer\" (its \
target). There are no keyframes. Animations sit in rows on their artboard's timeline; an \
animation_group is a folder of rows.
- Presets are whole entrances and exits in one node: fade_in, slide_in, grow_in, shrink_in, \
spin_in, blur_in, reveal_in, their _out partners, and text_in and text_out for text letter by \
letter or word by word. Prefer them. Property animations (move, rotate, scale, opacity, resize, \
fill, stroke, corner_radius, blur, shadow) take one property from a value to another; counter \
counts a number in a text; show and hide switch a layer on or off at one moment.

# How animations on one property combine
They are taken in order of start. move and rotate add up: each adds its own change to the \
resting value, and a missing \"from\" means no change yet. Every other kind replaces: the \
property stays at rest until the first animation starts, and from then on the most recently \
started one decides; a missing \"from\" means wherever the property is when it starts, and a \
missing \"to\" means back to rest. When the first animation has a \"from\", the layer holds \
that value before it starts too, which is how a fade_in at 800 ms keeps its layer invisible \
until then. scale values multiply the resting scale; opacity, sizes, colours, radii and blur are \
absolute.

# Units
Pixels; degrees, clockwise; milliseconds from the start of the artboard; opacity 0 to 100; \
colours \"#RRGGBB\" or \"#RRGGBBAA\". An easing is {\"type\": ..., \"intensity\": 0 to 100}: \
smooth eases in and out, slow_down brakes into the end, accelerate speeds away, natural starts \
quickly and lands softly, overshoot goes past the end and settles back, elastic wobbles, bounce \
bounces off the end, linear keeps one speed, and bezier takes x1, y1, x2, y2 as in CSS.

# How to work
1. Look before you edit. Call get_overview, then query_nodes with a narrow filter (an \
artboard, a parent, a type or a name) asking only for the fields you need, and get_node for a \
layer you are about to change and what already animates it. Use ids, never names. Look a type up \
with get_schema before you first add or change one.
2. Make one change per apply_edits call, with all its operations together, and refer to what \
you add in the same call by temp_id. Read the warnings in the result.
3. Check your work. After a visual change take a screenshot, at rest or at the moments that \
matter, and fix what looks wrong: overlaps, text spilling out of its box, layers cut off by the \
artboard's edge, poor contrast. Use evaluate to check timing and values exactly.
4. Keep the person's work. Change only what was asked: do not remove, rename, restyle or retime \
existing layers and animations unless asked. When a layer is already animated, fit new motion \
around what is there. The person may edit while you work, so read again before editing from old \
results. If a call of yours went wrong, undo_last_agent_change takes it back.

# Motion that feels good
- Durations: small elements and quick feedback 150 to 300 ms; most entrances, exits and moves \
300 to 600 ms; large or full-screen movement 600 to 1000 ms. Exits a little quicker than \
entrances.
- Easing: what arrives slows down (slow_down or natural), what leaves speeds up (accelerate), \
what travels from one place to another eases both ways (smooth). Keep overshoot, elastic and \
bounce for playful moments, and linear for steady motion such as a spin or a scroll.
- Distance: slides of 20 to 80 px read as polish; sliding the full width reads as a change of \
scene.
- Stagger related elements by 40 to 120 ms in reading order instead of moving them all at once \
(clone with time_offset, add animations with stepped starts, or give instances of one animated \
component stepped time_offset values).
- Give the eye time: let the main element land before supporting ones move, hold the finished \
composition for a second or more, and make sure the artboard's duration covers every animation.

# In the chat
Be brief. Say in a sentence or two what you changed and anything the person should look at; do \
not paste ids, JSON or tool results unless asked. Ask before large or destructive changes, and \
ask one short question when a request is unclear.
";

/// What the person is looking at in the editor.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Focus<'a> {
    /// The artboard in view.
    pub artboard: Option<&'a NodeId>,
    /// The selected nodes, in the order they were selected.
    pub selection: &'a [NodeId],
    /// Where the playhead is, in milliseconds.
    pub time_ms: Option<f64>,
}

/// The most selected nodes the prompt names.
const MAX_NAMED: usize = 12;

/// The guide, with a short description of what the person is looking at
/// when `focus` says.
///
/// ```
/// use moshionly_agent::guide::{Focus, system_prompt};
/// use moshionly_core::{Document, build};
///
/// let document = Document::new("Hello");
/// let (artboard, edits) = build::add_artboard(&document, "Scene", 800.0, 600.0)?;
/// let mut document = document;
/// document.apply(&edits)?;
///
/// let focus = Focus { artboard: Some(&artboard), ..Focus::default() };
/// let prompt = system_prompt(&document, Some(&focus));
/// assert!(prompt.starts_with("You are working in Moshionly"));
/// assert!(prompt.contains("looking at artboard \"Scene\""));
/// assert!(prompt.contains("800 by 600 px, 4000 ms long. Nothing is selected."));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn system_prompt(document: &Document, focus: Option<&Focus<'_>>) -> String {
    let mut prompt = GUIDE.to_owned();
    if let Some(focus) = focus {
        let description = describe(document, focus);
        if !description.is_empty() {
            prompt.push_str("\n# Right now\n");
            prompt.push_str(&description);
            prompt.push('\n');
        }
    }
    prompt
}

/// One or two sentences about the artboard in view, the playhead and the
/// selection.
fn describe(document: &Document, focus: &Focus<'_>) -> String {
    let mut text = String::new();
    if let Some(node) = focus.artboard.and_then(|id| document.get(id))
        && let Some(layer) = node.layer()
    {
        let name = if node.name().is_empty() {
            "an artboard".to_owned()
        } else {
            format!("artboard \"{}\"", node.name())
        };
        let _ = write!(
            text,
            "The person is looking at {name} (id \"{}\"), {} by {} px",
            node.id,
            number_text(layer.width),
            number_text(layer.height)
        );
        if let Some(duration) = duration_of(document, &node.id) {
            let _ = write!(text, ", {} ms long", number_text(duration));
        }
        if let Some(time) = focus.time_ms {
            let _ = write!(text, ", with the playhead at {} ms", number_text(time));
        }
        text.push('.');
    }
    let selected: Vec<String> = focus
        .selection
        .iter()
        .filter_map(|id| document.get(id))
        .map(|node| node.label())
        .collect();
    if !text.is_empty() {
        text.push(' ');
    }
    match selected.len() {
        0 if text.is_empty() => {}
        0 => text.push_str("Nothing is selected."),
        count => {
            let named = selected
                .iter()
                .take(MAX_NAMED)
                .cloned()
                .collect::<Vec<_>>()
                .join(", ");
            let more = if count > MAX_NAMED {
                format!(" and {} more", count - MAX_NAMED)
            } else {
                String::new()
            };
            let _ = write!(
                text,
                "Selected: {named}{more}. \"This\" and \"these\" in the person's words most likely \
                 mean the selection."
            );
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::testing::demo_document;

    #[test]
    fn the_guide_covers_what_a_model_needs() {
        for topic in [
            "get_overview",
            "query_nodes",
            "temp_id",
            "screenshot",
            "evaluate",
            "undo_last_agent_change",
            "add up",
            "replaces",
            "milliseconds",
            "Stagger",
            "make_component",
            "time_offset",
            "detach",
            "Be brief",
        ] {
            assert!(GUIDE.contains(topic), "the guide does not mention {topic}");
        }
        assert_eq!(system_prompt(&demo_document(), None), GUIDE);
    }

    #[test]
    fn the_prompt_describes_the_artboard_time_and_selection() {
        let document = demo_document();
        let scene = NodeId::from("scene");
        let selection = [
            NodeId::from("title"),
            NodeId::from("gone"),
            NodeId::from("badge"),
        ];
        let prompt = system_prompt(
            &document,
            Some(&Focus {
                artboard: Some(&scene),
                selection: &selection,
                time_ms: Some(1250.5),
            }),
        );
        let tail = prompt.strip_prefix(GUIDE).unwrap();
        assert_eq!(
            tail,
            "\n# Right now\nThe person is looking at artboard \"Launch\" (id \"scene\"), 1080 by \
             1080 px, 4000 ms long, with the playhead at 1250.5 ms. Selected: text \"title\" \
             (Title), rect \"badge\" (Badge). \"This\" and \"these\" in the person's words most \
             likely mean the selection.\n"
        );
    }

    #[test]
    fn a_long_selection_is_shortened_and_an_empty_focus_adds_nothing() {
        let document = demo_document();
        let many: Vec<NodeId> = (0..20).map(|_| NodeId::from("dot")).collect();
        let prompt = system_prompt(
            &document,
            Some(&Focus {
                selection: &many,
                ..Focus::default()
            }),
        );
        assert!(prompt.contains("and 8 more."), "{prompt}");
        assert_eq!(system_prompt(&document, Some(&Focus::default())), GUIDE);
    }
}
