//! Commands: everything the user can ask the editor to do.
//!
//! A command has a name, a category, an optional icon and an optional keyboard
//! shortcut ([`CommandId`]). Menus, buttons, shortcuts and the command palette
//! all refer to commands by id, so each one is described in exactly one place.
//! [`CommandRegistry`] holds what changes while the app runs: which commands
//! are currently enabled. It also turns key presses into commands, and
//! [`Palette`] lets the user find any command by typing.
//!
//! The registry only *names* what should happen. Carrying a command out is the
//! job of whoever owns the state it touches.

mod palette;
mod shortcut;

use std::collections::{HashMap, HashSet};

use egui::{Context, Event, Key, KeyboardShortcut, Modifiers};

pub use palette::Palette;
pub use shortcut::shortcut_text;

use crate::theme::icons;

/// The menu a command is listed under.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Category {
    /// Projects and files.
    File,
    /// Changing the selection.
    Edit,
    /// What the window shows.
    View,
    /// Playing the animation.
    Playback,
    /// The tools of the tool bar.
    Tools,
}

impl Category {
    /// Every category, in menu order.
    pub const ALL: [Self; 5] = [
        Self::File,
        Self::Edit,
        Self::View,
        Self::Playback,
        Self::Tools,
    ];

    /// The name shown as a menu heading.
    pub fn label(self) -> &'static str {
        match self {
            Self::File => "File",
            Self::Edit => "Edit",
            Self::View => "View",
            Self::Playback => "Playback",
            Self::Tools => "Tools",
        }
    }
}

const fn key(logical_key: Key) -> Option<KeyboardShortcut> {
    Some(KeyboardShortcut::new(Modifiers::NONE, logical_key))
}

const fn cmd(logical_key: Key) -> Option<KeyboardShortcut> {
    Some(KeyboardShortcut::new(Modifiers::COMMAND, logical_key))
}

const fn shift(logical_key: Key) -> Option<KeyboardShortcut> {
    Some(KeyboardShortcut::new(Modifiers::SHIFT, logical_key))
}

const fn cmd_alt(logical_key: Key) -> Option<KeyboardShortcut> {
    Some(KeyboardShortcut::new(
        Modifiers::COMMAND.plus(Modifiers::ALT),
        logical_key,
    ))
}

const fn cmd_shift(logical_key: Key) -> Option<KeyboardShortcut> {
    Some(KeyboardShortcut::new(
        Modifiers::COMMAND.plus(Modifiers::SHIFT),
        logical_key,
    ))
}

/// Declares the command table: the [`CommandId`] enum and, for every command,
/// its category, name, icon and shortcut.
macro_rules! commands {
    ($( $id:ident: $category:ident, $name:literal, $icon:expr, $shortcut:expr; )*) => {
        /// Names one command.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum CommandId {
            $( #[doc = $name] $id, )*
        }

        impl CommandId {
            /// Every command, in menu order.
            pub const ALL: &'static [Self] = &[ $( Self::$id, )* ];

            /// The name shown in menus and in the command palette.
            pub fn name(self) -> &'static str {
                match self { $( Self::$id => $name, )* }
            }

            /// The menu the command is listed under.
            pub fn category(self) -> Category {
                match self { $( Self::$id => Category::$category, )* }
            }

            /// The command's icon, if it has one.
            pub fn icon(self) -> Option<&'static str> {
                match self { $( Self::$id => $icon, )* }
            }

            /// The command's keyboard shortcut, if it has one.
            pub fn shortcut(self) -> Option<KeyboardShortcut> {
                match self { $( Self::$id => $shortcut, )* }
            }
        }
    };
}

commands! {
    NewProject: File, "New project", Some(icons::FILE_PLUS), cmd(Key::N);
    OpenProject: File, "Open…", Some(icons::FOLDER_OPEN), cmd(Key::O);
    Save: File, "Save", Some(icons::FLOPPY_DISK), cmd(Key::S);
    SaveAs: File, "Save as…", None, cmd_shift(Key::S);
    ImportImage: File, "Import image…", Some(icons::IMAGE), cmd_shift(Key::I);
    Export: File, "Export…", Some(icons::EXPORT), cmd(Key::E);

    Undo: Edit, "Undo", Some(icons::ARROW_U_UP_LEFT), cmd(Key::Z);
    Redo: Edit, "Redo", Some(icons::ARROW_U_UP_RIGHT), cmd_shift(Key::Z);
    Cut: Edit, "Cut", None, cmd(Key::X);
    Copy: Edit, "Copy", Some(icons::COPY), cmd(Key::C);
    Paste: Edit, "Paste", None, cmd(Key::V);
    Duplicate: Edit, "Duplicate", None, cmd(Key::D);
    Delete: Edit, "Delete", Some(icons::TRASH), key(Key::Backspace);
    SelectAll: Edit, "Select all", Some(icons::SELECTION_ALL), cmd(Key::A);
    Deselect: Edit, "Deselect", None, key(Key::Escape);
    Group: Edit, "Group selection", Some(icons::FOLDER_SIMPLE), cmd(Key::G);
    Ungroup: Edit, "Ungroup", None, cmd_shift(Key::G);
    BringToFront: Edit, "Bring to front", Some(icons::STACK), cmd_alt(Key::CloseBracket);
    BringForward: Edit, "Bring forward", None, cmd(Key::CloseBracket);
    SendBackward: Edit, "Send backward", None, cmd(Key::OpenBracket);
    SendToBack: Edit, "Send to back", Some(icons::STACK_SIMPLE), cmd_alt(Key::OpenBracket);

    ZoomIn: View, "Zoom in", Some(icons::MAGNIFYING_GLASS_PLUS), cmd(Key::Plus);
    ZoomOut: View, "Zoom out", Some(icons::MAGNIFYING_GLASS_MINUS), cmd(Key::Minus);
    ZoomToFit: View, "Zoom to fit", Some(icons::ARROWS_IN_SIMPLE), shift(Key::Num1);
    ZoomTo100: View, "Zoom to 100%", None, cmd(Key::Num0);
    ToggleChat: View, "Show or hide chat", Some(icons::CHAT_TEARDROP_TEXT), cmd(Key::J);
    ThemeSystem: View, "Theme: follow the system", Some(icons::CIRCLE_HALF), None;
    ThemeLight: View, "Theme: light", Some(icons::SUN), None;
    ThemeDark: View, "Theme: dark", Some(icons::MOON), None;
    CommandPalette: View, "Command palette", Some(icons::COMMAND), cmd(Key::K);
    SwitchMode: View, "Switch between Design and Animate", Some(icons::SWAP), key(Key::M);

    PlayPause: Playback, "Play or pause", Some(icons::PLAY), key(Key::Space);
    JumpToStart: Playback, "Jump to start", Some(icons::SKIP_BACK), key(Key::Home);
    JumpToEnd: Playback, "Jump to end", Some(icons::SKIP_FORWARD), key(Key::End);
    PreviousFrame: Playback, "Previous frame", Some(icons::CARET_LEFT), key(Key::Comma);
    NextFrame: Playback, "Next frame", Some(icons::CARET_RIGHT), key(Key::Period);
    BackTenFrames: Playback, "Back ten frames", Some(icons::CARET_DOUBLE_LEFT), shift(Key::Comma);
    AheadTenFrames: Playback, "Ahead ten frames", Some(icons::CARET_DOUBLE_RIGHT), shift(Key::Period);
    ToggleLoop: Playback, "Loop playback", Some(icons::REPEAT), key(Key::L);

    ToolSelect: Tools, "Select", Some(icons::CURSOR), key(Key::V);
    ToolArtboard: Tools, "Artboard", Some(icons::FRAME_CORNERS), key(Key::A);
    ToolRectangle: Tools, "Rectangle", Some(icons::RECTANGLE), key(Key::R);
    ToolEllipse: Tools, "Ellipse", Some(icons::CIRCLE), key(Key::O);
    ToolStar: Tools, "Star", Some(icons::STAR), key(Key::S);
    ToolPen: Tools, "Pen", Some(icons::PEN_NIB), key(Key::P);
    ToolText: Tools, "Text", Some(icons::TEXT_T), key(Key::T);
    ToolImage: Tools, "Image", Some(icons::IMAGE), key(Key::I);
    AskAi: Tools, "Ask AI", Some(icons::SPARKLE), None;
}

impl CommandId {
    /// Other key presses that run the same command, besides
    /// [`shortcut`](Self::shortcut). They are not shown in menus.
    pub fn alternate_shortcuts(self) -> &'static [KeyboardShortcut] {
        const ZOOM_IN: &[KeyboardShortcut] =
            &[KeyboardShortcut::new(Modifiers::COMMAND, Key::Equals)];
        const DELETE: &[KeyboardShortcut] = &[KeyboardShortcut::new(Modifiers::NONE, Key::Delete)];
        match self {
            // On most layouts "+" is Shift and "=", so accept the bare key too.
            Self::ZoomIn => ZOOM_IN,
            Self::Delete => DELETE,
            _ => &[],
        }
    }

    /// The shortcut as text for menus and tooltips, for example `⇧⌘Z`.
    pub fn shortcut_label(self) -> Option<String> {
        self.shortcut()
            .map(|shortcut| shortcut_text(&shortcut, cfg!(target_os = "macos")))
    }
}

/// Which commands are enabled, and the translation of key presses into
/// commands.
#[derive(Clone, Debug, Default)]
pub struct CommandRegistry {
    disabled: HashSet<CommandId>,
    /// Names that say more than the fixed one right now: "Undo Move".
    labels: HashMap<CommandId, String>,
    /// The text of the last paste event, which the platform hands over
    /// with ⌘V.
    pasted: Option<String>,
    /// Space is down and something else has already used it (see
    /// [`triggered`](Self::triggered)).
    space_spent: bool,
}

impl CommandRegistry {
    /// A registry with every command enabled.
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether `id` can run right now. Disabled commands are greyed out in
    /// menus and ignored by shortcuts.
    pub fn is_enabled(&self, id: CommandId) -> bool {
        !self.disabled.contains(&id)
    }

    /// Enables or disables a command.
    pub fn set_enabled(&mut self, id: CommandId, enabled: bool) {
        if enabled {
            self.disabled.remove(&id);
        } else {
            self.disabled.insert(id);
        }
    }

    /// The name to show for `id` right now: the label set with
    /// [`set_label`](Self::set_label), or the command's own name.
    pub fn label(&self, id: CommandId) -> &str {
        self.labels.get(&id).map_or(id.name(), String::as_str)
    }

    /// Gives a command a name that says more for now, such as "Undo Move",
    /// or goes back to its own name with `None`.
    pub fn set_label(&mut self, id: CommandId, label: Option<String>) {
        match label {
            Some(label) => {
                self.labels.insert(id, label);
            }
            None => {
                self.labels.remove(&id);
            }
        }
    }

    /// The text of the most recent paste event, taken once. The platform
    /// sends the clipboard's text with ⌘V; a paste chosen from a menu has
    /// none and must read the clipboard itself.
    pub fn take_pasted(&mut self) -> Option<String> {
        self.pasted.take()
    }

    /// The commands whose shortcuts were pressed this frame, in the order
    /// they should run. Each key press is consumed, so nothing else reacts to
    /// it.
    ///
    /// Three rules keep shortcuts out of the way:
    ///
    /// - While a text field has focus, only shortcuts that cannot be meant as
    ///   typing or text editing are honoured. `V` types a letter, and the
    ///   Edit commands (undo, copy, select all) belong to the text; ⌘S
    ///   still saves.
    /// - A shortcut with more modifiers wins over one with fewer, so ⇧⌘Z is
    ///   redo and never undo.
    /// - Space runs its command when it is released, and only if no pointer
    ///   button went down while it was held: holding Space is how the canvas
    ///   is panned.
    pub fn triggered(&mut self, ctx: &Context) -> Vec<CommandId> {
        let typing = ctx.text_edit_focused();
        let mut out = Vec::new();

        // Most specific shortcuts first.
        let mut bindings: Vec<(KeyboardShortcut, CommandId)> = CommandId::ALL
            .iter()
            .flat_map(|id| {
                id.shortcut()
                    .into_iter()
                    .chain(id.alternate_shortcuts().iter().copied())
                    .map(|shortcut| (shortcut, *id))
            })
            .filter(|(shortcut, _)| shortcut.logical_key != Key::Space)
            .collect();
        bindings.sort_by_key(|(shortcut, _)| std::cmp::Reverse(modifier_count(shortcut.modifiers)));

        ctx.input_mut(|input| {
            for (shortcut, id) in bindings {
                if typing
                    && (shortcut.conflicts_with_text_editing() || id.category() == Category::Edit)
                {
                    continue;
                }
                if input.consume_shortcut(&shortcut) && self.is_enabled(id) {
                    out.push(id);
                }
            }

            // The platform turns ⌘X, ⌘C and ⌘V into clipboard events.
            if !typing {
                let mut pasted = None;
                input.events.retain(|event| {
                    let id = match event {
                        Event::Cut => CommandId::Cut,
                        Event::Copy => CommandId::Copy,
                        Event::Paste(text) => {
                            pasted = Some(text.clone());
                            CommandId::Paste
                        }
                        _ => return true,
                    };
                    if self.is_enabled(id) {
                        out.push(id);
                    }
                    false
                });
                if pasted.is_some() {
                    self.pasted = pasted;
                }
            }

            // Space: a tap runs the command, a hold pans.
            if input.key_down(Key::Space) && input.pointer.any_down() {
                self.space_spent = true;
            }
            if input.key_released(Key::Space) {
                let tapped = !self.space_spent && !typing && input.modifiers.is_none();
                self.space_spent = false;
                if tapped
                    && let Some(id) = command_for_key(Key::Space)
                    && self.is_enabled(id)
                {
                    out.push(id);
                }
            }
        });
        out
    }

    /// The commands matching what was typed into the command palette, best
    /// match first. An empty query lists every command in menu order.
    ///
    /// A command matches when every word of the query appears in its name or
    /// category. Matches at the start of the name rank above matches at the
    /// start of a later word, which rank above matches inside a word.
    pub fn search(&self, query: &str) -> Vec<CommandId> {
        let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
        let mut ranked: Vec<(u32, usize, CommandId)> = CommandId::ALL
            .iter()
            .enumerate()
            .filter_map(|(order, id)| match_rank(*id, &words).map(|rank| (rank, order, *id)))
            .collect();
        ranked.sort_by_key(|(rank, order, _)| (*rank, *order));
        ranked.into_iter().map(|(_, _, id)| id).collect()
    }
}

fn modifier_count(modifiers: Modifiers) -> u8 {
    u8::from(modifiers.alt)
        + u8::from(modifiers.shift)
        + u8::from(modifiers.ctrl || modifiers.command || modifiers.mac_cmd)
}

/// The command bound to a bare key press.
fn command_for_key(logical_key: Key) -> Option<CommandId> {
    CommandId::ALL.iter().copied().find(|id| {
        id.shortcut()
            .is_some_and(|s| s.modifiers.is_none() && s.logical_key == logical_key)
    })
}

/// How well a command matches the query words: lower is better, `None` is no
/// match.
fn match_rank(id: CommandId, words: &[String]) -> Option<u32> {
    let name = id.name().to_lowercase();
    let category = id.category().label().to_lowercase();
    let mut rank = 0;
    for word in words {
        rank += if name.starts_with(word.as_str()) {
            0
        } else if name
            .split(|c: char| !c.is_alphanumeric())
            .any(|part| part.starts_with(word.as_str()))
        {
            1
        } else if name.contains(word.as_str()) {
            2
        } else if category.starts_with(word.as_str()) {
            3
        } else {
            return None;
        };
    }
    Some(rank)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_command_is_described() {
        let mut seen = HashSet::new();
        for id in CommandId::ALL {
            assert!(seen.insert(*id), "{id:?} is listed twice");
            assert!(!id.name().is_empty());
        }
        for category in Category::ALL {
            assert!(
                CommandId::ALL.iter().any(|id| id.category() == category),
                "{category:?} has no commands"
            );
        }
    }

    #[test]
    fn no_two_commands_share_a_shortcut() {
        let mut seen = HashSet::new();
        for id in CommandId::ALL {
            for shortcut in id.shortcut().iter().chain(id.alternate_shortcuts().iter()) {
                assert!(
                    seen.insert((shortcut.modifiers, shortcut.logical_key)),
                    "{id:?} reuses a shortcut"
                );
            }
        }
    }

    #[test]
    fn commands_can_be_disabled_and_enabled() {
        let mut registry = CommandRegistry::new();
        assert!(registry.is_enabled(CommandId::Undo));
        registry.set_enabled(CommandId::Undo, false);
        assert!(!registry.is_enabled(CommandId::Undo));
        assert!(registry.is_enabled(CommandId::Redo));
        registry.set_enabled(CommandId::Undo, true);
        assert!(registry.is_enabled(CommandId::Undo));
    }

    #[test]
    fn an_empty_query_lists_everything_in_order() {
        let registry = CommandRegistry::new();
        assert_eq!(registry.search(""), CommandId::ALL);
        assert_eq!(registry.search("   "), CommandId::ALL);
    }

    #[test]
    fn search_ranks_name_starts_first() {
        let registry = CommandRegistry::new();
        let found = registry.search("zoom");
        assert_eq!(
            found,
            [
                CommandId::ZoomIn,
                CommandId::ZoomOut,
                CommandId::ZoomToFit,
                CommandId::ZoomTo100
            ]
        );
        // "sel" starts "Select all" and "Select", starts a later word of
        // "Group selection", and is buried inside "Deselect".
        assert_eq!(
            registry.search("sel"),
            [
                CommandId::SelectAll,
                CommandId::ToolSelect,
                CommandId::Group,
                CommandId::Deselect
            ]
        );
    }

    #[test]
    fn search_needs_every_word_and_ignores_case() {
        let registry = CommandRegistry::new();
        assert_eq!(registry.search("THEME dark"), [CommandId::ThemeDark]);
        assert_eq!(registry.search("save as"), [CommandId::SaveAs]);
        assert!(registry.search("no such command").is_empty());
        // A category name finds its commands.
        assert!(registry.search("playback").contains(&CommandId::PlayPause));
    }

    /// Runs one frame with `events` and returns the commands triggered.
    fn frame(ctx: &Context, registry: &mut CommandRegistry, events: Vec<Event>) -> Vec<CommandId> {
        let mut triggered = Vec::new();
        let input = egui::RawInput {
            events,
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| triggered = registry.triggered(ui.ctx()));
        // Nothing is drawn in these tests, so the font texture is not needed.
        output.textures_delta.clear();
        triggered
    }

    fn key_event(modifiers: Modifiers, logical_key: Key, pressed: bool) -> Event {
        Event::Key {
            key: logical_key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers,
        }
    }

    fn press(modifiers: Modifiers, logical_key: Key) -> Vec<Event> {
        vec![key_event(modifiers, logical_key, true)]
    }

    #[test]
    fn a_shortcut_runs_its_command_once() {
        let ctx = Context::default();
        let mut registry = CommandRegistry::new();
        assert_eq!(
            frame(&ctx, &mut registry, press(Modifiers::COMMAND, Key::K)),
            [CommandId::CommandPalette]
        );
        assert_eq!(
            frame(&ctx, &mut registry, press(Modifiers::NONE, Key::R)),
            [CommandId::ToolRectangle]
        );
        // Nothing pressed, nothing runs.
        assert!(frame(&ctx, &mut registry, Vec::new()).is_empty());
    }

    #[test]
    fn the_more_specific_shortcut_wins() {
        let ctx = Context::default();
        let mut registry = CommandRegistry::new();
        let shift_command = Modifiers::COMMAND.plus(Modifiers::SHIFT);
        assert_eq!(
            frame(&ctx, &mut registry, press(shift_command, Key::Z)),
            [CommandId::Redo]
        );
        assert_eq!(
            frame(&ctx, &mut registry, press(Modifiers::COMMAND, Key::Z)),
            [CommandId::Undo]
        );
        // ⌘S saves; plain S is the star tool.
        assert_eq!(
            frame(&ctx, &mut registry, press(Modifiers::COMMAND, Key::S)),
            [CommandId::Save]
        );
        assert_eq!(
            frame(&ctx, &mut registry, press(Modifiers::NONE, Key::S)),
            [CommandId::ToolStar]
        );
    }

    #[test]
    fn disabled_commands_ignore_their_shortcuts() {
        let ctx = Context::default();
        let mut registry = CommandRegistry::new();
        registry.set_enabled(CommandId::Undo, false);
        assert!(frame(&ctx, &mut registry, press(Modifiers::COMMAND, Key::Z)).is_empty());
        registry.set_enabled(CommandId::Undo, true);
        assert_eq!(
            frame(&ctx, &mut registry, press(Modifiers::COMMAND, Key::Z)),
            [CommandId::Undo]
        );
    }

    #[test]
    fn alternate_keys_run_the_same_command() {
        let ctx = Context::default();
        let mut registry = CommandRegistry::new();
        assert_eq!(
            frame(&ctx, &mut registry, press(Modifiers::COMMAND, Key::Equals)),
            [CommandId::ZoomIn]
        );
        assert_eq!(
            frame(&ctx, &mut registry, press(Modifiers::NONE, Key::Delete)),
            [CommandId::Delete]
        );
    }

    #[test]
    fn clipboard_events_become_commands() {
        let ctx = Context::default();
        let mut registry = CommandRegistry::new();
        assert_eq!(
            frame(&ctx, &mut registry, vec![Event::Copy]),
            [CommandId::Copy]
        );
        assert_eq!(registry.take_pasted(), None);
        assert_eq!(
            frame(&ctx, &mut registry, vec![Event::Paste("x".to_owned())]),
            [CommandId::Paste]
        );
        assert_eq!(registry.take_pasted().as_deref(), Some("x"));
        assert_eq!(registry.take_pasted(), None, "taken once");
    }

    #[test]
    fn labels_say_more_than_names() {
        let mut registry = CommandRegistry::new();
        assert_eq!(registry.label(CommandId::Undo), "Undo");
        registry.set_label(CommandId::Undo, Some("Undo Move".to_owned()));
        assert_eq!(registry.label(CommandId::Undo), "Undo Move");
        assert_eq!(registry.label(CommandId::Redo), "Redo");
        registry.set_label(CommandId::Undo, None);
        assert_eq!(registry.label(CommandId::Undo), "Undo");
    }

    #[test]
    fn a_tap_on_space_plays_but_holding_it_to_pan_does_not() {
        let ctx = Context::default();
        let mut registry = CommandRegistry::new();
        let space = |pressed| vec![key_event(Modifiers::NONE, Key::Space, pressed)];

        // Down does nothing yet; up runs the command.
        assert!(frame(&ctx, &mut registry, space(true)).is_empty());
        assert_eq!(
            frame(&ctx, &mut registry, space(false)),
            [CommandId::PlayPause]
        );

        // Down, a pointer press (a pan), up: no command.
        assert!(frame(&ctx, &mut registry, space(true)).is_empty());
        let pointer = |pressed| {
            vec![Event::PointerButton {
                pos: egui::pos2(10.0, 10.0),
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            }]
        };
        assert!(frame(&ctx, &mut registry, pointer(true)).is_empty());
        assert!(frame(&ctx, &mut registry, pointer(false)).is_empty());
        assert!(frame(&ctx, &mut registry, space(false)).is_empty());

        // The next tap works again.
        assert!(frame(&ctx, &mut registry, space(true)).is_empty());
        assert_eq!(
            frame(&ctx, &mut registry, space(false)),
            [CommandId::PlayPause]
        );
    }

    #[test]
    fn specific_shortcuts_sort_before_general_ones() {
        let redo = CommandId::Redo.shortcut().unwrap();
        let undo = CommandId::Undo.shortcut().unwrap();
        assert!(modifier_count(redo.modifiers) > modifier_count(undo.modifiers));
        assert_eq!(command_for_key(Key::Space), Some(CommandId::PlayPause));
        assert_eq!(command_for_key(Key::F20), None);
    }
}
