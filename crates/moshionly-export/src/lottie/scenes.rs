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
//! measures nothing again.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use moshionly_core::scene::TextStyle;
use moshionly_core::text::{TextLayout, TextRequest};
use moshionly_core::{Document, NodeId, Scene, TextMeasurer, evaluate};

use super::clock::Clock;

/// How many scenes are kept. When there are more, the quarter looked at
/// longest ago is forgotten.
const KEPT: usize = 2048;

/// How many different texts keep their layouts. A counter shows a new
/// text for every value; past this many, texts are measured each time.
const LAYOUTS_KEPT: usize = 4096;

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

/// A text's layouts, each with the style and wrapping width it was laid
/// out with.
type LayoutsOfText = Vec<(TextStyle, Option<f64>, TextLayout)>;

/// Text layouts by text, style and wrapping width: each text is laid out
/// once, however many scenes show it.
struct Layouts<'a> {
    measurer: &'a dyn TextMeasurer,
    kept: RefCell<HashMap<String, LayoutsOfText>>,
}

impl<'a> Layouts<'a> {
    fn new(measurer: &'a dyn TextMeasurer) -> Self {
        Self {
            measurer,
            kept: RefCell::new(HashMap::new()),
        }
    }
}

impl TextMeasurer for Layouts<'_> {
    fn measure(&self, request: &TextRequest<'_>) -> TextLayout {
        if let Some(found) = self.kept.borrow().get(request.text).and_then(|layouts| {
            layouts
                .iter()
                .find(|(style, wrap, _)| style == request.style && *wrap == request.wrap_width)
        }) {
            return found.2.clone();
        }
        let layout = self.measurer.measure(request);
        let mut kept = self.kept.borrow_mut();
        if kept.len() < LAYOUTS_KEPT || kept.contains_key(request.text) {
            kept.entry(request.text.to_owned()).or_default().push((
                request.style.clone(),
                request.wrap_width,
                layout.clone(),
            ));
        }
        layout
    }
}

#[cfg(test)]
mod tests {
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
}
