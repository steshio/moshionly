# Architecture

This document is the contract between the parts of Moshionly. If code and this document disagree, fix one of them in the same pull request.

## Principles

1. **Design first, then actions.** A layer's properties describe how it looks at rest. Animations are separate objects that say "between these two moments, take this property of that layer to this value". There are no keyframes.
2. **A frame is a pure function.** `frame = evaluate(document, artboard, time)`. Nothing depends on the previous frame, so scrubbing, playback, export and agent screenshots all produce the same pixels.
3. **One way to change a document.** Every change, whether it comes from a mouse drag, undo, a file import or an AI agent, is a batch of three edits: add a node, update a node, remove a node.
4. **One schema.** Node types are defined once in Rust. The file format, the validator, the JSON Schema given to agents and the docs are derived from those definitions.
5. **Local first.** Projects are files on disk. No account, no server. Network access is used only when the user turns on an AI provider or sends a design from Figma.
6. **The engine does not know about the window.** Everything except `moshionly-app` runs headless, which is what makes the command line, the MCP server and tests possible.

## Crates

| Crate | Depends on | Responsibility |
|---|---|---|
| `moshionly-core` | nothing heavy | Document model, edits, undo, file format, schema, easing, and evaluation of a document at a time into a `Scene` |
| `moshionly-render` | core, Skia | Turns a `Scene` into pixels. Fonts, text layout, images, effects |
| `moshionly-export` | core, render | PNG, PNG sequence, GIF, MP4, WebM, ProRes and Lottie, as one export job with progress and cancellation |
| `moshionly-agent` | core | The tool surface for AI agents and the MCP server that exposes it. Rendering and export are supplied by the host through traits |
| `moshionly-ai` | none of the above | Chat providers: subscriptions through installed agent CLIs, and API keys |
| `moshionly-app` | all | The desktop editor (egui) and the `moshionly` command |

`plugins/figma` is a Figma plugin written in plain JavaScript. It converts a Figma selection into Moshionly nodes.

## Document

A document is a flat list of nodes. The tree is assembled from `parent`.

```json
{
  "format": "moshionly",
  "version": 2,
  "name": "Launch teaser",
  "nodes": [
    { "id": "ab12", "parent": null,   "index": "a0", "type": "artboard", "name": "Scene 1",
      "x": 0, "y": 0, "width": 1080, "height": 1080, "duration": 4000, "fill": "#FFFFFF" },
    { "id": "cd34", "parent": "ab12", "index": "a0", "type": "rect", "name": "Card",
      "x": 240, "y": 340, "width": 600, "height": 400, "fill": "#0D99FF", "corner_radius": 32 },
    { "id": "ef56", "parent": "ab12", "index": "a0", "type": "fade_in",
      "target": "cd34", "start": 0, "end": 600, "easing": { "type": "smooth", "intensity": 50 } }
  ]
}
```

Rules:

- `id` is a 12 character URL-safe random string, unique in the document.
- Every node belongs to one of two families: **layers** and **animations**. The `type` decides the family.
- Artboards are the roots (`parent: null`). A layer's parent is an artboard, a `group`, a `mask_group` or a `component`. An animation's parent is an artboard or an `animation_group` in the same artboard, and its `target` is a layer (or the artboard) in that artboard.
- Nodes refer to each other in exactly three ways: `parent`, an animation's `target`, and an instance's `component`, which names a `component` layer in any artboard. A component is never inside another component, and never shows itself (it holds no instance of itself, directly or through other components). Code that copies nodes remaps all three.
- `index` is a fractional index string. Siblings of the same family sort by `index` using plain string comparison. For layers, a larger index is drawn later, so it is in front. For animations the order is only the row order in the timeline.
- Lengths are in pixels. Angles are in degrees, clockwise positive. Times are in milliseconds from the start of the artboard. Opacity is 0 to 100. Colours are `#RRGGBB` or `#RRGGBBAA`.
- A layer's `x`, `y` is the position of its top-left corner in its parent's coordinate space. Rotation and scale happen around the centre of the layer's box.
- Unknown fields are rejected on edit and preserved nowhere. Unknown node types make a file fail to load with a clear message. The `version` number increases when the format changes, with a migration for every step.

### Layers

`artboard`, `group`, `mask_group`, `rect`, `ellipse`, `star`, `path`, `text`, `image`, `component`, `instance`.

A `mask_group` uses its first child (lowest index) as an alpha mask for the rest. `group` can clip its children to its box.

A `component` is a main component: it has the fields of a `group` and is drawn like one where it is. An `instance` shows it again: what the component draws itself (fill, stroke, corner radius, clip) and its layers, with the animations that target those layers, as the component looks `time_offset` milliseconds earlier, scaled to the instance's box. The component's own placement, effects and animations belong to where it is placed; the instance has its own. Instances hold no layers and have no per-instance changes yet. A `group` and a `component` can turn into each other by an update of `type`, which is how a group becomes a component in place.

Shared layer properties: `name`, `x`, `y`, `width`, `height`, `rotation`, `scale`, `opacity`, `hidden`, `locked`, `blend_mode`, `blur`, `shadow`, `source_id`. Shapes add `fill`, `stroke` and, where it applies, `corner_radius`. A paint is a colour string or a linear or radial gradient object. `source_id` records where an imported layer came from (for example `figma:<file key>:<node id>`) so a design can be re-imported without losing its animations.

### Animations

Every animation has `target`, `start`, `end` and, unless it is instantaneous, `easing`. Optional `from` and `to` values mean "whatever the value is at that moment" when omitted.

| Family | Types |
|---|---|
| Property | `move`, `rotate`, `scale`, `opacity`, `resize`, `fill`, `stroke`, `corner_radius`, `blur`, `shadow` |
| Instant | `show`, `hide` |
| Presets | `fade_in`/`fade_out`, `slide_in`/`slide_out`, `grow_in`/`grow_out`, `shrink_in`/`shrink_out`, `spin_in`/`spin_out`, `blur_in`/`blur_out`, `reveal_in`/`reveal_out` |
| Text | `text_in`, `text_out`, `counter` |
| Structure | `animation_group` |

Presets are stored as one node with a few settings and are expanded into property animations during evaluation, so they stay easy to edit.

### Combining animations

For one property of one layer, animations are sorted by `start` and combined by one of two rules.

- **Additive** (`move`, `rotate`): each animation contributes a delta that is added to the resting value. Two moves add up. A missing `from` is a delta of zero.
- **Replacing** (everything else): the value is the resting value until the first animation, and from then on the most recently started animation decides. A missing `from` is the value at the moment the animation starts. A missing `to` is the resting value. If the first animation has a `from`, that value also applies before it starts, which is how "fade in" keeps a layer invisible until its cue.

`scale` values are multipliers of the layer's resting scale. `opacity`, sizes, colours, radii and blur are absolute.

### Easing

```json
{ "type": "smooth", "intensity": 50 }
{ "type": "bezier", "x1": 0.2, "y1": 0, "x2": 0, "y2": 1 }
```

Types: `linear`, `smooth`, `natural`, `slow_down`, `accelerate`, `overshoot`, `elastic`, `bounce`, `bezier`. `intensity` is 0 to 100 and shapes the curve. Every easing maps a progress of 0 to 1 onto a value that starts at 0 and ends at 1.

## Edits and undo

```rust
enum Edit {
    Add { node: Node },
    Update { id: NodeId, patch: Map<String, Value> },   // null removes an optional field
    Remove { id: NodeId },                              // removes descendants and animations that target them
}
```

`Document::apply(&[Edit]) -> Result<Vec<Edit>, EditError>` validates the whole batch, applies it atomically and returns the inverse batch. Moving or reordering a node is an `Update` of `parent` and `index`. `id` and `type` are fixed, except that a `group` and a `component` can switch. The rules that span several nodes (the component rules above) are checked over the whole document at the end of a batch that touches a component or an instance or moves a node, so removing a component that instances still show is refused; `build::remove` detaches those instances first, into groups that look the same, in the same batch. `History` keeps undo and redo stacks of inverse batches. Interactive drags apply edits continuously and are merged into one undo step when the gesture ends.

## Evaluation and the scene

`evaluate(&Document, artboard_id, time_ms, &dyn TextMeasurer) -> Scene`

The `Scene` is a plain data tree with every value resolved for that instant: transforms, opacity, clip and mask, paints, shape geometry, text broken into drawable units, images and effects. It has no dependency on a graphics library and can be serialised. The renderer, the Lottie exporter and hit testing all read it.

An instance is evaluated in place: its component's subtree is evaluated with the animations of the component's artboard at the instance's time minus its `time_offset` (not limited to 0, so an instance whose offset has not passed shows the component before its animations start), scaled to the instance's box, and every scene node of it carries the instance's id, so hit testing, bounds and selection treat it as one layer. Nested instances are evaluated the same way, with a depth limit as a guard.

Text needs fonts to be measured, and fonts belong to the renderer. Core therefore asks a `TextMeasurer` for line and glyph-cluster boxes when it needs to split text for per-letter, per-word or per-line animation.

## Rendering

`moshionly-render` draws a `Scene` with Skia on a CPU raster surface into premultiplied RGBA. The same call serves the editor preview (at screen resolution), export (at output resolution) and agent screenshots. A GPU surface is a later optimisation and must not change results.

Effect order for a layer, innermost first: content, shadow, blur, mask and clip, blend mode, opacity.

## Export

Export steps through an artboard at `1000 / fps` millisecond intervals, renders each frame and feeds an encoder. PNG, PNG sequences and GIF are encoded in-process, and MP4 (H.264) with OpenH264 built from source. Formats that need a system encoder (WebM with VP9, ProRes 4444 with alpha) use `ffmpeg` when it is installed; `available_formats()` says which formats can be written right now and why one cannot.

Every format runs as the same `ExportJob`: `plan()` checks the settings without writing anything, `run()` reports progress and stops when its `CancelToken` is cancelled, and the file is written under a temporary name and renamed into place only when it is complete. The editor's Export dialog, the agents' `export` tool and `moshionly export` all use it.

Lottie export converts the scene model, not pixels. It works on a copy of the document in which every instance is detached (`build::without_instances`), since Lottie has no instances. Layers become Lottie layers, text becomes outlines made with the renderer's own fonts, and animations become keyframes: linear and Bézier-shaped easings exactly, others fitted with a few curve pieces and checked against core's values. Anything Lottie cannot show exactly is reported as a warning that names the layer. `crates/moshionly-export/README.md` has the details.

## Agents

`moshionly-agent` defines a small set of tools over a document handle:

| Tool | Purpose |
|---|---|
| `get_overview` | Artboards, sizes, durations, layer and animation counts, fonts in use |
| `query_nodes` | Find nodes by type, name, parent or target and return chosen fields |
| `get_node` | One node in full, with its box on the artboard and the animations that target it |
| `get_schema` | The JSON Schema for a node type, with defaults |
| `apply_edits` | An atomic batch of add, update, remove, move, clone, set-duration, make-component and detach operations |
| `screenshot` | Render an artboard or layer at a time, as a small PNG |
| `evaluate` | The values of chosen properties of chosen layers at chosen times, as numbers |
| `list_fonts` | The font families the renderer can use |
| `import_image` | Copy an image into the project's `assets/` folder and return the source to use |
| `export` | Render a file to disk |
| `undo_last_agent_change` | Undo the most recent change an agent made, never the person's own |

Agent edits go through the same validator and the same undo history as manual edits. A batch either applies completely or not at all, and errors come back as messages that say what to fix.

The tools are exposed three ways: to the built-in chat, over MCP on a local HTTP port (`http://127.0.0.1:47821/mcp`) while the app is open, and over MCP on stdio through `moshionly mcp` for headless use (which writes changes back to its file only with `--save`). The same local port accepts designs sent by the Figma plugin (`POST /import`); a design is checked and then added to the open document as one undo step: layers that came from the same Figma layers before are replaced and keep their animations, and the rest are added. The server listens on `127.0.0.1` only, and can be turned off in the app.

Screenshots, fonts and exports are supplied by the host through the agent crate's `FrameSource`, `FontSource` and `Exporter` traits. In the app they are the renderer, the renderer's font library and `moshionly-export`; agents' exports go to the project's `exports` folder.

## AI providers

`moshionly-ai` offers two kinds of provider behind one chat interface.

- **Subscription providers** run an agent CLI the user has already installed and signed in to (for example Claude Code). Moshionly starts the CLI in non-interactive mode with the Moshionly MCP server attached and streams its output. Moshionly never reads or stores the CLI's credentials.
- **API providers** call a model API directly with a key the user supplies (Anthropic, or any OpenAI-compatible endpoint including local models). Moshionly runs the tool loop itself. Keys are stored in the operating system keychain.

## The app

`moshionly-app` is an egui application. Layout: a canvas in the centre, layers on the left, a Design and Animate inspector on the right, a timeline along the bottom, a floating tool bar over the canvas, and an optional chat panel.

The open document lives in one `SessionHandle` from `moshionly-agent`, which holds the `Document` and its `History`. The editor, the built-in chat and the MCP server all work through that one handle, so an agent's edit appears on the canvas at once and the person can undo it. Around it the editor keeps view state: selection, current artboard, playhead, tool, zoom. The editor's rules (what a click selects, how a drag becomes edits, how an action is added) are plain functions tested without a window.

Artboards are drawn by evaluating and rendering on worker threads and showing the result as a texture; while playing, the canvas may draw less sharp to keep up and returns to full sharpness when stopped. Exports run on their own thread with a progress bar and Cancel. A design sent from Figma, or pasted from the plugin's Copy button, is added at once as one undo step, with a toast saying what arrived: layers that came from the same Figma layers before are replaced and keep their animations, and the rest are added. Once six sent designs, or 32 MB of them, were added in the last minute, the rest wait (the one that crosses 32 MB is still added whole).

The `moshionly` binary is the editor and the command line: `moshionly [file]` opens the window, `moshionly export` and `moshionly mcp` work without one.

All colours, spacing and type sizes come from one theme module so the look can be changed in one place.

## Packaging

`scripts/release-mac.sh` turns a release build into `Moshionly.app` and `Moshionly-<version>.dmg` in `dist/`. The bundle is assembled outside cargo: the script builds the `moshionly` binary for Apple silicon and macOS 11 or later, checks that it links only system libraries, draws `AppIcon.icns` with the app crate's `app_icon` example (the same mark as the window icon, on Apple's icon grid) and fills in `packaging/macos/Info.plist`. It then signs the app with the hardened runtime and no entitlements, has Apple notarize it, staples the ticket, and does the same for the `.dmg` that holds it. The signing identity and the `notarytool` profile come from the keychain and the environment (`MOSHIONLY_SIGN_IDENTITY`, `MOSHIONLY_NOTARY_PROFILE`), never from the repository. `--ad-hoc` makes the same bundle without an Apple account; CI builds it that way on every pull request.
