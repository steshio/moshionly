//! Drives the whole editor on the demo project the way a person would,
//! without a window: clicking, dragging, typing and undoing.

use egui::{Key, Modifiers, Pos2, pos2, vec2};
use egui_kittest::{
    Harness,
    kittest::{NodeT as _, Queryable as _},
};
use moshionly_agent::SessionHandle;
use moshionly_app::{settings::Settings, shell::Shell, theme};
use moshionly_core::NodeId;
use moshionly_render::{AssetLoader, FontLibrary, Renderer};

/// The demo project that comes with the engine.
const DEMO: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../moshionly-core/examples/demo.moshi"
);

/// The editor on the demo project, in a 1440 by 900 window.
fn harness() -> Harness<'static, Shell> {
    let session = SessionHandle::open(DEMO).expect("the demo file opens");
    let renderer = Renderer::new(FontLibrary::bundled(), AssetLoader::default());
    let shell = Shell::headless(Settings::default(), session, renderer);
    let mut harness = Harness::builder()
        .with_size(vec2(1440.0, 900.0))
        // Frames a sixtieth of a second apart, so two clicks make a double
        // click.
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

/// Presses and lets go of the primary button at `at` with `modifiers`,
/// within one frame.
fn press(harness: &mut Harness<'_, Shell>, at: Pos2, modifiers: Modifiers) {
    harness.event(egui::Event::ModifiersChanged(modifiers));
    for pressed in [true, false] {
        harness.event(egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers,
        });
    }
    harness.event(egui::Event::ModifiersChanged(Modifiers::NONE));
}

fn double_click(harness: &mut Harness<'_, Shell>, at: Pos2) {
    // Half a second after any earlier click, so this is not a triple click.
    for _ in 0..30 {
        harness.step();
    }
    harness.hover_at(at);
    harness.step();
    press(harness, at, Modifiers::NONE);
    harness.step();
    press(harness, at, Modifiers::NONE);
    harness.step();
    harness.run();
}

fn command_click(harness: &mut Harness<'_, Shell>, at: Pos2) {
    harness.hover_at(at);
    harness.step();
    press(harness, at, Modifiers::COMMAND);
    harness.step();
    harness.run();
}

fn drag(harness: &mut Harness<'_, Shell>, from: Pos2, to: Pos2) {
    harness.hover_at(from);
    harness.drag_at(from);
    // A few steps, as a hand would move.
    for step in 1..=4 {
        let t = step as f32 / 4.0;
        harness.hover_at(from + (to - from) * t);
        harness.run();
    }
    harness.drop_at(to);
    harness.run();
}

fn selection(harness: &Harness<'_, Shell>) -> Vec<String> {
    harness
        .state()
        .editor()
        .selection
        .ids()
        .iter()
        .map(|id| id.to_string())
        .collect()
}

/// `x` and `y` of a layer, as the document has them.
fn position(harness: &Harness<'_, Shell>, id: &str) -> (f64, f64) {
    let document = harness.state().editor().document().clone();
    let layer = document
        .get(&NodeId::from(id))
        .unwrap()
        .layer()
        .unwrap()
        .clone();
    (layer.x, layer.y)
}

#[test]
fn a_click_selects_the_outermost_group_under_the_pointer() {
    let mut harness = harness();
    // The card is at 240, 340 and 600 by 400 on the demo's artboard.
    let card = screen(&harness, pos2(400.0, 650.0));
    click(&mut harness, card);
    assert_eq!(selection(&harness), ["card"]);
    // A double click goes into the card and picks what is under it.
    let dot = screen(&harness, pos2(240.0 + 526.0, 340.0 + 326.0));
    double_click(&mut harness, dot);
    assert_eq!(selection(&harness), ["dot"]);
    // Escape lets go.
    harness.key_press(Key::Escape);
    harness.run();
    assert!(selection(&harness).is_empty());
}

#[test]
fn dragging_moves_the_layer_and_one_undo_puts_it_back() {
    let mut harness = harness();
    let before = position(&harness, "card");
    let from = screen(&harness, pos2(400.0, 650.0));
    let to = screen(&harness, pos2(430.0, 610.0));
    drag(&mut harness, from, to);
    assert_eq!(selection(&harness), ["card"]);
    let after = position(&harness, "card");
    assert_ne!(after, before, "the card moved");
    // The drag was one step, whatever it passed through.
    harness.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    harness.run();
    assert_eq!(position(&harness, "card"), before);
}

#[test]
fn typing_in_a_numeric_field_changes_the_layer() {
    let mut harness = harness();
    let card = screen(&harness, pos2(400.0, 650.0));
    click(&mut harness, card);
    harness.get_by_label("X").click();
    harness.run();
    harness.get_by_label("X").type_text("100");
    harness.run();
    harness.key_press(Key::Enter);
    harness.run();
    assert_eq!(position(&harness, "card").0, 100.0);
    assert_eq!(
        harness.state().editor().undo_label().as_deref(),
        Some("Undo Move")
    );
}

#[test]
fn the_layer_list_and_the_canvas_agree_on_the_selection() {
    let mut harness = harness();
    // From the list to the canvas. The timeline names the card too; the
    // list comes first.
    harness
        .get_all_by_label("Card")
        .next()
        .expect("the card's row")
        .click();
    harness.run();
    assert_eq!(selection(&harness), ["card"]);
    // From the canvas to the list: the card's row opens to show the dot.
    harness.key_press(Key::Escape);
    harness.run();
    let dot = screen(&harness, pos2(240.0 + 526.0, 340.0 + 326.0));
    command_click(&mut harness, dot);
    assert_eq!(selection(&harness), ["dot"]);
    // The row shows the selection: egui reports it as toggled on.
    let row = harness
        .get_all_by_label("Dot")
        .next()
        .expect("the dot's row");
    assert_eq!(
        row.accesskit_node().toggled(),
        Some(egui::accesskit::Toggled::True)
    );
}

#[test]
fn arrow_keys_nudge_and_delete_removes() {
    let mut harness = harness();
    let card = screen(&harness, pos2(400.0, 650.0));
    click(&mut harness, card);
    let before = position(&harness, "card");
    harness.key_press(Key::ArrowRight);
    harness.key_press_modifiers(Modifiers::SHIFT, Key::ArrowDown);
    harness.run();
    assert_eq!(
        position(&harness, "card"),
        (before.0 + 1.0, before.1 + 10.0)
    );
    harness.key_press(Key::Backspace);
    harness.run();
    assert!(
        !harness
            .state()
            .editor()
            .document()
            .contains(&NodeId::from("card"))
    );
    assert!(selection(&harness).is_empty());
    harness.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    harness.run();
    assert!(
        harness
            .state()
            .editor()
            .document()
            .contains(&NodeId::from("card"))
    );
}

#[test]
fn a_marquee_selects_what_it_touches() {
    let mut harness = harness();
    // From the empty top-left of the artboard into the card.
    let from = screen(&harness, pos2(40.0, 40.0));
    let to = screen(&harness, pos2(300.0, 400.0));
    drag(&mut harness, from, to);
    assert_eq!(selection(&harness), ["card"]);
}

/// The layers of the demo's artboard of `kind`, by id.
fn layers_of(harness: &Harness<'_, Shell>, kind: moshionly_core::NodeType) -> Vec<NodeId> {
    harness
        .state()
        .editor()
        .document()
        .nodes()
        .filter(|node| node.node_type() == kind)
        .map(|node| node.id.clone())
        .collect()
}

#[test]
fn the_rectangle_tool_draws_and_hands_back_to_select() {
    let mut harness = harness();
    harness.key_press(Key::R);
    harness.run();
    assert_eq!(
        harness.state().tool(),
        moshionly_app::shell::Tool::Rectangle
    );
    let from = screen(&harness, pos2(60.0, 60.0));
    let to = screen(&harness, pos2(200.0, 160.0));
    drag(&mut harness, from, to);
    let rects = layers_of(&harness, moshionly_core::NodeType::Rect);
    // The demo has one rect already: the card's accent.
    assert_eq!(rects.len(), 2);
    assert_eq!(harness.state().tool(), moshionly_app::shell::Tool::Select);
    let new = harness
        .state()
        .editor()
        .selection
        .single()
        .cloned()
        .unwrap();
    assert!(rects.contains(&new));
    let layer = harness
        .state()
        .editor()
        .document()
        .get(&new)
        .unwrap()
        .layer()
        .unwrap()
        .clone();
    assert!((layer.x - 60.0).abs() < 1.0 && (layer.width - 140.0).abs() < 1.0);
}

#[test]
fn the_text_tool_places_text_to_type_into() {
    let mut harness = harness();
    harness.key_press(Key::T);
    harness.run();
    let at = screen(&harness, pos2(80.0, 900.0));
    click(&mut harness, at);
    assert!(harness.state().is_editing_text());
    harness.event(egui::Event::Text("Hello".to_owned()));
    harness.run();
    harness.key_press(Key::Escape);
    harness.run();
    assert!(!harness.state().is_editing_text());
    let texts = layers_of(&harness, moshionly_core::NodeType::Text);
    let document = harness.state().editor().document().clone();
    let typed = texts
        .iter()
        .filter_map(|id| moshionly_app::editor::inspect::text_of(document.get(id).unwrap()))
        .any(|text| text.content == "Hello");
    assert!(typed, "the words were kept");
    // Adding and typing were one step.
    assert_eq!(
        harness.state().editor().undo_label().as_deref(),
        Some("Undo Add text")
    );
}

#[test]
fn the_pen_draws_a_path() {
    let mut harness = harness();
    harness.key_press(Key::P);
    harness.run();
    for world in [pos2(100.0, 100.0), pos2(200.0, 50.0), pos2(300.0, 150.0)] {
        let at = screen(&harness, world);
        click(&mut harness, at);
    }
    harness.key_press(Key::Enter);
    harness.run();
    let paths = layers_of(&harness, moshionly_core::NodeType::Path);
    assert_eq!(paths.len(), 1);
    assert_eq!(harness.state().tool(), moshionly_app::shell::Tool::Select);
    let layer = harness
        .state()
        .editor()
        .document()
        .get(&paths[0])
        .unwrap()
        .layer()
        .unwrap()
        .clone();
    assert!((layer.width - 200.0).abs() < 1.0 && (layer.height - 100.0).abs() < 1.0);
}

/// Dialogs that answer with fixed files.
struct FakeDialogs {
    open: Option<std::path::PathBuf>,
    save: Option<std::path::PathBuf>,
}

impl moshionly_app::dialogs::Dialogs for FakeDialogs {
    fn open_project(&self) -> Option<std::path::PathBuf> {
        self.open.clone()
    }

    fn save_project(&self, _suggested_name: &str) -> Option<std::path::PathBuf> {
        self.save.clone()
    }

    fn import_images(&self) -> Option<Vec<std::path::PathBuf>> {
        None
    }
}

/// A folder of its own for one test's files.
fn scratch(name: &str) -> std::path::PathBuf {
    let folder = std::env::temp_dir().join(format!("moshionly-app-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    folder
}

#[test]
fn unsaved_changes_are_asked_about_before_a_new_document() {
    let mut harness = harness();
    let card = screen(&harness, pos2(400.0, 650.0));
    click(&mut harness, card);
    harness.key_press(Key::ArrowRight);
    harness.run();
    assert!(harness.state().editor().has_unsaved_changes());

    // Cancel keeps everything as it was.
    harness.key_press_modifiers(Modifiers::COMMAND, Key::N);
    harness.run();
    assert!(harness.state().is_asking());
    harness.get_by_label("Cancel").click();
    harness.run();
    assert!(!harness.state().is_asking());
    assert!(
        harness
            .state()
            .editor()
            .document()
            .contains(&NodeId::from("card"))
    );

    // Don't save throws the changes away.
    harness.key_press_modifiers(Modifiers::COMMAND, Key::N);
    harness.run();
    harness.get_by_label("Don't save").click();
    harness.run();
    let document = harness.state().editor().document().clone();
    assert_eq!(document.name, "Untitled");
    assert_eq!(document.artboards().len(), 1);
    assert!(!document.contains(&NodeId::from("card")));
    assert_eq!(harness.state().project_name(), "Untitled");
}

#[test]
fn closing_with_unsaved_changes_asks_first() {
    let mut harness = harness();
    // Nothing to lose: the window may close.
    assert!(harness.state_mut().may_close());
    let card = screen(&harness, pos2(400.0, 650.0));
    click(&mut harness, card);
    harness.key_press(Key::ArrowRight);
    harness.run();
    assert!(!harness.state_mut().may_close());
    harness.run();
    assert!(harness.state().is_asking());
    harness.get_by_label("Don't save").click();
    harness.run();
    assert!(harness.state_mut().may_close());
}

#[test]
fn projects_open_and_save_through_the_dialogs() {
    let folder = scratch("files");
    let saved = folder.join("copy.moshi");
    let mut harness = harness();
    harness.state_mut().set_dialogs(Box::new(FakeDialogs {
        open: Some(DEMO.into()),
        save: Some(saved.clone()),
    }));
    // Save as writes the file and makes it the project's.
    harness.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::S);
    harness.run();
    assert!(saved.exists());
    assert!(!harness.state().editor().has_unsaved_changes());
    assert_eq!(harness.state().settings().recent.first(), Some(&saved));
    // A change and a plain save.
    let card = screen(&harness, pos2(400.0, 650.0));
    click(&mut harness, card);
    harness.key_press(Key::ArrowDown);
    harness.run();
    harness.key_press_modifiers(Modifiers::COMMAND, Key::S);
    harness.run();
    assert!(!harness.state().editor().has_unsaved_changes());
    let reread = moshionly_core::io::load(&saved).unwrap();
    let y = reread
        .get(&NodeId::from("card"))
        .unwrap()
        .layer()
        .unwrap()
        .y;
    assert_eq!(y, 341.0);
    // Opening asks for a file and replaces the document.
    harness.key_press_modifiers(Modifiers::COMMAND, Key::O);
    harness.run();
    let y = position(&harness, "card").1;
    assert_eq!(y, 340.0, "the demo as it is on disk");
    assert!(selection(&harness).is_empty());
    let _ = std::fs::remove_dir_all(&folder);
}

#[test]
fn a_broken_file_is_refused_in_plain_words() {
    let folder = scratch("broken");
    let broken = folder.join("broken.moshi");
    std::fs::write(&broken, "this is not a project").unwrap();
    let mut harness = harness();
    harness.state_mut().open_path(&broken);
    harness.run();
    // The demo is still open.
    assert!(
        harness
            .state()
            .editor()
            .document()
            .contains(&NodeId::from("card"))
    );
    assert!(!harness.state().settings().recent.contains(&broken));
    let _ = std::fs::remove_dir_all(&folder);
}
