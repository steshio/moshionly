//! One project from the first edit to the last undo, through the public API
//! only: build a two-artboard document with edits, save it, load it,
//! evaluate it at several times, and undo everything back to nothing.

use moshionly_core::build::{self, Reorder};
use moshionly_core::eval::{self, Property, PropertyValue};
use moshionly_core::model::{
    CornerRadius, FadePreset, Group, LayerProps, MoveAnimation, NodeData, Offset, Paint, RectLayer,
    ResizeAnchor, ResizeAnimation, RotateAnimation, SizeChange, SlidePreset, TextAnimation,
    TextLayer, TextSplit, Timing,
};
use moshionly_core::scene::{Content, ScenePaint};
use moshionly_core::{
    Affine, Color, Document, Easing, Edit, Family, History, MonospaceMeasurer, NodeId, NodeType,
    Point, Scene, evaluate, io,
};

/// Applies edits through the history and checks the document is still sound.
fn apply(history: &mut History, document: &mut Document, edits: Vec<Edit>) {
    history
        .apply(document, &edits)
        .unwrap_or_else(|error| panic!("{error}"));
    document
        .validate()
        .unwrap_or_else(|problem| panic!("{problem}"));
}

fn timing(target: &NodeId, start: f64, end: f64) -> Timing {
    Timing::new(target, start, end).with_easing(Easing::Linear)
}

fn frame(document: &Document, artboard: &NodeId, time: f64) -> Scene {
    evaluate(document, artboard, time, &MonospaceMeasurer::default()).unwrap()
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

fn same(a: Affine, b: Affine) -> bool {
    a.0.iter().zip(b.0).all(|(a, b)| close(*a, b))
}

fn text_of(scene: &Scene, id: &NodeId) -> (String, Vec<(String, f64)>) {
    let Content::Text(text) = &scene.find(id).expect("the text is in the scene").content else {
        panic!("not a text");
    };
    let units = text
        .units
        .iter()
        .map(|unit| (text.text[unit.start..unit.end].to_owned(), unit.opacity))
        .collect();
    (text.text.clone(), units)
}

#[test]
fn a_project_from_the_first_edit_to_the_last_undo() {
    let mut document = Document::new("Workflow");
    let mut history = History::new();
    // Every call is one undo step.
    let step = apply;

    // An artboard with a card, and in the card a bar and two texts.
    let (intro, edits) = build::add_artboard(&document, "Intro", 800.0, 600.0).unwrap();
    step(&mut history, &mut document, edits);
    let (card, edits) = build::add_layer(
        &document,
        &intro,
        Group {
            layer: LayerProps::new("Card", 100.0, 100.0, 400.0, 200.0),
            fill: Some(Paint::Solid(Color::WHITE)),
            corner_radius: CornerRadius::Uniform(16.0),
            ..Group::default()
        },
    )
    .unwrap();
    step(&mut history, &mut document, edits);
    let (bar, edits) = build::add_layer(
        &document,
        &card,
        RectLayer {
            layer: LayerProps::new("Bar", 20.0, 20.0, 100.0, 20.0),
            fill: Some(Paint::Solid(Color::rgb(0x0D, 0x99, 0xFF))),
            ..RectLayer::default()
        },
    )
    .unwrap();
    step(&mut history, &mut document, edits);
    let text = |name: &str, y: f64, content: &str| TextLayer {
        layer: LayerProps::new(name, 20.0, y, 100.0, 24.0),
        fill: Some(Paint::Solid(Color::BLACK)),
        content: content.to_owned(),
        font_size: 20.0,
        ..TextLayer::default()
    };
    let (title, edits) =
        build::add_layer(&document, &card, text("Title", 60.0, "Hello world")).unwrap();
    step(&mut history, &mut document, edits);
    let (count, edits) = build::add_layer(&document, &card, text("Count", 100.0, "42 %")).unwrap();
    step(&mut history, &mut document, edits);
    assert_eq!(
        document.children(&card, Family::Layer),
        [bar.clone(), title.clone(), count.clone()]
    );

    // Animations: the card fades in while it moves right; the bar grows
    // from its left edge and then turns; the title comes in word by word;
    // the count runs up from zero.
    let animations: Vec<NodeData> = vec![
        NodeData::FadeIn(FadePreset {
            timing: timing(&card, 0.0, 500.0),
        }),
        MoveAnimation {
            timing: timing(&card, 0.0, 1000.0),
            from: None,
            to: Some(Offset::new(200.0, 0.0)),
        }
        .into(),
        ResizeAnimation {
            timing: timing(&bar, 0.0, 1000.0),
            from: None,
            to: Some(SizeChange {
                width: Some(300.0),
                height: None,
            }),
            anchor: ResizeAnchor::Left,
        }
        .into(),
        RotateAnimation {
            timing: timing(&bar, 500.0, 1500.0),
            from: None,
            to: Some(90.0),
        }
        .into(),
        NodeData::TextIn(TextAnimation {
            split: TextSplit::Words,
            stagger: 500.0,
            ..TextAnimation::new(timing(&title, 1000.0, 2000.0))
        }),
        build::default_animation(NodeType::Counter, &count, 0.0)
            .map(|mut counter| {
                if let Some(timing) = counter.timing_mut() {
                    (timing.end, timing.easing) = (1000.0, Easing::Linear);
                }
                counter
            })
            .unwrap(),
    ];
    let mut move_id = None;
    for animation in animations {
        let is_move = animation.node_type() == NodeType::Move;
        let (id, edits) = build::add_animation(&document, animation, None).unwrap();
        step(&mut history, &mut document, edits);
        if is_move {
            move_id = Some(id);
        }
    }
    assert_eq!(document.children(&intro, Family::Animation).len(), 6);

    // Dragging the card down is many edits and one undo step.
    history.begin();
    for y in (105..=150).step_by(5) {
        history
            .apply(&mut document, &[Edit::set(&card, "y", y)])
            .unwrap();
    }
    history.commit(&document);
    // One artboard, four layers, six animations and the drag.
    assert_eq!(history.undo_depth(), 1 + 4 + 6 + 1);

    // A second artboard: a copy of the first, where the card slides out
    // instead of moving.
    let (edits, copies) = build::duplicate(&document, std::slice::from_ref(&intro)).unwrap();
    step(&mut history, &mut document, edits);
    let outro = copies[&intro].clone();
    let outro_card = copies[&card].clone();
    step(
        &mut history,
        &mut document,
        vec![
            Edit::set(&outro, "name", "Outro"),
            Edit::remove(&copies[&move_id.unwrap()]),
        ],
    );
    let (_, edits) = build::add_animation(
        &document,
        NodeData::SlideOut(SlidePreset {
            timing: timing(&outro_card, 1000.0, 2000.0),
            direction: moshionly_core::model::Direction::Right,
            distance: 300.0,
            fade: true,
        }),
        None,
    )
    .unwrap();
    step(&mut history, &mut document, edits);
    let edits = build::reorder(&document, &outro, Reorder::ToBack).unwrap();
    step(&mut history, &mut document, edits);
    assert_eq!(document.artboards(), [outro.clone(), intro.clone()]);
    assert_eq!(document.len(), 2 * (1 + 4 + 6));

    // Save, load, and get the same document back, byte for byte on disk.
    let folder = std::env::temp_dir().join(format!("moshionly-workflow-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let path = folder.join("workflow.moshi");
    io::save(&document, &path).unwrap();
    let saved = std::fs::read_to_string(&path).unwrap();
    let loaded = io::load(&path).unwrap();
    assert_eq!(loaded, document);
    assert_eq!(io::to_string(&loaded), saved);
    assert_eq!(
        saved.lines().count(),
        5 + document.len() + 2,
        "one node per line"
    );
    std::fs::remove_dir_all(&folder).unwrap();

    // Evaluate the loaded document. At 0 the card has not faded in yet.
    assert!(frame(&loaded, &intro, 0.0).nodes.is_empty());

    // A quarter of the way: half faded in, 50 of its 200 to the right.
    let scene = frame(&loaded, &intro, 250.0);
    let card_node = scene.find(&card).unwrap();
    assert!(close(card_node.opacity, 0.5));
    assert!(same(card_node.transform, Affine::translate(150.0, 150.0)));
    let bar_node = scene.find(&bar).unwrap();
    assert!(close(bar_node.width, 150.0), "{}", bar_node.width);
    assert!(
        same(bar_node.transform, Affine::translate(20.0, 20.0)),
        "its left edge has not moved"
    );
    assert_eq!(text_of(&frame(&loaded, &intro, 500.0), &count).0, "21 %");
    assert!(
        text_of(&scene, &title).1.is_empty(),
        "the title waits for its cue"
    );
    assert_eq!(
        eval::value_at(&loaded, &card, Property::Opacity, 250.0),
        Some(PropertyValue::Number(50.0))
    );

    // At one second everything about the card has arrived, the bar is full
    // length and half way through its turn.
    let scene = frame(&loaded, &intro, 1000.0);
    let card_node = scene.find(&card).unwrap();
    assert_eq!(card_node.opacity, 1.0);
    assert!(same(card_node.transform, Affine::translate(300.0, 150.0)));
    let bar_node = scene.find(&bar).unwrap();
    assert!(same(
        bar_node.transform,
        Affine::layer(20.0, 20.0, 300.0, 20.0, 45.0, 1.0)
    ));
    // The bar's centre on the artboard: card at (300, 150), bar centre at
    // (20 + 150, 20 + 10) inside it.
    let center = scene
        .transform_of(&bar)
        .unwrap()
        .apply(Point::new(150.0, 10.0));
    assert!(
        close(center.x, 470.0) && close(center.y, 180.0),
        "{center:?}"
    );
    assert_eq!(text_of(&scene, &count).0, "42 %");
    // The card's background is its first part, in the card's own space.
    let Content::Rect { fill, radii, .. } = &card_node.children()[0].content else {
        panic!("the card starts with its background");
    };
    assert_eq!(
        (fill, radii),
        (&Some(ScenePaint::solid(Color::WHITE)), &[16.0; 4])
    );

    // The title: first word half in at 1250, both in at 2000.
    let (_, units) = text_of(&frame(&loaded, &intro, 1250.0), &title);
    assert_eq!(units, [("Hello".to_owned(), 0.5)]);
    let (_, units) = text_of(&frame(&loaded, &intro, 1750.0), &title);
    assert_eq!(
        units,
        [("Hello".to_owned(), 1.0), ("world".to_owned(), 0.5)]
    );
    let (_, units) = text_of(&frame(&loaded, &intro, 2000.0), &title);
    assert_eq!(units, [("Hello world".to_owned(), 1.0)]);

    // The other artboard: no move, and a slide out that is half done at 1500.
    let scene = frame(&loaded, &outro, 1500.0);
    let card_node = scene.find(&outro_card).unwrap();
    assert!(close(card_node.opacity, 0.5));
    assert!(same(card_node.transform, Affine::translate(250.0, 150.0)));
    assert!(
        frame(&loaded, &outro, 2000.0).nodes.is_empty(),
        "slid out and faded away"
    );
    assert_eq!(eval::duration_of(&loaded, &outro), Some(4000.0));

    // Undo everything, step by step, back to an empty document.
    let complete = document.clone();
    let steps = history.undo_depth();
    assert_eq!(
        steps,
        12 + 4,
        "plus the copy, its two changes and the reorder"
    );
    let mut undone = 0;
    while history.undo(&mut document).unwrap() {
        document.validate().unwrap();
        undone += 1;
    }
    assert_eq!(undone, steps);
    assert!(document.is_empty());
    // And redo it all.
    while history.redo(&mut document).unwrap() {}
    assert_eq!(document, complete);
}

#[test]
fn the_demo_project_loads_and_plays() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/examples/demo.moshi");
    let text = std::fs::read_to_string(path).unwrap();
    let document = io::load(path).unwrap();
    document.validate().unwrap();
    assert_eq!(
        io::to_string(&document),
        text,
        "the example is in the saved form"
    );

    let scene_id = document.artboards()[0].clone();
    let id = |name: &str| NodeId::from(name);
    let measurer = MonospaceMeasurer::default();
    let at = |time: f64| evaluate(&document, &scene_id, time, &measurer).unwrap();

    // Before anything starts the card is invisible, so the frame is empty.
    let first = at(0.0);
    assert_eq!((first.width, first.height), (1080.0, 1080.0));
    assert!(first.nodes.is_empty());

    // The card fades in over the first half second while sliding up 80.
    let early = at(250.0);
    let card = early.find(&id("card")).unwrap();
    assert!(card.opacity > 0.5 && card.opacity < 1.0, "{}", card.opacity);
    let y = card.transform.apply(Point::ZERO).y;
    assert!(y > 340.0 && y < 420.0, "{y}");

    // The title arrives letter by letter between 500 and 1500.
    let units_at = |time: f64| {
        let scene = at(time);
        match &scene.find(&id("title")).unwrap().content {
            Content::Text(text) => text.units.clone(),
            other => panic!("{other:?}"),
        }
    };
    assert!(units_at(500.0).is_empty());
    let arriving = units_at(900.0);
    assert!(
        arriving.len() > 2,
        "several letters on their way: {arriving:?}"
    );
    assert!(
        arriving
            .windows(2)
            .all(|pair| pair[0].opacity >= pair[1].opacity)
    );
    assert_eq!(units_at(1500.0).len(), 1);

    // The counter runs from $0 to the amount in the text and never goes back.
    let raised_at =
        |time: f64| match eval::value_at(&document, &id("raised"), Property::Content, time) {
            Some(PropertyValue::Text(text)) => text,
            other => panic!("{other:?}"),
        };
    assert_eq!(raised_at(0.0), "$0 raised");
    assert_eq!(raised_at(2600.0), "$12,480 raised");
    let amounts: Vec<f64> = (9..=26)
        .map(|tenth| {
            moshionly_core::text::find_number(&raised_at(f64::from(tenth) * 100.0))
                .unwrap()
                .value
        })
        .collect();
    assert!(
        amounts.windows(2).all(|pair| pair[0] <= pair[1]),
        "{amounts:?}"
    );
    assert!(
        raised_at(1700.0).contains(','),
        "thousands keep their separator on the way"
    );

    // The dot pops in with an overshoot: for a moment it is larger than life.
    let dot_scale = |time: f64| match eval::value_at(&document, &id("dot"), Property::Scale, time) {
        Some(PropertyValue::Number(scale)) => scale,
        other => panic!("{other:?}"),
    };
    assert_eq!(dot_scale(1400.0), 0.0);
    assert!((14..=20).any(|tenth| dot_scale(f64::from(tenth) * 100.0) > 1.0));
    assert_eq!(dot_scale(2000.0), 1.0);

    // At the end everything rests where it was designed.
    let last = at(4000.0);
    let rest = moshionly_core::evaluate_at_rest(&document, &scene_id, &measurer).unwrap();
    assert_eq!(last.nodes, rest.nodes);
    assert!(same(
        last.find(&id("card")).unwrap().transform,
        Affine::translate(240.0, 340.0)
    ));
    // Every frame of the timeline evaluates, and the same time gives the same frame.
    for time in (0..=4000).step_by(50) {
        let time = f64::from(time);
        assert_eq!(at(time), at(time));
    }
}
