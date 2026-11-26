//! Effects in the editor the way a person uses them, without a window:
//! adding a shadow, blur, glow, noise or directional blur from the Effects
//! section's add menu, changing and undoing it, editing several layers at
//! once, and animating an effect from the Animate tab.

use egui::{Key, Modifiers, Vec2, vec2};
use egui_kittest::{
    Harness,
    kittest::{NodeT as _, Queryable as _},
};
use moshionly_agent::SessionHandle;
use moshionly_app::{
    editor::{
        effects::{Effect, NEW_BLUR},
        timeline::{action_icon, timeline_groups},
    },
    settings::Settings,
    shell::{InspectorTab, Shell},
    theme::{self, icons},
};
use moshionly_core::model::{DirectionalBlur, Glow, GlowChange, LayerProps, Noise, Shadow};
use moshionly_core::{Color, Edit, NodeId, NodeType};
use moshionly_render::{AssetLoader, FontLibrary, Renderer};
use serde_json::json;

/// The demo project that comes with the engine.
const DEMO: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../moshionly-core/examples/demo.moshi"
);

/// The editor on the demo project in a 1440 by 900 window, drawing on the
/// test's thread with the bundled fonts.
fn demo() -> Harness<'static, Shell> {
    demo_in(vec2(1440.0, 900.0))
}

/// The editor on the demo project in a window of `size`.
fn demo_in(size: Vec2) -> Harness<'static, Shell> {
    let session = SessionHandle::open(DEMO).expect("the demo opens");
    let renderer = Renderer::new(FontLibrary::bundled(), AssetLoader::default());
    let shell = Shell::headless(Settings::default(), session, renderer);
    let mut harness = Harness::builder()
        .with_size(size)
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

/// Selects the layers `ids`.
fn select(harness: &mut Harness<'_, Shell>, ids: &[&str]) {
    harness
        .state_mut()
        .editor_mut()
        .selection
        .set(ids.iter().map(|id| NodeId::from(*id)));
    harness.run();
}

/// The layer properties of `id` as the document has them now.
fn layer(harness: &Harness<'_, Shell>, id: &str) -> LayerProps {
    harness
        .state()
        .editor()
        .document()
        .get(&NodeId::from(id))
        .expect("the layer is there")
        .layer()
        .expect("it is a layer")
        .clone()
}

/// The field named `name` (and not the slider some fields come with).
fn field<'h>(harness: &'h Harness<'_, Shell>, name: &'h str) -> egui_kittest::Node<'h> {
    harness
        .get_all_by_label(name)
        .find(|node| node.accesskit_node().role() != egui::accesskit::Role::Slider)
        .unwrap_or_else(|| panic!("no field named {name}"))
}

/// Types `text` into the numeric field named `name` and presses Enter.
/// The field is clicked through accessibility, so it may be scrolled out of
/// view.
fn type_into(harness: &mut Harness<'_, Shell>, name: &str, text: &str) {
    field(harness, name).click_accesskit();
    harness.run();
    field(harness, name).type_text(text);
    harness.run();
    harness.key_press(Key::Enter);
    harness.run();
}

/// Chooses `item` ("Add glow") from the Effects section's add menu.
fn add_effect(harness: &mut Harness<'_, Shell>, item: &str) {
    harness.get_by_label("Add effect").click_accesskit();
    harness.run();
    harness.get_by_label(item).click_accesskit();
    harness.run();
}

fn undo(harness: &mut Harness<'_, Shell>) {
    harness.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    harness.run();
}

/// The name an undo of the last change would say.
fn undo_label(harness: &Harness<'_, Shell>) -> String {
    harness
        .state()
        .editor()
        .undo_label()
        .expect("something to undo")
}

#[test]
fn each_effect_is_added_from_the_menu_changed_and_undone() {
    let mut harness = demo();
    // The accent strip on the card has no effects.
    select(&mut harness, &["accent"]);
    for effect in Effect::ALL {
        assert!(!effect.is_on(&layer(&harness, "accent")), "{effect:?}");
    }

    add_effect(&mut harness, "Add shadow");
    assert_eq!(layer(&harness, "accent").shadow, Some(Shadow::default()));
    assert_eq!(undo_label(&harness), "Undo Add shadow");
    type_into(&mut harness, "Shadow blur", "20");
    assert_eq!(layer(&harness, "accent").shadow.unwrap().blur, 20.0);
    assert_eq!(undo_label(&harness), "Undo Change shadow");
    undo(&mut harness);
    assert_eq!(layer(&harness, "accent").shadow, Some(Shadow::default()));

    add_effect(&mut harness, "Add blur");
    assert_eq!(layer(&harness, "accent").blur, NEW_BLUR);
    type_into(&mut harness, "Layer blur", "12");
    assert_eq!(layer(&harness, "accent").blur, 12.0);
    undo(&mut harness);
    assert_eq!(layer(&harness, "accent").blur, NEW_BLUR);

    add_effect(&mut harness, "Add glow");
    assert_eq!(layer(&harness, "accent").glow, Some(Glow::default()));
    type_into(&mut harness, "Glow radius", "44");
    type_into(&mut harness, "Glow spread", "6");
    let glow = layer(&harness, "accent").glow.unwrap();
    assert_eq!((glow.radius, glow.spread), (44.0, 6.0));
    assert_eq!(undo_label(&harness), "Undo Change glow");
    // Each field typed into is one step.
    undo(&mut harness);
    assert_eq!(layer(&harness, "accent").glow.unwrap().spread, 0.0);
    assert_eq!(layer(&harness, "accent").glow.unwrap().radius, 44.0);
    undo(&mut harness);
    assert_eq!(layer(&harness, "accent").glow, Some(Glow::default()));

    add_effect(&mut harness, "Add noise");
    assert_eq!(layer(&harness, "accent").noise, Some(Noise::default()));
    // The amount is shown in percent and kept as a fraction.
    type_into(&mut harness, "Noise amount", "50");
    type_into(&mut harness, "Noise size", "3");
    let noise = layer(&harness, "accent").noise.unwrap();
    assert_eq!((noise.amount, noise.size, noise.animated), (0.5, 3.0, true));
    undo(&mut harness);
    assert_eq!(layer(&harness, "accent").noise.unwrap().size, 1.0);

    add_effect(&mut harness, "Add directional blur");
    assert_eq!(
        layer(&harness, "accent").directional_blur,
        Some(DirectionalBlur::default())
    );
    type_into(&mut harness, "Directional blur angle", "90");
    assert_eq!(
        layer(&harness, "accent").directional_blur.unwrap().angle,
        90.0
    );
    undo(&mut harness);
    assert_eq!(
        layer(&harness, "accent").directional_blur.unwrap().angle,
        0.0
    );

    // With every effect on, the menu has nothing left to offer.
    assert!(harness.query_by_label("Add effect").is_none());

    // Removing one takes it away as one step, and undo brings it back.
    harness.get_by_label("Remove glow").click_accesskit();
    harness.run();
    assert_eq!(layer(&harness, "accent").glow, None);
    assert_eq!(undo_label(&harness), "Undo Remove glow");
    assert!(harness.query_by_label("Add effect").is_some());
    undo(&mut harness);
    assert_eq!(layer(&harness, "accent").glow, Some(Glow::default()));
}

#[test]
fn removing_an_effect_wins_over_a_value_still_being_typed() {
    // Tall enough to show every effect, since these clicks are pointer
    // clicks.
    let mut harness = demo_in(vec2(1440.0, 1600.0));
    select(&mut harness, &["accent"]);
    for (effect, name) in [
        (Effect::Glow, "Glow radius"),
        (Effect::Blur, "Layer blur"),
        (Effect::Shadow, "Shadow blur"),
        (Effect::Noise, "Noise size"),
        (Effect::DirectionalBlur, "Directional blur amount"),
    ] {
        add_effect(&mut harness, &format!("Add {}", effect.lowercase()));
        field(&harness, name).click_accesskit();
        harness.run();
        field(&harness, name).type_text("7");
        harness.run();
        // A pointer click presses and lets go in one frame, as a tap on a
        // trackpad does: the typed value is kept in the same frame as the
        // removal.
        let remove = format!("Remove {}", effect.lowercase());
        harness.get_by_label(&remove).click();
        harness.run();
        assert!(!effect.is_on(&layer(&harness, "accent")), "{effect:?}");
        assert_eq!(undo_label(&harness), format!("Undo {remove}"));
        // The typed value was its own step: undo brings the effect back
        // as it was typed.
        undo(&mut harness);
        assert!(effect.is_on(&layer(&harness, "accent")), "{effect:?}");
        assert_eq!(
            undo_label(&harness),
            format!("Undo Change {}", effect.lowercase())
        );
    }
    assert_eq!(layer(&harness, "accent").glow.unwrap().radius, 7.0);
}

#[test]
fn several_layers_keep_their_own_effects() {
    let mut harness = demo();
    // The strip has a red glow and grain that stays still; the dot a wide
    // yellow glow and no grain.
    harness
        .state_mut()
        .editor_mut()
        .apply(
            "Give effects",
            &[
                Edit::update(
                    NodeId::from("accent"),
                    [
                        ("glow", json!({ "color": "#FF0000", "radius": 10 })),
                        ("noise", json!({ "amount": 0.6, "animated": false })),
                    ],
                ),
                Edit::set(
                    NodeId::from("dot"),
                    "glow",
                    json!({ "color": "#FFD60A", "radius": 30, "spread": 4 }),
                ),
            ],
        )
        .unwrap();
    select(&mut harness, &["accent", "dot"]);
    // Their glows differ in radius: the field says so.
    assert_eq!(
        field(&harness, "Glow radius")
            .accesskit_node()
            .value()
            .as_deref(),
        Some("Mixed")
    );
    type_into(&mut harness, "Glow radius", "50");
    let (accent, dot) = (layer(&harness, "accent"), layer(&harness, "dot"));
    assert_eq!(
        accent.glow,
        Some(Glow {
            color: Color::rgb(0xFF, 0, 0),
            radius: 50.0,
            spread: 0.0
        })
    );
    assert_eq!(
        dot.glow,
        Some(Glow {
            color: Color::rgb(0xFF, 0xD6, 0x0A),
            radius: 50.0,
            spread: 4.0
        })
    );
    // One step for both.
    undo(&mut harness);
    assert_eq!(layer(&harness, "accent").glow.unwrap().radius, 10.0);
    assert_eq!(layer(&harness, "dot").glow.unwrap().radius, 30.0);

    // Noise is offered because the dot has none; adding it gives the dot
    // the default and leaves the strip's own grain alone.
    add_effect(&mut harness, "Add noise");
    assert_eq!(layer(&harness, "dot").noise, Some(Noise::default()));
    let noise = layer(&harness, "accent").noise.unwrap();
    assert_eq!((noise.amount, noise.animated), (0.6, false));
    // Setting the size sets it on both, each keeping its strength.
    type_into(&mut harness, "Noise size", "2");
    assert_eq!(layer(&harness, "dot").noise.unwrap().amount, 0.2);
    assert_eq!(layer(&harness, "accent").noise.unwrap().amount, 0.6);
    assert_eq!(layer(&harness, "accent").noise.unwrap().size, 2.0);
    assert_eq!(layer(&harness, "dot").noise.unwrap().size, 2.0);
}

#[test]
fn artboards_offer_no_effects() {
    let mut harness = demo();
    select(&mut harness, &["scene"]);
    assert!(harness.query_by_label("Add effect").is_none());
    assert!(harness.query_by_label("Add shadow").is_none());
    // With a layer selected too, effects go to the layer only.
    select(&mut harness, &["scene", "dot"]);
    add_effect(&mut harness, "Add glow");
    assert!(layer(&harness, "dot").glow.is_some());
    assert!(layer(&harness, "scene").glow.is_none());
}

#[test]
fn a_glow_action_is_added_in_the_animate_tab() {
    let mut harness = demo();
    select(&mut harness, &["accent"]);
    // The strip glows red at rest.
    harness
        .state_mut()
        .editor_mut()
        .apply(
            "Give a glow",
            &[Edit::set(
                NodeId::from("accent"),
                "glow",
                json!({ "color": "#FF0000", "radius": 12 }),
            )],
        )
        .unwrap();
    harness.key_press(Key::M);
    harness.run();
    assert_eq!(harness.state().inspector_tab(), InspectorTab::Animate);

    // The add menu offers every effect, with its icon.
    harness
        .get_all_by_label("Add action")
        .next()
        .expect("an Add action button")
        .click();
    harness.run();
    for name in ["Shadow", "Blur", "Glow", "Noise", "Directional blur"] {
        assert!(harness.query_all_by_label(name).next().is_some(), "{name}");
    }
    // Below the fold of the menu, so chosen as a screen reader would.
    harness.get_by_label("Glow").click_accesskit();
    harness.run();

    let document = harness.state().editor().document().clone();
    let glows: Vec<NodeId> = document
        .animations_targeting(&NodeId::from("accent"))
        .into_iter()
        .filter(|node| node.node_type() == NodeType::Glow)
        .map(|node| node.id.clone())
        .collect();
    assert_eq!(glows.len(), 1);
    let glow = glows[0].clone();
    // It has a bar on the timeline, under the strip, with the glow's icon.
    let artboard = harness.state().editor().artboard().unwrap().clone();
    let groups = timeline_groups(&document, &artboard, &Default::default());
    let strip = groups.iter().find(|g| g.key == "accent").expect("a row");
    let bar = strip
        .bars
        .iter()
        .find(|bar| bar.key == glow.as_str())
        .expect("a bar for the glow");
    assert_eq!(bar.label, "Glow");
    assert_eq!(bar.icon, Some(icons::SUN_DIM));
    assert_eq!(action_icon(NodeType::Glow), icons::SUN_DIM);

    // Its From starts as the glow the strip has; writing it down gives
    // every part of that glow, ready to change.
    harness.get_by_label("Current value").click_accesskit();
    harness.run();
    let from = harness
        .state()
        .editor()
        .document()
        .get(&glow)
        .unwrap()
        .to_value()["from"]
        .clone();
    assert_eq!(
        serde_json::from_value::<GlowChange>(from).unwrap(),
        GlowChange {
            color: Some(Color::rgb(0xFF, 0, 0)),
            radius: Some(12.0),
            spread: Some(0.0),
        }
    );
    // Each part has its own field.
    type_into(&mut harness, "Spread", "8");
    let from = harness
        .state()
        .editor()
        .document()
        .get(&glow)
        .unwrap()
        .to_value()["from"]
        .clone();
    assert_eq!(from["spread"].as_f64(), Some(8.0));
    assert_eq!(from["radius"].as_f64(), Some(12.0));
}

#[test]
fn effects_draw_on_the_canvas_without_moving_the_selection_box() {
    let mut harness = demo();
    select(&mut harness, &["dot"]);
    let artboard = NodeId::from("scene");
    let dot = NodeId::from("dot");
    let before = harness.state_mut().editor_mut().box_of(&dot).unwrap();
    let version = harness.state().editor().artboard_version(&artboard);
    add_effect(&mut harness, "Add glow");
    // The artboard is drawn again.
    assert!(harness.state().editor().artboard_version(&artboard) > version);
    harness.run();
    let picture = harness.state().pictures().picture(&artboard).unwrap();
    assert_eq!(
        picture.key.revision,
        harness.state().editor().artboard_version(&artboard)
    );
    // The selection keeps to the dot's own box, as handles go on the
    // layer and not its glow; what it draws reaches past it.
    let editor = harness.state_mut().editor_mut();
    assert_eq!(editor.box_of(&dot).unwrap(), before);
    let scene = editor.scene(&artboard).unwrap();
    let bounds = editor.renderer().bounds_of(&scene, &dot).unwrap();
    let drawn = bounds.visual.expect("the dot draws something");
    let glow = Glow::default();
    assert!(
        drawn.width > 52.0 + glow.radius,
        "the glow is part of what the dot draws: {drawn:?}"
    );
}
