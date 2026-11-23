# Moshionly

**Open-source motion design for the desktop.** Animate with simple actions instead of keyframes, bring your designs in from Figma, export video, GIF and Lottie, and let the AI you already pay for work in the same file as you.

Moshionly is a native app written in Rust. It runs on macOS first, with Linux and Windows to follow. Your projects are files on your own disk.

> **Status: early.** The project is being built in the open, crate by crate. The engine, the AI providers and the editor's window are in place; the editor still shows sample content because it is not yet wired to the engine. See [the roadmap](#roadmap) for exactly what works today.

![The Moshionly editor](docs/images/editor-light.png)

## Why another motion tool

Most animation software asks you to think in keyframes and curves. That is powerful, and it is also why a ten-second product animation can take an afternoon.

Moshionly works the other way round:

1. **Design the scene** the way it should look at rest.
2. **Add actions** to layers: fade in, slide, scale, move, change colour. Each action has a start, an end and an easing.
3. **Play.** The design is the resting state and the actions play on top of it.

There is nothing to keyframe and nothing breaks when you change the design later. Because every animation is a small, readable object, an AI agent can read your file, add or adjust animations, look at a rendered frame, and hand the result back fully editable.

## What it is built to do

- **Canvas and layers**: artboards, rectangles, ellipses, stars, pen paths, text, images, groups and masks.
- **Styling**: solid and gradient fills, strokes, corner radius, shadow, blur, blend modes.
- **Actions**: move, scale, rotate, opacity, resize, colour and more, plus ready-made presets (fade, slide, grow, shrink, spin, blur, reveal) and text animations by letter, word or line.
- **Easing with one slider**: smooth, natural, slow down, accelerate, overshoot, elastic, bounce, or your own curve.
- **Timeline** where each row is an action you can drag and stretch.
- **Export** to MP4, GIF, PNG sequence, transparent video and Lottie.
- **Figma import** through a plugin that keeps layers editable and lets you re-import a changed design without losing its animations.
- **AI chat with the provider you choose**: use a subscription you already have through its command-line agent (Claude Code and others), or an API key for Anthropic, OpenAI or any OpenAI-compatible endpoint, including local models.
- **Agent-ready**: a built-in MCP server gives any agent the same tools the built-in chat uses: read the file, apply edits atomically, take a screenshot, export.

## Getting started

You need [Rust](https://rustup.rs) (stable). On macOS you also need the Xcode command line tools (`xcode-select --install`).

```bash
git clone https://github.com/steshio/moshionly.git
cd moshionly
cargo run --release
```

The first build downloads and compiles dependencies and takes a few minutes. Later builds are fast. Today this opens the editor with sample content.

To try the engine on a real file, print a frame of the demo project as data:

```bash
cargo run -p moshionly-core --example inspect -- crates/moshionly-core/examples/demo.moshi 800
```

To see which AI providers are ready on your machine:

```bash
cargo run -p moshionly-ai --example chat -- --list
```

Optional: install [ffmpeg](https://ffmpeg.org) (`brew install ffmpeg`) to unlock ProRes and WebM export.

## How the project is organised

```
crates/
  moshionly-core     document model, edits, undo, easing, evaluation at a time
  moshionly-render   draws a scene with Skia
  moshionly-export   PNG, GIF, video, Lottie
  moshionly-agent    tools for AI agents and the MCP server
  moshionly-ai       chat providers: subscriptions and API keys
  moshionly-app      the desktop editor
plugins/figma        the Figma plugin
docs/                architecture and file format
```

Read [docs/architecture.md](docs/architecture.md) for how the pieces fit and [docs/file-format.md](docs/file-format.md) for the `.moshi` file format. The short version: a document is a flat list of layers and actions, a frame is a pure function of the document and a time, and every change is one of three edits.

## Make it yours

Moshionly is MIT licensed. Fork it, rename it, change the look, add your own actions and exporters, build a product on it.

- The whole look lives in one theme module in `moshionly-app`.
- A new action is one type in `moshionly-core` plus its evaluation rule.
- A new export format is one module in `moshionly-export`.
- A new AI provider is one file in `moshionly-ai`.

## Roadmap

Done:

- [x] Document model, edits, undo, the `.moshi` file format
- [x] Animation engine: actions, presets, easing, text animation, counters
- [x] AI providers: Claude Code, Cursor Agent, Codex and Gemini command-line agents, the Anthropic API and any OpenAI-compatible API
- [x] Editor window: theme, controls, canvas with pan and zoom, timeline, command palette (showing sample content)

Next:

- [ ] Skia renderer
- [ ] Agent tools and MCP server
- [ ] Figma plugin and re-import
- [ ] Wire the editor to the engine: real layers, inspector, timeline and playback
- [ ] Export: PNG, GIF, MP4, Lottie
- [ ] Chat panel in the editor
- [ ] Components, video and audio layers, shader effects
- [ ] Linux and Windows builds

## Contributing

Issues and pull requests are welcome. Start with [CONTRIBUTING.md](CONTRIBUTING.md).

## License

[MIT](LICENSE)
