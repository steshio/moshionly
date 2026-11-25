# moshionly-agent

The tools that let an AI agent work in a Moshionly file as safely and capably as a person, and the Model Context Protocol (MCP) server that offers them to agents outside the app. The built-in chat, external agents such as Claude Code, and headless scripts all go through this crate.

It depends on `moshionly-core` only. Rendering, export and the list of fonts belong to the app, which hands them in through traits (`FrameSource`, `Exporter`, `FontSource`); without them the tools that need them say so, and everything else works.

## The tools

| Tool | What it does |
|---|---|
| `get_overview` | File name, artboards with size and duration, layer and animation counts, fonts in use, what the host can do |
| `query_nodes` | Find nodes by family, type, name, parent, artboard, target, component, time range or ids; choose the fields; nest children; capped results that say how to narrow |
| `get_node` | One node in full, with its box in artboard coordinates, the animations on it, and its links to components: the component an instance shows, a component's instances, the component a layer is part of |
| `get_schema` | The node types, or the fields, defaults, value types and a ready-made operation for one type |
| `apply_edits` | An all-or-nothing batch of `add`, `update`, `remove`, `move`, `clone`, `set_duration`, `make_component` and `detach`, with temporary ids, as one undo step |
| `screenshot` | An artboard or a layer at rest or at chosen moments, as PNG images |
| `evaluate` | Property values of layers at chosen times, to check motion as numbers |
| `list_fonts` | The font families the host can draw, and those the file uses |
| `import_image` | Copies an image into the project's `assets` folder and returns the `source` to use |
| `export` | Writes an artboard to a file in the project's `exports` folder through the host's exporter |
| `undo_last_agent_change` | Undoes the last agent change, never the person's own work |

Components work through `apply_edits` like everything else: `make_component` turns a group into a component in place or wraps layers in a new one, `add` with `{"type":"instance","component":…}` places a copy (the component may be a `temp_id` from the same call, and the copy takes the component's size and look unless they are given), `detach` turns an instance into a group that looks the same, and `remove` on a component detaches its instances first and says so. `clone` with a `time_offset` also delays copied instances, which staggers copies of one animated component.

Every edit goes through `Document::apply` and the undo history, labelled with who made it (`Origin::User`, `Agent` or `Import`). Errors come back as results a model can act on: the operation, the field, and what to do instead. `guide::GUIDE` is the advice the model gets about working in a file (look first, units, how animations combine, durations and easing, checking with screenshots, keeping the person's work).

## Using it from the app

```rust
use std::sync::Arc;
use moshionly_agent::import::ImportSink;
use moshionly_agent::mcp::{McpServer, http::{HttpConfig, HttpServer}};
use moshionly_agent::{Host, SessionHandle, ToolBridge, Tools};

// The open document, shared by the window, the chat and the server.
let session = SessionHandle::open("Launch.moshi")?;
let _repaint = session.subscribe(|change| { /* ask the window to repaint */ });

// What the app can do for the tools.
let host = Host::new()
    .with_frames(Arc::new(my_renderer))
    .with_exporter(Arc::new(my_exporter))
    .with_fonts(Arc::new(my_fonts));
let tools = Tools::new(session.clone(), host);

// Agents outside the app: MCP at http://127.0.0.1:47821/mcp, and designs
// from the Figma plugin at /import, which the app adds as one undo step.
let (imports, inbox) = ImportSink::new();
let server = HttpServer::start_with_imports(McpServer::new(tools.clone()), HttpConfig::default(), imports)?;

// The built-in chat: the same tools as plain data, and its system prompt.
let bridge = ToolBridge::new(tools);
let prompt = bridge.system_prompt(None);

// Each frame: add what arrived. Layers that came over before are replaced
// and keep their animations; the rest are added.
if let Ok(pending) = inbox.try_recv() {
    pending.apply(&session, &ImportTarget::ReplaceMatching)?;
}
```

For `moshionly mcp`, serve the same tools on standard input and output with `moshionly_agent::mcp::stdio::serve(&McpServer::new(tools))`.

## MCP

The server speaks both eras of the protocol: the stateless `2026-07-28` revision, where each request carries its version in `params._meta` and `server/discover` describes the server, and the `2025-03-26` to `2025-11-25` revisions, which open with `initialize` and get a session id over HTTP. It offers tools only, with read-only and destructive hints, and gives the agent guide as its instructions. Responses are plain JSON; the server never needs to stream.

Connect Claude Code to a running app or the example below:

```text
claude mcp add --transport http moshionly http://127.0.0.1:47821/mcp
```

### Safety

- The HTTP server listens on `127.0.0.1` only.
- `/mcp` refuses any request with an `Origin` other than its own address, and any `Host` other than this machine, so a web page cannot drive it. Agent tools send no `Origin`.
- `/import` and `/health` also answer the Figma plugin panel, whose origin is `null`, including the private-network preflight Chromium sends. Since any sandboxed page looks the same, an import is size-capped, checked through core, must carry images as `data:` URIs, and is handed to the app as a `PendingImport`; at most four wait for the app at once. The Moshionly app adds it without asking, as one undo step the person can take back, and stops adding more once six designs, or 32 MB of them, came in within the last minute (the one that crosses 32 MB is still added whole): that slows down a page posting designs in a loop, and turning off "Let agents on this computer connect" in the app stops it.
- Tools write files only into the project's `assets` and `exports` folders, never over an existing file (a host can allow other export paths with `ToolsConfig::export_anywhere`).

## The Figma import envelope

```json
{ "format": "moshionly-import", "version": 1,
  "source": { "app": "figma", "file_name": "Launch", "file_key": "abc" },
  "nodes": [ "...layers in Moshionly's node JSON..." ],
  "notes": [ { "source_id": "figma:abc:1:2", "layer_name": "Blob", "message": "..." } ] }
```

`POST http://localhost:47821/import` answers `202` with `{ "ok": true, "status": "pending", "layers": 2, "notes": 0 }`, or an error with `{ "ok": false, "error": "..." }`: `400` for a design that cannot be used, `413` for one that is too large, and `429` while four designs are already waiting for the app to take them. An optional `"mode"` of `"new_artboards"` or `"replace_matching"` is still read into `PendingImport::mode`; the Moshionly app ignores it and always replaces what came over before.

## Trying it

```text
cargo run -p moshionly-agent --example serve -- [file.moshi] [--port N] [--save]
```

serves a file (or a small demo) over HTTP MCP, prints the URL and every change, and with `--save` writes changes back to the file. It has no renderer, so screenshots and exports are not available there.

## Tests

`cargo test -p moshionly-agent` runs everything that needs no network, display or installed program: every tool against small documents including failures and atomicity, the protocol in both eras, the HTTP and stdio transports with an in-process client on a free port, and the import endpoint.

`tests/live_claude.rs` is a check with the real `claude` CLI: it serves a small document, asks Claude Code in one short turn to fade in a named layer, and checks the change is one undo step that restores the file. It is ignored by default because it spends the account's usage:

```text
cargo test -p moshionly-agent --test live_claude -- --ignored --nocapture
```
