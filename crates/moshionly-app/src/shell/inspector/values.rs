//! The From and To of the selected action.
//!
//! Both are optional in the file. A missing `from` means "the value the
//! layer has when the action starts", and a missing `to` means "the value
//! from the Design tab", so the panel shows a button naming that value
//! instead of numbers. Clicking it writes the value down, ready to change;
//! the × next to a written value goes back to leaving it out.
//!
//! Each action type's value is made of parts (X and Y of a move, colour and
//! width of a stroke), listed by [`parts`]. The value as the file has it is
//! JSON, and the parts are read from and written into that JSON. Some types
//! let a written value leave parts out (a resize may change only the
//! width): such a part shows, in quiet ink, the value it stands for, and
//! a written one has its own × to leave it out again.

use egui::{Align, Layout, Ui};
use moshionly_core::eval::{Property, PropertyValue, value_at};
use moshionly_core::text::find_number;
use moshionly_core::{Color, Document, Edit, Node, NodeType};
use serde_json::{Value, json};

use super::Bind;
use crate::{
    theme::{Theme, icons},
    widgets::{
        ColorField, ColorResponse, IconButton, NumericField, NumericResponse, TextButton,
        numeric::format_value, pair, property_row, section,
    },
};

use super::design::{hex, rgba};

/// The two ends of an action's change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Slot {
    /// The value at the start.
    From,
    /// The value at the end.
    To,
}

impl Slot {
    /// The field in the file.
    pub(super) fn key(self) -> &'static str {
        match self {
            Self::From => "from",
            Self::To => "to",
        }
    }

    /// The label of its row.
    fn label(self) -> &'static str {
        match self {
            Self::From => "From",
            Self::To => "To",
        }
    }

    /// What a missing value means, as the button that writes it down says.
    pub(super) fn implicit(self) -> &'static str {
        match self {
            Self::From => "Current value",
            Self::To => "Design value",
        }
    }
}

/// One editable part of a value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Part {
    /// A number at `key` in the value (the whole value when `None`),
    /// shown multiplied by `scale`.
    Number {
        /// Where it is in the value.
        key: Option<&'static str>,
        /// The label.
        label: &'static str,
        /// The unit shown after it.
        unit: &'static str,
        /// What the stored number is multiplied by to be shown.
        scale: f64,
        /// The lowest number shown.
        min: f64,
        /// The highest number shown.
        max: f64,
    },
    /// A colour at `key` in the value (the whole value when `None`).
    Color {
        /// Where it is in the value.
        key: Option<&'static str>,
        /// The label.
        label: &'static str,
    },
}

const fn number(
    key: Option<&'static str>,
    label: &'static str,
    unit: &'static str,
    min: f64,
    max: f64,
) -> Part {
    Part::Number {
        key,
        label,
        unit,
        scale: 1.0,
        min,
        max,
    }
}

const BIG: f64 = 100_000.0;

/// The parts of the from and to values of an action type, or none for
/// types that have no from and to.
pub(super) fn parts(ty: NodeType) -> &'static [Part] {
    use NodeType as T;
    const MOVE: [Part; 2] = [
        number(Some("x"), "X", "px", -BIG, BIG),
        number(Some("y"), "Y", "px", -BIG, BIG),
    ];
    const ROTATE: [Part; 1] = [number(None, "Angle", "°", -BIG, BIG)];
    const SCALE: [Part; 1] = [Part::Number {
        key: None,
        label: "Scale",
        unit: "%",
        scale: 100.0,
        min: 0.0,
        max: BIG,
    }];
    const OPACITY: [Part; 1] = [number(None, "Opacity", "%", 0.0, 100.0)];
    const RESIZE: [Part; 2] = [
        number(Some("width"), "W", "px", 0.0, BIG),
        number(Some("height"), "H", "px", 0.0, BIG),
    ];
    const FILL: [Part; 1] = [Part::Color {
        key: None,
        label: "Fill",
    }];
    const STROKE: [Part; 2] = [
        Part::Color {
            key: Some("paint"),
            label: "Colour",
        },
        number(Some("width"), "Width", "px", 0.0, BIG),
    ];
    const RADIUS: [Part; 1] = [number(None, "Radius", "px", 0.0, BIG)];
    const BLUR: [Part; 1] = [number(None, "Blur", "px", 0.0, 1000.0)];
    const SHADOW: [Part; 4] = [
        Part::Color {
            key: Some("color"),
            label: "Colour",
        },
        number(Some("x"), "X", "px", -BIG, BIG),
        number(Some("y"), "Y", "px", -BIG, BIG),
        number(Some("blur"), "Blur", "px", 0.0, 1000.0),
    ];
    const COUNTER: [Part; 1] = [number(None, "Number", "", -1e12, 1e12)];
    match ty {
        T::Move => &MOVE,
        T::Rotate => &ROTATE,
        T::Scale => &SCALE,
        T::Opacity => &OPACITY,
        T::Resize => &RESIZE,
        T::Fill => &FILL,
        T::Stroke => &STROKE,
        T::CornerRadius => &RADIUS,
        T::Blur => &BLUR,
        T::Shadow => &SHADOW,
        T::Counter => &COUNTER,
        _ => &[],
    }
}

/// Whether a written value of an action type may leave parts out, each
/// then standing for the layer's own value. The parts of a move may not:
/// a missing one is no offset, which is what 0 shows.
pub(super) fn parts_optional(ty: NodeType) -> bool {
    matches!(ty, NodeType::Resize | NodeType::Stroke | NodeType::Shadow)
}

/// `value` with the part at `key` left out, as an edit writes it: an edit
/// merges an object into the one there, so the part is set to `null`,
/// which removes it. With no part left the whole value is `null`, which
/// means the same: every part is the layer's own.
pub(super) fn without_part(value: &Value, key: &str) -> Value {
    let mut object = value.as_object().cloned().unwrap_or_default();
    object.remove(key);
    if object.values().all(Value::is_null) {
        return Value::Null;
    }
    object.insert(key.to_owned(), Value::Null);
    Value::Object(object)
}

/// The part of `value` at `key`: the value itself for `None`.
fn at<'v>(value: &'v Value, key: Option<&str>) -> &'v Value {
    match key {
        Some(key) => &value[key],
        None => value,
    }
}

/// The number a part shows, before scaling. A radius given per corner shows
/// its first corner.
pub(super) fn read_number(value: &Value, key: Option<&str>) -> Option<f64> {
    let part = at(value, key);
    part.as_f64().or_else(|| part.as_array()?.first()?.as_f64())
}

/// The colour a part shows. `None` for a gradient, which this panel does
/// not edit.
pub(super) fn read_color(value: &Value, key: Option<&str>) -> Option<Color> {
    Color::parse(at(value, key).as_str()?).ok()
}

/// `value` with the part at `key` replaced by `part`.
pub(super) fn with_part(value: &Value, key: Option<&str>, part: Value) -> Value {
    match key {
        None => part,
        Some(key) => {
            let mut object = value.as_object().cloned().unwrap_or_default();
            object.insert(key.to_owned(), part);
            Value::Object(object)
        }
    }
}

/// The value an action's missing `from` or `to` stands for, written out:
/// what the layer has at the action's start (for `from`) or at rest with
/// the action done (for `to`), in the action's own terms.
pub(super) fn resolved(document: &Document, node: &Node, slot: Slot) -> Option<Value> {
    let ty = node.node_type();
    let target = node.target()?;
    let (start, end) = node.data.span()?;
    let time = match slot {
        Slot::From => start,
        Slot::To => end,
    };
    let get = |property| value_at(document, target, property, time);
    let number = |property| match get(property) {
        Some(PropertyValue::Number(n)) => Some(n),
        _ => None,
    };
    Some(match ty {
        // Moves and turns add to the design: nothing added is zero.
        NodeType::Move => json!({ "x": 0.0, "y": 0.0 }),
        NodeType::Rotate => json!(0.0),
        NodeType::Scale => {
            let rest = document.get(target)?.layer()?.scale;
            let now = number(Property::Scale)?;
            json!(if rest == 0.0 { 1.0 } else { now / rest })
        }
        NodeType::Opacity => json!(number(Property::Opacity)?),
        NodeType::Blur => json!(number(Property::Blur)?),
        NodeType::Resize => json!({
            "width": number(Property::Width)?,
            "height": number(Property::Height)?,
        }),
        NodeType::Fill => match get(Property::Fill)? {
            PropertyValue::Paint(Some(paint)) => serde_json::to_value(paint).ok()?,
            _ => json!("#00000000"),
        },
        NodeType::Stroke => match get(Property::Stroke)? {
            PropertyValue::Stroke(Some(stroke)) => json!({
                "paint": serde_json::to_value(&stroke.paint).ok()?,
                "width": stroke.width,
            }),
            _ => json!({ "paint": "#00000000", "width": 0.0 }),
        },
        NodeType::CornerRadius => match get(Property::CornerRadius)? {
            PropertyValue::CornerRadius(radius) => serde_json::to_value(radius).ok()?,
            _ => return None,
        },
        NodeType::Shadow => match get(Property::Shadow)? {
            PropertyValue::Shadow(Some(shadow)) => json!({
                "color": shadow.color.to_hex(),
                "x": shadow.x,
                "y": shadow.y,
                "blur": shadow.blur,
            }),
            _ => json!({ "color": "#00000000", "x": 0.0, "y": 0.0, "blur": 0.0 }),
        },
        NodeType::Counter => match get(Property::Content)? {
            PropertyValue::Text(text) => json!(find_number(&text).map_or(0.0, |n| n.value)),
            _ => return None,
        },
        _ => return None,
    })
}

/// What happened to one value in a frame.
enum Change {
    /// The value was written down or left out: one step.
    Set(Value),
    /// A number field moved, perhaps over many frames.
    Field(NumericResponse, Value),
    /// A colour field moved, perhaps over many frames.
    Color(ColorResponse, Value),
}

/// The From and To section of the selected action `node`, for the types
/// that have them.
pub(super) fn values_section(ui: &mut Ui, bind: &mut Bind<'_>, node: &Node) {
    let ty = node.node_type();
    let parts = parts(ty);
    if parts.is_empty() {
        return;
    }
    let document = std::sync::Arc::clone(bind.editor.document());
    let current = node.to_value();
    let mut open = bind.state.open.values;
    let mut changes: Vec<(Slot, Change)> = Vec::new();
    section(
        ui,
        "Values",
        &mut open,
        |_| {},
        |ui| {
            for slot in [Slot::From, Slot::To] {
                let stored = current.get(slot.key()).filter(|v| !v.is_null());
                let change = match stored {
                    None => implicit_row(ui, slot).then(|| {
                        let value = resolved(&document, node, slot).unwrap_or(Value::Null);
                        Change::Set(value)
                    }),
                    Some(value) => {
                        let implied = resolved(&document, node, slot);
                        written_rows(ui, node, slot, parts, value, implied.as_ref())
                    }
                };
                if let Some(change) = change {
                    changes.push((slot, change));
                }
            }
        },
    );
    bind.state.open.values = open;
    let id = node.id.clone();
    for (slot, change) in changes {
        let summary = format!("Change {}", slot.label().to_lowercase());
        let edit = |value: &Value| vec![Edit::set(id.clone(), slot.key(), value.clone())];
        match change {
            Change::Set(value) => bind.apply(&summary, &edit(&value)),
            Change::Field(response, value) => bind.field(&response, &summary, || edit(&value)),
            Change::Color(response, value) => bind.color(&response, &summary, || edit(&value)),
        }
    }
}

/// The row of a value left out: a button that writes it down. Returns
/// whether it was clicked.
fn implicit_row(ui: &mut Ui, slot: Slot) -> bool {
    property_row(ui, slot.label(), |ui| {
        ui.add(
            TextButton::new(slot.implicit())
                .fill_width(true)
                .tooltip("Click to set a value of its own"),
        )
        .clicked()
    })
}

/// The rows of a written-down value: a header with a button to leave it
/// out again, then its parts. `implied` is what the value stands for when
/// left out, which a part left out shows.
fn written_rows(
    ui: &mut Ui,
    node: &Node,
    slot: Slot,
    parts: &[Part],
    value: &Value,
    implied: Option<&Value>,
) -> Option<Change> {
    let mut change = None;
    let reset = property_row(ui, slot.label(), |ui| {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.add(IconButton::new(icons::X).tooltip(match slot {
                Slot::From => "Start from the current value",
                Slot::To => "End on the design value",
            }))
            .clicked()
        })
        .inner
    });
    if reset {
        return Some(Change::Set(Value::Null));
    }
    let field = Field {
        slot,
        value,
        implied,
        optional: parts_optional(node.node_type()),
        salt: (node.id.as_str(), slot.key()),
    };
    let mut index = 0;
    while index < parts.len() {
        let part = parts[index];
        let next = parts.get(index + 1).copied();
        // Two numbers in a row share one line: X and Y, W and H.
        if let (Part::Number { .. }, Some(second @ Part::Number { .. })) = (part, next)
            && parts.len() == 2
        {
            let (mut first_out, mut second_out) = (None, None);
            property_row(ui, "", |ui| {
                pair(
                    ui,
                    |ui| first_out = number_field(ui, part, &field, true),
                    |ui| second_out = number_field(ui, second, &field, true),
                );
            });
            change = change.or(first_out).or(second_out);
            index += 2;
            continue;
        }
        let out = match part {
            Part::Number { label, .. } => {
                property_row(ui, label, |ui| number_field(ui, part, &field, false))
            }
            Part::Color { key, label } => {
                property_row(ui, label, |ui| color_field(ui, key, label, &field))
            }
        };
        change = change.or(out);
        index += 1;
    }
    change
}

/// What the fields of one written value share.
struct Field<'v> {
    /// The end of the action the value is.
    slot: Slot,
    /// The value as written.
    value: &'v Value,
    /// What the value stands for when left out.
    implied: Option<&'v Value>,
    /// Whether parts may be left out of it.
    optional: bool,
    /// Keeps the fields of this value apart from others.
    salt: (&'v str, &'v str),
}

impl Field<'_> {
    /// The part at `key` as written, if it is.
    fn written(&self, key: Option<&str>) -> Option<&Value> {
        Some(at(self.value, key)).filter(|part| !part.is_null())
    }

    /// Draws `field` and, for a written part that may be left out, a ×
    /// that leaves it out. Returns the field's outcome, and the change the
    /// × makes when it was clicked.
    ///
    /// A part that may be left out is drawn the same way whether it is
    /// written or not, with room kept for the ×, so the field keeps its
    /// identity when its first edit writes it: a scrub, a step with the
    /// arrow keys or an open colour picker carries on instead of stopping.
    fn with_leave_out<R>(
        &self,
        ui: &mut Ui,
        key: Option<&'static str>,
        label: &str,
        field: impl FnOnce(&mut Ui) -> R,
    ) -> (R, Option<Change>) {
        let Some(key) = key.filter(|_| self.optional) else {
            return (field(ui), None);
        };
        let theme = Theme::of(ui);
        let side = theme.size.control - 4.0;
        let room = (ui.available_width() - side - ui.spacing().item_spacing.x).max(0.0);
        let out = ui
            .allocate_ui(egui::vec2(room, theme.size.control), field)
            .inner;
        if self.written(Some(key)).is_none() {
            ui.allocate_space(egui::vec2(side, side));
            return (out, None);
        }
        let tooltip = match self.slot {
            Slot::From => format!("{label}: start from the current value"),
            Slot::To => format!("{label}: end on the design value"),
        };
        let leave_out = ui
            .add(
                IconButton::new(icons::X)
                    .size(side)
                    .dimmed(true)
                    .tooltip(&tooltip),
            )
            .clicked();
        (
            out,
            leave_out.then(|| Change::Set(without_part(self.value, key))),
        )
    }
}

/// A number part's field. A part left out shows the value it stands for,
/// quietly; a radius given per corner shows as words, as this panel sets
/// one radius for all corners. Returns the whole new value when it moved.
fn number_field(ui: &mut Ui, part: Part, field: &Field<'_>, labelled: bool) -> Option<Change> {
    let Part::Number {
        key,
        label,
        unit,
        scale,
        min,
        max,
    } = part
    else {
        return None;
    };
    let written = field.written(key);
    if let Some(corners) = written.and_then(Value::as_array) {
        let corners: Vec<String> = corners
            .iter()
            .filter_map(Value::as_f64)
            .map(|radius| format_value(radius, 1))
            .collect();
        ui.label(format!("Per corner: {}", corners.join(", ")));
        return None;
    }
    let stored = written.and_then(Value::as_f64);
    let implied = field.implied.and_then(|value| read_number(value, key));
    let mut shown = stored.or(implied).unwrap_or(0.0) * scale;
    let (response, leave_out) = field.with_leave_out(ui, key, label, |ui| {
        let mut number = NumericField::new(&mut shown, label)
            .unit(unit)
            .range(min..=max)
            .decimals(if unit == "%" && scale > 1.0 { 0 } else { 1 })
            .implied(stored.is_none() && field.optional)
            .id_salt(("value", field.salt, label));
        if labelled {
            number = number.label(label);
        }
        number.show(ui)
    });
    let moved = response.response.changed() || response.finished;
    leave_out.or_else(|| {
        moved.then(|| {
            let next = with_part(field.value, key, json!(shown / scale));
            Change::Field(response, next)
        })
    })
}

/// A colour part's field. A part left out shows the colour it stands for;
/// a gradient shows as words, as this panel edits colours only. Returns the
/// whole new value when it moved.
fn color_field(
    ui: &mut Ui,
    key: Option<&'static str>,
    label: &str,
    field: &Field<'_>,
) -> Option<Change> {
    let shown = match field.written(key) {
        Some(_) => Some(field.value),
        None => field.implied,
    };
    let Some(color) = shown.and_then(|value| read_color(value, key)) else {
        let words = match shown.map(|value| at(value, key)) {
            Some(Value::Object(_)) => "Gradient",
            _ => field.slot.implicit(),
        };
        ui.label(words);
        return None;
    };
    let mut picked = rgba(color);
    let (response, leave_out) = field.with_leave_out(ui, key, label, |ui| {
        ColorField::new(&mut picked, label)
            .id_salt(("value-color", field.salt, label))
            .show(ui)
    });
    leave_out.or_else(|| {
        (response.changed || response.finished).then(|| {
            let next = with_part(field.value, key, hex(picked));
            Change::Color(response, next)
        })
    })
}

#[cfg(test)]
mod tests {
    use moshionly_core::{NodeId, io};

    use super::*;

    fn document() -> Document {
        io::parse(
            r##"{"format":"moshionly","version":1,"name":"T","nodes":[
            {"id":"ab","parent":null,"index":"a0","type":"artboard","duration":4000},
            {"id":"box","parent":"ab","index":"a0","type":"rect","width":200,"height":100,"opacity":80,"scale":2,"fill":"#FF0000","corner_radius":12},
            {"id":"t","parent":"ab","index":"a1","type":"text","content":"$1,250 raised"},
            {"id":"fade","parent":"ab","index":"a0","type":"opacity","target":"box","start":0,"end":500,"to":20},
            {"id":"op","parent":"ab","index":"a1","type":"opacity","target":"box","start":1000,"end":1500},
            {"id":"sc","parent":"ab","index":"a2","type":"scale","target":"box","start":0,"end":500},
            {"id":"mv","parent":"ab","index":"a3","type":"move","target":"box","start":0,"end":500},
            {"id":"fill","parent":"ab","index":"a4","type":"fill","target":"box","start":0,"end":500},
            {"id":"size","parent":"ab","index":"a5","type":"resize","target":"box","start":0,"end":500},
            {"id":"r","parent":"ab","index":"a6","type":"corner_radius","target":"box","start":0,"end":500},
            {"id":"count","parent":"ab","index":"a7","type":"counter","target":"t","start":0,"end":500}
            ]}"##,
        )
        .unwrap()
    }

    fn resolve(id: &str, slot: Slot) -> Value {
        let document = document();
        let node = document.get(&NodeId::from(id)).unwrap().clone();
        resolved(&document, &node, slot).unwrap()
    }

    #[test]
    fn a_missing_from_is_the_value_when_the_action_starts() {
        // The first opacity ends at 20, so the second starts from there.
        assert_eq!(resolve("op", Slot::From), json!(20.0));
        // A missing to is the design's value.
        assert_eq!(resolve("op", Slot::To), json!(80.0));
        // Scale is a multiplier of the design's scale.
        assert_eq!(resolve("sc", Slot::From), json!(1.0));
        assert_eq!(resolve("mv", Slot::To), json!({"x": 0.0, "y": 0.0}));
        assert_eq!(resolve("fill", Slot::To), json!("#FF0000"));
        assert_eq!(
            resolve("size", Slot::From),
            json!({"width": 200.0, "height": 100.0})
        );
        assert_eq!(resolve("r", Slot::From), json!(12.0));
        assert_eq!(resolve("count", Slot::To), json!(1250.0));
    }

    #[test]
    fn parts_are_read_and_written_in_the_value() {
        let offset = json!({"x": 10.0, "y": -4.0});
        assert_eq!(read_number(&offset, Some("y")), Some(-4.0));
        assert_eq!(read_number(&json!([8.0, 1.0, 2.0, 3.0]), None), Some(8.0));
        assert_eq!(read_number(&json!({"width": 5.0}), Some("height")), None);
        assert_eq!(
            with_part(&offset, Some("x"), json!(25.0)),
            json!({"x": 25.0, "y": -4.0})
        );
        assert_eq!(with_part(&json!(3.0), None, json!(4.0)), json!(4.0));
        // A part written into nothing starts an object.
        assert_eq!(
            with_part(&Value::Null, Some("width"), json!(9.0)),
            json!({"width": 9.0})
        );
        assert_eq!(
            read_color(&json!({"paint": "#0D99FF"}), Some("paint")),
            Some(Color::rgb(0x0D, 0x99, 0xFF))
        );
        assert_eq!(read_color(&json!({"type": "linear_gradient"}), None), None);
        // A part left out is removed by the merge; the last one takes the
        // whole value with it.
        let size = json!({"width": 200.0, "height": 40.0});
        assert_eq!(
            without_part(&size, "height"),
            json!({"width": 200.0, "height": null})
        );
        assert_eq!(without_part(&json!({"width": 200.0}), "width"), Value::Null);
    }

    #[test]
    fn every_property_action_has_parts() {
        for ty in NodeType::ALL {
            let has_from = moshionly_core::schema::field_names(*ty)
                .iter()
                .any(|name| name == "from");
            assert_eq!(!parts(*ty).is_empty(), has_from, "{ty}");
        }
    }
}
