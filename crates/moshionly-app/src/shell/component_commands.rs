//! The commands about components: making one from the selection, placing
//! instances of it, going from an instance to its main, and detaching an
//! instance into a plain group.

use moshionly_core::NodeId;
use moshionly_core::build;

use super::Shell;
use crate::{
    commands::{CommandId, CommandRegistry},
    editor::{
        Editor,
        components::{instance_beside, instances_in, mains_in, shown_main},
    },
    widgets::Toast,
};

impl Shell {
    /// Carries out a component command. Returns false for other commands.
    pub(super) fn component_command(&mut self, id: CommandId) -> bool {
        match id {
            CommandId::CreateComponent => self.create_component(),
            CommandId::InsertInstance => self.insert_instance(),
            CommandId::GoToMainComponent => self.go_to_main(),
            CommandId::DetachInstance => self.detach_instances(),
            _ => return false,
        }
        true
    }

    /// Makes the selection a component and selects it: a selected group
    /// becomes one in place, other layers are wrapped in a new one.
    fn create_component(&mut self) {
        let ids = self.editor.selection.ids().to_vec();
        // Text that hugs its words is given the box it is drawn in first,
        // as grouping does, so that the component is made around the words.
        let settle = self.editor.settle_edits(&ids, false);
        let mut working = (**self.editor.document()).clone();
        if let Err(error) = working.apply(&settle) {
            self.toasts.push(Toast::error(error.problem.to_string()));
            return;
        }
        match build::make_component(&working, &ids) {
            Ok((component, edits)) => {
                if self.apply_or_toast("Create component", &[settle, edits].concat()) {
                    self.editor.selection.select(component);
                }
            }
            Err(error) => self.toasts.push(Toast::warning(error.to_string())),
        }
    }

    /// Places an instance beside each selected main component, and selects
    /// the new instances.
    fn insert_instance(&mut self) {
        let mains = mains_in(self.editor.document(), self.editor.selection.ids());
        let mut working = (**self.editor.document()).clone();
        let mut edits = Vec::new();
        let mut added = Vec::new();
        for main in &mains {
            match instance_beside(&working, main) {
                Ok((instance, more)) => {
                    if working.apply(&more).is_ok() {
                        edits.extend(more);
                        added.push(instance);
                    }
                }
                Err(error) => self.toasts.push(Toast::warning(error.to_string())),
            }
        }
        let summary = if added.len() == 1 {
            "Insert instance"
        } else {
            "Insert instances"
        };
        if self.apply_or_toast(summary, &edits) {
            self.editor.selection.set(added);
        }
    }

    /// Selects the main component the selected instances show, switches to
    /// its artboard and brings it into view.
    fn go_to_main(&mut self) {
        let Some(main) = shown_main(self.editor.document(), self.editor.selection.ids()) else {
            return;
        };
        self.editor.selection.select(main.clone());
        self.editor.set_actions([]);
        self.editor.follow_selection();
        if let (Some(canvas), Some(found)) = (self.canvas, self.editor.box_of(&main)) {
            self.fit(canvas, found.bounds());
        }
    }

    /// Turns the selected instances into plain groups that look the same,
    /// and selects the groups.
    fn detach_instances(&mut self) {
        let instances = instances_in(self.editor.document(), self.editor.selection.ids());
        let mut working = (**self.editor.document()).clone();
        let mut edits = Vec::new();
        let mut groups = Vec::new();
        for instance in &instances {
            match build::detach(&working, instance) {
                Ok((group, more)) => {
                    if working.apply(&more).is_ok() {
                        edits.extend(more);
                        groups.push(group);
                    }
                }
                Err(error) => self.toasts.push(Toast::warning(error.to_string())),
            }
        }
        let summary = if groups.len() == 1 {
            "Detach instance"
        } else {
            "Detach instances"
        };
        if self.apply_or_toast(summary, &edits) {
            // Instances that were not selected keep their selection.
            let kept: Vec<NodeId> = self
                .editor
                .selection
                .ids()
                .iter()
                .filter(|id| !instances.contains(id))
                .cloned()
                .collect();
            self.editor.selection.set(kept.into_iter().chain(groups));
        }
    }
}

/// Enables the component commands that make sense for the selection.
pub(super) fn enable_component_commands(registry: &mut CommandRegistry, editor: &Editor) {
    let document = editor.document();
    let ids = editor.selection.ids();
    registry.set_enabled(
        CommandId::CreateComponent,
        !ids.is_empty() && build::make_component(document, ids).is_ok(),
    );
    registry.set_enabled(
        CommandId::InsertInstance,
        !mains_in(document, ids).is_empty(),
    );
    registry.set_enabled(
        CommandId::GoToMainComponent,
        shown_main(document, ids).is_some(),
    );
    registry.set_enabled(
        CommandId::DetachInstance,
        !instances_in(document, ids).is_empty(),
    );
}

#[cfg(test)]
mod tests {
    use moshionly_core::io;

    use super::*;

    fn editor() -> Editor {
        let document = io::parse(
            r##"{"format":"moshionly","version":2,"name":"Test","nodes":[
            {"id":"ab","parent":null,"index":"a0","type":"artboard","width":1000,"height":1000},
            {"id":"card","parent":"ab","index":"a0","type":"component","width":200,"height":100},
            {"id":"dot","parent":"card","index":"a0","type":"ellipse","width":20,"height":20},
            {"id":"copy","parent":"ab","index":"a1","type":"instance","component":"card","x":300,"width":200,"height":100},
            {"id":"box","parent":"ab","index":"a2","type":"rect"},
            {"id":"bag","parent":"ab","index":"a3","type":"group"},
            {"id":"inner","parent":"bag","index":"a0","type":"rect"}
            ]}"##,
        )
        .unwrap();
        Editor::new(
            moshionly_agent::SessionHandle::from_document(document),
            moshionly_render::Renderer::new(
                moshionly_render::FontLibrary::bundled(),
                moshionly_render::AssetLoader::default(),
            ),
        )
    }

    /// The component commands enabled with `ids` selected.
    fn enabled(editor: &mut Editor, ids: &[&str]) -> Vec<CommandId> {
        editor.selection.set(ids.iter().map(|id| NodeId::from(*id)));
        let mut registry = CommandRegistry::new();
        enable_component_commands(&mut registry, editor);
        [
            CommandId::CreateComponent,
            CommandId::InsertInstance,
            CommandId::GoToMainComponent,
            CommandId::DetachInstance,
        ]
        .into_iter()
        .filter(|id| registry.is_enabled(*id))
        .collect()
    }

    #[test]
    fn component_commands_follow_the_selection() {
        let mut editor = editor();
        assert!(enabled(&mut editor, &[]).is_empty());
        // Plain layers can become a component; nothing else applies.
        assert_eq!(enabled(&mut editor, &["box"]), [CommandId::CreateComponent]);
        assert_eq!(
            enabled(&mut editor, &["box", "bag"]),
            [CommandId::CreateComponent]
        );
        // A main can have instances, but is a component already.
        assert_eq!(enabled(&mut editor, &["card"]), [CommandId::InsertInstance]);
        // A layer inside a main cannot become another component.
        assert!(enabled(&mut editor, &["dot"]).is_empty());
        // An instance leads to its main and can be detached; wrapping it in
        // a component of its own is fine.
        assert_eq!(
            enabled(&mut editor, &["copy"]),
            [
                CommandId::CreateComponent,
                CommandId::GoToMainComponent,
                CommandId::DetachInstance
            ]
        );
        // Artboards are never components.
        assert!(enabled(&mut editor, &["ab"]).is_empty());
    }
}
