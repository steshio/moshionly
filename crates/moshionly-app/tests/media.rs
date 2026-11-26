//! Video and sound in the editor, driven without a window: importing
//! through the dialog, the clips on the timeline, playing and pausing.
//!
//! The media files are made with `ffmpeg` in a folder of their own; the
//! tests that need it say so and pass when it is not installed.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use egui::{Key, Modifiers, Pos2, vec2};
use egui_kittest::{
    Harness,
    kittest::{NodeT as _, Queryable as _},
};
use moshionly_app::{
    audio::{AudioCall, SilentAudio},
    commands::CommandId,
    dialogs::Dialogs,
    editor::timeline::{SOUND_GROUP, timeline_groups},
    settings::Settings,
    shell::{Shell, untitled_session},
    theme,
    timeline::rows,
};
use moshionly_core::{NodeId, NodeType, eval::duration_of};
use moshionly_export::media::MediaTools;
use moshionly_render::{AssetLoader, FontLibrary, Renderer};

/// The tools, or `None` (after saying so) when they are not installed.
fn tools(test: &str) -> Option<MediaTools> {
    let tools = MediaTools::locate();
    if tools.is_none() {
        println!("skipped {test}: ffmpeg is not installed");
    }
    tools
}

/// A folder of its own for one test's files, emptied first.
fn scratch(name: &str) -> PathBuf {
    let folder =
        std::env::temp_dir().join(format!("moshionly-app-media-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).unwrap();
    folder
}

/// Makes `name` in `dir` with `ffmpeg` and these arguments.
fn make(tools: &MediaTools, dir: &Path, name: &str, args: &[&str]) -> PathBuf {
    let path = dir.join(name);
    let output = Command::new(tools.ffmpeg())
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(args)
        .arg(&path)
        .output()
        .expect("ffmpeg runs");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    path
}

/// Two seconds of a 160 by 90 test picture with a tone.
fn video(tools: &MediaTools, dir: &Path) -> PathBuf {
    make(
        tools,
        dir,
        "wave.mp4",
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc=size=160x90:rate=10:duration=2",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000:duration=2",
            "-c:v",
            "mpeg4",
            "-c:a",
            "aac",
            "-shortest",
        ],
    )
}

/// Six seconds of a tone: longer than a new scene.
fn sound(tools: &MediaTools, dir: &Path) -> PathBuf {
    make(
        tools,
        dir,
        "tone.wav",
        &[
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=330:sample_rate=48000:duration=6",
            "-c:a",
            "pcm_s16le",
        ],
    )
}

/// Dialogs that pick `media` when asked for video or sound, and count how
/// often they were asked.
struct MediaDialogs {
    media: Vec<PathBuf>,
    asked: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

impl Dialogs for MediaDialogs {
    fn open_project(&self) -> Option<PathBuf> {
        None
    }

    fn save_project(&self, _suggested_name: &str) -> Option<PathBuf> {
        None
    }

    fn import_images(&self) -> Option<Vec<PathBuf>> {
        None
    }

    fn import_media(&self) -> Option<Vec<PathBuf>> {
        self.asked
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Some(self.media.clone())
    }
}

/// The editor on a new project, reading media with `tools` (none when
/// `None`), answering the media dialog with `media`, and writing down what
/// it asks of its sound output.
fn harness(
    tools: Option<&MediaTools>,
    media: Vec<PathBuf>,
) -> (
    Harness<'static, Shell>,
    SilentAudio,
    std::sync::Arc<std::sync::atomic::AtomicUsize>,
) {
    let renderer = Renderer::new(
        FontLibrary::bundled(),
        match tools {
            Some(tools) => moshionly_app::media::asset_loader(None, Some(tools)),
            None => AssetLoader::default(),
        },
    );
    let mut shell = Shell::headless(Settings::default(), untitled_session(), renderer);
    shell.set_media(tools.cloned());
    let asked = std::sync::Arc::default();
    shell.set_dialogs(Box::new(MediaDialogs {
        media,
        asked: std::sync::Arc::clone(&asked),
    }));
    let audio = SilentAudio::recording();
    shell.set_audio(Box::new(audio.clone()));
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
    (harness, audio, asked)
}

/// The nodes of a type in the open document.
fn nodes_of(harness: &Harness<'_, Shell>, ty: NodeType) -> Vec<NodeId> {
    harness
        .state()
        .editor()
        .document()
        .nodes()
        .filter(|node| node.node_type() == ty)
        .map(|node| node.id.clone())
        .collect()
}

#[test]
fn video_and_sound_are_imported_shown_on_the_timeline_and_played() {
    let Some(tools) = tools("video_and_sound_are_imported_shown_on_the_timeline_and_played") else {
        return;
    };
    let dir = scratch("import");
    let files = vec![video(&tools, &dir), sound(&tools, &dir)];
    let (mut harness, audio, asked) = harness(Some(&tools), files.clone());

    // ⌥⌘I asks for files and adds them, as one undo step.
    harness.key_press_modifiers(Modifiers::COMMAND | Modifiers::ALT, Key::I);
    harness.run();
    assert_eq!(asked.load(std::sync::atomic::Ordering::Relaxed), 1);
    let videos = nodes_of(&harness, NodeType::Video);
    let sounds = nodes_of(&harness, NodeType::Audio);
    assert_eq!((videos.len(), sounds.len()), (1, 1));
    let shell = harness.state();
    let document = std::sync::Arc::clone(shell.editor().document());
    let film = document.get(&videos[0]).unwrap();
    // A project never saved uses the files where they are.
    assert_eq!(
        film.data.source(),
        Some(files[0].to_string_lossy().as_ref())
    );
    let layer = film.layer().unwrap();
    assert_eq!((layer.width, layer.height), (160.0, 90.0));
    let clip = film.data.clip().unwrap();
    assert_eq!(clip.start, 0.0);
    assert!((clip.end - 2000.0).abs() < 100.0, "{clip:?}");
    let tone = document.get(&sounds[0]).unwrap();
    let (start, end) = tone.data.span().unwrap();
    assert_eq!(start, 0.0);
    assert!((end - 6000.0).abs() < 100.0, "{end}");
    // The scene grew to fit the sound, and said so.
    let artboard = shell.editor().artboard().unwrap().clone();
    assert_eq!(duration_of(&document, &artboard), Some(end.ceil()));
    assert!(
        harness
            .query_by_label_contains("The scene now lasts")
            .is_some()
    );
    assert_eq!(shell.editor().selection.ids(), videos.as_slice());
    assert_eq!(
        shell.editor().undo_label().as_deref(),
        Some("Undo Add video and sound")
    );
    // On the timeline: the video's clip, and the sound in its own group.
    let groups = timeline_groups(&document, &artboard, &HashSet::new());
    let keys: Vec<&str> = groups.iter().map(|group| group.key.as_str()).collect();
    assert_eq!(keys, [videos[0].as_str(), SOUND_GROUP]);
    assert_eq!(groups[0].bars[0].label, "wave.mp4");
    assert_eq!(groups[1].bars[0].label, "tone.wav");

    // Space plays, with the scene's sound once it is mixed.
    harness.key_press(Key::Space);
    harness.run_steps(2);
    assert!(harness.state().is_playing());
    let mut mixed = None;
    for _ in 0..600 {
        harness.step();
        mixed = audio.calls().into_iter().find_map(|call| match call {
            AudioCall::Track(Some(length)) => Some(length),
            _ => None,
        });
        if mixed.is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let length = mixed.expect("the sound was mixed");
    assert!((length - end.ceil()).abs() < 1.0, "{length}");
    let calls = audio.calls();
    assert!(
        matches!(calls.first(), Some(AudioCall::Play(at)) if *at == 0.0),
        "{calls:?}"
    );
    // A new track while playing goes on from the playhead.
    let track = calls
        .iter()
        .position(|call| matches!(call, AudioCall::Track(Some(_))))
        .unwrap();
    if harness.state().is_playing() {
        assert!(
            matches!(calls.get(track + 1), Some(AudioCall::Seek(_))),
            "{calls:?}"
        );
    }
    // Space again pauses, and the sound stops.
    if !harness.state().is_playing() {
        harness.key_press(Key::Space);
        harness.run_steps(2);
    }
    harness.key_press(Key::Space);
    harness.run_steps(2);
    assert!(!harness.state().is_playing());
    assert_eq!(audio.calls().last(), Some(&AudioCall::Stop));
    let _ = std::fs::remove_dir_all(&dir);
}

/// A file dropped on the window.
#[derive(Debug)]
struct Dropped(PathBuf);

impl egui::DroppedFile for Dropped {
    fn path(&self) -> &Path {
        &self.0
    }

    fn bytes(&self) -> Result<Vec<u8>, String> {
        std::fs::read(&self.0).map_err(|error| error.to_string())
    }
}

#[test]
fn without_ffmpeg_media_says_how_to_get_it() {
    let dir = scratch("no-ffmpeg");
    let file = dir.join("song.mp3");
    std::fs::write(&file, b"not really sound").unwrap();
    let (mut harness, _audio, asked) = harness(None, vec![file.clone()]);
    let registry = harness.state_mut().commands_mut();
    assert!(!registry.is_enabled(CommandId::ImportMedia));
    assert_eq!(
        registry.disabled_reason(CommandId::ImportMedia),
        Some(moshionly_app::media::FFMPEG_NEEDED)
    );
    // The shortcut does nothing; the dialog is never shown.
    harness.key_press_modifiers(Modifiers::COMMAND | Modifiers::ALT, Key::I);
    harness.run();
    assert_eq!(asked.load(std::sync::atomic::Ordering::Relaxed), 0);
    // A sound dropped on the window says how to get ffmpeg.
    harness
        .input_mut()
        .dropped_files
        .push(std::sync::Arc::new(Dropped(file)));
    harness.run();
    assert!(
        harness
            .query_by_label_contains(moshionly_app::media::FFMPEG_NEEDED)
            .is_some()
    );
    assert!(nodes_of(&harness, NodeType::Audio).is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

fn field<'h>(harness: &'h Harness<'_, Shell>, name: &'h str) -> egui_kittest::Node<'h> {
    harness
        .get_all_by_label(name)
        .find(|node| node.accesskit_node().role() != egui::accesskit::Role::Slider)
        .unwrap_or_else(|| panic!("no field named {name}"))
}

/// Types `text` into the numeric field named `name` and presses Enter.
fn type_into(harness: &mut Harness<'_, Shell>, name: &str, text: &str) {
    field(harness, name).click_accesskit();
    harness.run();
    field(harness, name).type_text(text);
    harness.run();
    harness.key_press(Key::Enter);
    harness.run();
}

/// The editor on a scene with a video and a sound whose files are not
/// there, without ffmpeg.
fn scene_harness() -> Harness<'static, Shell> {
    scene_harness_with("")
}

/// [`scene_harness`] with more nodes, written as JSON objects each
/// followed by a comma.
fn scene_harness_with(more: &str) -> Harness<'static, Shell> {
    let document = moshionly_core::io::parse(&format!(
        r##"{{"format":"moshionly","version":3,"name":"Media","nodes":[
        {more}
        {{"id":"ab","parent":null,"index":"a0","type":"artboard","duration":5000}},
        {{"id":"film","parent":"ab","index":"a0","type":"video","source":"/no/film.mp4",
         "width":320,"height":180,"start":1000,"end":3000,"trim_start":500}},
        {{"id":"song","parent":"ab","index":"a0","type":"audio","source":"/no/song.mp3",
         "start":0,"end":2000}}
        ]}}"##
    ))
    .unwrap();
    let renderer = Renderer::new(FontLibrary::bundled(), AssetLoader::default());
    let shell = Shell::headless(
        Settings::default(),
        moshionly_agent::SessionHandle::from_document(document),
        renderer,
    );
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

#[test]
fn the_export_dialog_offers_sound_where_it_can_go() {
    let mut harness = scene_harness();
    harness.state_mut().open_export_dialog();
    harness.run();
    let choose = |harness: &mut Harness<'_, Shell>, format| {
        let setup = harness.state_mut().export_setup_mut().unwrap();
        setup.choices.set_format(format);
        harness.run();
    };
    choose(&mut harness, moshionly_export::Format::Mp4);
    assert!(harness.query_by_label("Include sound").is_some());
    assert!(harness.query_by_label_contains("with sound").is_some());
    // Unticked, the MP4 is silent.
    harness.get_by_label("Include sound").click();
    harness.run();
    assert!(!harness.state().export_setup().unwrap().choices.sound);
    assert!(harness.query_by_label_contains("with sound").is_none());
    // A GIF holds no sound: nothing to choose.
    choose(&mut harness, moshionly_export::Format::Gif);
    assert!(harness.query_by_label("Include sound").is_none());
}

#[test]
fn the_inspector_sets_when_a_video_and_a_sound_play() {
    let mut harness = scene_harness();
    let clip = |harness: &Harness<'_, Shell>, id: &str| {
        *harness
            .state()
            .editor()
            .document()
            .get(&NodeId::from(id))
            .unwrap()
            .data
            .clip()
            .unwrap()
    };
    // The video's section in the Design tab.
    harness
        .state_mut()
        .editor_mut()
        .selection
        .select(NodeId::from("film"));
    harness.run();
    assert!(harness.query_by_label("Replace video…").is_some());
    assert!(
        harness
            .query_all_by_label_contains("film.mp4")
            .next()
            .is_some()
    );
    type_into(&mut harness, "Start", "2");
    type_into(&mut harness, "Trim start", "0.25");
    type_into(&mut harness, "Volume", "40");
    let film = clip(&harness, "film");
    assert_eq!(
        (film.start, film.end),
        (2000.0, 4000.0),
        "moved, same length"
    );
    assert_eq!(film.trim_start, 250.0);
    assert_eq!(film.volume, 0.4);

    // A sound, which belongs to no layer, in the Animate tab.
    harness.state_mut().editor_mut().selection.clear();
    harness.run();
    harness.state_mut().inspector_mut().tab = moshionly_app::shell::InspectorTab::Animate;
    harness
        .state_mut()
        .editor_mut()
        .set_actions([NodeId::from("song")]);
    harness.run();
    assert!(harness.query_by_label("Replace sound…").is_some());
    type_into(&mut harness, "End", "3.5");
    assert_eq!(clip(&harness, "song").end, 3500.0);
    assert_eq!(
        harness.state().editor().undo_label().as_deref(),
        Some("Undo Change end")
    );
}

/// The middle of the timeline bar `key` on screen.
fn bar_center(harness: &Harness<'_, Shell>, key: &str) -> Pos2 {
    let shell = harness.state();
    let document = shell.editor().document();
    let artboard = shell.editor().artboard().unwrap().clone();
    let groups = timeline_groups(document, &artboard, &HashSet::new());
    let (row, bar) = rows(&groups)
        .iter()
        .enumerate()
        .find_map(|(index, row)| match *row {
            moshionly_app::timeline::Row::Bar(g, b) if groups[g].bars[b].key == key => {
                Some((index, groups[g].bars[b].clone()))
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("no bar {key}"));
    let layout = shell.timeline_layout().expect("the timeline was drawn");
    layout.bar_rect(row, bar.start, bar.end).center()
}

/// Presses at `from`, moves to `to` in a few steps and lets go.
fn drag(harness: &mut Harness<'_, Shell>, from: Pos2, to: Pos2) {
    harness.hover_at(from);
    harness.drag_at(from);
    for step in 1..=4 {
        let t = step as f32 / 4.0;
        harness.hover_at(from + (to - from) * t);
        harness.run();
    }
    harness.drop_at(to);
    harness.run();
}

#[test]
fn dragging_an_action_of_a_video_leaves_its_clip_where_it_is() {
    let mut harness = scene_harness_with(
        r##"{"id":"fade","parent":"ab","index":"a1","type":"fade_in","target":"film",
         "start":1000,"end":1500},"##,
    );
    let times = |harness: &Harness<'_, Shell>, id: &str| {
        let document = harness.state().editor().document();
        let node = document.get(&NodeId::from(id)).unwrap();
        match node.data.clip() {
            Some(clip) => (clip.start, clip.end),
            None => node.data.span().unwrap(),
        }
    };
    let shift = |harness: &mut Harness<'_, Shell>, key: &str, ms: f64| {
        let from = bar_center(harness, key);
        let layout = harness.state().timeline_layout().unwrap();
        let to = from + vec2(layout.scale.points_per_ms * ms as f32, 0.0);
        drag(harness, from, to);
    };
    // The video picked on the canvas, then its fade on the timeline.
    harness
        .state_mut()
        .editor_mut()
        .selection
        .select(NodeId::from("film"));
    harness.run();
    let fade = bar_center(&harness, "fade");
    harness.hover_at(fade);
    harness.drag_at(fade);
    harness.drop_at(fade);
    harness.run();
    assert_eq!(
        harness.state().editor().actions(),
        [NodeId::from("fade")].as_slice()
    );
    assert_eq!(
        harness.state().editor().selection.ids(),
        [NodeId::from("film")].as_slice()
    );
    shift(&mut harness, "fade", 400.0);
    assert_eq!(times(&harness, "fade"), (1400.0, 1900.0));
    assert_eq!(times(&harness, "film"), (1000.0, 3000.0), "the clip stayed");
    assert_eq!(
        harness.state().editor().undo_label().as_deref(),
        Some("Undo Move action")
    );
    harness.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    harness.run();

    // The same with the fade chosen in the Animate tab.
    harness
        .state_mut()
        .editor_mut()
        .set_actions([NodeId::from("fade")]);
    harness.run();
    shift(&mut harness, "fade", 400.0);
    assert_eq!(times(&harness, "fade"), (1400.0, 1900.0));
    assert_eq!(times(&harness, "film"), (1000.0, 3000.0), "the clip stayed");
    harness.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    harness.run();

    // With no action chosen, the selected video's clip is dragged.
    harness.state_mut().editor_mut().set_actions([]);
    harness.run();
    shift(&mut harness, "film/clip", 400.0);
    assert_eq!(times(&harness, "film"), (1400.0, 3400.0));
    assert_eq!(times(&harness, "fade"), (1000.0, 1500.0));
    assert_eq!(
        harness.state().editor().undo_label().as_deref(),
        Some("Undo Move clip")
    );
    harness.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    harness.run();

    // A clip picked together with the fade moves with it.
    let clip = bar_center(&harness, "film/clip");
    harness.hover_at(clip);
    harness.drag_at(clip);
    harness.drop_at(clip);
    harness.run();
    let fade = bar_center(&harness, "fade");
    harness.hover_at(fade);
    harness.event(egui::Event::ModifiersChanged(Modifiers::SHIFT));
    for pressed in [true, false] {
        harness.event(egui::Event::PointerButton {
            pos: fade,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::SHIFT,
        });
    }
    harness.event(egui::Event::ModifiersChanged(Modifiers::NONE));
    harness.run();
    assert_eq!(
        harness.state().editor().actions(),
        [NodeId::from("fade")].as_slice()
    );
    shift(&mut harness, "fade", 400.0);
    assert_eq!(times(&harness, "fade"), (1400.0, 1900.0));
    assert_eq!(
        times(&harness, "film"),
        (1400.0, 3400.0),
        "picked, so moved"
    );
}
