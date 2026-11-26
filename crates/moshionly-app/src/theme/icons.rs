//! Icon glyphs.
//!
//! Icons come from the Phosphor icon font (regular weight, version 2.1, MIT
//! licensed; see `assets/icons/Phosphor-LICENSE.txt`). Each constant is the
//! one-character string for that icon in the font registered as
//! [`ICON_FAMILY`](super::fonts::ICON_FAMILY).
//!
//! Widgets take icons as `&str`, so swapping the icon set means replacing the
//! font file and this table and nothing else.

use egui::{Align2, Color32, Painter, Pos2};

/// Paints one icon glyph centred on `center`.
///
/// `size` is the icon's box in points (16 for most of the interface).
pub fn paint(painter: &Painter, center: Pos2, icon: &str, size: f32, color: Color32) {
    painter.text(
        center,
        Align2::CENTER_CENTER,
        icon,
        super::fonts::icon_font(size),
        color,
    );
}

/// The `cursor` icon.
pub const CURSOR: &str = "\u{e1dc}";
/// The `hand` icon.
pub const HAND: &str = "\u{e298}";
/// The `frame-corners` icon.
pub const FRAME_CORNERS: &str = "\u{e626}";
/// The `rectangle` icon.
pub const RECTANGLE: &str = "\u{e3f0}";
/// The `circle` icon.
pub const CIRCLE: &str = "\u{e18a}";
/// The `star` icon.
pub const STAR: &str = "\u{e46a}";
/// The `pen-nib` icon.
pub const PEN_NIB: &str = "\u{e3ac}";
/// The `text-t` icon.
pub const TEXT_T: &str = "\u{e48a}";
/// The `image` icon.
pub const IMAGE: &str = "\u{e2ca}";
/// The `sparkle` icon.
pub const SPARKLE: &str = "\u{e6a2}";
/// The `arrow-u-up-left` icon.
pub const ARROW_U_UP_LEFT: &str = "\u{e08a}";
/// The `arrow-u-up-right` icon.
pub const ARROW_U_UP_RIGHT: &str = "\u{e08c}";
/// The `play` icon.
pub const PLAY: &str = "\u{e3d0}";
/// The `pause` icon.
pub const PAUSE: &str = "\u{e39e}";
/// The `stop` icon.
pub const STOP: &str = "\u{e46c}";
/// The `repeat` icon.
pub const REPEAT: &str = "\u{e3f6}";
/// The `repeat-once` icon.
pub const REPEAT_ONCE: &str = "\u{e3f8}";
/// The `skip-back` icon.
pub const SKIP_BACK: &str = "\u{e5a4}";
/// The `skip-forward` icon.
pub const SKIP_FORWARD: &str = "\u{e5a6}";
/// The `export` icon.
pub const EXPORT: &str = "\u{eaf0}";
/// The `chat-circle` icon.
pub const CHAT_CIRCLE: &str = "\u{e168}";
/// The `chat-teardrop-text` icon.
pub const CHAT_TEARDROP_TEXT: &str = "\u{e178}";
/// The `eye` icon.
pub const EYE: &str = "\u{e220}";
/// The `eye-slash` icon.
pub const EYE_SLASH: &str = "\u{e224}";
/// The `eye-closed` icon.
pub const EYE_CLOSED: &str = "\u{e222}";
/// The `lock-simple` icon.
pub const LOCK_SIMPLE: &str = "\u{e308}";
/// The `lock-simple-open` icon.
pub const LOCK_SIMPLE_OPEN: &str = "\u{e30a}";
/// The `caret-down` icon.
pub const CARET_DOWN: &str = "\u{e136}";
/// The `caret-right` icon.
pub const CARET_RIGHT: &str = "\u{e13a}";
/// The `caret-up` icon.
pub const CARET_UP: &str = "\u{e13c}";
/// The `caret-left` icon.
pub const CARET_LEFT: &str = "\u{e138}";
/// The `caret-up-down` icon.
pub const CARET_UP_DOWN: &str = "\u{e140}";
/// The `plus` icon.
pub const PLUS: &str = "\u{e3d4}";
/// The `minus` icon.
pub const MINUS: &str = "\u{e32a}";
/// The `x` icon.
pub const X: &str = "\u{e4f6}";
/// The `check` icon.
pub const CHECK: &str = "\u{e182}";
/// The `magnifying-glass` icon.
pub const MAGNIFYING_GLASS: &str = "\u{e30c}";
/// The `magnifying-glass-plus` icon.
pub const MAGNIFYING_GLASS_PLUS: &str = "\u{e310}";
/// The `magnifying-glass-minus` icon.
pub const MAGNIFYING_GLASS_MINUS: &str = "\u{e30e}";
/// The `dots-three` icon.
pub const DOTS_THREE: &str = "\u{e1fe}";
/// The `dots-three-vertical` icon.
pub const DOTS_THREE_VERTICAL: &str = "\u{e208}";
/// The `dots-six-vertical` icon.
pub const DOTS_SIX_VERTICAL: &str = "\u{eae2}";
/// The `sliders-horizontal` icon.
pub const SLIDERS_HORIZONTAL: &str = "\u{e434}";
/// The `sidebar-simple` icon.
pub const SIDEBAR_SIMPLE: &str = "\u{ec24}";
/// The `folder-simple` icon.
pub const FOLDER_SIMPLE: &str = "\u{e25a}";
/// The `intersect` icon.
pub const INTERSECT: &str = "\u{e2d2}";
/// The `path` icon.
pub const PATH: &str = "\u{e39c}";
/// The `bezier-curve` icon.
pub const BEZIER_CURVE: &str = "\u{eb00}";
/// The `link-simple` icon.
pub const LINK_SIMPLE: &str = "\u{e2e6}";
/// The `link-simple-break` icon.
pub const LINK_SIMPLE_BREAK: &str = "\u{e2e8}";
/// The `trash` icon.
pub const TRASH: &str = "\u{e4a6}";
/// The `copy` icon.
pub const COPY: &str = "\u{e1ca}";
/// The `gear-six` icon.
pub const GEAR_SIX: &str = "\u{e272}";
/// The `sun` icon.
pub const SUN: &str = "\u{e472}";
/// The `moon` icon.
pub const MOON: &str = "\u{e330}";
/// The `circle-half` icon.
pub const CIRCLE_HALF: &str = "\u{e18c}";
/// The `command` icon.
pub const COMMAND: &str = "\u{e1c4}";
/// The `arrows-out-cardinal` icon.
pub const ARROWS_OUT_CARDINAL: &str = "\u{e0a4}";
/// The `arrows-out` icon.
pub const ARROWS_OUT: &str = "\u{e0a2}";
/// The `arrow-clockwise` icon.
pub const ARROW_CLOCKWISE: &str = "\u{e036}";
/// The `arrow-counter-clockwise` icon.
pub const ARROW_COUNTER_CLOCKWISE: &str = "\u{e038}";
/// The `drop-half` icon.
pub const DROP_HALF: &str = "\u{e566}";
/// The `resize` icon.
pub const RESIZE: &str = "\u{ed6e}";
/// The `palette` icon.
pub const PALETTE: &str = "\u{e6c8}";
/// The `paint-bucket` icon.
pub const PAINT_BUCKET: &str = "\u{e392}";
/// The `corners-out` icon.
pub const CORNERS_OUT: &str = "\u{e1d0}";
/// The `angle` icon.
pub const ANGLE: &str = "\u{e7bc}";
/// The `warning` icon.
pub const WARNING: &str = "\u{e4e0}";
/// The `info` icon.
pub const INFO: &str = "\u{e2ce}";
/// The `check-circle` icon.
pub const CHECK_CIRCLE: &str = "\u{e184}";
/// The `x-circle` icon.
pub const X_CIRCLE: &str = "\u{e4f8}";
/// The `paper-plane-tilt` icon.
pub const PAPER_PLANE_TILT: &str = "\u{e398}";
/// The `magnet` icon.
pub const MAGNET: &str = "\u{e680}";
/// The `stack` icon.
pub const STACK: &str = "\u{e466}";
/// The `stack-simple` icon.
pub const STACK_SIMPLE: &str = "\u{e468}";
/// The `timer` icon.
pub const TIMER: &str = "\u{e492}";
/// The `lightning` icon.
pub const LIGHTNING: &str = "\u{e2de}";
/// The `magic-wand` icon.
pub const MAGIC_WAND: &str = "\u{e6b6}";
/// The `arrow-right` icon.
pub const ARROW_RIGHT: &str = "\u{e06c}";
/// The `arrow-left` icon.
pub const ARROW_LEFT: &str = "\u{e058}";
/// The `arrow-up` icon.
pub const ARROW_UP: &str = "\u{e08e}";
/// The `arrow-down` icon.
pub const ARROW_DOWN: &str = "\u{e03e}";
/// The `floppy-disk` icon.
pub const FLOPPY_DISK: &str = "\u{e248}";
/// The `folder-open` icon.
pub const FOLDER_OPEN: &str = "\u{e256}";
/// The `file-plus` icon.
pub const FILE_PLUS: &str = "\u{e236}";
/// The `file` icon.
pub const FILE: &str = "\u{e230}";
/// The `download-simple` icon.
pub const DOWNLOAD_SIMPLE: &str = "\u{e20c}";
/// The `list` icon.
pub const LIST: &str = "\u{e2f0}";
/// The `rows` icon.
pub const ROWS: &str = "\u{e5a2}";
/// The `text-aa` icon.
pub const TEXT_AA: &str = "\u{e6ee}";
/// The `selection` icon.
pub const SELECTION: &str = "\u{e69a}";
/// The `square` icon.
pub const SQUARE: &str = "\u{e45e}";
/// The `squares-four` icon.
pub const SQUARES_FOUR: &str = "\u{e464}";
/// The `film-strip` icon.
pub const FILM_STRIP: &str = "\u{e792}";
/// The `video-camera` icon.
pub const VIDEO_CAMERA: &str = "\u{e4da}";
/// The `speaker-high` icon.
pub const SPEAKER_HIGH: &str = "\u{e44a}";
/// The `paperclip` icon.
pub const PAPERCLIP: &str = "\u{e39a}";
/// The `arrows-horizontal` icon.
pub const ARROWS_HORIZONTAL: &str = "\u{eb06}";
/// The `arrows-vertical` icon.
pub const ARROWS_VERTICAL: &str = "\u{eb04}";
/// The `wave-sine` icon.
pub const WAVE_SINE: &str = "\u{ea9a}";
/// The `gradient` icon.
pub const GRADIENT: &str = "\u{eb42}";
/// The `drop` icon.
pub const DROP: &str = "\u{e210}";
/// The `shapes` icon.
pub const SHAPES: &str = "\u{ec5e}";
/// The `robot` icon.
pub const ROBOT: &str = "\u{e762}";
/// The `keyboard` icon.
pub const KEYBOARD: &str = "\u{e2d8}";
/// The `question` icon.
pub const QUESTION: &str = "\u{e3e8}";
/// The `clock` icon.
pub const CLOCK: &str = "\u{e19a}";
/// The `circle-dashed` icon.
pub const CIRCLE_DASHED: &str = "\u{e602}";
/// The `rectangle-dashed` icon.
pub const RECTANGLE_DASHED: &str = "\u{e3f2}";
/// The `crop` icon.
pub const CROP: &str = "\u{e1d4}";
/// The `plus-circle` icon.
pub const PLUS_CIRCLE: &str = "\u{e3d6}";
/// The `eyedropper` icon.
pub const EYEDROPPER: &str = "\u{e568}";
/// The `text-align-left` icon.
pub const TEXT_ALIGN_LEFT: &str = "\u{e484}";
/// The `text-align-center` icon.
pub const TEXT_ALIGN_CENTER: &str = "\u{e480}";
/// The `text-align-right` icon.
pub const TEXT_ALIGN_RIGHT: &str = "\u{e486}";
/// The `arrows-in-line-horizontal` icon.
pub const ARROWS_IN_LINE_HORIZONTAL: &str = "\u{e530}";
/// The `arrows-in-simple` icon.
pub const ARROWS_IN_SIMPLE: &str = "\u{e09e}";
/// The `arrows-out-simple` icon.
pub const ARROWS_OUT_SIMPLE: &str = "\u{e0a6}";
/// The `circle-notch` icon.
pub const CIRCLE_NOTCH: &str = "\u{eb44}";
/// The `at` icon.
pub const AT: &str = "\u{e0ac}";
/// The `microphone` icon.
pub const MICROPHONE: &str = "\u{e326}";
/// The `polygon` icon.
pub const POLYGON: &str = "\u{e6d0}";
/// The `subtract` icon.
pub const SUBTRACT: &str = "\u{ebd6}";
/// The `unite` icon.
pub const UNITE: &str = "\u{e87e}";
/// The `exclude` icon.
pub const EXCLUDE: &str = "\u{e882}";
/// The `selection-all` icon.
pub const SELECTION_ALL: &str = "\u{e746}";
/// The `flip-horizontal` icon.
pub const FLIP_HORIZONTAL: &str = "\u{ed6a}";
/// The `flip-vertical` icon.
pub const FLIP_VERTICAL: &str = "\u{ed6c}";
/// The `article` icon.
pub const ARTICLE: &str = "\u{e0a8}";
/// The `layout` icon.
pub const LAYOUT: &str = "\u{e6d6}";
/// The `browser` icon.
pub const BROWSER: &str = "\u{e0f4}";
/// The `cube` icon.
pub const CUBE: &str = "\u{e1da}";
/// The `align-left` icon.
pub const ALIGN_LEFT: &str = "\u{e50e}";
/// The `align-center-horizontal` icon.
pub const ALIGN_CENTER_HORIZONTAL: &str = "\u{e50a}";
/// The `align-right` icon.
pub const ALIGN_RIGHT: &str = "\u{e510}";
/// The `align-top` icon.
pub const ALIGN_TOP: &str = "\u{e512}";
/// The `align-center-vertical` icon.
pub const ALIGN_CENTER_VERTICAL: &str = "\u{e50c}";
/// The `align-bottom` icon.
pub const ALIGN_BOTTOM: &str = "\u{e506}";
/// The `text-italic` icon.
pub const TEXT_ITALIC: &str = "\u{e5c0}";
/// The `text-b` icon.
pub const TEXT_B: &str = "\u{e5be}";
/// The `line-segment` icon.
pub const LINE_SEGMENT: &str = "\u{e6d2}";
/// The `chart-line` icon.
pub const CHART_LINE: &str = "\u{e154}";
/// The `arrow-line-left` icon.
pub const ARROW_LINE_LEFT: &str = "\u{e062}";
/// The `arrow-line-right` icon.
pub const ARROW_LINE_RIGHT: &str = "\u{e064}";
/// The `caret-double-right` icon.
pub const CARET_DOUBLE_RIGHT: &str = "\u{e12a}";
/// The `caret-double-left` icon.
pub const CARET_DOUBLE_LEFT: &str = "\u{e128}";
/// The `tree-structure` icon.
pub const TREE_STRUCTURE: &str = "\u{e67c}";
/// The `share-network` icon.
pub const SHARE_NETWORK: &str = "\u{e408}";
/// The `swap` icon.
pub const SWAP: &str = "\u{e83c}";
/// The `arrows-clockwise` icon.
pub const ARROWS_CLOCKWISE: &str = "\u{e094}";
/// The `rocket-launch` icon.
pub const ROCKET_LAUNCH: &str = "\u{e3fe}";
/// The `hash` icon.
pub const HASH: &str = "\u{e2a2}";
/// The `number-square-one` icon.
pub const NUMBER_SQUARE_ONE: &str = "\u{e36c}";
/// The `percent` icon.
pub const PERCENT: &str = "\u{e3b6}";
/// The `ruler` icon.
pub const RULER: &str = "\u{e6b8}";
/// The `grid-four` icon.
pub const GRID_FOUR: &str = "\u{e296}";
/// The `dots-nine` icon.
pub const DOTS_NINE: &str = "\u{e1fc}";
/// The `mask-happy` icon.
pub const MASK_HAPPY: &str = "\u{e9f4}";
/// The `scan` icon.
pub const SCAN: &str = "\u{ebb6}";
/// The `diamonds-four` icon: a main component.
pub const DIAMONDS_FOUR: &str = "\u{e8f4}";
/// The `diamond` icon: an instance of a component.
pub const DIAMOND: &str = "\u{e1ec}";
/// The `arrow-square-out` icon.
pub const ARROW_SQUARE_OUT: &str = "\u{e5de}";
/// The `sun-dim` icon: a glow.
pub const SUN_DIM: &str = "\u{e474}";
/// The `wind` icon: a directional blur.
pub const WIND: &str = "\u{e5d2}";
