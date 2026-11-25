//! Components in the editor on the demo project, the way a person would
//! use them: making one, placing instances, editing the main, going back
//! to it, detaching and deleting.

use egui::{Key, Modifiers, Pos2, pos2, vec2};
use egui_kittest::{Harness, kittest::Queryable as _};
use moshionly_agent::SessionHandle;
use moshionly_app::{
    settings::Settings,
    shell::{LayersTab, Shell},
    theme,
};
use moshionly_core::{Edit, NodeId, NodeType, SceneNode, build};
use moshionly_render::{AssetLoader, FontLibrary, Renderer};

/// The demo project that comes with the engine.
const DEMO: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../moshionly-core/examples/demo.moshi"
);

/// The editor on the demo project, in a 1440 by 900 window.
fn harness() -> Harness<'static, Shell> {
    let session = SessionHandle::open(DEMO).expect("the demo opens");
    let renderer = Renderer::new(FontLibrary::bundled(), AssetLoader::default());
    let shell = Shell::headless(Settings::default(), session, renderer);
    let mut harness = Harness::builder()
        .with_size(vec2(1440.0, 900.0))
        .with_step_dt(1.0 / 60.0)
        .with_context_setup(theme::install)
        .build_ui_state(
            |ui, shell: &mut Shell| {
                shell.show(ui);
            },
            shell,
        );
    harness.run();
    harness
}

/// Where a point of the canvas is on screen.
fn screen(harness: &Harness<'_, Shell>, world: Pos2) -> Pos2 {
    let shell = harness.state();
    let canvas = shell.canvas_rect().expect("the canvas has been drawn");
    shell.camera().world_to_screen(canvas, world)
}

fn click(harness: &mut Harness<'_, Shell>, at: Pos2) {
    harness.hover_at(at);
    harness.drag_at(at);
    harness.drop_at(at);
    harness.run();
}

/// Drags from `from` to `to` in a few steps, as a hand would. Frames are
/// stepped rather than run to rest: a row's tooltip keeps asking for more.
fn drag(harness: &mut Harness<'_, Shell>, from: Pos2, to: Pos2) {
    harness.hover_at(from);
    harness.drag_at(from);
    harness.step();
    for step in 1..=4 {
        let t = step as f32 / 4.0;
        harness.hover_at(from + (to - from) * t);
        harness.step();
    }
    harness.drop_at(to);
    harness.step();
    harness.step();
}

/// Clicks the first button named `label`.
fn press_button(harness: &mut Harness<'_, Shell>, label: &str) {
    harness.get_by_label(label).click();
    harness.run();
}

/// The centre of the first widget named `label`: in the left panel that
/// is the row, which comes before the timeline's.
fn center_of(harness: &Harness<'_, Shell>, label: &str) -> Pos2 {
    harness
        .get_all_by_label(label)
        .next()
        .unwrap_or_else(|| panic!("nothing named {label}"))
        .rect()
        .center()
}

fn selection(harness: &Harness<'_, Shell>) -> Vec<NodeId> {
    harness.state().editor().selection.ids().to_vec()
}

fn of_type(harness: &Harness<'_, Shell>, ty: NodeType) -> Vec<NodeId> {
    harness
        .state()
        .editor()
        .document()
        .nodes()
        .filter(|node| node.node_type() == ty)
        .map(|node| node.id.clone())
        .collect()
}

fn type_of(harness: &Harness<'_, Shell>, id: &NodeId) -> Option<NodeType> {
    harness
        .state()
        .editor()
        .document()
        .get(id)
        .map(|node| node.node_type())
}

/// `x`, `y`, `width` and `height` of a layer.
fn frame(harness: &Harness<'_, Shell>, id: &NodeId) -> (f64, f64, f64, f64) {
    let document = harness.state().editor().document().clone();
    let layer = document.get(id).unwrap().layer().unwrap().clone();
    (layer.x, layer.y, layer.width, layer.height)
}

/// What the canvas draws for `layer` on `artboard` now.
fn drawn(harness: &mut Harness<'_, Shell>, artboard: &str, layer: &NodeId) -> SceneNode {
    let scene = harness
        .state_mut()
        .editor_mut()
        .scene(&NodeId::from(artboard))
        .expect("the artboard is drawn");
    scene.find(layer).expect("the layer is drawn").clone()
}

/// Makes the demo's card a component with ⌥⌘K, after a click on it.
fn card_component(harness: &mut Harness<'_, Shell>) -> NodeId {
    // The card is at 240, 340 and 600 by 400 on the demo's artboard.
    let card = screen(harness, pos2(400.0, 650.0));
    click(harness, card);
    harness.key_press_modifiers(Modifiers::COMMAND | Modifiers::ALT, Key::K);
    harness.run();
    NodeId::from("card")
}

/// Places an instance of the selected main with the inspector's button,
/// and returns it.
fn insert_instance(harness: &mut Harness<'_, Shell>) -> NodeId {
    press_button(harness, "Insert instance");
    let added = selection(harness);
    assert_eq!(added.len(), 1, "the new instance is selected");
    added[0].clone()
}

#[test]
fn a_component_is_made_placed_edited_and_detached() {
    let mut harness = harness();
    let card = card_component(&mut harness);
    // The group became a component in place.
    assert_eq!(type_of(&harness, &card), Some(NodeType::Component));
    assert_eq!(selection(&harness), [card.clone()]);
    assert_eq!(
        harness.state().editor().undo_label().as_deref(),
        Some("Undo Create component")
    );

    // An instance goes beside it, the same size.
    let instance = insert_instance(&mut harness);
    assert_eq!(type_of(&harness, &instance), Some(NodeType::Instance));
    assert_eq!(frame(&harness, &instance), (880.0, 340.0, 600.0, 400.0));

    // On the canvas it is one layer: a click anywhere on it, on the copy
    // of the accent bar too, selects the instance, and a double click does
    // not go into it. (Its right part hangs off the artboard.)
    harness.key_press(Key::Escape);
    harness.run();
    assert!(selection(&harness).is_empty());
    for at in [pos2(950.0, 345.0), pos2(1050.0, 700.0)] {
        let point = screen(&harness, at);
        click(&mut harness, point);
        assert_eq!(selection(&harness), [instance.clone()], "at {at:?}");
    }
    let dot = screen(&harness, pos2(1000.0, 600.0));
    harness.hover_at(dot);
    harness.run();
    for _ in 0..2 {
        harness.event(egui::Event::PointerButton {
            pos: dot,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        });
        harness.event(egui::Event::PointerButton {
            pos: dot,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        });
        harness.step();
    }
    harness.run();
    assert_eq!(selection(&harness), [instance.clone()]);

    // Editing a layer inside the main changes what the instance draws.
    let before = drawn(&mut harness, "scene", &instance);
    harness
        .state_mut()
        .editor_mut()
        .apply(
            "Recolour",
            &[Edit::set(NodeId::from("accent"), "fill", "#00C853")],
        )
        .unwrap();
    harness.run();
    let after = drawn(&mut harness, "scene", &instance);
    assert_ne!(before, after, "the copy follows its main");

    // Detaching leaves a group in its place that looks the same.
    harness.key_press_modifiers(Modifiers::COMMAND | Modifiers::ALT, Key::B);
    harness.run();
    assert_eq!(type_of(&harness, &instance), None);
    let group = selection(&harness)[0].clone();
    assert_eq!(type_of(&harness, &group), Some(NodeType::Group));
    assert_eq!(frame(&harness, &group), (880.0, 340.0, 600.0, 400.0));
    assert_eq!(
        harness.state().editor().undo_label().as_deref(),
        Some("Undo Detach instance")
    );
}

#[test]
fn the_components_list_places_instances_and_go_to_main_comes_back() {
    let mut harness = harness();
    let card = card_component(&mut harness);
    // A second artboard to place instances on, the current one.
    let document = harness.state().editor().document().clone();
    let (away, edits) = build::add_artboard(&document, "Away", 1080.0, 1080.0).unwrap();
    let editor = harness.state_mut().editor_mut();
    editor.apply("Add artboard", &edits).unwrap();
    editor.selection.clear();
    editor.set_artboard(Some(away.clone()));
    harness.key_press_modifiers(Modifiers::SHIFT, Key::Num1);
    harness.run();

    press_button(&mut harness, "Components");
    assert_eq!(harness.state().layers_tab(), LayersTab::Components);

    // A click on the card's row places an instance in the middle of the
    // current artboard.
    let row = center_of(&harness, "Card");
    click(&mut harness, row);
    let instance = selection(&harness)[0].clone();
    let document = harness.state().editor().document().clone();
    assert_eq!(document.get(&instance).unwrap().parent, Some(away.clone()));
    assert_eq!(document.get(&instance).unwrap().component(), Some(&card));
    assert_eq!(frame(&harness, &instance), (240.0, 340.0, 600.0, 400.0));
    assert!(
        harness
            .query_by_label("Main component · 1 instance")
            .is_none(),
        "an instance is selected, not the main"
    );
    // The timeline of its artboard says what it plays: the card's title,
    // amount and dot come in on the card's own timeline.
    assert_eq!(
        harness.state().editor().artboard(),
        Some(&away),
        "the instance's artboard is the current one"
    );
    assert!(harness.query_by_label("Plays Card").is_some());

    // Dragging the row onto the canvas places one where it lands.
    let origin = harness
        .state()
        .editor()
        .artboard_rect(&away)
        .expect("the artboard is on the canvas")
        .min;
    let target = origin + vec2(700.0, 800.0);
    let row = center_of(&harness, "Card");
    let to = screen(&harness, target);
    drag(&mut harness, row, to);
    let dropped = selection(&harness)[0].clone();
    assert_ne!(dropped, instance);
    let (x, y, width, height) = frame(&harness, &dropped);
    assert!(
        (x + width / 2.0 - 700.0).abs() <= 1.0 && (y + height / 2.0 - 800.0).abs() <= 1.0,
        "centred where it was let go: {x}, {y}"
    );
    assert_eq!(
        harness
            .state()
            .editor()
            .document()
            .instances_of(&card)
            .len(),
        2
    );

    // Go to main selects the card on its own artboard.
    press_button(&mut harness, "Go to main");
    assert_eq!(selection(&harness), [card.clone()]);
    assert_eq!(
        harness.state().editor().artboard(),
        Some(&NodeId::from("scene"))
    );
    assert!(
        harness
            .query_by_label("Main component · 2 instances")
            .is_some()
    );
}

#[test]
fn deleting_a_main_leaves_groups_and_says_so() {
    let mut harness = harness();
    let card = card_component(&mut harness);
    let instance = insert_instance(&mut harness);
    harness
        .state_mut()
        .editor_mut()
        .selection
        .select(card.clone());
    harness.run();
    harness.key_press(Key::Backspace);
    harness.run();
    assert_eq!(type_of(&harness, &card), None);
    assert_eq!(type_of(&harness, &instance), None);
    assert!(of_type(&harness, NodeType::Instance).is_empty());
    // The instance's place holds a group that looks like it.
    let groups = of_type(&harness, NodeType::Group);
    assert_eq!(groups.len(), 1);
    assert_eq!(frame(&harness, &groups[0]), (880.0, 340.0, 600.0, 400.0));
    assert!(
        harness
            .query_by_label("Detached 1 instance of Card")
            .is_some()
    );
    // One step brings it all back.
    harness.state_mut().editor_mut().undo().unwrap();
    assert_eq!(type_of(&harness, &instance), Some(NodeType::Instance));
}

/// Selects `ids` and copies them with ⌘C. Returns the clipboard text.
fn copy(harness: &mut Harness<'_, Shell>, ids: &[NodeId]) -> String {
    harness
        .state_mut()
        .editor_mut()
        .selection
        .set(ids.iter().cloned());
    harness.run();
    harness.event(egui::Event::Copy);
    harness.step();
    let copied = harness
        .output()
        .platform_output
        .commands
        .iter()
        .find_map(|command| match command {
            egui::OutputCommand::CopyText(text) => Some(text.clone()),
            _ => None,
        })
        .expect("the layers went on the clipboard");
    harness.run();
    copied
}

/// Selects `id` and pastes `text` with ⌘V. Returns what was pasted.
fn paste(harness: &mut Harness<'_, Shell>, text: &str, id: &str) -> Vec<NodeId> {
    harness
        .state_mut()
        .editor_mut()
        .selection
        .select(NodeId::from(id));
    harness.run();
    harness.event(egui::Event::Paste(text.to_owned()));
    harness.run();
    selection(harness)
}

fn parent_of(harness: &Harness<'_, Shell>, id: &NodeId) -> Option<NodeId> {
    harness.state().editor().document().get(id)?.parent.clone()
}

#[test]
fn a_main_or_its_instance_pasted_into_the_main_goes_beside_it() {
    let mut harness = harness();
    let card = card_component(&mut harness);
    let instance = insert_instance(&mut harness);
    let scene = NodeId::from("scene");

    // ⌘C, ⌘V on the main: a second main just in front of it, as a copy
    // cannot go inside the main it was copied from.
    let main = copy(&mut harness, std::slice::from_ref(&card));
    let pasted = paste(&mut harness, &main, "card");
    assert_eq!(of_type(&harness, NodeType::Component).len(), 2);
    assert_eq!(pasted.len(), 1);
    assert_eq!(type_of(&harness, &pasted[0]), Some(NodeType::Component));
    assert_eq!(parent_of(&harness, &pasted[0]), Some(scene.clone()));
    assert_eq!(
        harness.state().editor().document().next_sibling(&card),
        Some(&pasted[0]),
        "just in front of the main"
    );
    // With a layer inside the main selected, the same.
    let pasted = paste(&mut harness, &main, "accent");
    assert_eq!(of_type(&harness, NodeType::Component).len(), 3);
    assert_eq!(parent_of(&harness, &pasted[0]), Some(scene.clone()));

    // An instance of the card cannot show inside the card either.
    let copied = copy(&mut harness, std::slice::from_ref(&instance));
    let pasted = paste(&mut harness, &copied, "accent");
    assert_eq!(type_of(&harness, &pasted[0]), Some(NodeType::Instance));
    assert_eq!(parent_of(&harness, &pasted[0]), Some(scene.clone()));
    assert_eq!(
        harness
            .state()
            .editor()
            .document()
            .instances_of(&card)
            .len(),
        2
    );

    // A plain layer still goes inside the selected main.
    let accent = copy(&mut harness, &[NodeId::from("accent")]);
    let pasted = paste(&mut harness, &accent, "card");
    assert_eq!(parent_of(&harness, &pasted[0]), Some(card.clone()));
}

/// Types `text` into the first field named `label` and presses Enter.
fn type_into(harness: &mut Harness<'_, Shell>, label: &str, text: &str) {
    let field = harness
        .get_all_by_label(label)
        .next()
        .unwrap_or_else(|| panic!("no field named {label}"));
    field.click_accesskit();
    harness.run();
    harness
        .get_all_by_label(label)
        .next()
        .unwrap()
        .type_text(text);
    harness.run();
    harness.key_press(Key::Enter);
    harness.run();
}

#[test]
fn instances_follow_a_main_resized_in_the_inspector() {
    let mut harness = harness();
    let card = card_component(&mut harness);
    let instance = insert_instance(&mut harness);
    harness
        .state_mut()
        .editor_mut()
        .selection
        .select(card.clone());
    harness.run();
    type_into(&mut harness, "Width", "800");
    assert_eq!(frame(&harness, &card), (240.0, 340.0, 800.0, 400.0));
    // The instance was the main's size: it still is, where it was.
    assert_eq!(frame(&harness, &instance), (880.0, 340.0, 800.0, 400.0));
    assert_eq!(
        harness.state().editor().undo_label().as_deref(),
        Some("Undo Resize")
    );
    harness.state_mut().editor_mut().undo().unwrap();
    assert_eq!(frame(&harness, &instance), (880.0, 340.0, 600.0, 400.0));
}

#[test]
fn the_time_offset_is_typed_as_one_undo_step() {
    let mut harness = harness();
    card_component(&mut harness);
    let instance = insert_instance(&mut harness);
    let field = harness
        .get_all_by_label("Time offset")
        .next()
        .expect("the instance's time offset");
    field.click_accesskit();
    harness.run();
    harness
        .get_all_by_label("Time offset")
        .next()
        .unwrap()
        .type_text("300");
    harness.run();
    harness.key_press(Key::Enter);
    harness.run();
    let offset = |harness: &Harness<'_, Shell>| {
        let document = harness.state().editor().document().clone();
        match &document.get(&instance).unwrap().data {
            moshionly_core::NodeData::Instance(instance) => instance.time_offset,
            _ => panic!("not an instance"),
        }
    };
    assert_eq!(offset(&harness), 300.0);
    assert_eq!(
        harness.state().editor().undo_label().as_deref(),
        Some("Undo Change time offset")
    );
    harness.state_mut().editor_mut().undo().unwrap();
    assert_eq!(offset(&harness), 0.0);
}
