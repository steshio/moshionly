//! Whole scenes of the artboard, evaluated at the moments the converter
//! asks about and kept for a while.
//!
//! Most values come straight from the document's properties. Some exist
//! only in an evaluated scene: where a layer is on the artboard after its
//! groups have moved it, which part of it a reveal leaves showing, and how
//! its text is cut into pieces. Those are read from scenes, evaluated with
//! the renderer's fonts so text is measured as it is drawn.
//!
//! The converter looks at the same moments many times, so scenes are kept,
//! and the scenes it has not looked at for longest are forgotten first.
//! Text is laid out once for each text and style: a scene evaluated again
//! measures nothing again. Layouts are kept the same way, up to a limit,
//! so a text whose size, spacing or box animates, and so is laid out anew
//! at most moments, cannot fill memory.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::mem::size_of;
use std::rc::Rc;

use moshionly_core::model::Font;
use moshionly_core::scene::TextStyle;
use moshionly_core::text::{TextCluster, TextLayout, TextLine, TextRequest};
use moshionly_core::{Document, NodeId, Scene, TextMeasurer, evaluate};

use super::clock::Clock;

/// How many scenes are kept. When there are more, the quarter looked at
/// longest ago is forgotten.
const KEPT: usize = 2048;

/// How much the kept text layouts may take, in bytes. Past it, the
/// layouts looked at longest ago are forgotten first.
const LAYOUTS_BUDGET: usize = 64 << 20;

/// Scenes of one artboard by frame.
pub(crate) struct Scenes<'a> {
    document: &'a Document,
    artboard: &'a NodeId,
    layouts: Layouts<'a>,
    clock: Clock,
    kept: Kept<u64, Option<Rc<Scene>>>,
}

impl<'a> Scenes<'a> {
    /// Scenes of `artboard`, with text laid out by `measurer`.
    pub fn new(
        document: &'a Document,
        artboard: &'a NodeId,
        measurer: &'a dyn TextMeasurer,
        clock: Clock,
    ) -> Self {
        Self {
            document,
            artboard,
            layouts: Layouts::new(measurer),
            clock,
            kept: Kept::new(),
        }
    }

    /// The artboard at frame `frame`. `None` only when it cannot be
    /// evaluated at all, which a checked plan rules out.
    pub fn at(&self, frame: f64) -> Option<Rc<Scene>> {
        let key = frame.to_bits();
        if let Some(scene) = self.kept.get(&key) {
            return scene;
        }
        let scene = self.evaluate(frame).map(Rc::new);
        self.kept.insert(key, scene.clone(), 1, KEPT);
        scene
    }

    /// The artboard at frame `frame`, evaluated now and not kept: for
    /// callers that keep what they need of it themselves.
    pub fn evaluate(&self, frame: f64) -> Option<Scene> {
        evaluate(
            self.document,
            self.artboard,
            self.clock.ms(frame),
            &self.layouts,
        )
        .ok()
    }
}

/// Values by key, of which the ones looked at longest ago are forgotten
/// first once they add up to more than a limit.
pub(crate) struct Kept<K, V> {
    entries: RefCell<HashMap<K, (V, usize, u64)>>,
    /// The sizes of the entries added up, in whatever unit they are given.
    size: Cell<usize>,
    /// Counts lookups, to tell which entries were looked at last.
    clock: Cell<u64>,
}

impl<K: std::hash::Hash + Eq, V: Clone> Kept<K, V> {
    /// Nothing kept yet.
    pub fn new() -> Self {
        Self {
            entries: RefCell::new(HashMap::new()),
            size: Cell::new(0),
            clock: Cell::new(0),
        }
    }

    fn tick(&self) -> u64 {
        let now = self.clock.get() + 1;
        self.clock.set(now);
        now
    }

    /// The value kept for `key`, if any.
    pub fn get(&self, key: &K) -> Option<V> {
        let now = self.tick();
        let mut entries = self.entries.borrow_mut();
        let (value, _, used) = entries.get_mut(key)?;
        *used = now;
        Some(value.clone())
    }

    /// Keeps `value` for `key`, counting `size` toward `limit`. When the
    /// sizes pass the limit, the entries looked at longest ago are
    /// forgotten until a quarter of the limit is free again.
    pub fn insert(&self, key: K, value: V, size: usize, limit: usize) {
        let now = self.tick();
        let mut entries = self.entries.borrow_mut();
        if let Some((_, old, _)) = entries.insert(key, (value, size, now)) {
            self.size.set(self.size.get() - old);
        }
        self.size.set(self.size.get() + size);
        if self.size.get() <= limit {
            return;
        }
        let mut ages: Vec<(u64, usize)> = entries
            .values()
            .map(|(_, size, used)| (*used, *size))
            .collect();
        ages.sort_unstable();
        let target = limit - limit / 4;
        let mut total = self.size.get();
        let mut threshold = 0;
        for (used, size) in ages {
            if total <= target {
                break;
            }
            total -= size;
            threshold = used;
        }
        entries.retain(|_, (_, _, used)| *used > threshold);
        self.size.set(total);
    }
}

/// Everything a text is laid out from: the text, its style and its
/// wrapping width, numbers to the bit.
#[derive(Debug, PartialEq, Eq, Hash)]
struct LayoutKey {
    text: String,
    family: String,
    weight: u16,
    italic: bool,
    /// Font size, line height and letter spacing.
    sizes: [u64; 3],
    align: u8,
    wrap: Option<u64>,
}

impl LayoutKey {
    fn of(request: &TextRequest<'_>) -> Self {
        // Every field by name, so that one added later is not missed.
        let TextStyle {
            font:
                Font {
                    family,
                    weight,
                    italic,
                },
            font_size,
            line_height,
            letter_spacing,
            align,
        } = request.style;
        Self {
            text: request.text.to_owned(),
            family: family.clone(),
            weight: *weight,
            italic: *italic,
            sizes: [
                font_size.to_bits(),
                line_height.to_bits(),
                letter_spacing.to_bits(),
            ],
            align: *align as u8,
            wrap: request.wrap_width.map(f64::to_bits),
        }
    }

    /// Roughly what keeping `layout` for this key takes, in bytes.
    fn size_with(&self, layout: &TextLayout) -> usize {
        size_of::<(Self, TextLayout, usize, u64)>()
            + self.text.capacity()
            + self.family.capacity()
            + layout.lines.capacity() * size_of::<TextLine>()
            + layout.clusters.capacity() * size_of::<TextCluster>()
    }
}

/// Text layouts by text, style and wrapping width: each text is laid out
/// once, however many scenes show it, as long as there is room.
struct Layouts<'a> {
    measurer: &'a dyn TextMeasurer,
    kept: Kept<LayoutKey, TextLayout>,
    /// How much `kept` may take, in bytes.
    budget: usize,
}

impl<'a> Layouts<'a> {
    fn new(measurer: &'a dyn TextMeasurer) -> Self {
        Self {
            measurer,
            kept: Kept::new(),
            budget: LAYOUTS_BUDGET,
        }
    }
}

impl TextMeasurer for Layouts<'_> {
    fn measure(&self, request: &TextRequest<'_>) -> TextLayout {
        let key = LayoutKey::of(request);
        if let Some(layout) = self.kept.get(&key) {
            return layout;
        }
        let layout = self.measurer.measure(request);
        let size = key.size_with(&layout);
        self.kept.insert(key, layout.clone(), size, self.budget);
        layout
    }
}

#[cfg(test)]
mod tests {
    use moshionly_core::MonospaceMeasurer;
    use moshionly_core::model::TextAlign;

    use super::*;

    #[test]
    fn the_entries_looked_at_last_are_kept() {
        let kept: Kept<u32, &str> = Kept::new();
        for key in 0..8 {
            kept.insert(key, "scene", 1, 8);
        }
        // Looking at the first two keeps them when room is made.
        assert_eq!(kept.get(&0), Some("scene"));
        assert_eq!(kept.get(&1), Some("scene"));
        kept.insert(8, "scene", 1, 8);
        assert_eq!(kept.get(&0), Some("scene"));
        assert_eq!(kept.get(&1), Some("scene"));
        assert_eq!(kept.get(&8), Some("scene"));
        // Only the ones looked at longest ago went, not everything.
        assert_eq!(kept.get(&2), None);
        assert_eq!(kept.get(&7), Some("scene"));
        assert!(kept.size.get() <= 6);
    }

    #[test]
    fn sizes_count_toward_the_limit() {
        let kept: Kept<u32, u32> = Kept::new();
        kept.insert(1, 1, 60, 100);
        kept.insert(2, 2, 30, 100);
        kept.insert(2, 2, 30, 100);
        assert_eq!(kept.size.get(), 90);
        kept.insert(3, 3, 30, 100);
        assert_eq!(kept.get(&1), None, "the oldest and largest went");
        assert_eq!((kept.get(&2), kept.get(&3)), (Some(2), Some(3)));
    }

    /// A measurer that counts what it lays out.
    #[derive(Default)]
    struct Counting {
        measured: Cell<usize>,
    }

    impl TextMeasurer for Counting {
        fn measure(&self, request: &TextRequest<'_>) -> TextLayout {
            self.measured.set(self.measured.get() + 1);
            MonospaceMeasurer::default().measure(request)
        }
    }

    fn style(font_size: f64) -> TextStyle {
        TextStyle {
            font: Font::default(),
            font_size,
            line_height: font_size * 1.2,
            letter_spacing: 0.0,
            align: TextAlign::Left,
        }
    }

    #[test]
    fn each_text_and_style_is_laid_out_once() {
        let measurer = Counting::default();
        let layouts = Layouts::new(&measurer);
        let measure = |text: &str, style: &TextStyle, wrap_width: Option<f64>| {
            layouts.measure(&TextRequest {
                text,
                style,
                wrap_width,
            })
        };
        let plain = style(20.0);
        let first = measure("Launch", &plain, None);
        assert_eq!(measure("Launch", &plain, None), first);
        assert_eq!(measurer.measured.get(), 1);
        // Any difference is laid out on its own.
        let bold = TextStyle {
            font: Font {
                weight: 700,
                ..Font::default()
            },
            ..plain.clone()
        };
        let spaced = TextStyle {
            letter_spacing: 2.0,
            ..plain.clone()
        };
        let centred = TextStyle {
            align: TextAlign::Center,
            ..plain.clone()
        };
        for (text, style, wrap) in [
            ("Launch", &style(21.0), None),
            ("Launch", &bold, None),
            ("Launch", &spaced, None),
            ("Launch", &centred, None),
            ("Launch", &plain, Some(40.0)),
            ("Launched", &plain, None),
        ] {
            measure(text, style, wrap);
        }
        assert_eq!(measurer.measured.get(), 7);
        assert_eq!(measure("Launch", &plain, None), first);
        assert_eq!(measurer.measured.get(), 7);
    }

    #[test]
    fn layouts_of_a_style_that_animates_stay_within_their_budget() {
        // A paragraph whose size changes at every moment looked at: each
        // moment lays it out anew.
        let measurer = Counting::default();
        let mut layouts = Layouts::new(&measurer);
        layouts.budget = 1 << 20;
        let text = "Every letter of this paragraph rises into place. ".repeat(5);
        let measure = |layouts: &Layouts<'_>, size: f64| {
            layouts.measure(&TextRequest {
                text: &text,
                style: &style(size),
                wrap_width: Some(300.0),
            })
        };
        let sizes: Vec<f64> = (0..20_000)
            .map(|step| 12.0 + f64::from(step) / 1000.0)
            .collect();
        for size in &sizes {
            measure(&layouts, *size);
            assert!(layouts.kept.size.get() <= layouts.budget);
        }
        assert_eq!(measurer.measured.get(), sizes.len());
        // The latest are kept, the first forgotten.
        measure(&layouts, sizes[sizes.len() - 1]);
        assert_eq!(measurer.measured.get(), sizes.len());
        measure(&layouts, sizes[0]);
        assert_eq!(measurer.measured.get(), sizes.len() + 1);
    }
}
