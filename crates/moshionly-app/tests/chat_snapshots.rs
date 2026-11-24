//! Headless pictures of the AI chat: the panel empty, mid-answer with tool
//! calls, after an error and with no AI ready, and the AI settings dialog,
//! each in light and dark at 1x and 2x.
//!
//! The providers are scripted stand-ins and keys live in memory, so nothing
//! reaches a real AI. Like the other snapshot tests these only check that
//! rendering worked; look at the files (see `common::output_dir`) when the
//! chat's look changes:
//!
//! ```text
//! MOSHIONLY_SNAPSHOT_DIR=/tmp/shots cargo test -p moshionly-app --test chat_snapshots
//! ```

mod common;

use std::sync::Arc;
use std::time::{Duration, Instant};

use common::{Variant, assert_rendered, can_render, snapshot};
use egui::{Rect, Vec2, pos2, vec2};
use moshionly_agent::{Frame, FrameRequest, FrameSource, Host, HostError, SessionHandle, Tools};
use moshionly_ai::{
    AiSettings, ChatEvent, ErrorKind, ImageData, MemoryKeyStore, ModelInfo, ProviderRegistry,
    ProviderStatus, ReadyState,
};
use moshionly_app::chat::testing::{ScriptedProvider, ScriptedSource, Step, status, statuses};
use moshionly_app::chat::{Chat, ChatInputs, Focus};
use moshionly_app::theme::Theme;
use moshionly_core::{Document, NodeId, NodeType, build};
use serde_json::json;

/// The chat panel's size in the snapshots: the panel's default width.
const PANEL: Vec2 = vec2(320.0, 720.0);
/// A window with the panel on its right, for the settings dialog.
const WINDOW: Vec2 = vec2(900.0, 720.0);
const WAIT: Duration = Duration::from_secs(10);

/// A small picture standing in for a rendered frame: a soft gradient with a
/// card on it.
fn picture(width: u32, height: u32) -> Vec<u8> {
    let image = image::RgbaImage::from_fn(width, height, |x, y| {
        let card = x > width / 4 && x < width * 3 / 4 && y > height / 3 && y < height * 2 / 3;
        if card {
            image::Rgba([255, 255, 255, 255])
        } else {
            let t = x as f32 / width as f32;
            image::Rgba([(110.0 + 80.0 * t) as u8, (114.0 + 30.0 * t) as u8, 239, 255])
        }
    });
    let mut bytes = std::io::Cursor::new(Vec::new());
    image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
    bytes.into_inner()
}

struct PictureFrames;

impl FrameSource for PictureFrames {
    fn render(&self, _: &FrameRequest<'_>) -> Result<Frame, HostError> {
        Ok(Frame {
            png: picture(240, 150),
            width: 240,
            height: 150,
            region: moshionly_core::Rect {
                x: 0.0,
                y: 0.0,
                width: 800.0,
                height: 500.0,
            },
            layer_box: None,
        })
    }
}

/// A document with an artboard and a headline, its tools, and the ids of
/// the artboard and the headline.
fn document() -> (Tools, NodeId, NodeId) {
    let mut document = Document::new("Launch");
    let (artboard, edits) = build::add_artboard(&document, "Launch", 800.0, 500.0).unwrap();
    document.apply(&edits).unwrap();
    let rect = build::default_layer(NodeType::Rect).unwrap();
    let (layer, edits) = build::add_layer(&document, &artboard, rect).unwrap();
    document.apply(&edits).unwrap();
    let host = Host::new().with_frames(Arc::new(PictureFrames));
    let tools = Tools::new(SessionHandle::from_document(document), host);
    (tools, artboard, layer)
}

/// A chat over `source` with keys in memory, its providers checked.
fn chat(source: ScriptedSource) -> Chat {
    let registry = ProviderRegistry::new(AiSettings::default(), Arc::new(MemoryKeyStore::new()));
    let mut chat = Chat::with_source(registry, Arc::new(source), None);
    chat.controller_mut().ensure_checked();
    assert!(
        chat.controller_mut().settle(WAIT),
        "providers were not checked"
    );
    chat
}

/// Polls until `done` holds.
fn wait_until(chat: &mut Chat, done: impl Fn(&Chat) -> bool) {
    let deadline = Instant::now() + WAIT;
    while !done(chat) {
        chat.controller_mut().poll();
        assert!(Instant::now() < deadline, "timed out");
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// Renders the chat as `name`, in every variant: the panel on the right
/// of a picture of `size`, with the canvas colour left of it when the
/// picture is wider than the panel.
fn render(name: &str, size: Vec2, make: impl Fn() -> (Chat, Tools, NodeId)) {
    if !can_render() {
        return;
    }
    for variant in Variant::ALL {
        let (mut chat, tools, artboard) = make();
        let mut frame = 0;
        let (path, image) = snapshot(name, size, variant, |ui| {
            frame += 1;
            let full = ui.max_rect();
            let panel = Rect::from_min_max(pos2(full.right() - PANEL.x, full.top()), full.max);
            if panel.left() > full.left() {
                let canvas = Rect::from_min_max(full.min, pos2(panel.left(), full.bottom()));
                ui.painter()
                    .rect_filled(canvas, 0.0, Theme::of(ui).color.canvas);
                ui.painter()
                    .vline(panel.left(), panel.y_range(), Theme::of(ui).hairline());
            }
            let mut child = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(panel)
                    .layout(egui::Layout::top_down(egui::Align::Min)),
            );
            child.spacing_mut().item_spacing.y = 0.0;
            let selection = [];
            chat.show(
                &mut child,
                ChatInputs {
                    tools: &tools,
                    mcp_url: Some("http://127.0.0.1:47821/mcp"),
                    focus: Focus {
                        artboard: Some(&artboard),
                        selection: &selection,
                        time_ms: Some(0.0),
                    },
                    dialogs: None,
                },
            );
        });
        assert_rendered(&image, size, variant);
        println!("wrote {}", path.display());
    }
}

/// The send-a-message call the snapshots share.
fn send(chat: &mut Chat, tools: &Tools, artboard: &NodeId, text: &str) {
    let selection = [];
    chat.send(
        text,
        &ChatInputs {
            tools,
            mcp_url: Some("http://127.0.0.1:47821/mcp"),
            focus: Focus {
                artboard: Some(artboard),
                selection: &selection,
                time_ms: None,
            },
            dialogs: None,
        },
    )
    .expect("the message is sent");
}

fn ready_claude() -> Vec<ProviderStatus> {
    let mut statuses = statuses(&[]);
    statuses[0] = status(
        "claude-code",
        ReadyState::Ready,
        "Claude Code 2.1.325 is installed and signed in.",
    )
    .unwrap();
    statuses
}

#[test]
fn chat_empty() {
    render("chat-empty", PANEL, || {
        let provider = ScriptedProvider::new("claude-code", Vec::new()).with_models(vec![
            ModelInfo::new("sonnet", "Sonnet"),
            ModelInfo::new("opus", "Opus"),
        ]);
        let chat = chat(ScriptedSource::new(ready_claude()).with_provider(provider));
        let (tools, artboard, _) = document();
        (chat, tools, artboard)
    });
}

#[test]
fn chat_streaming_with_tools() {
    render("chat-streaming", PANEL, || {
        let (tools, artboard, layer) = document();
        let first = vec![
            Step::thinking(
                "The rectangle is the only layer. A fade and a short rise read as one gesture; \
                 400 ms keeps it quick.",
            ),
            Step::text("I'll fade the card in and let it rise a little."),
            Step::call(
                "c1",
                "apply_edits",
                json!({
                    "label": "Fade in the card",
                    "operations": [
                        { "op": "add", "node": { "type": "fade_in", "target": layer.to_string(), "start": 0, "end": 400 } }
                    ]
                }),
            ),
            Step::call(
                "c2",
                "screenshot",
                json!({ "artboard": artboard.to_string(), "time": 200 }),
            ),
            Step::text(
                "Done. The card now fades in over **400 ms**, easing out at the end.\n\n\
                 You could also:\n- slide it up by `24 px` as it appears\n- stagger the \
                 other layers after it",
            ),
            Step::Event(ChatEvent::Usage {
                input_tokens: 18_400,
                output_tokens: 612,
                cost_usd: Some(0.0213),
            }),
        ];
        let second = vec![
            Step::text("Adding a gentle bounce to the card…"),
            Step::Event(ChatEvent::ToolCallStarted {
                id: "c3".into(),
                name: "apply_edits".into(),
                input: json!({ "label": "Bounce the card in", "operations": [] }),
            }),
            Step::WaitForStop,
        ];
        let provider = ScriptedProvider::with_scripts("anthropic", vec![first, second]);
        let mut statuses = statuses(&["anthropic"]);
        statuses
            .iter_mut()
            .for_each(|s| s.key_stored = s.info.id == "anthropic");
        let mut chat = chat(ScriptedSource::new(statuses).with_provider(provider));
        chat.attach(ImageData::png(picture(160, 100)));
        send(&mut chat, &tools, &artboard, "Make the card fade in");
        assert!(chat.controller_mut().settle(WAIT));
        send(&mut chat, &tools, &artboard, "Now make it bounce");
        wait_until(&mut chat, |chat| {
            chat.controller()
                .conversation()
                .messages()
                .last()
                .is_some_and(|answer| answer.tool_calls().count() == 1)
        });
        (chat, tools, artboard)
    });
}

#[test]
fn chat_error() {
    render("chat-error", PANEL, || {
        let provider = ScriptedProvider::new(
            "claude-code",
            vec![Step::Event(ChatEvent::Error {
                message: "not signed in: Invalid API key · Please run /login".into(),
                kind: ErrorKind::NotSignedIn,
            })],
        );
        let mut chat = chat(ScriptedSource::new(ready_claude()).with_provider(provider));
        let (tools, artboard, _) = document();
        send(
            &mut chat,
            &tools,
            &artboard,
            "Fade the title in over half a second",
        );
        assert!(chat.controller_mut().settle(WAIT));
        (chat, tools, artboard)
    });
}

#[test]
fn chat_no_provider_ready() {
    if !can_render() {
        return;
    }
    // The settings open by themselves the first time; this shows the panel
    // the person sees after closing them.
    for variant in Variant::ALL {
        let mut chat = chat(ScriptedSource::new(statuses(&[])));
        let (tools, artboard, _) = document();
        let size = PANEL;
        let (path, image) = snapshot("chat-no-provider", size, variant, |ui| {
            let selection = [];
            chat.show(
                ui,
                ChatInputs {
                    tools: &tools,
                    mcp_url: None,
                    focus: Focus {
                        artboard: Some(&artboard),
                        selection: &selection,
                        time_ms: None,
                    },
                    dialogs: None,
                },
            );
            if chat.is_settings_open() {
                chat.close_settings();
            }
        });
        assert_rendered(&image, size, variant);
        println!("wrote {}", path.display());
    }
}

#[test]
fn chat_settings() {
    render("chat-settings", WINDOW, || {
        let mut statuses = vec![
            status("claude-code", ReadyState::Ready, "Claude Code 2.1.325 is installed and signed in."),
            status(
                "cursor-agent",
                ReadyState::NotSignedIn,
                "Cursor Agent is installed but not signed in. Run `cursor-agent login` in Terminal.",
            ),
            status("codex", ReadyState::NotInstalled, "The codex command was not found."),
            status("gemini-cli", ReadyState::NotInstalled, "The gemini command was not found."),
            status("anthropic", ReadyState::MissingKey, "Add an API key to use Anthropic."),
            status("openai", ReadyState::Ready, "The key was accepted."),
            status("ollama", ReadyState::Unreachable, "Nothing answered at http://localhost:11434/v1."),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
        statuses
            .iter_mut()
            .for_each(|s| s.key_stored = s.info.id == "openai");
        let mut chat = chat(ScriptedSource::new(statuses));
        chat.open_settings();
        chat.expand_in_settings(Some("anthropic"));
        let (tools, artboard, _) = document();
        (chat, tools, artboard)
    });
}
