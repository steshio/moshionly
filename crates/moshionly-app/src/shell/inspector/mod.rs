//! The inspector on the right of the window: Design and Animate.
//!
//! The Design tab edits how the selected layers look at rest. Every change
//! goes through the editor as an edit; a value scrubbed or typed into a
//! field, or a colour dragged in a picker, is one undo step. Values that
//! differ across the selection show as mixed, and setting one sets it on
//! every selected layer that has it.

mod action;
mod animate;
mod design;
mod easing;
mod text;
mod values;

use egui::{RichText, ScrollArea, Ui};
use moshionly_core::Edit;

use crate::{
    editor::{
        Editor,
        describe::{display_name, type_label},
        inspect::{Inspected, Shared},
    },
    theme::{Theme, icons},
    widgets::{
        ColorResponse, NumericField, NumericResponse, divider, empty_state, padded, section_header,
        segmented,
    },
};

/// The two faces of the inspector.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum InspectorTab {
    /// How the selected layer looks at rest.
    #[default]
    Design,
    /// What the selected layer does over time.
    Animate,
}

/// What the inspector remembers: the tab, which sections are open and the
/// field being edited.
#[derive(Clone, Debug)]
pub struct InspectorState {
    /// The tab showing.
    pub tab: InspectorTab,
    /// Changing the width changes the height in proportion, and back.
    pub keep_proportions: bool,
    open: Sections,
    /// A field's change is being recorded as one undo step, for the
    /// layers selected when it began.
    editing: Option<Vec<moshionly_core::NodeId>>,
    /// The font families the renderer can draw, looked up once.
    families: Vec<moshionly_render::FontFamily>,
    /// Milliseconds between actions added to several layers at once, and
    /// between actions spaced out one after another.
    stagger: f64,
}

impl InspectorState {
    /// Milliseconds between actions spaced out one after another.
    pub fn stagger(&self) -> f64 {
        self.stagger
    }
}

impl Default for InspectorState {
    fn default() -> Self {
        Self {
            tab: InspectorTab::default(),
            keep_proportions: false,
            open: Sections::default(),
            editing: None,
            families: Vec::new(),
            stagger: 100.0,
        }
    }
}

/// Which sections of the inspector are open.
#[derive(Clone, Copy, Debug)]
struct Sections {
    artboard: bool,
    layout: bool,
    appearance: bool,
    fill: bool,
    stroke: bool,
    effects: bool,
    text: bool,
    image: bool,
    shape: bool,
    actions: bool,
    easing: bool,
    values: bool,
    settings: bool,
}

impl Default for Sections {
    fn default() -> Self {
        Self {
            artboard: true,
            layout: true,
            appearance: true,
            fill: true,
            stroke: true,
            effects: true,
            text: true,
            image: true,
            shape: true,
            actions: true,
            easing: true,
            values: true,
            settings: true,
        }
    }
}

/// What happened in the inspector.
#[derive(Debug, Default)]
pub(super) struct InspectorOutput {
    /// An edit was refused: why, in a few words.
    pub error: Option<String>,
    /// The person asked to choose another image for the selected images.
    pub replace_image: bool,
}

/// The inspector's view of the editor for one frame.
pub(super) struct Bind<'a> {
    editor: &'a mut Editor,
    state: &'a mut InspectorState,
    out: &'a mut InspectorOutput,
}

impl Bind<'_> {
    /// Applies a change that may come in many steps (scrubbing a field,
    /// dragging in a colour picker). The first change opens an undo step
    /// that the end of the edit closes.
    fn continuous(
        &mut self,
        changed: bool,
        finished: bool,
        summary: &str,
        edits: impl FnOnce() -> Vec<Edit>,
    ) {
        if changed {
            if self.state.editing.is_none() {
                self.editor.begin_gesture(summary);
                self.state.editing = Some(self.editor.selection.ids().to_vec());
            }
            self.apply(summary, &edits());
        }
        if finished && self.state.editing.take().is_some() {
            self.editor.commit_gesture();
        }
    }

    /// Applies what a numeric field did, as one undo step.
    fn field(
        &mut self,
        response: &NumericResponse,
        summary: &str,
        edits: impl FnOnce() -> Vec<Edit>,
    ) {
        self.continuous(
            response.response.changed(),
            response.finished,
            summary,
            edits,
        );
    }

    /// Applies what a colour field did, as one undo step.
    fn color(
        &mut self,
        response: &ColorResponse,
        summary: &str,
        edits: impl FnOnce() -> Vec<Edit>,
    ) {
        self.continuous(response.changed, response.finished, summary, edits);
    }

    /// Applies edits as one undo step, keeping the reason when they are
    /// refused.
    fn apply(&mut self, summary: &str, edits: &[Edit]) {
        if edits.is_empty() {
            return;
        }
        if let Err(error) = self.editor.apply(summary, edits) {
            self.out.error = Some(error.problem.message);
        }
    }
}

/// Shows a numeric field for a value shared by the selection, set up by
/// `configure`. Returns what it did and the value it holds now.
fn numeric(
    ui: &mut Ui,
    value: &Shared<f64>,
    name: &str,
    configure: impl FnOnce(NumericField<'_>) -> NumericField<'_>,
) -> (NumericResponse, f64) {
    let mut number = *value.value();
    let field = NumericField::new(&mut number, name)
        .mixed(value.is_mixed())
        .id_salt(name);
    let response = configure(field).show(ui);
    (response, number)
}

/// Lets go of an edit the inspector began that can no longer finish
/// properly, so that later changes are not recorded into it: when
/// something else closed its undo step (an undo, or the canvas starting a
/// gesture of its own, which `canvas_gesture` says), or the selection
/// changed under it (the field that began it now edits other layers, or is
/// gone), in which case its step is closed here.
fn end_stale_edit(state: &mut InspectorState, editor: &mut Editor, canvas_gesture: bool) {
    let Some(began) = &state.editing else {
        return;
    };
    if !editor.in_gesture() || canvas_gesture {
        state.editing = None;
    } else if began.as_slice() != editor.selection.ids() {
        state.editing = None;
        editor.commit_gesture();
    }
}

/// Draws the inspector for the selection. `canvas_gesture` says that the
/// canvas holds an undo step open (a drag, or text being added).
pub(super) fn inspector(
    ui: &mut Ui,
    state: &mut InspectorState,
    editor: &mut Editor,
    canvas_gesture: bool,
) -> InspectorOutput {
    let theme = Theme::of(ui);
    let mut out = InspectorOutput::default();
    end_stale_edit(state, editor, canvas_gesture);

    ui.add_space(theme.space.md);
    padded(ui, |ui| {
        segmented(
            ui,
            &mut state.tab,
            &[
                (InspectorTab::Design, "Design"),
                (InspectorTab::Animate, "Animate"),
            ],
        );
    });
    ui.add_space(theme.space.md);
    divider(ui);

    let document = std::sync::Arc::clone(editor.document());
    let ids = editor.selection.ids().to_vec();
    let Some(first) = ids.first().and_then(|id| document.get(id)) else {
        empty_state(
            ui,
            icons::CURSOR,
            "Nothing selected",
            "Select a layer on the canvas or in the list to edit it.",
            None,
        );
        return out;
    };
    let title = if ids.len() == 1 {
        display_name(first)
    } else {
        format!("{} layers", ids.len())
    };
    section_header(ui, &title, None, |ui| {
        if ids.len() == 1 {
            ui.label(
                RichText::new(type_label(first.node_type()))
                    .font(theme.small())
                    .color(theme.color.text_tertiary),
            );
        }
    });

    ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let mut bind = Bind {
                editor,
                state,
                out: &mut out,
            };
            let inspected = Inspected::new(&document, &ids);
            match bind.state.tab {
                InspectorTab::Design => design::design_tab(ui, &mut bind, &inspected),
                InspectorTab::Animate => animate::animate_tab(ui, &mut bind, &inspected),
            }
            divider(ui);
            ui.add_space(theme.space.xl);
        });
    out
}

#[cfg(test)]
mod tests {
    use moshionly_core::NodeId;

    use super::*;
    use crate::editor::view::tests::editor;

    fn id(text: &str) -> NodeId {
        NodeId::from(text)
    }

    #[test]
    fn an_edit_left_open_is_closed_when_the_selection_changes() {
        let mut editor = editor();
        let mut state = InspectorState::default();
        editor.selection.select(id("loose"));
        editor.begin_gesture("Change opacity");
        state.editing = Some(editor.selection.ids().to_vec());
        editor
            .apply("Change opacity", &[Edit::set("loose", "opacity", 50.0)])
            .unwrap();
        // The same selection: the edit goes on.
        end_stale_edit(&mut state, &mut editor, false);
        assert!(state.editing.is_some() && editor.in_gesture());
        // Another selection: the step is closed and forgotten.
        editor.selection.select(id("card"));
        end_stale_edit(&mut state, &mut editor, false);
        assert!(state.editing.is_none());
        assert!(!editor.in_gesture());
        assert_eq!(editor.undo_label().as_deref(), Some("Undo Change opacity"));
    }

    #[test]
    fn an_edit_closed_elsewhere_is_forgotten() {
        let mut editor = editor();
        let mut state = InspectorState::default();
        editor.selection.select(id("loose"));
        // An undo closed the step.
        state.editing = Some(editor.selection.ids().to_vec());
        end_stale_edit(&mut state, &mut editor, false);
        assert!(state.editing.is_none());
        // A drag on the canvas opened a step of its own: it is left alone.
        state.editing = Some(vec![id("card")]);
        editor.begin_gesture("Move");
        end_stale_edit(&mut state, &mut editor, true);
        assert!(state.editing.is_none());
        assert!(editor.in_gesture(), "the canvas's step stays open");
    }
}
