# Moshionly for Figma

A Figma plugin that sends your design to [Moshionly](../../README.md) as real, editable layers: frames, shapes, text and images arrive as Moshionly artboards, groups, shapes, text and images, ready to animate. When you change the design in Figma later, send it again and Moshionly swaps in the new version **without losing the animations** you added.

## Install

You need the Figma desktop app and a copy of this repository.

1. In Figma, open any design file.
2. In the menu, choose **Plugins → Development → Import plugin from manifest…**
3. Pick `plugins/figma/manifest.json` from this repository.

The plugin now appears under **Plugins → Development → Moshionly**. There is nothing to build or install first.

## Send a design

1. Open Moshionly.
2. In Figma, select the frames or layers you want. Selecting a section sends the frames in it.
3. Run **Plugins → Development → Moshionly**.
4. Click **Send to Moshionly**.

Moshionly then asks where to put the design. Nothing changes in your project until you accept it there.

**Copy** puts the same design on the clipboard instead; paste it into Moshionly with Cmd+V. Use it when Moshionly runs on another computer, or when sending is blocked (see [Troubleshooting](#troubleshooting)). If the design took a while to prepare, the button changes to **Copy now**: click it once more, because the clipboard only accepts text right after a click.

After each send the panel lists **notes**: everything Moshionly shows differently from Figma, layer by layer, and why.

## Send it again after changing it

Animate the design in Moshionly, change it in Figma, then send it again with **Replace the matching layers and keep their animations** chosen in the panel. Moshionly puts the new version in place of the old one and moves each animation over to the same layer in the new version.

Layers are matched by where they came from: the Figma file and the layer's own identity in it, not its name. So:

- Renaming, restyling, moving and resizing layers keeps their animations, and so does moving a layer into another group.
- A layer you delete in Figma and draw again is a new layer: its old animations are dropped.
- Send from the same Figma file. A copy of the file (**Duplicate**) has new identities.
- Frames on the canvas become artboards, and an artboard can only be replaced by an artboard: keep the frame a frame.

The first choice, **Add it as new artboards**, always adds a fresh copy next to your other artboards. The panel remembers your choice between runs; it is a suggestion, and the final choice is made in Moshionly when it asks where to put the design.

## What comes over

| In Figma | In Moshionly |
|---|---|
| A frame on the canvas (also a component or an instance there) | An artboard at the same place on the canvas |
| A frame, component or instance inside another layer | A group with the frame's fill, stroke, corner radius and clipping |
| A group | A group |
| A group made with **Use as mask**, or a mask among other layers | A mask group |
| Rectangle | Rectangle, with each corner's radius |
| Ellipse, arc, ring | Ellipse with its start, sweep and inner radius |
| Star | Star with its points and inner radius |
| Polygon, line, vector, boolean shape | A pen path with the exact outline (a boolean shape becomes its result) |
| Text | Text with its font, weight, italic, size, line height, letter spacing, alignment, box sizing and case |
| Rectangle filled with an image | Image, with its corner radius and stroke |
| Image fill of a frame | An image at the back of the frame's group, plus a tint layer for any colour painted over it |
| Solid fills, linear and radial gradients | The same paint, gradients exactly where Figma draws them |
| Stroke | One stroke with its width, inside / centre / outside, line ends and corner joins |
| Drop shadow, layer blur | Shadow and blur |
| Opacity, blend mode, rotation, hidden, locked, names | The same |

Auto layout, constraints and padding are not part of Moshionly: every layer arrives where Figma placed it, at its final size.

## What is approximated

Each of these is listed in the panel's notes when it happens.

- **More than one fill or stroke.** Moshionly layers have one of each. Fills that are hidden or covered by an opaque fill are simply dropped; colours stacked on each other are mixed into the one colour you see. Otherwise the top fill is kept.
- **Angular and diamond gradients** become radial gradients with the same colours.
- **Dashed strokes** are drawn solid. **Sides of different widths** take the widest. **Arrow ends** become flat ends. **Strokes of varying width** become even.
- **Several drop shadows**: the strongest is kept. **Shadow spread**, **inner shadows**, **background blur**, and Figma's noise, texture and glass effects are left out. A progressive blur becomes an even one.
- **Mixed text styles**: Moshionly text has one style, so the whole text takes the style of most of its characters. The note lists the other styles. **Justified** text is aligned left; **underline**, **strikethrough**, **small caps**, **list bullets**, **paragraph spacing** and **truncation** with "…" are left out.
- **Auto line height** becomes 120% of the font size, which is close for most fonts.
- **Smoothed corners** become plain round corners.
- **A frame on the canvas** cannot turn, fade or have a shadow as an artboard; when it has round corners, a stroke, effects, opacity or a blend mode, those go on a group just inside the artboard, and a shadow is cut off at the artboard's edge. An artboard always cuts off what sticks out of it, even when **Clip content** is off for the frame in Figma. A turned frame arrives upright.
- **A background image** of a frame that is cropped, tiled or adjusted covers the frame instead.
- **Blend modes** linear burn and linear dodge become multiply and screen.
- **Luminance masks** mask by transparency.
- **Vector masks** ignore how see-through the mask shape is. The plugin makes the shape's colours solid so Moshionly masks the same way; a vector mask with a shadow or blur keeps its look, with a note.

## What the plugin assumes about Figma

Figma's documentation does not settle these, so the plugin follows what Figma users report. If Figma turns out to behave differently, these are the places to fix.

- **A drop shadow keeps its direction on the page** when its layer, or a group around it, is turned or mirrored. Moshionly turns a shadow with its layer, so the plugin turns the offset back to match.

## What is sent as a picture

When Moshionly has no way to rebuild a layer, Figma renders it and the layer arrives as an image that looks the same but cannot be edited as shapes:

- images in shapes other than rectangles (an avatar circle, say), and images that are cropped, tiled, turned, colour-adjusted, mirrored, or have something painted over them;
- video, pattern and shader fills, and brush strokes;
- mirrored, skewed or stretched text;
- skewed groups and frames;
- layer types Moshionly does not have, such as stickies, text on a path, repeated shapes and embeds.

Pictures are embedded in what is sent. Large photos are scaled down to twice the size they are shown at (at least 1,024 pixels on the longer side, at most 4,096), which keeps them sharp on high-resolution screens without making the design huge.

## Troubleshooting

**"Moshionly is not running."** Open the Moshionly app and send again. The plugin talks to it on this computer only, at `http://localhost:47821`. If something else uses that port, Moshionly cannot listen there.

**Nothing happens in Figma in the browser.** Browsers may block a web page from reaching apps on your computer, or ask first. Allow it if your browser asks, use the Figma desktop app, or use **Copy**.

**"Moshionly did not take the design."** The message after it is Moshionly's own reason. If it mentions a field or a layer, please report it with the design: the plugin should never send something Moshionly refuses.

**"This design is … more than the 64 MB Moshionly takes at once."** Send fewer frames at a time. Big photos are the usual cause.

**Fonts look different.** Moshionly uses the fonts installed on your computer. Install the same fonts there as in Figma.

**Sending again added a copy instead of replacing.** Choose **Replace the matching layers and keep their animations** before sending, and send from the same Figma file. Plugins loaded from a manifest learn the file's identity from Figma; if Figma does not give it, the plugin keeps a small identity of its own inside the file (invisible plugin data) so later sends still match.

**Figma refuses the manifest.** Figma may ask for a plugin id of its own. Create a new plugin in Figma (**Plugins → Development → New plugin…**), copy the `id` from its manifest into `plugins/figma/manifest.json`, and import again.

## For developers

The plugin is plain JavaScript with no build step and no dependencies.

| File | What it is |
|---|---|
| `manifest.json` | What Figma reads. Network access is limited to the Moshionly app on this computer. |
| `code.js` | Runs in Figma's sandbox. The first part converts nodes to Moshionly layers and only reads node properties, so it works on plain objects shaped like Figma nodes; the end of the file talks to Figma and the panel. |
| `ui.html` | The panel. Its `delivery` script sends and copies the design. Both its scripts are tested in Node, the panel against a stand-in page. |
| `fixtures/` | Builders for Figma-like nodes, three whole designs, and the rules check (`check-layers.js` with `core-fields.json`). |
| `test/` | `node --test` tests. |
| `acceptance/` | A small Rust tool, outside the Cargo workspace, that checks output with Moshionly's own import code. |
| `tools/` | `convert-fixtures.js` writes the fixture designs as envelopes; `send.js` posts an envelope to a running Moshionly the way the panel does. |

Run the tests (Node 18 or later, nothing to install):

```bash
cd plugins/figma
node --test
```

Check the output with Moshionly's own import code (Node and Rust):

```bash
plugins/figma/acceptance/run.sh
```

It converts the fixture designs, runs each through the checks behind the app's `/import`, imports it as new artboards, into an artboard, and once more over itself after animating every layer (every animation must survive), and checks that `fixtures/core-fields.json` still matches core. When core's node types change, rewrite that file with `cargo run --manifest-path plugins/figma/acceptance/Cargo.toml -- fields plugins/figma/fixtures/core-fields.json`.

### What the plugin sends

`POST http://localhost:47821/import` (then `http://127.0.0.1:47821/import`), with a JSON body:

```json
{
  "format": "moshionly-import",
  "version": 1,
  "source": { "app": "figma", "file_name": "Launch", "file_key": "abc123" },
  "mode": "replace_matching",
  "nodes": [
    { "id": "1:2", "type": "artboard", "name": "Card", "x": 0, "y": 0, "width": 360, "height": 480,
      "source_id": "figma:abc123:1:2", "fill": "#FFFFFF" },
    { "id": "1:3", "parent": "1:2", "type": "text", "name": "Title", "x": 24, "y": 32, "width": 200, "height": 40,
      "source_id": "figma:abc123:1:3", "content": "Launch day", "font": { "family": "Inter", "weight": 700 }, "font_size": 32 }
  ],
  "notes": [
    { "source_id": "figma:abc123:1:3", "layer_name": "Title", "message": "Justified text is aligned left; Moshionly does not justify." }
  ]
}
```

`nodes` are layers in Moshionly's node format ([docs/file-format.md](../../docs/file-format.md)), parents before children, siblings back to front, with Figma's node ids as temporary ids. Every layer has `source_id` = `figma:<file key>:<node id>`; layers the plugin adds for part of a node append `/<part>` (`/frame`, `/fill`, `/overlay`, `/mask`). Images are always embedded `data:` URIs. `mode` is `new_artboards` or `replace_matching`, a suggestion; the person decides in Moshionly. **Copy** puts the same JSON on the clipboard.

In fixtures, which are JSON, Figma's mixed value (`figma.mixed`) is written as the string `"MIXED"`, and text runs that Figma reports through `getStyledTextSegments` are listed under `styledTextSegments`.
