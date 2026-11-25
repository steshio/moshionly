# The `.moshi` file format

A Moshionly project is one JSON file with the extension `.moshi`. This document describes everything that can be in it. It is for people who want to read or write project files by hand, and for authors of plugins and tools that produce them.

The reference tables in the middle of this document are generated from the code and checked by a test, so they always match the version of Moshionly they ship with.

## The file

```json
{
  "format": "moshionly",
  "version": 3,
  "name": "Launch teaser",
  "nodes": [
    { "id": "ab12", "parent": null, "index": "a0", "type": "artboard", "name": "Scene 1",
      "width": 1080, "height": 1080, "fill": "#FFFFFF" },
    { "id": "cd34", "parent": "ab12", "index": "a0", "type": "rect", "name": "Card",
      "x": 240, "y": 340, "width": 600, "height": 400, "fill": "#0D99FF", "corner_radius": 32 },
    { "id": "ef56", "parent": "ab12", "index": "a0", "type": "fade_in",
      "target": "cd34", "start": 0, "end": 600 }
  ]
}
```

| Field | Value | Meaning |
|---|---|---|
| `format` | `"moshionly"` | Says what the file is. Anything else is refused as "not a Moshionly project". |
| `version` | whole number | The version of the format. This document describes version 3, which added video and sound (`video` and `audio`); version 2 added components (`component` and `instance`). Older files are read as they are. |
| `name` | string | The name of the project. May be left out. |
| `nodes` | list of nodes | Everything in the project, as a flat list. |

A file with a higher `version` than the app knows is refused with a message that asks to update the app. A file with a lower version is brought up to date when it is loaded, one version at a time.

When Moshionly saves a file it writes one node per line, in tree order, with the fields of each node in a fixed order, and it leaves out every field that is at its default. Saving the same project twice gives the same bytes, so project files work well in version control. None of that is required when you write a file yourself: order and white space are free.

## Nodes

A project is a flat list of nodes. Every node is either a **layer** (something that is drawn) or an **animation** (something that changes a layer over time). The `type` field says which, and decides which other fields the node has.

Every node has these four fields:

| Field | Value | Meaning |
|---|---|---|
| `id` | string | Unique in the file. 1 to 64 characters from `A-Z`, `a-z`, `0-9`, `_` and `-`. Moshionly generates 12 random characters. |
| `parent` | id or `null` | The node this one is inside. `null` (or left out) for an artboard, which is a root. |
| `index` | string | Orders the node among its siblings. See below. |
| `type` | string | One of the layer or animation types in the reference. |

Fields that a type does not have are an error, not ignored. So is an unknown `type`. This catches typing mistakes instead of hiding them.

### The tree

The nodes form trees through `parent`:

| A node of type | Sits inside |
|---|---|
| `artboard` | nothing: `parent` is `null` |
| any other layer | an `artboard`, a `group`, a `mask_group` or a `component` |
| any animation | the `artboard` whose timeline it is on, or an `animation_group` in that artboard |

An animation does not sit inside the layer it changes. It is listed in the timeline of the artboard and names its layer in `target`. The target must be a layer in the same artboard (for a `fill` animation it may be the artboard itself). Removing a layer removes the layers inside it and every animation that targets any of them.

An `instance` names the `component` it shows in its `component` field, which may be in any artboard. `parent`, `target` and `component` are the only ways a node refers to another. See [Components](#components).

### Order

Siblings are ordered by `index`, compared as plain strings, character by character. Layers and animations are ordered separately, so a layer and an animation under the same artboard can both have the index `a0`.

- For layers the order is the drawing order: a larger index is drawn later, which puts it in front.
- For animations the order is the row order in the timeline. It also breaks ties: when two animations on the same property start at the same moment, the lower row wins.

Indices are *fractional indices*: strings built so that a new one can always be made between two others without renumbering anything. `a0`, `a1`, `a2` and so on are valid indices and are all a plugin or a hand-written file needs. Moshionly generates values such as `a0V` when a node is inserted between `a0` and `a1`.

When a file is loaded, a group of siblings whose indices are not valid or not all different gets fresh indices in the order the old ones sort in (and, for equal ones, the order of the file). So a hand-written file can simply number its nodes.

## Units and conventions

- **Lengths** are in pixels.
- **Positions**: `x` and `y` are the top-left corner of a layer's box, measured from the top-left corner of its parent. `y` grows downward.
- **Angles** are in degrees, clockwise.
- **Rotation and scale** turn and resize a layer about the centre of its box, so `x` and `y` describe the box before it is turned.
- **Times** are in milliseconds from the start of the artboard.
- **Opacity** runs from 0 (invisible) to 100 (opaque).
- **Colours** are written `#RRGGBB` or `#RRGGBBAA`, in either case.
- **Gradient and path coordinates** on a layer are fractions of the layer's box: `[0, 0]` is its top-left corner and `[1, 1]` its bottom-right.
- **Defaults**: a field that is left out has the default shown in the reference. "none" means the field is simply absent: no fill, no shadow.

## Reference

<!-- reference:start -->

### Fields of every layer

| Field | Value | Default | Meaning |
|---|---|---|---|
| `name` | string | `""` | Name shown in the layer list. |
| `x` | number | `0` | Left edge of the layer's box in its parent's coordinates, in pixels. For an artboard: its position on the canvas. |
| `y` | number | `0` | Top edge of the layer's box in its parent's coordinates, in pixels, growing downward. |
| `width` | number | `100` | Width of the box in pixels, 0 or more. |
| `height` | number | `100` | Height of the box in pixels, 0 or more. |
| `rotation` | number | `0` | Rotation about the centre of the box in degrees, clockwise. |
| `scale` | number | `1` | Uniform scale about the centre of the box, 0 or more; 1 is actual size. |
| `opacity` | number | `100` | Opacity from 0 (invisible) to 100 (opaque). |
| `hidden` | boolean | `false` | Whether the layer starts hidden. `show` and `hide` animations change this over time. |
| `locked` | boolean | `false` | Whether the editor refuses to select or move the layer. Has no effect on rendering. |
| `blend_mode` | [blend_mode](#blend_mode) | `"normal"` | How the layer combines with what is behind it. |
| `blur` | number | `0` | Blur of the whole layer in pixels, 0 or more: twice the standard deviation of the Gaussian. |
| `shadow` | [shadow](#shadow) | none | Drop shadow. |
| `source_id` | string | none | Where an imported layer came from, for example `figma:<file key>:<node id>`. Re-importing a design matches layers by this value to keep their animations. |

### Layer types

#### `artboard`

A root frame with a size, a background and its own timeline. Everything else lives inside one.

| Field | Value | Default | Meaning |
|---|---|---|---|
| `duration` | number | `4000` | Length of the artboard's timeline in milliseconds, at least 1. |
| `fill` | [paint](#paint) | none | Background. Default: none, a transparent artboard. |

#### `group`

A box that holds other layers and moves them together; can clip them and have its own fill, stroke and rounded corners.

| Field | Value | Default | Meaning |
|---|---|---|---|
| `fill` | [paint](#paint) | none | Background drawn behind the children. |
| `stroke` | [stroke](#stroke) | none | Outline of the box, drawn over the children. |
| `corner_radius` | [corner_radius](#corner_radius) | `0` | Rounding of the box corners in pixels. |
| `clip` | boolean | `false` | Whether children are cut off at the (rounded) box. |

#### `mask_group`

A group whose first child (lowest index) is an alpha mask for the other children.

It has no fields of its own.

#### `rect`

A rectangle with optional rounded corners.

| Field | Value | Default | Meaning |
|---|---|---|---|
| `fill` | [paint](#paint) | none | Paint of the inside. |
| `stroke` | [stroke](#stroke) | none | Outline. |
| `corner_radius` | [corner_radius](#corner_radius) | `0` | Rounding of the corners in pixels. |

#### `ellipse`

An ellipse; can be cut down to an arc, a pie slice or a ring.

| Field | Value | Default | Meaning |
|---|---|---|---|
| `fill` | [paint](#paint) | none | Paint of the inside. |
| `stroke` | [stroke](#stroke) | none | Outline. |
| `start_angle` | number | `0` | Where the arc starts, in degrees clockwise from the 3 o'clock position. |
| `sweep` | number | `360` | How far the arc runs from its start, in degrees clockwise, from -360 to 360. 360 is the whole ellipse. |
| `inner_radius` | number | `0` | Radius of the hole as a fraction of the outer radius, 0 to 1. 0 is a filled ellipse (a pie slice for an arc), larger values make a ring. |

#### `star`

A star with a chosen number of points; an inner radius of 1 makes a polygon.

| Field | Value | Default | Meaning |
|---|---|---|---|
| `fill` | [paint](#paint) | none | Paint of the inside. |
| `stroke` | [stroke](#stroke) | none | Outline. |
| `points` | whole number | `5` | Number of points, 3 to 64. |
| `inner_radius` | number | `0.4` | Radius of the inner corners as a fraction of the outer radius, 0 to 1. 1 makes a regular polygon with twice as many corners. |

#### `path`

A pen-drawn shape made of straight and curved segments.

| Field | Value | Default | Meaning |
|---|---|---|---|
| `fill` | [paint](#paint) | none | Paint of the inside. |
| `stroke` | [stroke](#stroke) | none | Outline. |
| `path` | [path_data](#path_data) | `{"subpaths":[],"fill_rule":"non_zero"}` | The outline, with coordinates normalised to the box: `[0, 0]` is its top-left corner and `[1, 1]` its bottom-right. |

#### `text`

A block of text in one font and style.

| Field | Value | Default | Meaning |
|---|---|---|---|
| `fill` | [paint](#paint) | none | Paint of the glyphs. |
| `stroke` | [stroke](#stroke) | none | Outline of the glyphs. |
| `content` | string | `""` | The text. Line breaks are `\n`. |
| `font` | [font](#font) | `{"family":"Inter","weight":400,"italic":false}` | The font. |
| `font_size` | number | `48` | Font size in pixels, more than 0. |
| `line_height` | number | `120` | Distance between baselines as a percentage of the font size, more than 0. |
| `letter_spacing` | number | `0` | Extra space after every letter as a percentage of the font size; may be negative. |
| `align` | `"left"`, `"center"` or `"right"` | `"left"` | Horizontal alignment of lines in the box. |
| `vertical_align` | `"top"`, `"middle"` or `"bottom"` | `"top"` | Vertical alignment of the text in the box. |
| `auto_size` | `"width_and_height"`, `"height"` or `"fixed"` | `"width_and_height"` | How the box follows the content. |
| `case` | `"original"`, `"upper"`, `"lower"` or `"title"` | `"original"` | Letter case applied when drawing. |

#### `image`

A bitmap image from a file or a data URI.

| Field | Value | Default | Meaning |
|---|---|---|---|
| `source` | string | required | Where the image comes from: a path relative to the project file, an absolute path, or a `data:` URI. |
| `fit` | `"cover"`, `"contain"` or `"stretch"` | `"cover"` | How the image fills the box when their proportions differ. |
| `stroke` | [stroke](#stroke) | none | Outline of the box. |
| `corner_radius` | [corner_radius](#corner_radius) | `0` | Rounding of the corners in pixels. |

#### `component`

A main component: a group whose look, layers and their animations every instance of it shows; edit it and every instance changes.

| Field | Value | Default | Meaning |
|---|---|---|---|
| `fill` | [paint](#paint) | none | Background drawn behind the children. |
| `stroke` | [stroke](#stroke) | none | Outline of the box, drawn over the children. |
| `corner_radius` | [corner_radius](#corner_radius) | `0` | Rounding of the box corners in pixels. |
| `clip` | boolean | `false` | Whether children are cut off at the (rounded) box. |

#### `instance`

A copy of a main component: shows its look, layers and animations, time_offset milliseconds later; holds no layers of its own.

| Field | Value | Default | Meaning |
|---|---|---|---|
| `component` | id | required | Id of the main component this instance shows: a `component` layer anywhere in the document, in any artboard. |
| `time_offset` | number | `0` | How much later than the component this instance plays the component's animations, in milliseconds; negative plays them earlier. Give each copy a little more to stagger them. |

#### `video`

A video clip from a file: between start and end it shows the file's frames like an image and plays its sound.

| Field | Value | Default | Meaning |
|---|---|---|---|
| `source` | string | required | Where the video comes from: a path relative to the project file or an absolute path. Video cannot be embedded as a `data:` URI. |
| `fit` | `"cover"`, `"contain"` or `"stretch"` | `"cover"` | How the frames fill the box when their proportions differ. |
| `stroke` | [stroke](#stroke) | none | Outline of the box. |
| `corner_radius` | [corner_radius](#corner_radius) | `0` | Rounding of the corners in pixels. |
| `start` | number | required | When the clip starts playing, in milliseconds from the start of the artboard, 0 or more. |
| `end` | number | required | When the clip stops playing, in milliseconds from the start of the artboard; after `start`. |
| `trim_start` | number | `0` | Where in the file the clip begins, in milliseconds from the start of the file, 0 or more: at `start` the clip is this far into its file. |
| `volume` | number | `1` | How loud the clip's sound plays, from 0 (silent) to 1 (as recorded). |

### Fields of every animation

Every animation that runs over time has these. `show`, `hide`, `audio` and `animation_group` list their own fields below.

| Field | Value | Default | Meaning |
|---|---|---|---|
| `name` | string | `""` | Label shown on the timeline row. |
| `target` | id | required | Id of the layer this animation changes. It must be in the same artboard as the animation. |
| `start` | number | required | When the animation starts, in milliseconds from the start of the artboard, 0 or more. |
| `end` | number | required | When the animation ends, in milliseconds from the start of the artboard. Not before `start`; equal to `start` makes it instant. |
| `easing` | [easing](#easing) | `{"type":"smooth","intensity":50}` | How progress is paced between `start` and `end`. |

### Property animations

Each changes one property of its target between two values.

#### `move`

Shifts a layer by an offset from its resting position. Additive: several moves add up.

| Field | Value | Default | Meaning |
|---|---|---|---|
| `from` | [offset](#offset) | none | Offset at the start, in pixels. Default: no offset. |
| `to` | [offset](#offset) | none | Offset at the end, in pixels. Default: no offset, back at rest. |

#### `rotate`

Turns a layer by an angle added to its resting rotation. Additive: several rotations add up.

| Field | Value | Default | Meaning |
|---|---|---|---|
| `from` | number | none | Extra rotation at the start, in degrees clockwise. Default 0. |
| `to` | number | none | Extra rotation at the end, in degrees clockwise. Default 0, back at rest. |

#### `scale`

Scales a layer by a multiplier of its resting scale.

| Field | Value | Default | Meaning |
|---|---|---|---|
| `from` | number | none | Multiplier at the start, 0 or more. Default: the value at that moment. |
| `to` | number | none | Multiplier at the end, 0 or more. Default 1, the resting size. |

#### `opacity`

Fades a layer to an opacity from 0 to 100.

| Field | Value | Default | Meaning |
|---|---|---|---|
| `from` | number | none | Opacity at the start, 0 to 100. Default: the value at that moment. |
| `to` | number | none | Opacity at the end, 0 to 100. Default: the layer's resting opacity. |

#### `resize`

Changes the width and height of a layer's box while one anchor point stays fixed.

| Field | Value | Default | Meaning |
|---|---|---|---|
| `from` | [size_change](#size_change) | none | Size at the start. Default: the size at that moment. |
| `to` | [size_change](#size_change) | none | Size at the end. Default: the layer's resting size. |
| `anchor` | [resize_anchor](#resize_anchor) | `"center"` | Which point of the box stays fixed. |

#### `fill`

Changes a layer's fill, or an artboard's background, to another colour or gradient.

| Field | Value | Default | Meaning |
|---|---|---|---|
| `from` | [paint](#paint) | none | Fill at the start. Default: the fill at that moment. |
| `to` | [paint](#paint) | none | Fill at the end. Default: the layer's resting fill. |

#### `stroke`

Changes the paint and width of a layer's stroke.

| Field | Value | Default | Meaning |
|---|---|---|---|
| `from` | [stroke_change](#stroke_change) | none | Stroke at the start. Default: the stroke at that moment. |
| `to` | [stroke_change](#stroke_change) | none | Stroke at the end. Default: the layer's resting stroke. |

#### `corner_radius`

Changes the corner rounding of a rect, group or image.

| Field | Value | Default | Meaning |
|---|---|---|---|
| `from` | [corner_radius](#corner_radius) | none | Radius at the start: a number or four numbers. Default: the radius at that moment. |
| `to` | [corner_radius](#corner_radius) | none | Radius at the end: a number or four numbers. Default: the layer's resting radius. |

#### `blur`

Changes how blurred a layer is.

| Field | Value | Default | Meaning |
|---|---|---|---|
| `from` | number | none | Blur at the start in pixels, 0 or more. Default: the blur at that moment. |
| `to` | number | none | Blur at the end in pixels, 0 or more. Default: the layer's resting blur. |

#### `shadow`

Changes a layer's drop shadow: colour, offset and blur.

| Field | Value | Default | Meaning |
|---|---|---|---|
| `from` | [shadow_change](#shadow_change) | none | Shadow at the start. Default: the shadow at that moment. |
| `to` | [shadow_change](#shadow_change) | none | Shadow at the end. Default: the layer's resting shadow. |

### Instant animations

These happen at one moment and have no easing.

#### `show`

Makes a layer visible at one moment; the layer is hidden before the first show.

| Field | Value | Default | Meaning |
|---|---|---|---|
| `name` | string | `""` | Label shown on the timeline row. |
| `target` | id | required | Id of the layer to show or hide. It must be in the same artboard. |
| `start` | number | required | The moment it happens, in milliseconds from the start of the artboard, 0 or more. |
| `end` | number | none | Accepted so that every animation can be written with a `start` and an `end`, and ignored: the switch happens at `start`. |

#### `hide`

Makes a layer invisible at one moment.

| Field | Value | Default | Meaning |
|---|---|---|---|
| `name` | string | `""` | Label shown on the timeline row. |
| `target` | id | required | Id of the layer to show or hide. It must be in the same artboard. |
| `start` | number | required | The moment it happens, in milliseconds from the start of the artboard, 0 or more. |
| `end` | number | none | Accepted so that every animation can be written with a `start` and an `end`, and ignored: the switch happens at `start`. |

### Preset animations

Ready-made entrances and exits, each stored as one node with a few settings.

#### `fade_in`

The layer fades in from invisible; it stays invisible until this starts.

It has no fields of its own.

#### `fade_out`

The layer fades out to invisible.

It has no fields of its own.

#### `slide_in`

The layer travels into its resting position from a distance, fading in.

| Field | Value | Default | Meaning |
|---|---|---|---|
| `direction` | `"left"`, `"right"`, `"up"` or `"down"` | `"up"` | The direction the layer travels in. |
| `distance` | number | `100` | How far it travels, in pixels, 0 or more. |
| `fade` | boolean | `true` | Whether the layer also fades. Without it the layer is visible the whole time. |

#### `slide_out`

The layer travels away from its resting position, fading out.

| Field | Value | Default | Meaning |
|---|---|---|---|
| `direction` | `"left"`, `"right"`, `"up"` or `"down"` | `"up"` | The direction the layer travels in. |
| `distance` | number | `100` | How far it travels, in pixels, 0 or more. |
| `fade` | boolean | `true` | Whether the layer also fades. Without it the layer is visible the whole time. |

#### `grow_in`

The layer appears while growing from a smaller size.

| Field | Value | Default | Meaning |
|---|---|---|---|
| `scale` | number | `0.5` | Size it starts at, as a multiplier of the resting size, 0 or more. |
| `fade` | boolean | `true` | Whether the layer also fades in. |

#### `grow_out`

The layer disappears while growing to a larger size.

| Field | Value | Default | Meaning |
|---|---|---|---|
| `scale` | number | `1.5` | Size it ends at, as a multiplier of the resting size, 0 or more. |
| `fade` | boolean | `true` | Whether the layer also fades out. |

#### `shrink_in`

The layer appears while shrinking from a larger size.

| Field | Value | Default | Meaning |
|---|---|---|---|
| `scale` | number | `1.5` | Size it starts at, as a multiplier of the resting size, 0 or more. |
| `fade` | boolean | `true` | Whether the layer also fades in. |

#### `shrink_out`

The layer disappears while shrinking to a smaller size.

| Field | Value | Default | Meaning |
|---|---|---|---|
| `scale` | number | `0.5` | Size it ends at, as a multiplier of the resting size, 0 or more. |
| `fade` | boolean | `true` | Whether the layer also fades out. |

#### `spin_in`

The layer turns into place while growing from nothing.

| Field | Value | Default | Meaning |
|---|---|---|---|
| `angle` | number | `180` | How far it turns, in degrees clockwise; negative turns the other way. |
| `fade` | boolean | `false` | Whether the layer also fades. |

#### `spin_out`

The layer turns away while shrinking to nothing.

| Field | Value | Default | Meaning |
|---|---|---|---|
| `angle` | number | `180` | How far it turns, in degrees clockwise; negative turns the other way. |
| `fade` | boolean | `false` | Whether the layer also fades. |

#### `blur_in`

The layer fades in while coming into focus.

| Field | Value | Default | Meaning |
|---|---|---|---|
| `blur` | number | `20` | Blur when out of focus, in pixels, 0 or more; added to the layer's own blur. |

#### `blur_out`

The layer fades out while going out of focus.

| Field | Value | Default | Meaning |
|---|---|---|---|
| `blur` | number | `20` | Blur when out of focus, in pixels, 0 or more; added to the layer's own blur. |

#### `reveal_in`

A straight edge wipes across the layer and uncovers it.

| Field | Value | Default | Meaning |
|---|---|---|---|
| `direction` | `"left"`, `"right"`, `"up"` or `"down"` | `"right"` | The direction the wipe edge travels in. With `right`, `reveal_in` uncovers the layer from its left side to its right. |

#### `reveal_out`

A straight edge wipes across the layer and covers it.

| Field | Value | Default | Meaning |
|---|---|---|---|
| `direction` | `"left"`, `"right"`, `"up"` or `"down"` | `"right"` | The direction the wipe edge travels in. With `right`, `reveal_in` uncovers the layer from its left side to its right. |

### Text animations

These work on the content of a text layer.

#### `text_in`

Text appears letter by letter, word by word or line by line, each with a fade, slide, scale or blur.

| Field | Value | Default | Meaning |
|---|---|---|---|
| `split` | `"letters"`, `"words"` or `"lines"` | `"letters"` | What a unit is. |
| `effect` | `"fade"`, `"slide_up"`, `"slide_down"`, `"scale"` or `"blur"` | `"fade"` | What each unit does. |
| `stagger` | number | `40` | Delay between one unit starting and the next, in milliseconds, 0 or more. When the delays do not fit between `start` and `end` they are shortened so the units play one after another. |
| `order` | `"forward"` or `"backward"` | `"forward"` | Which unit goes first. |
| `distance` | number | `100` | Travel of the `slide_up` and `slide_down` effects as a percentage of the line height, 0 or more. |
| `scale` | number | `0.5` | Size a unit starts at (`text_in`) or ends at (`text_out`) with the `scale` effect, as a multiplier, 0 or more. |
| `blur` | number | `20` | Blur of an out-of-focus unit with the `blur` effect, in pixels, 0 or more. |

#### `text_out`

Text disappears letter by letter, word by word or line by line.

| Field | Value | Default | Meaning |
|---|---|---|---|
| `split` | `"letters"`, `"words"` or `"lines"` | `"letters"` | What a unit is. |
| `effect` | `"fade"`, `"slide_up"`, `"slide_down"`, `"scale"` or `"blur"` | `"fade"` | What each unit does. |
| `stagger` | number | `40` | Delay between one unit starting and the next, in milliseconds, 0 or more. When the delays do not fit between `start` and `end` they are shortened so the units play one after another. |
| `order` | `"forward"` or `"backward"` | `"forward"` | Which unit goes first. |
| `distance` | number | `100` | Travel of the `slide_up` and `slide_down` effects as a percentage of the line height, 0 or more. |
| `scale` | number | `0.5` | Size a unit starts at (`text_in`) or ends at (`text_out`) with the `scale` effect, as a multiplier, 0 or more. |
| `blur` | number | `20` | Blur of an out-of-focus unit with the `blur` effect, in pixels, 0 or more. |

#### `counter`

Counts the first number in a text layer up or down, keeping the text around it.

| Field | Value | Default | Meaning |
|---|---|---|---|
| `from` | number | none | Number shown at the start. Default: the number at that moment. |
| `to` | number | none | Number shown at the end. Default: the number written in the text. |

### Sound

Sound on the timeline. It changes no layer, so it has no `target`. Video layers play their own sound too.

#### `audio`

Sound from a file that plays between start and end on the artboard's timeline; it draws nothing and has no target.

| Field | Value | Default | Meaning |
|---|---|---|---|
| `name` | string | `""` | Label shown on the timeline row. |
| `source` | string | required | Where the sound comes from: a path relative to the project file or an absolute path, of a sound file or a video file with sound. Sound cannot be embedded as a `data:` URI. |
| `start` | number | required | When the clip starts playing, in milliseconds from the start of the artboard, 0 or more. |
| `end` | number | required | When the clip stops playing, in milliseconds from the start of the artboard; after `start`. |
| `trim_start` | number | `0` | Where in the file the clip begins, in milliseconds from the start of the file, 0 or more: at `start` the clip is this far into its file. |
| `volume` | number | `1` | How loud the clip's sound plays, from 0 (silent) to 1 (as recorded). |

### Timeline structure

Folders for the timeline. They do not change playback.

#### `animation_group`

A folder that organises animations on the timeline; it does not change playback.

| Field | Value | Default | Meaning |
|---|---|---|---|
| `name` | string | `""` | Label shown on the timeline row. |

### Which layers an animation works on

| Animation | Works on |
|---|---|
| `move` | every layer except `artboard` |
| `rotate` | every layer except `artboard` |
| `scale` | every layer except `artboard` |
| `opacity` | every layer except `artboard` |
| `resize` | every layer except `artboard` |
| `fill` | `artboard`, `group`, `rect`, `ellipse`, `star`, `path`, `text`, `component` |
| `stroke` | `group`, `rect`, `ellipse`, `star`, `path`, `text`, `image`, `component`, `video` |
| `corner_radius` | `group`, `rect`, `image`, `component`, `video` |
| `blur` | every layer except `artboard` |
| `shadow` | every layer except `artboard` |
| `show` | every layer except `artboard` |
| `hide` | every layer except `artboard` |
| `fade_in` | every layer except `artboard` |
| `fade_out` | every layer except `artboard` |
| `slide_in` | every layer except `artboard` |
| `slide_out` | every layer except `artboard` |
| `grow_in` | every layer except `artboard` |
| `grow_out` | every layer except `artboard` |
| `shrink_in` | every layer except `artboard` |
| `shrink_out` | every layer except `artboard` |
| `spin_in` | every layer except `artboard` |
| `spin_out` | every layer except `artboard` |
| `blur_in` | every layer except `artboard` |
| `blur_out` | every layer except `artboard` |
| `reveal_in` | every layer except `artboard` |
| `reveal_out` | every layer except `artboard` |
| `text_in` | `text` |
| `text_out` | `text` |
| `counter` | `text` |

### Value types

#### `blend_mode`

How a layer's pixels combine with what is behind it.

| Value | Meaning |
|---|---|
| `"normal"` | Draw over what is behind. The default. |
| `"multiply"` | Multiply colours; always darker. |
| `"screen"` | Inverse multiply; always lighter. |
| `"overlay"` | Multiply dark areas, screen light ones. |
| `"darken"` | Keep the darker of the two colours. |
| `"lighten"` | Keep the lighter of the two colours. |
| `"color_dodge"` | Brighten what is behind to reflect the layer. |
| `"color_burn"` | Darken what is behind to reflect the layer. |
| `"hard_light"` | Like overlay, driven by the layer instead of the backdrop. |
| `"soft_light"` | A gentler hard light. |
| `"difference"` | Absolute difference of the two colours. |
| `"exclusion"` | Like difference with lower contrast. |
| `"hue"` | Hue of the layer, saturation and luminosity of the backdrop. |
| `"saturation"` | Saturation of the layer, hue and luminosity of the backdrop. |
| `"color"` | Hue and saturation of the layer, luminosity of the backdrop. |
| `"luminosity"` | Luminosity of the layer, hue and saturation of the backdrop. |

#### `shadow`

A drop shadow behind a layer.

| Field | Value | Default | Meaning |
|---|---|---|---|
| `color` | colour | `"#00000040"` | Shadow colour; its alpha is the shadow's strength. |
| `x` | number | `0` | Horizontal offset in pixels, positive to the right. |
| `y` | number | `4` | Vertical offset in pixels, positive downward. |
| `blur` | number | `12` | Blur in pixels, 0 or more: twice the standard deviation of the Gaussian. |

#### `paint`

What an area or an outline is painted with: one colour, or a gradient. In files a paint is either a colour string (`"#0D99FF"`) or a gradient object (`{ "type": "linear_gradient", "stops": [...] }`).

- colour: One colour.
- [gradient](#gradient): A linear or radial gradient.

#### `gradient`

A gradient. Its geometry is given in the unit square of the layer's box: `[0, 0]` is the top-left corner of the box and `[1, 1]` the bottom-right.

With `"type": "linear_gradient"`: Colours change along a line.

| Field | Value | Default | Meaning |
|---|---|---|---|
| `start` | `[number, number]` | `[0.5,0]` | Where offset 0 sits, as `[x, y]` in the unit square of the box. |
| `end` | `[number, number]` | `[0.5,1]` | Where offset 1 sits, as `[x, y]` in the unit square of the box. |
| `stops` | list of [gradient_stop](#gradient_stop) | required | Colour stops, at least two, with offsets that do not decrease. |

With `"type": "radial_gradient"`: Colours change outward from a centre.

| Field | Value | Default | Meaning |
|---|---|---|---|
| `center` | `[number, number]` | `[0.5,0.5]` | Where offset 0 sits, as `[x, y]` in the unit square of the box. |
| `radius` | `[number, number]` | `[0.5,0.5]` | Horizontal and vertical radius where offset 1 sits, as fractions of the box width and height. |
| `rotation` | number | `0` | Rotation of the ellipse about its centre in degrees, clockwise. |
| `stops` | list of [gradient_stop](#gradient_stop) | required | Colour stops, at least two, with offsets that do not decrease. |

#### `gradient_stop`

One colour at one position of a gradient.

| Field | Value | Default | Meaning |
|---|---|---|---|
| `offset` | number | required | Position along the gradient, 0 to 1. |
| `color` | colour | required | Colour at that position. |

#### `stroke`

The outline of a layer.

| Field | Value | Default | Meaning |
|---|---|---|---|
| `paint` | [paint](#paint) | `"#000000"` | Colour or gradient of the outline. |
| `width` | number | `1` | Thickness in pixels, 0 or more. |
| `position` | `"inside"`, `"center"` or `"outside"` | `"center"` | Whether the outline sits inside, on, or outside the edge. |
| `cap` | `"butt"`, `"round"` or `"square"` | `"butt"` | Shape of the ends of open paths. |
| `join` | `"miter"`, `"round"` or `"bevel"` | `"miter"` | Shape of corners. |

#### `stroke_position`

Where a stroke sits relative to the edge of the shape.

| Value | Meaning |
|---|---|
| `"inside"` | Entirely inside the shape. |
| `"center"` | Half inside, half outside. |
| `"outside"` | Entirely outside the shape. |

#### `stroke_cap`

The shape of the ends of an open stroked path.

| Value | Meaning |
|---|---|
| `"butt"` | Cut off flat at the end point. |
| `"round"` | A half circle beyond the end point. |
| `"square"` | A half square beyond the end point. |

#### `stroke_join`

The shape of a stroke's corners.

| Value | Meaning |
|---|---|
| `"miter"` | A sharp corner. |
| `"round"` | A rounded corner. |
| `"bevel"` | A corner cut off flat. |

#### `corner_radius`

Corner rounding: one radius for every corner, or one per corner. In files this is a number, or an array of four numbers in the order top-left, top-right, bottom-right, bottom-left. Radii are in pixels and are limited to half the shorter side when drawn.

- number: The same radius on all four corners.
- `[number, number, number, number]`: Top-left, top-right, bottom-right, bottom-left.

#### `path_data`

The outline of a `path` layer: one or more subpaths made of anchors. On a layer, coordinates are normalised to the layer's box: `[0, 0]` is the top-left corner of the box and `[1, 1]` the bottom-right, so resizing the layer stretches the path. In a scene the same type holds pixels in the layer's own coordinate space.

| Field | Value | Default | Meaning |
|---|---|---|---|
| `subpaths` | list of [subpath](#subpath) | `[]` | The pieces of the outline. |
| `fill_rule` | `"non_zero"` or `"even_odd"` | `"non_zero"` | Which enclosed areas count as inside when subpaths overlap. |

#### `subpath`

One continuous run of anchors.

| Field | Value | Default | Meaning |
|---|---|---|---|
| `anchors` | list of [anchor](#anchor) | required | The points the outline passes through, in drawing order. |
| `closed` | boolean | `false` | Whether the last anchor connects back to the first. |

#### `anchor`

A point on a path, with optional curve handles. The segment between two anchors is a cubic Bezier whose control points are the first anchor's `out` handle and the second anchor's `in` handle. A missing handle sits on its anchor, so two anchors without handles are joined by a straight line.

| Field | Value | Default | Meaning |
|---|---|---|---|
| `point` | `[number, number]` | required | Position of the anchor as `[x, y]`. |
| `in` | `[number, number]` | none | Position (not offset) of the handle that shapes the curve arriving at this anchor, as `[x, y]`. Default: none, the curve arrives straight. |
| `out` | `[number, number]` | none | Position (not offset) of the handle that shapes the curve leaving this anchor, as `[x, y]`. Default: none, the curve leaves straight. |

#### `fill_rule`

How overlapping subpaths decide what is inside.

| Value | Meaning |
|---|---|
| `"non_zero"` | A point is inside when the outline winds around it a non-zero number of times. |
| `"even_odd"` | A point is inside when a ray from it crosses the outline an odd number of times, so a shape drawn inside another cuts a hole. |

#### `font`

Which font draws a text layer. The renderer resolves the family name against installed and bundled fonts and falls back to a default face when it is missing.

| Field | Value | Default | Meaning |
|---|---|---|---|
| `family` | string | `"Inter"` | Family name, for example `"Inter"`. |
| `weight` | whole number | `400` | Weight from 1 to 1000; 400 is regular and 700 bold. |
| `italic` | boolean | `false` | Whether to use the italic face. |

#### `text_align`

Horizontal alignment of text lines inside the text box.

| Value | Meaning |
|---|---|
| `"left"` | Lines start at the left edge. |
| `"center"` | Lines are centred. |
| `"right"` | Lines end at the right edge. |

#### `vertical_align`

Vertical alignment of the text block inside the text box.

| Value | Meaning |
|---|---|
| `"top"` | The first line sits at the top. |
| `"middle"` | The block is centred. |
| `"bottom"` | The last line sits at the bottom. |

#### `auto_size`

How a text layer's box follows its content.

| Value | Meaning |
|---|---|
| `"width_and_height"` | The box hugs the text on both axes; lines break only at line breaks in the content. The stored `width` and `height` are then only a hint, and the box grows from the side named by the alignment. |
| `"height"` | The width is fixed and text wraps to it; the height follows the text. |
| `"fixed"` | Both sides are fixed. Text wraps to the width and may overflow the height. |

#### `text_case`

Letter case applied to the content before it is drawn.

| Value | Meaning |
|---|---|
| `"original"` | As typed. |
| `"upper"` | ALL CAPITALS. |
| `"lower"` | all lower case. |
| `"title"` | First Letter Of Each Word Capitalised. |

#### `image_fit`

How an image fills its box.

| Value | Meaning |
|---|---|
| `"cover"` | Scale to cover the whole box, cropping what does not fit. |
| `"contain"` | Scale to fit inside the box, leaving empty space. |
| `"stretch"` | Stretch to the box, ignoring the image's proportions. |

#### `easing`

The pacing of an animation.

With `"type": "linear"`: Constant speed.

With `"type": "smooth"`: Eases in and out evenly. The default.

| Field | Value | Default | Meaning |
|---|---|---|---|
| `intensity` | number | `50` | Shape of the curve, 0 to 100. |

With `"type": "natural"`: A quick start and a long, soft landing.

| Field | Value | Default | Meaning |
|---|---|---|---|
| `intensity` | number | `50` | Shape of the curve, 0 to 100. |

With `"type": "slow_down"`: Starts at full speed and brakes into the end. Good for things arriving.

| Field | Value | Default | Meaning |
|---|---|---|---|
| `intensity` | number | `50` | Shape of the curve, 0 to 100. |

With `"type": "accelerate"`: Starts slowly and ends at full speed. Good for things leaving.

| Field | Value | Default | Meaning |
|---|---|---|---|
| `intensity` | number | `50` | Shape of the curve, 0 to 100. |

With `"type": "overshoot"`: Goes past the end value, then settles back onto it.

| Field | Value | Default | Meaning |
|---|---|---|---|
| `intensity` | number | `50` | How far it goes past, 0 to 100. |

With `"type": "elastic"`: A damped spring: wobbles around the end value before resting on it.

| Field | Value | Default | Meaning |
|---|---|---|---|
| `intensity` | number | `50` | How wobbly the spring is, 0 to 100. |

With `"type": "bounce"`: A dropped ball: hits the end value and bounces back off it a few times.

| Field | Value | Default | Meaning |
|---|---|---|---|
| `intensity` | number | `50` | How bouncy the ball is, 0 to 100. |

With `"type": "bezier"`: A cubic Bezier curve from (0, 0) to (1, 1), as in CSS `cubic-bezier`.

| Field | Value | Default | Meaning |
|---|---|---|---|
| `x1` | number | required | Horizontal position of the first control point, 0 to 1. |
| `y1` | number | required | Vertical position of the first control point. May leave 0 to 1. |
| `x2` | number | required | Horizontal position of the second control point, 0 to 1. |
| `y2` | number | required | Vertical position of the second control point. May leave 0 to 1. |

#### `offset`

A distance on both axes, in pixels.

| Field | Value | Default | Meaning |
|---|---|---|---|
| `x` | number | `0` | Distance to the right, in pixels; negative moves left. |
| `y` | number | `0` | Distance downward, in pixels; negative moves up. |

#### `size_change`

A size, of which either side may be left alone.

| Field | Value | Default | Meaning |
|---|---|---|---|
| `width` | number | none | Width in pixels, 0 or more. Default: unchanged. |
| `height` | number | none | Height in pixels, 0 or more. Default: unchanged. |

#### `resize_anchor`

The point of a box that a `resize` keeps fixed.

| Value | Meaning |
|---|---|
| `"top_left"` | The top-left corner. |
| `"top"` | The middle of the top edge. |
| `"top_right"` | The top-right corner. |
| `"left"` | The middle of the left edge. |
| `"center"` | The centre of the box. |
| `"right"` | The middle of the right edge. |
| `"bottom_left"` | The bottom-left corner. |
| `"bottom"` | The middle of the bottom edge. |
| `"bottom_right"` | The bottom-right corner. |

#### `stroke_change`

The animatable parts of a stroke, of which either may be left alone.

| Field | Value | Default | Meaning |
|---|---|---|---|
| `paint` | [paint](#paint) | none | Colour or gradient of the stroke. Default: unchanged. |
| `width` | number | none | Width in pixels, 0 or more. Default: unchanged. |

#### `shadow_change`

The parts of a shadow, of which any may be left alone.

| Field | Value | Default | Meaning |
|---|---|---|---|
| `color` | colour | none | Shadow colour; its alpha is the strength. Default: unchanged. |
| `x` | number | none | Horizontal offset in pixels. Default: unchanged. |
| `y` | number | none | Vertical offset in pixels. Default: unchanged. |
| `blur` | number | none | Blur in pixels, 0 or more. Default: unchanged. |

#### `direction`

A direction of travel.

| Value | Meaning |
|---|---|
| `"left"` | Toward the left. |
| `"right"` | Toward the right. |
| `"up"` | Toward the top. |
| `"down"` | Toward the bottom. |

#### `text_split`

How a text animation cuts the text into units.

| Value | Meaning |
|---|---|
| `"letters"` | Every letter on its own (spaces are skipped). |
| `"words"` | Every word on its own. |
| `"lines"` | Every line on its own, after wrapping. |

#### `text_effect`

What each unit of a text animation does. Every effect includes a fade, so units are invisible before they come in and after they go out.

| Value | Meaning |
|---|---|
| `"fade"` | Fades only. |
| `"slide_up"` | Travels upward: `text_in` rises into place, `text_out` rises away. |
| `"slide_down"` | Travels downward: `text_in` drops into place, `text_out` drops away. |
| `"scale"` | Scales about the unit's centre. |
| `"blur"` | Comes into or goes out of focus. |

#### `text_order`

Which unit of a text animation goes first.

| Value | Meaning |
|---|---|
| `"forward"` | From the first unit to the last. |
| `"backward"` | From the last unit to the first. |

<!-- reference:end -->

## How animations combine

A frame is a pure function of the file and a time. Nothing depends on the frame before, so jumping to any time gives the same picture as playing up to it.

For one property of one layer, the animations that act on it are put in the order they start (for equal starts, in timeline row order) and combined by one of two rules.

### The additive rule: `move` and `rotate`

Every animation contributes a change that is added to the resting value. Two moves add up.

- `from` and `to` are offsets from where the layer rests, not positions. A missing `from` or `to` is no offset.
- After it ends, an animation keeps contributing its `to`.
- Before it starts, an animation contributes nothing. The exception is the first one on that property: its `from` holds from time 0, so a layer that will move in from the side waits there.

A layer at `x: 100` with a `move` from `{ "x": -300 }` (and no `to`) between 400 and 800:

| Time | 0 | 400 | 600 | 800 | later |
|---|---|---|---|---|---|
| x (linear easing) | -200 | -200 | -50 | 100 | 100 |

### The replacing rule: everything else

The value is the resting value until the first animation starts. From then on the most recently started animation decides.

- A missing `from` is the value at the moment the animation starts. If another animation was still running, it is cut off there and the new one carries on from that value, without a jump.
- A missing `to` is the resting value.
- If the first animation has a `from`, that value also holds before the animation starts. This is what keeps a layer invisible until its fade begins.

A layer at `opacity: 80` with an `opacity` animation from `0` (no `to`) between 200 and 400, and a second one to `20` (no `from`) between 300 and 500:

| Time | 0 | 250 | 300 | 400 | 500 | later |
|---|---|---|---|---|---|---|
| opacity (linear easing) | 0 | 20 | 40 | 30 | 20 | 20 |

What the values mean:

- `scale` values multiply the layer's resting `scale`. `1` is the resting size.
- `opacity`, sizes, colours, radii, blur and shadows are absolute values.
- A `stroke` or `shadow` animation may name only some parts (`{ "width": 4 }`); the parts it does not name are treated like a missing value.
- A layer without a stroke counts as having a stroke of width 0, and a layer without a shadow as having a fully transparent one, so both can be animated in.

An easing may carry a value past its target on the way (`overshoot`, `elastic`). Opacity, sizes, radii and blur are then limited to their valid range. Positions and rotation are not, and scale only stops at 0.

### Show and hide

`show` and `hide` switch a layer's visibility at one moment. A layer with a `show` is hidden until the first one happens; a layer whose first is a `hide` is visible until then. After that the most recent one decides. The layer's own `hidden` field is what applies when it has neither.

### Resize and its anchor

A `resize` changes the size of a layer's box. Its `anchor` names the point of the box that stays where it is: with `left`, the box grows and shrinks to the right. This also holds for a layer that is turned or scaled. Children of a group are not scaled by a resize of the group.

A text layer whose `auto_size` is `width_and_height` takes its size from its text, so a `resize` has no visible effect on it.

### How colours and gradients blend

- Two colours blend channel by channel, on premultiplied values, so fading from a transparent colour does not pass through grey.
- No fill at all blends as the other side's paint at zero alpha: a fill can fade in from nothing.
- A colour and a gradient blend as if the colour were that gradient with every stop in the one colour.
- Two gradients of the same kind blend their geometry (start and end, or centre, radius and rotation) and their stops. With the same number of stops, the stops pair up in order and each pair blends its offset and colour, so stops slide. With different numbers of stops there is no pairing: both gradients are read at every offset where either has a stop, and the colours blend there.
- A linear and a radial gradient have no in-between. The first shows for the first half of the way, the second for the rest.

### Presets

A preset such as `slide_in` is one node with a few settings. While a frame is evaluated it is expanded into what it does to the layer:

| Preset | Fade | Scale | Position | Rotation | Blur | Clip |
|---|---|---|---|---|---|---|
| `fade_in` / `fade_out` | 0 to 1 / 1 to 0 | | | | | |
| `slide_in` | 0 to 1 (if `fade`) | | from `distance` away, against `direction`, to rest | | | |
| `slide_out` | 1 to 0 (if `fade`) | | from rest to `distance` away, along `direction` | | | |
| `grow_in`, `shrink_in` | 0 to 1 (if `fade`) | `scale` to 1 | | | | |
| `grow_out`, `shrink_out` | 1 to 0 (if `fade`) | 1 to `scale` | | | | |
| `spin_in` | 0 to 1 (if `fade`) | 0 to 1 | | `-angle` to 0 | | |
| `spin_out` | 1 to 0 (if `fade`) | 1 to 0 | | 0 to `angle` | | |
| `blur_in` / `blur_out` | 0 to 1 / 1 to 0 | | | | `blur` to 0 / 0 to `blur` | |
| `reveal_in` / `reveal_out` | | | | | | a straight edge travels in `direction` and uncovers / covers the layer |

What a preset does is laid on top of the layer's own values, it does not replace them:

- its fade multiplies the layer's opacity, so a `fade_in` on a layer at 50% opacity ends at 50%;
- its scale multiplies the layer's scale;
- its position adds to the layer's position, in the parent's coordinates, like a `move`;
- its rotation adds to the layer's rotation;
- its blur adds to the layer's blur.

So a preset and a property animation on the same layer both take effect: a `slide_in` together with a `move` slides the layer in along the path of the move.

Among themselves, presets on one layer follow the replacing rule, separately for fade, scale, position, rotation, blur and clip. Two things make sequences work the way they look in the timeline:

- An exit (`*_out`) starts from wherever the layer is at that moment. If it interrupts an entrance, there is no jump.
- An entrance (`*_in`) starts from its own start value, which also holds before it begins: a layer with a `fade_in` is invisible until then. It also clears what an earlier exit left behind, so a layer that slid out and later fades back in comes back in its place.

A layer that a preset has made invisible (faded to 0, scaled to 0, or fully covered) is not drawn at all.

### Text animations

`text_in` and `text_out` work on a text layer. The text is cut into pieces by `split`: every letter (spaces are skipped), every word, or every line after wrapping. Each piece plays the `effect`, and pieces start one after another:

- with *n* pieces, the delay between two starts is `stagger`, or `(end - start) / n` if that is smaller;
- every piece takes the same time, and the last piece ends exactly at `end`;
- `order: "backward"` starts with the last piece;
- `easing` paces each piece.

| Effect | `text_in`: the piece starts | `text_out`: the piece ends |
|---|---|---|
| `fade` | invisible | invisible |
| `slide_up` | invisible, `distance` below its place | invisible, `distance` above its place |
| `slide_down` | invisible, `distance` above its place | invisible, `distance` below its place |
| `scale` | invisible, at `scale` times its size | invisible, at `scale` times its size |
| `blur` | invisible, blurred by `blur` | invisible, blurred by `blur` |

`distance` is a percentage of the line height. Scaling is about the centre of the piece. Like presets, a `text_out` starts from wherever each piece is, and a `text_in` starts from its own start value, which holds before it begins: a text with a `text_in` shows nothing until then.

### Counters

A `counter` animates the first number in a text layer's content. The number follows the replacing rule, with the number written in the text as the resting value: `from: 0` and no `to` counts from 0 up to what the text says.

The number keeps the way it is written, and the text around it stays:

| Content | Shown on the way |
|---|---|
| `$1,250.50 raised` | `$0.00 raised`, `$625.25 raised`, `$1,250.50 raised` |
| `10 000 users` | `0 users`, `5 000 users`, `10 000 users` |
| `€ 1.234,56` | `€ 0,00`, `€ 617,28`, `€ 1.234,56` |
| `007` | `000`, `004`, `007` |
| `+15%` | `0%`, `+8%`, `+15%` |

Recognised are thousands separators (`,`, a space, `'`, or `.` with a decimal comma), decimals with a point or a comma, leading zeros and a sign directly in front. The number of decimals stays as written. A text without a number is left alone.

### Easing

`easing` says how an animation is paced between `start` and `end`. Every easing maps progress from 0 to 1 onto a value that starts at 0 and ends at 1. The types are listed under [easing](#easing) in the reference. `intensity` runs from 0 to 100 and shapes the curve; 50 is the default.

```json
{ "type": "smooth", "intensity": 50 }
{ "type": "bezier", "x1": 0.2, "y1": 0, "x2": 0, "y2": 1 }
```

`bezier` takes the four numbers of a CSS `cubic-bezier`.

### The length of the timeline

An artboard's `duration` is the length of its timeline. Animations may end after it; export stops at the duration. A frame can be asked for at any time, also past the end.

## Components

A `component` is a main component: a box of layers that can be shown again elsewhere, like a group that is reused. It sits in an artboard or a group like any other layer and is drawn there exactly like a `group` with the same fields. An `instance` shows a copy of it:

```json
{ "id": "btn", "parent": "ab12", "index": "a0", "type": "component", "name": "Button",
  "x": 40, "y": 40, "width": 240, "height": 64, "fill": "#0D99FF", "corner_radius": 32 },
{ "id": "lbl", "parent": "btn", "index": "a0", "type": "text", "content": "Buy", "x": 90, "y": 12 },
{ "id": "in1", "parent": "ab12", "index": "a1", "type": "fade_in", "target": "lbl", "start": 0, "end": 400 },
{ "id": "cp1", "parent": "ab12", "index": "a1", "type": "instance", "component": "btn",
  "x": 40, "y": 140, "width": 240, "height": 64, "time_offset": 150 }
```

What an instance shows at a moment is what the component's own type draws (its fill, stroke, corner radius and clip) and the layers inside it, with every animation that targets a layer inside it, as the component looks `time_offset` milliseconds earlier. So editing the component, its layers or their animations changes every instance, and instances with growing offsets play the same motion one after another. Before its offset has passed, an instance shows the component as it is before its animations start; a negative offset plays them earlier.

- The animations of the layers inside a component live in the component's own artboard, like any animation; instances in other artboards play them too.
- The component's own placement and effects (position, size, rotation, scale, opacity, blend mode, blur, shadow) and the animations that target the component itself belong to where the component is placed. They are not part of what instances show.
- An instance is a layer of its own: its position, size, rotation, scale, opacity, blend mode, blur, shadow and the animations that target it are its own, and it is selected and moved as one layer.
- When an instance's box is not the size of the component's, what it shows is scaled to fill its box: `width` divided by the component's width across, `height` divided by its height down. A `resize` animation on the instance changes its box the way it changes a group's: the component's fill, stroke and clip follow the box, and the layers keep the scale of the instance's resting `width` and `height`. When the editor or an agent's `update` resizes a component, an instance that was its size takes the new size in the same step, each side on its own, so what it shows is not squeezed; an instance resized by hand keeps its size.
- Instances hold no layers, and there are no per-instance changes (overrides) yet.

The rules: an instance's `component` must name a `component` layer; a component cannot be inside another component; and no component may show itself, by holding an instance of itself or of a component that shows it. Instances of other components inside a component are fine. Removing a component that instances still show is refused; the editor and the agents' `remove` first *detach* those instances, replacing each with a `group` that looks the same, with copies of the component's layers and of their animations (except that a group from an instance resized out of proportion keeps stroke widths and font sizes, and one from an instance with a negative `time_offset` squeezes the animations under way at 0 ms into what is left of them; the agents' tools warn when that happens). A `group` and a `component` have the same fields, so an update can turn one into the other by setting `type`.

Version 2 of the format added `component` and `instance`, and version 3 added `video` and `audio`. An older file is read as it is and saved as version 3.

## Images

The `source` of an `image` layer is one of:

- a path relative to the folder the project file is in, written with `/`: `assets/photo.png`;
- an absolute path;
- a `data:` URI that carries the image itself: `data:image/png;base64,...`.

Keeping images in or below the project's folder and using relative paths makes the project portable. Web addresses are not loaded.

## Designs from other tools

A plugin that sends a design to Moshionly sends a list of layer nodes in the format of this document, with a few allowances:

- `id` can be any text that is unique in the list. Every node gets a new id on import.
- `index` may be left out. Siblings then keep the order of the list.
- `parent` names another node of the list, or is left out for a top-level layer.
- Only layers are accepted, no animations.
- A top-level `artboard` becomes an artboard, or, when the design is placed inside an existing artboard, a group that clips and keeps the artboard's size and fill.

Set `source_id` on every layer to something that identifies it in the tool it comes from, for example `figma:<file key>:<node id>`. When the same design is sent again, Moshionly pairs the old layers with the new ones by `source_id` and moves each layer's animations over to its new version. Layers that share a `source_id` (copies of one component, say) are paired in order: the first old one with the first new one, and so on. Animations of layers that no longer exist are removed, and so are animations of a type that does not work on what a layer has become.

A design may hold `component` and `instance` nodes; an instance's `component` then names a component of the design, which gets its new id. A component that was made in Moshionly from an imported group stays a component when the design is sent again and that group is still there, and its instances elsewhere follow it; if the group is gone, those instances are detached first.

## A worked example

This is `crates/moshionly-core/examples/demo.moshi`: a card that fades and slides in, a title that arrives letter by letter, an amount that counts up, and a dot that pops in. Each node is on one line, as Moshionly saves it.

<!-- example:start -->

```json
{
  "format": "moshionly",
  "version": 3,
  "name": "Launch day",
  "nodes": [
    {"id":"scene","parent":null,"index":"a0","type":"artboard","name":"Launch day","width":1080,"height":1080,"fill":"#F2F3F5"},
    {"id":"card","parent":"scene","index":"a0","type":"group","name":"Card","x":240,"y":340,"width":600,"height":400,"shadow":{"color":"#0000002E","y":24,"blur":64},"fill":"#FFFFFF","corner_radius":32,"clip":true},
    {"id":"accent","parent":"card","index":"a0","type":"rect","name":"Accent","width":600,"height":12,"fill":{"type":"linear_gradient","start":[0,0.5],"end":[1,0.5],"stops":[{"offset":0,"color":"#0D99FF"},{"offset":1,"color":"#7B61FF"}]}},
    {"id":"title","parent":"card","index":"a1","type":"text","name":"Title","x":48,"y":72,"fill":"#111111","content":"Launch day","font":{"weight":700},"font_size":64},
    {"id":"raised","parent":"card","index":"a2","type":"text","name":"Raised","x":48,"y":196,"fill":"#0D99FF","content":"$12,480 raised","font_size":40},
    {"id":"dot","parent":"card","index":"a3","type":"ellipse","name":"Dot","x":500,"y":300,"width":52,"height":52,"fill":"#0D99FF"},
    {"id":"cardfade","parent":"scene","index":"a0","type":"fade_in","name":"Card fades in","target":"card","start":0,"end":500,"easing":{"type":"slow_down"}},
    {"id":"cardslide","parent":"scene","index":"a1","type":"slide_in","name":"Card slides up","target":"card","start":0,"end":700,"easing":{"type":"slow_down"},"distance":80,"fade":false},
    {"id":"titlein","parent":"scene","index":"a2","type":"text_in","name":"Title, letter by letter","target":"title","start":500,"end":1500,"easing":{"type":"slow_down"},"effect":"slide_up","distance":60},
    {"id":"count","parent":"scene","index":"a3","type":"counter","name":"Count up","target":"raised","start":900,"end":2600,"easing":{"type":"slow_down","intensity":70},"from":0},
    {"id":"dotin","parent":"scene","index":"a4","type":"grow_in","name":"Dot pops","target":"dot","start":1400,"end":2000,"easing":{"type":"overshoot"},"scale":0}
  ]
}
```

<!-- example:end -->

The layers:

- `scene` is the artboard, 1080 by 1080. Its `duration` is not written because it is the default, 4000.
- `card` is a group used as a frame: it has a fill, rounded corners and a shadow, and it clips its children.
- `accent`, `title`, `raised` and `dot` are inside the card, so their `x` and `y` are measured from the card's top-left corner. The two texts have no `width` or `height` that matter: their `auto_size` is the default, `width_and_height`, so their boxes follow their text.

The animations, all in the timeline of `scene`:

- `cardfade` and `cardslide` both act on the card from time 0. The slide has `fade: false`, so the fade comes from `cardfade` alone and ends earlier than the slide. Both are presets, laid on top of the card's own values.
- `titlein` cuts the title into its 9 letters. With the default `stagger` of 40 the letters start 40 apart and each takes 680, so that the last one ends at 1500. Each rises from 60% of a line below its place.
- `count` runs the number in "$12,480 raised" from 0. It has no `to`, so it ends on the number that is written.
- `dotin` grows the dot from nothing with an overshoot.

What is in the frame at a few times (text measured with the built-in estimate):

| Time | The card | The title | The amount | The dot |
|---|---|---|---|---|
| 0 | not drawn: its fade is at 0 | | | |
| 250 | 91% opaque, 15 below its place | nothing yet | `$0 raised` | not drawn: scaled to 0 |
| 900 | in place, fully opaque | all 9 letters on their way, the first at 95% and 2 below its place, the last at 40% and 27 below | `$0 raised`, about to start | not drawn |
| 1700 | in place | complete, one unit again | `$11,798 raised` | at 111% of its size, on the overshoot |
| 4000 | in place | complete | `$12,480 raised` | at its size |

To see any frame of any file as JSON, run:

```text
cargo run -p moshionly-core --example inspect -- crates/moshionly-core/examples/demo.moshi 900
```
