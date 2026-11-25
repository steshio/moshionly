# Moshionly

**Open-source motion design for the desktop.** Animate with simple actions instead of keyframes, bring your designs in from Figma, export video, GIF and Lottie, and let the AI you already pay for work in the same file as you.

Moshionly is a native app written in Rust. It runs on macOS first, with Linux and Windows to follow. Your projects are files on your own disk.

> **Status: early, and usable end to end.** You can design and animate in the editor, play it back, export video, GIF, PNG and Lottie, chat with the AI you choose, let outside agents work in the open file, and bring designs in from Figma. It is young software: expect rough edges, and keep copies of work that matters. See [the roadmap](#roadmap) for what works today and what is next.

![The Moshionly editor](docs/images/editor-light.png)

## Why another motion tool

Most animation software asks you to think in keyframes and curves. That is powerful, and it is also why a ten-second product animation can take an afternoon.

Moshionly works the other way round:

1. **Design the scene** the way it should look at rest.
2. **Add actions** to layers: fade in, slide, scale, move, change colour. Each action has a start, an end and an easing.
3. **Play.** The design is the resting state and the actions play on top of it.

There is nothing to keyframe and nothing breaks when you change the design later. Because every animation is a small, readable object, an AI agent can read your file, add or adjust animations, look at a rendered frame, and hand the result back fully editable.

## What it is built to do

- **Canvas and layers**: artboards, rectangles, ellipses, stars, pen paths, text, images, video, groups and masks.
- **Video and sound**: drop in video clips and sound files, place and trim them on the timeline, hear them in sync while playing, and get the sound in MP4, WebM and ProRes exports (needs [ffmpeg](https://ffmpeg.org)).
- **Components**: make a reusable piece once, place copies of it anywhere, and edit the main one to change every copy, animations included. Each copy can play the motion a little later, for staggered effects.
- **Styling**: solid and gradient fills, strokes, corner radius, shadow, blur, blend modes.
- **Actions**: move, scale, rotate, opacity, resize, colour and more, plus ready-made presets (fade, slide, grow, shrink, spin, blur, reveal) and text animations by letter, word or line.
- **Easing with one slider**: smooth, natural, slow down, accelerate, overshoot, elastic, bounce, or your own curve.
- **Timeline** where each row is an action you can drag and stretch.
- **Export** to MP4, GIF, PNG sequence, transparent video and Lottie, with sound in the video formats.
- **Figma import** through a plugin that keeps layers editable and lets you re-import a changed design without losing its animations.
- **AI chat with the provider you choose**: use a subscription you already have through its command-line agent (Claude Code and others), or an API key for Anthropic, OpenAI or any OpenAI-compatible endpoint, including local models.
- **Agent-ready**: a built-in MCP server gives any agent the same tools the built-in chat uses: read the file, apply edits atomically, take a screenshot, export.

## Install on a Mac

1. Get `Moshionly-<version>.dmg` from the [releases page](https://github.com/steshio/moshionly/releases), or from whoever shared it with you.
2. Open it, and drag Moshionly onto the Applications folder next to it.
3. Open Moshionly from Applications, Launchpad or Spotlight. The first time, macOS checks that you want to open an app from the internet: choose Open.

Signed releases are not published yet. Until they are, a `.dmg` someone shares with you is not signed by Apple, and macOS refuses to open it at first: see [Without an Apple account](#without-an-apple-account) for how to open it anyway.

Moshionly runs on Macs with Apple silicon (M1 and later). It is built for macOS 11 or later and tested on the latest macOS. Intel Macs are not supported (macOS 27 itself runs only on Apple silicon).

Double-clicking a `.moshi` file does not open it in Moshionly yet: open projects from the File menu.

The `moshionly` command line comes inside the app, for agents and scripts:

```bash
# Serve a project to an agent over MCP, for example with Claude Code
claude mcp add moshionly -- /Applications/Moshionly.app/Contents/MacOS/moshionly mcp --save /path/to/project.moshi

# Export without opening the window
/Applications/Moshionly.app/Contents/MacOS/moshionly export project.moshi --out project.mp4
```

The agents button in the app shows the same command, with the full path.

## Getting started

To build and run Moshionly from the source code, you need [Rust](https://rustup.rs) (stable). On macOS you also need the Xcode command line tools (`xcode-select --install`).

```bash
git clone https://github.com/steshio/moshionly.git
cd moshionly
cargo run --release
```

The first build downloads and compiles dependencies and takes a few minutes. Later builds are fast. This opens the editor with a new, empty project. To open the demo project instead:

```bash
cargo run --release -- crates/moshionly-core/examples/demo.moshi
```

In the editor:

- **Design** in the Design tab: draw shapes and text, style them in the inspector, arrange layers.
- **Animate** in the Animate tab (press M): pick a layer, choose "Add action", then drag its bar on the timeline. Space plays.
- **Export** with the Export button or ⌘E: PNG, PNG sequence, GIF, MP4, WebM, ProRes or Lottie.
- **Chat** with the chat button in the top bar. Its settings let you pick a subscription you already have (Claude Code and other command-line agents) or an API key.
- **Connect an agent** with the agents button in the top bar. It shows the exact setup for Claude Code, Cursor, VS Code and other MCP clients, which then work in the open file while you watch, and every change they make can be undone.
- **Bring a design in from Figma** with the plugin, loaded in Figma's desktop app as [plugins/figma/README.md](plugins/figma/README.md) explains. A design sent from Figma, or pasted after the plugin's Copy button, is added at once as one step you can undo; sending it again later updates it and keeps its animations.

The same `moshionly` command also works without a window:

```bash
# Export an artboard (the format comes from the file extension, or --format)
cargo run --release -- export crates/moshionly-core/examples/demo.moshi --out demo.mp4

# Serve a file to an agent over MCP on standard input and output
cargo run --release -- mcp --save crates/moshionly-core/examples/demo.moshi
```

`moshionly --help`, `moshionly export --help` and `moshionly mcp --help` list every option.

Optional: install [ffmpeg](https://ffmpeg.org) (`brew install ffmpeg`, version 5.1 or newer) to unlock video and sound layers, sound in exports, and ProRes and WebM export. Without it everything else works, and the app says where ffmpeg is needed.

## Build the Mac app yourself

One command builds `Moshionly.app` and its installer, signs them, has Apple check them, and staples Apple's approval to both, so the app opens on other people's Macs without a warning:

```bash
scripts/release-mac.sh
```

It needs the same tools as [Getting started](#getting-started) (Rust and the Xcode command line tools), works from any folder, and takes a few minutes the first time. Apple's check usually takes a few minutes too, but Apple can take hours over your first submissions; leave the script running until it finishes. You get two things in `dist/` at the top of the repository:

- `dist/Moshionly.app`, the app itself;
- `dist/Moshionly-<version>.dmg`, the installer to give to people.

Running it again replaces them. Before it builds anything, the script checks that everything it needs is in place and says in plain words what is missing. `scripts/release-mac.sh --help` lists the options:

- `--skip-notarize` signs with your Developer ID but does not send the app to Apple. macOS will still warn people who download it.
- `--ad-hoc` needs no Apple account at all (see below).
- `--out <folder>` puts the results in another folder.

### One-time signing setup

This is for the maintainer who publishes releases. It needs a paid [Apple Developer Program](https://developer.apple.com/programs/) membership, and the certificate can only be created by the team's Account Holder.

1. **Create a Developer ID Application certificate.** In Xcode: Xcode › Settings › Accounts, select your team, choose Manage Certificates, then + › Developer ID Application. (Or create it on [developer.apple.com](https://developer.apple.com/account/resources/certificates/list) with a certificate signing request from Keychain Access.) To check that it is in your keychain:

   ```bash
   security find-identity -v -p codesigning
   ```

   The list should include a line with `Developer ID Application: <your name> (<your Team ID>)`.

2. **Make an app-specific password** at [account.apple.com](https://account.apple.com), under Sign-In and Security › App-Specific Passwords. Apple's notary service uses it instead of your real password.

3. **Save it for the notary tool**, once:

   ```bash
   xcrun notarytool store-credentials moshionly-notary --apple-id <your Apple ID> --team-id <your Team ID>
   ```

   It asks for the app-specific password and keeps it in your login keychain. Nothing goes into the repository. Your Team ID is the ten-character code shown in the certificate's name and under Membership details on developer.apple.com.

The first time the script signs, macOS may ask for your login password so that `codesign` can use the certificate: choose Always Allow. If the script says Xcode's licence has not been accepted, run `sudo xcodebuild -license accept` once.

The script finds the Developer ID certificate on its own. Two settings, from the environment, change what it uses:

- `MOSHIONLY_SIGN_IDENTITY`: which certificate to sign with, by name or by the SHA-1 hash `security find-identity` shows. Needed only when your keychain has more than one Developer ID Application certificate.
- `MOSHIONLY_NOTARY_PROFILE`: the name you gave `notarytool store-credentials`, if it is not `moshionly-notary`.

### Without an Apple account

Contributors who want to try the Mac app without a developer membership can build it with an ad-hoc signature, which needs no account:

```bash
scripts/release-mac.sh --ad-hoc
```

The app runs normally on the Mac that built it. On another Mac, macOS refuses to open it at first, because Apple has not checked it. To open it anyway: try to open it once, then go to System Settings › Privacy & Security, scroll down to the message about Moshionly, and choose Open Anyway.

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
packaging/macos      the Mac app's Info.plist template
scripts/             release-mac.sh, which builds the Mac app and its installer
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
- [x] Editor window: theme, controls, canvas with pan and zoom, timeline, command palette
- [x] Skia renderer: shapes, gradients, strokes, masks, shadows, blur, images, text with bundled Inter
- [x] Agent tools and MCP server, and the endpoint the Figma plugin sends to
- [x] Figma plugin and re-import (tested against sample designs; not yet run inside Figma)
- [x] The editor on real files: open, save, layers, canvas editing, Design inspector, undo
- [x] Animate tab, editable timeline and playback
- [x] Export: PNG, PNG sequence, GIF, MP4, WebM and ProRes (with ffmpeg), and Lottie
- [x] Chat panel and AI settings in the editor
- [x] Connect an agent to the open file, take in designs from Figma, and the `moshionly export` and `moshionly mcp` commands
- [x] The Mac app: `Moshionly.app` and a `.dmg` from one command, which can also sign, notarize and staple them

Next:

- [ ] Run the Figma plugin inside Figma and polish the round trip
- [ ] Publish the signed `.dmg` with each release, and open `.moshi` files by double-clicking them
- [x] Components: reusable pieces whose copies follow the main one, animations included
- [x] Video and audio layers: clips on the timeline, played in sync, with sound in MP4, WebM and ProRes exports (with ffmpeg)
- [ ] Shader effects
- [ ] Linux and Windows builds

## Contributing

Issues and pull requests are welcome. Start with [CONTRIBUTING.md](CONTRIBUTING.md).

## License

[MIT](LICENSE)
