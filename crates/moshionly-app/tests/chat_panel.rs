//! Drives the AI chat panel the way a person would, without a window:
//! typing, sending with Enter, stopping an answer, choosing an AI, the
//! settings and their keys, and the buttons under an error.
//!
//! Providers are scripted stand-ins and keys live in memory, so nothing
//! reaches a real AI and the system's keychain is never touched.

use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::{Key, Modifiers, vec2};
use egui_kittest::{Harness, kittest::Queryable as _};
use moshionly_agent::{Host, SessionHandle, Tools};
use moshionly_ai::{
    AiSettings, ChatEvent, ErrorKind, KeyStore as _, MemoryKeyStore, ModelInfo, ProviderRegistry,
    ReadyState, StopReason,
};
use moshionly_app::chat::testing::{ScriptedProvider, ScriptedSource, Step, status, statuses};
use moshionly_app::chat::{AnswerState, Chat, ChatInputs, Focus, KeyNote};
use moshionly_app::theme;
use moshionly_core::{Document, NodeId, build};

const WAIT: Duration = Duration::from_secs(10);
/// Frames for a dialog to appear and settle into its size and place.
const DIALOG_FRAMES: usize = 6;

/// What the test harness keeps between frames.
struct State {
    chat: Chat,
    tools: Tools,
    artboard: NodeId,
    closed: bool,
    changed: bool,
}

/// The chat over `source`, with keys in `keys`, filling a window wide
/// enough for the settings dialog.
fn harness(source: ScriptedSource, keys: Arc<MemoryKeyStore>) -> Harness<'static, State> {
    let registry = ProviderRegistry::new(AiSettings::default(), keys);
    let chat = Chat::with_source(registry, Arc::new(source), None);
    let mut document = Document::new("Test");
    let (artboard, edits) = build::add_artboard(&document, "Launch", 800.0, 500.0).unwrap();
    document.apply(&edits).unwrap();
    let tools = Tools::new(SessionHandle::from_document(document), Host::new());
    let state = State {
        chat,
        tools,
        artboard,
        closed: false,
        changed: false,
    };
    let mut harness = Harness::builder()
        .with_size(vec2(900.0, 800.0))
        .with_context_setup(theme::install)
        .build_ui_state(
            |ui, state: &mut State| {
                let selection = [];
                let output = state.chat.show(
                    ui,
                    ChatInputs {
                        tools: &state.tools,
                        mcp_url: Some("http://127.0.0.1:47821/mcp"),
                        focus: Focus {
                            artboard: Some(&state.artboard),
                            selection: &selection,
                            time_ms: None,
                        },
                        dialogs: None,
                    },
                );
                state.closed |= output.close;
                state.changed |= output.document_changed;
            },
            state,
        );
    settle(&mut harness, |state| {
        state.chat.controller().statuses().is_some()
    });
    harness
}

/// Runs frames until `done` holds. Spinners keep asking for frames, so the
/// harness's own `run` would never settle while an answer arrives.
fn settle(harness: &mut Harness<'static, State>, done: impl Fn(&State) -> bool) {
    let deadline = Instant::now() + WAIT;
    loop {
        harness.run_steps(1);
        if done(harness.state()) {
            harness.run_steps(2);
            return;
        }
        assert!(Instant::now() < deadline, "timed out");
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn idle(state: &State) -> bool {
    !state.chat.controller().is_busy()
}

fn ready(ids: &[&str]) -> ScriptedSource {
    ScriptedSource::new(statuses(ids))
}

/// Types `text` into the composer.
fn type_message(harness: &mut Harness<'static, State>, text: &str) {
    harness.get_by_label("Message").click();
    harness.run_steps(1);
    harness.get_by_label("Message").type_text(text);
    harness.run_steps(1);
}

#[test]
fn enter_sends_and_shift_enter_starts_a_new_line() {
    let provider = ScriptedProvider::new("anthropic", vec![Step::text("On it.")]);
    let mut harness = harness(
        ready(&["anthropic"]).with_provider(provider.clone()),
        Arc::default(),
    );
    type_message(&mut harness, "Fade in");
    harness.key_press_modifiers(Modifiers::SHIFT, Key::Enter);
    harness.run_steps(1);
    harness.get_by_label("Message").type_text("the title");
    harness.run_steps(1);
    assert!(provider.requests().is_empty(), "Shift+Enter must not send");

    harness.key_press(Key::Enter);
    settle(&mut harness, |state| {
        idle(state) && !state.chat.controller().conversation().is_empty()
    });
    let requests = provider.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].messages[0].text, "Fade in\nthe title");
    let answer = harness.state().chat.controller().conversation().messages()[1].clone();
    assert_eq!(answer.text(), "On it.");
    assert!(
        harness.query_by_label("On it.").is_some(),
        "the answer is shown"
    );
}

#[test]
fn the_next_message_can_be_typed_as_soon_as_the_answer_is_in() {
    let provider = ScriptedProvider::new("anthropic", vec![Step::text("On it.")]);
    let mut harness = harness(
        ready(&["anthropic"]).with_provider(provider.clone()),
        Arc::default(),
    );
    type_message(&mut harness, "Fade in");
    harness.key_press(Key::Enter);
    settle(&mut harness, |state| {
        idle(state) && state.chat.controller().conversation().messages().len() == 2
    });
    assert!(harness.get_by_label("Message").is_focused());

    // No click: the text goes straight into the box, and Enter sends it.
    harness.get_by_label("Message").type_text("Then move it");
    harness.run_steps(1);
    harness.key_press(Key::Enter);
    settle(&mut harness, |state| {
        idle(state) && state.chat.controller().conversation().messages().len() == 4
    });
    let requests = provider.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[1].messages.last().unwrap().text, "Then move it");
}

#[test]
fn the_composer_leaves_the_keyboard_alone_after_a_click_elsewhere() {
    let provider = ScriptedProvider::new(
        "anthropic",
        vec![Step::text("Working on it"), Step::WaitForStop],
    );
    let mut harness = harness(
        ready(&["anthropic"]).with_provider(provider.clone()),
        Arc::default(),
    );
    type_message(&mut harness, "Make it bounce");
    harness.key_press(Key::Enter);
    settle(&mut harness, |state| {
        state.chat.controller().conversation().messages().len() == 2
    });

    // The person clicks elsewhere while the answer arrives, then stops it.
    let blank = harness.get_by_label("Working on it").rect().left_bottom() + vec2(0.0, 120.0);
    for pressed in [true, false] {
        harness.event(egui::Event::PointerButton {
            pos: blank,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::default(),
        });
    }
    harness.run_steps(1);
    harness.get_by_label("Stop").click();
    settle(&mut harness, idle);
    assert!(!harness.get_by_label("Message").is_focused());
}

#[test]
fn stop_ends_a_running_answer_and_the_composer_waits_meanwhile() {
    let provider = ScriptedProvider::new(
        "anthropic",
        vec![Step::text("Working on it"), Step::WaitForStop],
    );
    let mut harness = harness(
        ready(&["anthropic"]).with_provider(provider.clone()),
        Arc::default(),
    );
    type_message(&mut harness, "Make it bounce");
    harness.key_press(Key::Enter);
    settle(&mut harness, |state| {
        state.chat.controller().conversation().messages().len() == 2
            && state.chat.controller().conversation().messages()[1].text() == "Working on it"
    });
    assert!(harness.state().chat.controller().is_busy());
    assert!(harness.query_by_label("Send").is_none());

    // Typing while the answer arrives does nothing: the box is disabled.
    harness.get_by_label("Message").type_text("more");
    harness.run_steps(1);
    harness.key_press(Key::Enter);
    harness.run_steps(1);
    assert_eq!(provider.requests().len(), 1);

    harness.get_by_label("Stop").click();
    settle(&mut harness, idle);
    let answer = &harness.state().chat.controller().conversation().messages()[1];
    assert_eq!(answer.state, AnswerState::Finished(StopReason::Cancelled));
    assert!(provider.was_stopped());
    assert!(harness.query_by_label("Send").is_some());
}

#[test]
fn a_suggestion_is_sent_when_clicked() {
    let provider = ScriptedProvider::new("anthropic", vec![Step::text("It holds one artboard.")]);
    let mut harness = harness(
        ready(&["anthropic"]).with_provider(provider.clone()),
        Arc::default(),
    );
    harness
        .get_by_label("Describe what is on this artboard")
        .click();
    settle(&mut harness, |state| {
        idle(state) && !state.chat.controller().conversation().is_empty()
    });
    assert_eq!(
        provider.requests()[0].messages[0].text,
        "Describe what is on this artboard"
    );
    // The system prompt carries the guide and what the person looks at.
    let prompt = provider.requests()[0].system_prompt.clone().unwrap();
    assert!(prompt.contains("Launch"), "{prompt}");
}

#[test]
fn tool_chips_open_to_show_the_call() {
    let provider = ScriptedProvider::new(
        "anthropic",
        vec![
            Step::call("t1", "get_overview", serde_json::json!({})),
            Step::text("Done."),
        ],
    );
    let mut harness = harness(
        ready(&["anthropic"]).with_provider(provider),
        Arc::default(),
    );
    type_message(&mut harness, "What is here?");
    harness.key_press(Key::Enter);
    settle(&mut harness, |state| {
        idle(state) && !state.chat.controller().conversation().is_empty()
    });
    assert!(harness.query_by_label("get_overview").is_none());
    harness.get_by_label_contains("Overview").click();
    harness.run_steps(2);
    assert!(
        harness.query_by_label("get_overview").is_some(),
        "the tool's name shows"
    );
}

#[test]
fn an_error_offers_settings_and_another_try() {
    let provider = ScriptedProvider::with_scripts(
        "claude-code",
        vec![
            vec![Step::Event(ChatEvent::Error {
                message: "not signed in: please sign in".into(),
                kind: ErrorKind::NotSignedIn,
            })],
            vec![Step::text("Signed in now.")],
        ],
    );
    let mut harness = harness(
        ready(&["claude-code"]).with_provider(provider.clone()),
        Arc::default(),
    );
    type_message(&mut harness, "Fade the title in");
    harness.key_press(Key::Enter);
    settle(&mut harness, |state| {
        idle(state) && !state.chat.controller().conversation().is_empty()
    });
    assert!(
        harness.query_by_label_contains("run claude once").is_some(),
        "the way forward is shown"
    );

    harness.get_by_label("Open AI settings").click();
    harness.run_steps(DIALOG_FRAMES);
    assert!(harness.state().chat.is_settings_open());
    harness.get_by_label("Done").click();
    harness.run_steps(2);
    assert!(!harness.state().chat.is_settings_open());

    harness.get_by_label("Try again").click();
    settle(&mut harness, |state| {
        idle(state)
            && state.chat.controller().conversation().messages()[1].text() == "Signed in now."
    });
    assert_eq!(provider.requests().len(), 2);
    assert_eq!(
        harness
            .state()
            .chat
            .controller()
            .conversation()
            .messages()
            .len(),
        2
    );
}

#[test]
fn an_answer_with_nothing_in_it_says_so_and_offers_another_try() {
    // A model that finishes at once, without a word.
    let provider = ScriptedProvider::new("anthropic", Vec::new());
    let mut harness = harness(
        ready(&["anthropic"]).with_provider(provider.clone()),
        Arc::default(),
    );
    type_message(&mut harness, "Fade the title in");
    harness.key_press(Key::Enter);
    settle(&mut harness, |state| {
        idle(state) && !state.chat.controller().conversation().is_empty()
    });
    assert!(
        harness
            .query_by_label("Anthropic finished without a reply")
            .is_some(),
        "the empty answer is shown"
    );

    harness.get_by_label("Try again").click();
    settle(&mut harness, |state| {
        idle(state) && provider.requests().len() == 2
    });
    let messages = harness.state().chat.controller().conversation().messages();
    assert_eq!(messages.len(), 2, "the message was sent again, not added");
    assert_eq!(messages[0].text(), "Fade the title in");
}

#[test]
fn with_no_ai_ready_the_panel_says_so_and_opens_the_settings() {
    let mut harness = harness(ready(&[]), Arc::default());
    harness.run_steps(2);
    assert!(
        harness.state().chat.is_settings_open(),
        "opened by themselves"
    );
    harness.get_by_label("Done").click();
    harness.run_steps(2);
    assert!(!harness.state().chat.is_settings_open());
    // They do not open by themselves again, but the panel offers them.
    harness.run_steps(2);
    assert!(!harness.state().chat.is_settings_open());
    harness.get_by_label("Set up AI").click();
    harness.run_steps(2);
    assert!(harness.state().chat.is_settings_open());
}

#[test]
fn a_key_typed_in_the_settings_goes_to_the_key_store_and_is_not_kept() {
    const KEY: &str = "sk-ant-test-0123456789abcdefghij";
    let keys = Arc::new(MemoryKeyStore::new());
    let source = ScriptedSource::new(vec![
        status("claude-code", ReadyState::Ready, "Ready.").unwrap(),
        status("anthropic", ReadyState::MissingKey, "Add a key.").unwrap(),
    ]);
    let mut harness = harness(source, Arc::clone(&keys));
    harness.state_mut().chat.open_settings();
    harness.run_steps(DIALOG_FRAMES);
    harness.get_by_label("Anthropic: needs a key").click();
    harness.run_steps(DIALOG_FRAMES);

    harness.get_by_label("API key").click();
    harness.run_steps(1);
    harness.get_by_label("API key").type_text(KEY);
    harness.run_steps(1);
    harness.get_by_label("Save").click();
    settle(&mut harness, |state| {
        state.chat.controller().key_note("anthropic") == Some(&KeyNote::Saved)
    });
    assert_eq!(keys.get("anthropic").unwrap().unwrap().expose(), KEY);
    settle(&mut harness, |state| {
        state
            .chat
            .controller()
            .status("anthropic")
            .unwrap()
            .key_stored
    });

    // The key is not in the field, nor anywhere else on screen.
    let field = harness.get_by_label("API key");
    assert!(field.value().is_none_or(|value| value.is_empty()));
    assert!(harness.query_by_label_contains(KEY).is_none());

    harness.get_by_label("Remove").click();
    settle(&mut harness, |state| {
        state.chat.controller().key_note("anthropic") == Some(&KeyNote::Removed)
    });
    assert!(keys.get("anthropic").unwrap().is_none());
}

#[test]
fn turning_a_provider_off_saves_it_in_the_settings() {
    let source = ready(&["claude-code", "anthropic"])
        .with_provider(ScriptedProvider::new("claude-code", Vec::new()))
        .with_provider(ScriptedProvider::new("anthropic", Vec::new()));
    let mut harness = harness(source, Arc::default());
    assert_eq!(
        harness
            .state()
            .chat
            .controller()
            .selected()
            .unwrap()
            .info
            .id,
        "claude-code"
    );
    harness.state_mut().chat.open_settings();
    harness.run_steps(DIALOG_FRAMES);
    harness
        .get_by_label("Offer Claude Code in the chat")
        .click();
    settle(&mut harness, |state| {
        state
            .chat
            .controller()
            .selected()
            .is_some_and(|s| s.info.id == "anthropic")
    });
    assert!(
        !harness
            .state()
            .chat
            .controller()
            .settings()
            .is_enabled("claude-code")
    );
}

#[test]
fn the_picker_switches_the_ai_and_its_model() {
    let source = ready(&["claude-code", "anthropic"])
        .with_provider(ScriptedProvider::new("claude-code", Vec::new()))
        .with_provider(
            ScriptedProvider::new("anthropic", Vec::new()).with_models(vec![
                ModelInfo::new("claude-opus-5-5", "Opus"),
                ModelInfo::new("claude-sonnet-5-5", "Sonnet"),
            ]),
        );
    let mut harness = harness(source, Arc::default());
    harness.get_by_label_contains("AI: Claude Code").click();
    harness.run_steps(2);
    harness.get_by_label("Anthropic").click();
    settle(&mut harness, |state| {
        state
            .chat
            .controller()
            .selected()
            .is_some_and(|s| s.info.id == "anthropic")
    });
    assert_eq!(
        harness
            .state()
            .chat
            .controller()
            .settings()
            .default_provider(),
        Some("anthropic")
    );

    settle(&mut harness, |state| {
        matches!(
            state.chat.controller().models("anthropic"),
            Some(moshionly_app::chat::ModelList::Loaded(_))
        )
    });
    harness.get_by_label_contains("AI: Anthropic").click();
    harness.run_steps(2);
    harness.get_by_label("Sonnet").click();
    harness.run_steps(2);
    assert_eq!(
        harness
            .state()
            .chat
            .controller()
            .settings()
            .provider("anthropic")
            .model(),
        Some("claude-sonnet-5-5")
    );
    assert!(harness.query_by_label("AI: Anthropic · Sonnet").is_some());
}

#[test]
fn new_chat_starts_over_and_close_is_reported() {
    let provider = ScriptedProvider::new("anthropic", vec![Step::text("Hello.")]);
    let mut harness = harness(
        ready(&["anthropic"]).with_provider(provider),
        Arc::default(),
    );
    type_message(&mut harness, "Hi");
    harness.key_press(Key::Enter);
    settle(&mut harness, |state| {
        idle(state) && !state.chat.controller().conversation().is_empty()
    });
    harness.get_by_label("New chat").click();
    harness.run_steps(2);
    assert!(harness.state().chat.controller().conversation().is_empty());

    harness.get_by_label("Close chat").click();
    harness.run_steps(1);
    assert!(harness.state().closed);
    assert!(!harness.state().changed);
}
