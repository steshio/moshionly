# Contributing to Moshionly

Thanks for helping. This guide covers how to build, what a good change looks like, and where things live.

## Build and run

```bash
cargo run                 # debug build of the editor
cargo run --release       # optimised build
cargo test --workspace    # all tests
cargo fmt --all           # format before you commit
cargo clippy --workspace --all-targets
```

macOS needs the Xcode command line tools. Linux needs development packages for fontconfig, freetype, xkbcommon, wayland and GTK 3; the list is in `.github/workflows/ci.yml`.

## Ground rules

- **Read [docs/architecture.md](docs/architecture.md) first.** It defines how the crates relate and how documents, edits and evaluation work. A change that alters those rules updates that document in the same pull request.
- **Small commits that build.** Every commit should compile and pass tests on its own. Prefer several focused commits over one large one.
- **One way to change a document.** All changes go through `Document::apply` with `Edit` batches. Do not mutate nodes directly from the app or from tools.
- **Keep the engine headless.** Only `moshionly-app` may depend on a windowing or UI library.
- **Test the rule, not the pixels, where you can.** Evaluation and edits are pure functions and cheap to test. Renderer tests compare against reference images in `crates/moshionly-render/tests`.
- **No new required system dependencies.** Optional tools such as `ffmpeg` must be detected at run time and the feature hidden when they are missing.

## Commit messages

Use a short imperative subject with the area as a prefix:

```
core: add fractional index generation
render: draw gradient strokes
app: snap timeline bars to 10 ms
```

## Adding things

| To add | Start in |
|---|---|
| A layer type | `moshionly-core` node definitions, then the scene builder, then `moshionly-render` |
| An action or preset | `moshionly-core` animation definitions and its evaluation rule |
| An easing | `moshionly-core` easing module |
| An export format | a new module in `moshionly-export` |
| An agent tool | `moshionly-agent` tools |
| An AI provider | a new file in `moshionly-ai` |
| A change to the look | the theme module in `moshionly-app` |

## Reporting bugs

Include your operating system, the steps, and if possible the `.moshi` file. A frame is a pure function of the file and a time, so a file plus a timestamp is usually enough to reproduce a rendering bug.

## Conduct

Be kind and assume good intent. Critique the work, not the person.
