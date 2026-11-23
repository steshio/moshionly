//! What the tests of the tools share: a small document and ways to call
//! tools and read their answers.

use moshionly_core::{Document, io};
use serde_json::Value;

use crate::host::Host;
use crate::session::{Session, SessionHandle};
use crate::tools::Tools;

/// A project with two artboards.
///
/// `scene` (1080 by 1080, 4 seconds) holds the group `card` with the rect
/// `accent`, the text `title` and the ellipse `dot` inside it, and the rect
/// `badge` in front of the card. Its timeline has the fade `cardfade` on
/// the card, the move `dotmove` on the dot, and the folder `extras` holding
/// the opacity animation `titlefade` on the title. `second` is empty.
pub(crate) const DEMO: &str = r##"{
  "format": "moshionly",
  "version": 1,
  "name": "Launch day",
  "nodes": [
    {"id":"scene","parent":null,"index":"a0","type":"artboard","name":"Launch","width":1080,"height":1080,"duration":4000,"fill":"#F2F3F5"},
    {"id":"card","parent":"scene","index":"a0","type":"group","name":"Card","x":240,"y":340,"width":600,"height":400,"fill":"#FFFFFF","corner_radius":32,"clip":true},
    {"id":"accent","parent":"card","index":"a0","type":"rect","name":"Accent","width":600,"height":12,"fill":"#0D99FF"},
    {"id":"title","parent":"card","index":"a1","type":"text","name":"Title","x":48,"y":72,"fill":"#111111","content":"Launch day","font":{"weight":700},"font_size":64},
    {"id":"dot","parent":"card","index":"a2","type":"ellipse","name":"Dot","x":500,"y":300,"width":52,"height":52,"fill":"#0D99FF"},
    {"id":"badge","parent":"scene","index":"a1","type":"rect","name":"Badge","x":40,"y":40,"width":120,"height":48,"fill":"#111111","corner_radius":24},
    {"id":"cardfade","parent":"scene","index":"a0","type":"fade_in","name":"Card fades in","target":"card","start":0,"end":500,"easing":{"type":"slow_down"}},
    {"id":"dotmove","parent":"scene","index":"a1","type":"move","target":"dot","start":600,"end":1200,"from":{"x":-100}},
    {"id":"extras","parent":"scene","index":"a2","type":"animation_group","name":"Extras"},
    {"id":"titlefade","parent":"extras","index":"a0","type":"opacity","target":"title","start":500,"end":1500,"from":0},
    {"id":"second","parent":null,"index":"a1","type":"artboard","name":"Second","x":1180,"width":800,"height":600}
  ]
}"##;

/// The document of [`DEMO`].
pub(crate) fn demo_document() -> Document {
    io::parse(DEMO).expect("the demo project is valid")
}

/// A session on the demo document, not saved anywhere.
pub(crate) fn demo_session() -> SessionHandle {
    SessionHandle::new(Session::new(demo_document()))
}

/// The tools on the demo document, in a host that can do nothing extra.
pub(crate) fn demo_tools() -> Tools {
    Tools::new(demo_session(), Host::new())
}

/// Calls a tool that is expected to succeed and returns its text as JSON.
#[track_caller]
pub(crate) fn call(tools: &Tools, name: &str, arguments: Value) -> Value {
    let output = tools.call(name, arguments);
    let text = output.joined_text();
    assert!(!output.is_error, "{name} failed: {text}");
    serde_json::from_str(&text).unwrap_or_else(|error| panic!("{name}: {error}\n{text}"))
}

/// Calls a tool that is expected to succeed and returns its text.
#[track_caller]
pub(crate) fn call_text(tools: &Tools, name: &str, arguments: Value) -> String {
    let output = tools.call(name, arguments);
    let text = output.joined_text();
    assert!(!output.is_error, "{name} failed: {text}");
    text
}

/// Calls a tool that is expected to fail and returns its message.
#[track_caller]
pub(crate) fn call_error(tools: &Tools, name: &str, arguments: Value) -> String {
    let output = tools.call(name, arguments);
    let text = output.joined_text();
    assert!(
        output.is_error,
        "{name} should have failed, but said: {text}"
    );
    text
}
