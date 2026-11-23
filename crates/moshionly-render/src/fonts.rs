//! The fonts text is drawn with.
//!
//! A [`FontLibrary`] holds the fonts bundled with Moshionly (Inter), the
//! fonts the app registers from files or bytes, and, unless it was made
//! with [`FontLibrary::bundled`], the fonts installed on the machine.
//!
//! A text layer names a family, a weight and whether it is italic. The
//! library looks for the family among registered fonts first and installed
//! fonts second, and picks the nearest weight the family has. A family
//! that cannot be found is drawn in Inter, the default family, and the
//! frame reports it. Letters the chosen font does not have (emoji, other
//! scripts) are looked for in Inter and then, when installed fonts are in
//! use, in whatever installed font has them.
//!
//! The library is cheap to clone and every clone shares the same fonts, so
//! a font registered through one clone is seen by the renderer holding
//! another. It is also the [`TextMeasurer`](moshionly_core::TextMeasurer)
//! that evaluation needs, so text is split exactly where it is drawn.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use skia_safe as sk;
use skia_safe::textlayout::{FontCollection, TypefaceFontProvider};
use thiserror::Error;

use crate::text::{self, LayoutCache};

/// The family text falls back to: Inter, which is bundled.
pub const DEFAULT_FAMILY: &str = "Inter";

/// The font files bundled into the crate: Inter in three weights.
const BUNDLED: [&[u8]; 3] = [
    include_bytes!("../assets/fonts/Inter-Regular.ttf"),
    include_bytes!("../assets/fonts/Inter-Medium.ttf"),
    include_bytes!("../assets/fonts/Inter-SemiBold.ttf"),
];

/// How many faces one font collection file (`.ttc`) may add.
const MAX_FACES_PER_FILE: u32 = 64;

/// Why a font could not be added.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum FontError {
    /// The file could not be read.
    #[error("the font file \"{path}\" could not be read: {reason}")]
    Unreadable {
        /// The file.
        path: PathBuf,
        /// What the system said.
        reason: String,
    },
    /// The bytes are not a font Skia can use.
    #[error(
        "the data is not a font that can be used; TrueType (.ttf), OpenType (.otf) and font \
         collections (.ttc) work"
    )]
    NotAFont,
}

/// One face of a font: a family in one weight and slant.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FontFace {
    /// The family name, as it goes in a text layer's `font.family`.
    pub family: String,
    /// Weight from 1 to 1000; 400 is regular and 700 bold.
    pub weight: u16,
    /// Whether the face is italic or oblique.
    pub italic: bool,
}

/// A family and the faces it comes in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontFamily {
    /// The family name, as it goes in a text layer's `font.family`.
    pub name: String,
    /// The weights the family comes in, lightest first.
    pub weights: Vec<u16>,
    /// Whether the family has an italic face.
    pub italic: bool,
    /// Whether the family comes with Moshionly or was registered by the
    /// app, rather than installed on the machine.
    pub registered: bool,
}

/// The fonts text is drawn with. See the [module docs](self).
#[derive(Clone)]
pub struct FontLibrary {
    shared: Arc<Mutex<FontState>>,
}

impl std::fmt::Debug for FontLibrary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state = self.state();
        f.debug_struct("FontLibrary")
            .field("registered", &state.faces)
            .field("system_fonts", &state.system.is_some())
            .finish()
    }
}

impl Default for FontLibrary {
    /// Bundled fonts and the fonts installed on the machine.
    fn default() -> Self {
        Self::with_system_fonts()
    }
}

impl FontLibrary {
    /// The bundled fonts and the fonts installed on the machine. This is
    /// what the app draws with.
    pub fn with_system_fonts() -> Self {
        Self::build(Some(sk::FontMgr::new()))
    }

    /// The bundled fonts only, plus whatever is added later. Text looks the
    /// same on every machine, which is what tests and reproducible renders
    /// need; emoji and scripts Inter does not cover are drawn as boxes.
    pub fn bundled() -> Self {
        Self::build(None)
    }

    fn build(system: Option<sk::FontMgr>) -> Self {
        let mut state = FontState {
            // FreeType reads font files the same way on every platform;
            // where Skia has no FreeType of its own (macOS), the system's
            // font manager reads them.
            reader: sk::FontMgr::custom_empty()
                .or_else(|| system.clone())
                .unwrap_or_default(),
            provider: TypefaceFontProvider::new(),
            system,
            faces: Vec::new(),
            known_families: HashMap::new(),
            generation: 0,
            collection: FontCollection::new(),
            layouts: LayoutCache::default(),
        };
        for bytes in BUNDLED {
            let added = state.add(sk::Data::new_copy(bytes));
            debug_assert!(
                added.as_ref().is_ok_and(|faces| !faces.is_empty()),
                "the bundled fonts load"
            );
        }
        state.collection = text::collection(&state);
        Self {
            shared: Arc::new(Mutex::new(state)),
        }
    }

    /// Registers the faces in a font file's bytes and returns them. A font
    /// collection (`.ttc`) adds every face it holds.
    ///
    /// A registered family takes precedence over an installed family of the
    /// same name.
    pub fn add_font_bytes(&self, bytes: impl Into<Vec<u8>>) -> Result<Vec<FontFace>, FontError> {
        let bytes = bytes.into();
        self.state().add(sk::Data::new_copy(&bytes))
    }

    /// Registers the faces in a font file and returns them.
    pub fn add_font_file(&self, path: impl AsRef<Path>) -> Result<Vec<FontFace>, FontError> {
        let path = path.as_ref();
        let bytes = std::fs::read(path).map_err(|error| FontError::Unreadable {
            path: path.to_owned(),
            reason: error.to_string(),
        })?;
        self.add_font_bytes(bytes)
    }

    /// Whether text in `family` is drawn in that family rather than in the
    /// default one. Names are compared ignoring case, as fonts are matched.
    pub fn has_family(&self, family: &str) -> bool {
        self.state().has_family(family)
    }

    /// Every family that can be used: registered ones first, then
    /// installed ones, each group in alphabetical order.
    pub fn families(&self) -> Vec<FontFamily> {
        self.state().families()
    }

    /// Whether the fonts installed on the machine are used.
    pub fn uses_system_fonts(&self) -> bool {
        self.state().system.is_some()
    }

    /// The fonts, locked for this thread. A panic while the lock was held
    /// left nothing half-changed that matters, so a poisoned lock is used
    /// as it is.
    pub(crate) fn state(&self) -> MutexGuard<'_, FontState> {
        self.shared.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// What a [`FontLibrary`] shares between its clones.
pub(crate) struct FontState {
    /// Turns font bytes into typefaces.
    reader: sk::FontMgr,
    /// Bundled and registered typefaces, by family.
    pub(crate) provider: TypefaceFontProvider,
    /// The fonts installed on the machine, when they are used.
    pub(crate) system: Option<sk::FontMgr>,
    /// Bundled and registered faces, in the order they were added.
    faces: Vec<FontFace>,
    /// Answers of [`FontState::has_family`], by lower-case name. Asking
    /// the system is slow.
    known_families: HashMap<String, bool>,
    /// Goes up whenever fonts are added, so that anything laid out with
    /// the old fonts can tell it is stale.
    pub(crate) generation: u64,
    /// Finds fonts for text, falling back letter by letter.
    pub(crate) collection: FontCollection,
    /// Texts laid out recently.
    pub(crate) layouts: LayoutCache,
}

// SAFETY: `FontState` holds Skia objects that skia-safe does not mark
// `Send` (font managers, the typeface provider and the font collection,
// which paragraphs built from it point back to) because they are
// reference counted and could be shared with code on another thread.
// Here they cannot be: every one of them is created by `FontLibrary::build`
// and is reachable only through `FontState`, which is reachable only
// through the library's `Mutex`, so they are never used from two threads
// at once. Skia's reference counts are atomic, so moving the owner between
// threads is sound. Typefaces taken out of them are `Send + Sync` in
// skia-safe and safe to share.
unsafe impl Send for FontState {}

impl FontState {
    fn add(&mut self, data: sk::Data) -> Result<Vec<FontFace>, FontError> {
        let mut added = Vec::new();
        for index in 0..MAX_FACES_PER_FILE {
            let Some(typeface) = self.reader.new_from_data(data.clone(), index) else {
                break;
            };
            let face = describe(&typeface);
            self.provider.register_typeface(typeface, None);
            self.faces.push(face.clone());
            added.push(face);
        }
        if added.is_empty() {
            return Err(FontError::NotAFont);
        }
        self.known_families.clear();
        self.collection.clear_caches();
        self.generation += 1;
        Ok(added)
    }

    pub(crate) fn has_family(&mut self, family: &str) -> bool {
        let key = family.to_lowercase();
        if let Some(known) = self.known_families.get(&key) {
            return *known;
        }
        let found = self
            .faces
            .iter()
            .any(|face| face.family.to_lowercase() == key)
            || self
                .system
                .as_ref()
                .is_some_and(|system| system.match_family(family).count() > 0);
        self.known_families.insert(key, found);
        found
    }

    fn families(&self) -> Vec<FontFamily> {
        let mut registered: BTreeMap<String, FontFamily> = BTreeMap::new();
        for face in &self.faces {
            let family = registered
                .entry(face.family.clone())
                .or_insert_with(|| FontFamily {
                    name: face.family.clone(),
                    weights: Vec::new(),
                    italic: false,
                    registered: true,
                });
            family.weights.push(face.weight);
            family.italic |= face.italic;
        }
        let mut installed: BTreeMap<String, FontFamily> = BTreeMap::new();
        if let Some(system) = &self.system {
            for name in system.family_names() {
                if name.is_empty() || name.starts_with('.') || registered.contains_key(&name) {
                    // Names starting with a dot are the system's own
                    // interface fonts, which cannot be asked for by name.
                    continue;
                }
                let mut styles = system.match_family(&name);
                let mut family = FontFamily {
                    name: name.clone(),
                    weights: Vec::new(),
                    italic: false,
                    registered: false,
                };
                for index in 0..styles.count() {
                    let (style, _) = styles.style(index);
                    family.weights.push(weight_of(style));
                    family.italic |= style.slant() != sk::font_style::Slant::Upright;
                }
                installed.insert(name, family);
            }
        }
        registered
            .into_values()
            .chain(installed.into_values())
            .map(|mut family| {
                family.weights.sort_unstable();
                family.weights.dedup();
                family
            })
            .collect()
    }
}

/// The face a typeface is.
fn describe(typeface: &sk::Typeface) -> FontFace {
    let style = typeface.font_style();
    FontFace {
        family: typeface.family_name(),
        weight: weight_of(style),
        italic: style.slant() != sk::font_style::Slant::Upright,
    }
}

fn weight_of(style: sk::FontStyle) -> u16 {
    (*style.weight()).clamp(1, 1000) as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inter_is_bundled_in_three_weights() {
        let library = FontLibrary::bundled();
        assert!(!library.uses_system_fonts());
        assert_eq!(
            library.families(),
            [FontFamily {
                name: "Inter".into(),
                weights: vec![400, 500, 600],
                italic: false,
                registered: true,
            }]
        );
        assert!(library.has_family("Inter"));
        assert!(library.has_family("inter"), "names ignore case");
        assert!(!library.has_family("Helvetica"), "no installed fonts");
        assert!(!library.has_family(""));
    }

    #[test]
    fn fonts_can_be_added_from_bytes_and_files() {
        let library = FontLibrary::bundled();
        let shared = library.clone();
        assert_eq!(
            library.add_font_bytes(b"not a font".to_vec()),
            Err(FontError::NotAFont)
        );
        let missing = library.add_font_file("/no/such/font.ttf");
        assert!(
            matches!(missing, Err(FontError::Unreadable { .. })),
            "{missing:?}"
        );

        let before = library.state().generation;
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/fonts/Inter-Medium.ttf");
        let added = library.add_font_file(path).unwrap();
        assert_eq!(
            added,
            [FontFace {
                family: "Inter".into(),
                weight: 500,
                italic: false
            }]
        );
        // Every clone sees it, and layouts made before can tell.
        assert!(shared.state().generation > before);
        assert_eq!(shared.families()[0].weights, [400, 500, 600]);
    }

    #[test]
    fn installed_fonts_are_listed_when_used() {
        let library = FontLibrary::with_system_fonts();
        assert!(library.uses_system_fonts());
        let families = library.families();
        assert_eq!(families[0].name, "Inter", "registered families come first");
        // What else is installed depends on the machine; a machine with no
        // fonts at all still has Inter.
        assert!(families[1..].iter().all(|family| !family.registered));
        if let Some(installed) = families.get(1) {
            assert!(library.has_family(&installed.name), "{installed:?}");
        } else {
            println!("skipped: no fonts are installed on this machine");
        }
    }

    #[test]
    fn the_library_can_go_to_another_thread() {
        fn sendable<T: Send + Sync>() {}
        sendable::<FontLibrary>();
        let library = FontLibrary::bundled();
        let found = std::thread::spawn(move || library.has_family("Inter"))
            .join()
            .unwrap();
        assert!(found);
    }
}
