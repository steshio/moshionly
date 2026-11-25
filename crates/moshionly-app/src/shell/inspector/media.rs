//! Video and sound in the inspector: the Video section of the Design tab,
//! and the settings of a sound in the Animate tab.
//!
//! Both show the file, when the clip plays (Start and End, in seconds on
//! the timeline), from where in the file (Trim start) and how loud
//! (Volume), with a way to play another file instead. Start moves the
//! clip and keeps its length, as on the timeline; End changes its length.

use egui::{RichText, Ui};
use moshionly_core::model::{ClipTiming, ImageFit, NodeData};
use moshionly_core::{Document, Edit, Node, NodeId, NodeType};
use serde_json::Value;

use super::{Bind, animate::several_actions};
use crate::{
    editor::{
        clips::{ClipDrag, clip_edit, dragged},
        inspect::{Inspected, Shared, set_all},
        timeline::{file_label, sound_name},
    },
    theme::{Theme, icons},
    timeline::Edge,
    widgets::{
        ButtonKind, Dropdown, IconButton, NumericField, NumericResponse, TextButton, control_row,
        divider, padded, property_row, section, section_header, slider_with_value_mixed,
    },
};

/// How a video fills its box, with the names the image section uses.
const FITS: [(ImageFit, &str); 3] = [
    (ImageFit::Cover, "Fill"),
    (ImageFit::Contain, "Fit"),
    (ImageFit::Stretch, "Stretch"),
];

/// Typed units for time fields: seconds, or milliseconds.
const SECONDS: &[(&str, f64)] = &[("ms", 0.001)];

/// A clip's field that a time field sets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ClipField {
    Start,
    End,
    TrimStart,
}

impl ClipField {
    fn summary(self) -> &'static str {
        match self {
            ClipField::Start => "Change start",
            ClipField::End => "Change end",
            ClipField::TrimStart => "Change trim",
        }
    }
}

/// The clips' times and volume across the selection.
struct ClipValues {
    start: Shared<f64>,
    end: Shared<f64>,
    trim_start: Shared<f64>,
    volume: Shared<f64>,
}

impl ClipValues {
    fn of(inspected: &Inspected<'_>) -> Option<Self> {
        let read = |field: fn(&ClipTiming) -> f64| {
            inspected.number(move |node| node.data.clip().map(field))
        };
        Some(Self {
            start: read(|clip| clip.start)?,
            end: read(|clip| clip.end)?,
            trim_start: read(|clip| clip.trim_start)?,
            volume: read(|clip| clip.volume)?,
        })
    }
}

/// What the clip rows did.
#[derive(Default)]
struct ClipChanges {
    times: Vec<(ClipField, NumericResponse, f64)>,
    volume: Option<(NumericResponse, f64)>,
}

/// The clip of `id` with `field` set to `ms`, by the timeline's rules:
/// Start moves the clip, End sets its length, and neither goes before 0.
fn with_time(clip: &ClipTiming, field: ClipField, ms: f64) -> ClipTiming {
    match field {
        ClipField::Start => dragged(clip, ClipDrag::Move, ms, ms + clip.duration()),
        ClipField::End => dragged(clip, ClipDrag::Trim(Edge::End), clip.start, ms),
        ClipField::TrimStart => ClipTiming {
            trim_start: ms.max(0.0),
            ..*clip
        },
    }
}

/// The edits that set `field` to `ms` on the clips of `ids` in `document`.
fn time_edits(document: &Document, ids: &[NodeId], field: ClipField, ms: f64) -> Vec<Edit> {
    ids.iter()
        .filter_map(|id| {
            let clip = document.get(id)?.data.clip()?;
            clip_edit(document, id, &with_time(clip, field, ms))
        })
        .collect()
}

/// A time field in seconds for a value shared by the selection. Returns
/// what it did and the time it holds, in whole milliseconds.
fn time_field(ui: &mut Ui, value: &Shared<f64>, name: &str) -> (NumericResponse, f64) {
    let mut seconds = value.value() / 1000.0;
    let response = NumericField::new(&mut seconds, name)
        .unit("s")
        .conversions(SECONDS)
        .range(0.0..=36_000.0)
        .step(0.05)
        .decimals(2)
        .mixed(value.is_mixed())
        .id_salt(("clip-time", name))
        .show(ui);
    (response, (seconds * 1000.0).round())
}

/// The rows for when the clips play and how loud.
fn clip_rows(ui: &mut Ui, values: &ClipValues) -> ClipChanges {
    let mut changes = ClipChanges::default();
    for (field, label, value) in [
        (ClipField::Start, "Start", &values.start),
        (ClipField::End, "End", &values.end),
        (ClipField::TrimStart, "Trim start", &values.trim_start),
    ] {
        let (response, ms) = property_row(ui, label, |ui| time_field(ui, value, label));
        changes.times.push((field, response, ms));
    }
    let mut percent = (values.volume.value() * 100.0).round();
    let response = property_row(ui, "Volume", |ui| {
        slider_with_value_mixed(
            ui,
            &mut percent,
            "Volume",
            0.0..=100.0,
            "%",
            values.volume.is_mixed(),
        )
    });
    changes.volume = Some((response, percent));
    changes
}

/// Applies what the clip rows did to the clips of `ids`, each a step of
/// its own.
fn apply_clip_changes(bind: &mut Bind<'_>, ids: &[NodeId], changes: ClipChanges) {
    let began = bind.began();
    for (field, response, ms) in changes.times {
        bind.field(&response, field.summary(), || {
            time_edits(&began, ids, field, ms)
        });
    }
    if let Some((response, percent)) = changes.volume {
        let volume = (percent.clamp(0.0, 100.0) * 10.0).round() / 1000.0;
        bind.field(&response, "Change volume", || {
            set_all(ids, "volume", volume)
        });
    }
}

/// The source of a video layer.
fn video_of(node: &Node) -> Option<&moshionly_core::model::VideoLayer> {
    match &node.data {
        NodeData::Video(video) => Some(video),
        _ => None,
    }
}

/// A row naming the file, with the whole source in its tooltip.
fn source_row(ui: &mut Ui, shown: &str, about: &str) {
    let theme = Theme::of(ui);
    property_row(ui, "Source", |ui| {
        ui.label(
            RichText::new(shown)
                .font(theme.body())
                .color(theme.color.text_secondary),
        )
        .on_hover_text(about);
    });
}

/// The Video section for the selected video layers: the file, how it
/// fills the box, when it plays, and another file instead.
pub(super) fn video_section(ui: &mut Ui, bind: &mut Bind<'_>, inspected: &Inspected<'_>) {
    let ids = inspected.ids_where(|node| node.node_type() == NodeType::Video);
    // Borrowed, not copied, like an image's source.
    let source = inspected.read(|node| video_of(node).map(|v| v.source.as_str()));
    let fit = inspected.read(|node| video_of(node).map(|v| v.fit));
    let (Some(source), Some(fit), Some(values)) = (source, fit, ClipValues::of(inspected)) else {
        return;
    };
    let (shown, about) = if source.is_mixed() {
        (
            "Several videos".to_owned(),
            "The selected layers play different videos.".to_owned(),
        )
    } else {
        (file_label(source.value()), (*source.value()).to_owned())
    };
    let mut fit_value = *fit.value();
    let mut fitted = false;
    let mut replace = false;
    let mut changes = ClipChanges::default();
    let mut open = bind.state.open.media;
    section(
        ui,
        "Video",
        &mut open,
        |_| {},
        |ui| {
            source_row(ui, &shown, &about);
            property_row(ui, "Fill", |ui| {
                fitted = Dropdown::new(&mut fit_value, "Video fill", &FITS)
                    .mixed(fit.is_mixed())
                    .show(ui)
                    .changed();
            });
            changes = clip_rows(ui, &values);
            control_row(ui, |ui| {
                replace = ui
                    .add(
                        TextButton::new("Replace video…")
                            .kind(ButtonKind::Quiet)
                            .icon(icons::FILM_STRIP),
                    )
                    .clicked();
            });
        },
    );
    bind.state.open.media = open;
    if fitted {
        let value = serde_json::to_value(fit_value).unwrap_or(Value::Null);
        bind.apply("Change video fill", &set_all(&ids, "fit", value));
    }
    apply_clip_changes(bind, &ids, changes);
    if replace {
        bind.out.replace_media = Some(ids);
    }
}

/// The settings of one sound, `node`, in the Animate tab: its file, when
/// it plays and how loud, with buttons to delete or duplicate it.
pub(super) fn sound_settings(ui: &mut Ui, bind: &mut Bind<'_>, node: &Node) {
    let theme = Theme::of(ui);
    divider(ui);
    let mut duplicate = false;
    let mut delete = false;
    section_header(ui, &sound_name(node), None, |ui| {
        delete = ui
            .add(IconButton::new(icons::TRASH).tooltip("Delete sound"))
            .clicked();
        duplicate = ui
            .add(IconButton::new(icons::COPY).tooltip("Duplicate sound"))
            .clicked();
    });
    let document = std::sync::Arc::clone(bind.editor.document());
    let ids = [node.id.clone()];
    let sound = Inspected::new(&document, &ids);
    let Some(values) = ClipValues::of(&sound) else {
        return;
    };
    let source = node.data.source().unwrap_or_default();
    let mut replace = false;
    let mut changes = ClipChanges::default();
    padded(ui, |ui| {
        source_row(ui, &file_label(source), source);
        changes = clip_rows(ui, &values);
        control_row(ui, |ui| {
            replace = ui
                .add(
                    TextButton::new("Replace sound…")
                        .kind(ButtonKind::Quiet)
                        .icon(icons::SPEAKER_HIGH),
                )
                .clicked();
        });
    });
    ui.add_space(theme.space.md);
    apply_clip_changes(bind, &ids, changes);
    if replace {
        bind.out.replace_media = Some(ids.to_vec());
    }
    let result = if delete {
        bind.editor.delete_actions().map(drop)
    } else if duplicate {
        bind.editor.duplicate_actions().map(drop)
    } else {
        Ok(())
    };
    if let Err(error) = result {
        bind.out.error = Some(error.to_string());
    }
}

/// The Animate tab when only sound is selected, which belongs to no layer:
/// the settings of one sound, or what several can do together.
pub(super) fn sound_only(ui: &mut Ui, bind: &mut Bind<'_>) {
    let document = std::sync::Arc::clone(bind.editor.document());
    let selected = bind.editor.actions().to_vec();
    match selected.as_slice() {
        [one] => {
            if let Some(node) = document.get(one).cloned() {
                sound_settings(ui, bind, &node);
            }
        }
        many => several_actions(ui, bind, &format!("{} sounds", many.len())),
    }
}

#[cfg(test)]
mod tests {
    use moshionly_core::io;

    use super::*;

    fn document() -> Document {
        io::parse(
            r##"{"format":"moshionly","version":3,"name":"Clips","nodes":[
            {"id":"ab","parent":null,"index":"a0","type":"artboard","duration":5000},
            {"id":"a","parent":"ab","index":"a0","type":"video","source":"a.mp4",
             "start":1000,"end":3000,"trim_start":500},
            {"id":"b","parent":"ab","index":"a1","type":"video","source":"b.mp4",
             "start":0,"end":500}
            ]}"##,
        )
        .unwrap()
    }

    fn clip(document: &Document, id: &str) -> ClipTiming {
        *document
            .get(&NodeId::from(id))
            .unwrap()
            .data
            .clip()
            .unwrap()
    }

    #[test]
    fn start_moves_each_clip_and_end_sets_its_length() {
        let doc = document();
        let ids = [NodeId::from("a"), NodeId::from("b")];
        let mut moved = doc.clone();
        moved
            .apply(&time_edits(&doc, &ids, ClipField::Start, 2000.0))
            .unwrap();
        assert_eq!(
            clip(&moved, "a"),
            ClipTiming {
                trim_start: 500.0,
                ..ClipTiming::new(2000.0, 4000.0)
            }
        );
        assert_eq!(clip(&moved, "b"), ClipTiming::new(2000.0, 2500.0));
        let mut ended = doc.clone();
        ended
            .apply(&time_edits(&doc, &ids, ClipField::End, 2500.0))
            .unwrap();
        assert_eq!(clip(&ended, "a").end, 2500.0);
        assert_eq!(clip(&ended, "b").end, 2500.0);
        // An end before the start leaves the shortest clip.
        let short = time_edits(&doc, &ids[..1], ClipField::End, 0.0);
        let mut shortened = doc.clone();
        shortened.apply(&short).unwrap();
        assert!(clip(&shortened, "a").end > 1000.0);
        let trimmed = time_edits(&doc, &ids[..1], ClipField::TrimStart, 1200.0);
        let mut later = doc.clone();
        later.apply(&trimmed).unwrap();
        assert_eq!(clip(&later, "a").trim_start, 1200.0);
        // Nothing changes, nothing is edited.
        assert!(time_edits(&doc, &ids[..1], ClipField::Start, 1000.0).is_empty());
    }

    #[test]
    fn several_clips_show_what_they_share() {
        let doc = document();
        let ids = [NodeId::from("a"), NodeId::from("b")];
        let inspected = Inspected::new(&doc, &ids);
        let values = ClipValues::of(&inspected).unwrap();
        assert!(values.start.is_mixed());
        assert_eq!(values.volume, Shared::Same(1.0));
        assert_eq!(*values.trim_start.value(), 500.0);
        let none = Inspected::new(&doc, &[NodeId::from("ab")]);
        assert!(ClipValues::of(&none).is_none());
    }
}
