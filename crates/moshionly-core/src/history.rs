//! Undo and redo.
//!
//! A [`History`] sits next to a [`Document`] and records, for every batch
//! applied through it, the batch that undoes it. It never changes the
//! document on its own: every method that does takes the document as an
//! argument, so the two can live side by side in the application state.
//!
//! Apply edits through [`History::apply`] rather than `Document::apply`,
//! otherwise the recorded steps no longer match the document.

use crate::document::Document;
use crate::edit::{Edit, EditError, Journal};

/// Undo and redo stacks for one document, and a revision counter.
#[derive(Debug, Clone)]
pub struct History {
    undo: Vec<Vec<Edit>>,
    redo: Vec<Vec<Edit>>,
    /// What the open transaction has touched so far.
    open: Option<Journal>,
    revision: u64,
    limit: usize,
}

impl Default for History {
    fn default() -> Self {
        Self::new()
    }
}

impl History {
    /// How many undo steps are kept unless [`History::with_limit`] says otherwise.
    pub const DEFAULT_LIMIT: usize = 500;

    /// An empty history that keeps [`History::DEFAULT_LIMIT`] undo steps.
    pub fn new() -> Self {
        Self::with_limit(Self::DEFAULT_LIMIT)
    }

    /// An empty history that keeps at most `limit` undo steps (at least 1).
    /// When the limit is reached the oldest step is forgotten.
    pub fn with_limit(limit: usize) -> Self {
        Self {
            undo: Vec::new(),
            redo: Vec::new(),
            open: None,
            revision: 0,
            limit: limit.max(1),
        }
    }

    /// A number that changes every time the document is changed through this
    /// history: by an applied batch, an undo, a redo or a cancelled
    /// transaction. Refused batches do not change it.
    ///
    /// Compare it with the value you saw last to know whether anything
    /// derived from the document (a rendered frame, a layer list) is stale.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Whether there is a step to undo.
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty() || self.open.is_some()
    }

    /// Whether there is a step to redo.
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Number of steps that can be undone, not counting an open transaction.
    pub fn undo_depth(&self) -> usize {
        self.undo.len()
    }

    /// Number of steps that can be redone.
    pub fn redo_depth(&self) -> usize {
        self.redo.len()
    }

    /// Applies a batch to the document and records how to undo it.
    ///
    /// Outside a transaction the batch becomes one undo step. Inside one it
    /// is folded into the step the transaction will produce. A batch that
    /// changes something clears the redo stack. A refused batch changes
    /// neither the document nor the history.
    pub fn apply(&mut self, document: &mut Document, edits: &[Edit]) -> Result<(), EditError> {
        match &mut self.open {
            Some(journal) => {
                let mut step = Journal::default();
                document.apply_journaled(edits, &mut step)?;
                if !document.inverse_of(&step).is_empty() {
                    self.redo.clear();
                    self.revision += 1;
                }
                journal.absorb(step);
            }
            None => {
                let inverse = document.apply(edits)?;
                self.record(inverse);
            }
        }
        Ok(())
    }

    /// Opens a transaction: everything applied until [`History::commit`]
    /// becomes a single undo step.
    ///
    /// Use it for a continuous gesture such as dragging a layer, which
    /// applies an edit on every mouse move. The step that is recorded goes
    /// straight from the state at the end back to the state at the start, no
    /// matter how many edits happened in between. Calling it while a
    /// transaction is open keeps that transaction.
    pub fn begin(&mut self) {
        self.open.get_or_insert_with(Journal::default);
    }

    /// Whether a transaction is open.
    pub fn in_transaction(&self) -> bool {
        self.open.is_some()
    }

    /// Closes the open transaction and records it as one undo step. A
    /// transaction that changed nothing records nothing.
    pub fn commit(&mut self, document: &Document) {
        if let Some(journal) = self.open.take() {
            let inverse = document.inverse_of(&journal);
            if !inverse.is_empty() {
                self.push_undo(inverse);
            }
        }
    }

    /// Closes the open transaction and puts the document back as it was when
    /// the transaction was opened. Nothing is recorded.
    pub fn cancel(&mut self, document: &mut Document) -> Result<(), EditError> {
        let Some(journal) = self.open.take() else {
            return Ok(());
        };
        let inverse = document.inverse_of(&journal);
        if !inverse.is_empty() {
            document.apply(&inverse)?;
            self.revision += 1;
        }
        Ok(())
    }

    /// Undoes the most recent step. An open transaction is committed first
    /// and is then the step that is undone.
    ///
    /// Returns whether there was anything to undo. An error means the
    /// document was changed behind the history's back; the stacks are
    /// cleared because they can no longer be trusted.
    pub fn undo(&mut self, document: &mut Document) -> Result<bool, EditError> {
        self.commit(document);
        let Some(batch) = self.undo.pop() else {
            return Ok(false);
        };
        match document.apply(&batch) {
            Ok(inverse) => {
                self.redo.push(inverse);
                self.revision += 1;
                Ok(true)
            }
            Err(error) => {
                self.clear();
                Err(error)
            }
        }
    }

    /// Redoes the most recently undone step.
    ///
    /// Returns whether there was anything to redo. Errors are as for
    /// [`History::undo`].
    pub fn redo(&mut self, document: &mut Document) -> Result<bool, EditError> {
        self.commit(document);
        let Some(batch) = self.redo.pop() else {
            return Ok(false);
        };
        match document.apply(&batch) {
            Ok(inverse) => {
                self.undo.push(inverse);
                self.revision += 1;
                Ok(true)
            }
            Err(error) => {
                self.clear();
                Err(error)
            }
        }
    }

    /// Makes the most recent undo step permanent: its change stays in the
    /// document, but undo no longer reaches it and goes on with the step
    /// before. For a change that is part of something else rather than an
    /// edit of its own, such as pointing image layers at copies made while
    /// saving a project in another folder. An open transaction is left as
    /// it is. Returns whether there was a step to make permanent.
    pub fn forget_last(&mut self) -> bool {
        self.undo.pop().is_some()
    }

    /// Forgets every step and any open transaction, for example after
    /// loading another file into the same document. The revision still
    /// changes, because whatever was derived from the document is stale.
    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.open = None;
        self.revision += 1;
    }

    /// Records the inverse of a batch that was just applied.
    fn record(&mut self, inverse: Vec<Edit>) {
        if !inverse.is_empty() {
            self.push_undo(inverse);
            self.revision += 1;
        }
    }

    fn push_undo(&mut self, inverse: Vec<Edit>) {
        self.redo.clear();
        self.undo.push(inverse);
        if self.undo.len() > self.limit {
            let excess = self.undo.len() - self.limit;
            self.undo.drain(..excess);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::NodeId;
    use crate::model::Node;
    use serde_json::{Value, json};

    fn add(value: Value) -> Edit {
        Edit::add(Node::from_value(value).unwrap())
    }

    fn update(id: &str, patch: Value) -> Edit {
        Edit::update(id, patch.as_object().unwrap().clone())
    }

    fn setup() -> (Document, History) {
        let mut document = Document::new("Test");
        let mut history = History::new();
        history
            .apply(
                &mut document,
                &[
                    add(json!({ "id": "ab", "index": "a0", "type": "artboard" })),
                    add(json!({ "id": "r1", "parent": "ab", "index": "a0", "type": "rect" })),
                ],
            )
            .unwrap();
        (document, history)
    }

    fn x_of(document: &Document, id: &str) -> f64 {
        document.get(&NodeId::from(id)).unwrap().layer().unwrap().x
    }

    #[test]
    fn undo_and_redo_walk_the_steps() {
        let (mut document, mut history) = setup();
        let created = document.clone();
        history
            .apply(&mut document, &[update("r1", json!({ "x": 10 }))])
            .unwrap();
        history
            .apply(&mut document, &[update("r1", json!({ "x": 20 }))])
            .unwrap();
        assert_eq!((history.undo_depth(), history.redo_depth()), (3, 0));

        assert!(history.undo(&mut document).unwrap());
        assert_eq!(x_of(&document, "r1"), 10.0);
        assert!(history.undo(&mut document).unwrap());
        assert_eq!(document, created);
        assert!(history.undo(&mut document).unwrap());
        assert!(document.is_empty());
        assert!(!history.undo(&mut document).unwrap(), "nothing left");
        assert!(!history.can_undo() && history.can_redo());

        assert!(history.redo(&mut document).unwrap());
        assert_eq!(document, created);
        assert!(history.redo(&mut document).unwrap());
        assert!(history.redo(&mut document).unwrap());
        assert_eq!(x_of(&document, "r1"), 20.0);
        assert!(!history.redo(&mut document).unwrap());
    }

    #[test]
    fn a_step_made_permanent_is_skipped_by_undo() {
        let (mut document, mut history) = setup();
        history
            .apply(&mut document, &[update("r1", json!({ "x": 10 }))])
            .unwrap();
        history
            .apply(&mut document, &[update("r1", json!({ "y": 30 }))])
            .unwrap();
        let revision = history.revision();

        assert!(history.forget_last());
        assert_eq!(history.revision(), revision, "the document did not change");
        assert_eq!(history.undo_depth(), 2);
        // Undo goes on with the step before; the permanent change stays.
        assert!(history.undo(&mut document).unwrap());
        let rect = document.get(&NodeId::from("r1")).unwrap().layer().unwrap();
        assert_eq!((rect.x, rect.y), (0.0, 30.0));

        assert!(history.forget_last());
        assert!(!history.forget_last(), "nothing left to make permanent");
        assert!(!history.undo(&mut document).unwrap());
    }

    #[test]
    fn a_new_edit_clears_redo() {
        let (mut document, mut history) = setup();
        history
            .apply(&mut document, &[update("r1", json!({ "x": 10 }))])
            .unwrap();
        history.undo(&mut document).unwrap();
        assert!(history.can_redo());
        history
            .apply(&mut document, &[update("r1", json!({ "y": 5 }))])
            .unwrap();
        assert!(!history.can_redo());
    }

    #[test]
    fn the_revision_changes_exactly_when_the_document_does() {
        let (mut document, mut history) = setup();
        let mut seen = history.revision();
        let mut changed = |history: &History| {
            let changed = history.revision() != seen;
            seen = history.revision();
            changed
        };

        history
            .apply(&mut document, &[update("r1", json!({ "x": 10 }))])
            .unwrap();
        assert!(changed(&history));
        // A refused batch and a batch that changes nothing leave it alone.
        assert!(
            history
                .apply(&mut document, &[update("r1", json!({ "opacity": 500 }))])
                .is_err()
        );
        assert!(!changed(&history));
        history
            .apply(&mut document, &[update("r1", json!({ "x": 10 }))])
            .unwrap();
        assert!(!changed(&history));
        assert_eq!(history.undo_depth(), 2, "and it is not an undo step");

        history.undo(&mut document).unwrap();
        assert!(changed(&history));
        history.redo(&mut document).unwrap();
        assert!(changed(&history));
        history.redo(&mut document).unwrap();
        assert!(!changed(&history), "nothing to redo");
    }

    #[test]
    fn a_transaction_is_one_undo_step() {
        let (mut document, mut history) = setup();
        let before = document.clone();
        let depth = history.undo_depth();

        history.begin();
        assert!(history.in_transaction());
        for x in 1..=50 {
            let revision = history.revision();
            history
                .apply(
                    &mut document,
                    &[update("r1", json!({ "x": x, "y": x * 2 }))],
                )
                .unwrap();
            assert_ne!(history.revision(), revision, "every drag step redraws");
        }
        // A refused edit in the middle of the gesture does not end it.
        assert!(
            history
                .apply(&mut document, &[update("r1", json!({ "scale": -1 }))])
                .is_err()
        );
        history
            .apply(
                &mut document,
                &[add(
                    json!({ "id": "r2", "parent": "ab", "index": "a1", "type": "rect" }),
                )],
            )
            .unwrap();
        history.commit(&document);
        assert!(!history.in_transaction());
        assert_eq!(history.undo_depth(), depth + 1);
        // The step goes straight back, however many edits there were.
        assert_eq!(
            history.undo.last().unwrap(),
            &[
                update("r1", json!({ "x": null, "y": null })),
                Edit::remove("r2")
            ]
        );

        let after = document.clone();
        history.undo(&mut document).unwrap();
        assert_eq!(document, before);
        history.redo(&mut document).unwrap();
        assert_eq!(document, after);
    }

    #[test]
    fn a_transaction_that_ends_where_it_began_records_nothing() {
        let (mut document, mut history) = setup();
        let depth = history.undo_depth();
        history.begin();
        history
            .apply(&mut document, &[update("r1", json!({ "x": 30 }))])
            .unwrap();
        history
            .apply(&mut document, &[update("r1", json!({ "x": 0 }))])
            .unwrap();
        history.commit(&document);
        assert_eq!(history.undo_depth(), depth);
        // Committing without a transaction is harmless.
        history.commit(&document);
    }

    #[test]
    fn cancel_rolls_the_gesture_back() {
        let (mut document, mut history) = setup();
        let before = document.clone();
        let depth = history.undo_depth();
        history.begin();
        history
            .apply(&mut document, &[update("r1", json!({ "x": 30 }))])
            .unwrap();
        history.apply(&mut document, &[Edit::remove("r1")]).unwrap();
        let revision = history.revision();
        history.cancel(&mut document).unwrap();
        assert_eq!(document, before);
        assert_eq!(history.undo_depth(), depth);
        assert_ne!(history.revision(), revision);
        assert!(!history.in_transaction());
        history.cancel(&mut document).unwrap();
    }

    #[test]
    fn undo_closes_an_open_transaction_first() {
        let (mut document, mut history) = setup();
        history.begin();
        history
            .apply(&mut document, &[update("r1", json!({ "x": 30 }))])
            .unwrap();
        assert!(history.can_undo());
        assert!(history.undo(&mut document).unwrap());
        assert_eq!(x_of(&document, "r1"), 0.0);
        assert!(!history.in_transaction());
        assert!(history.redo(&mut document).unwrap());
        assert_eq!(x_of(&document, "r1"), 30.0);
    }

    #[test]
    fn the_oldest_steps_are_forgotten_at_the_limit() {
        let mut document = Document::new("Test");
        let mut history = History::with_limit(3);
        history
            .apply(
                &mut document,
                &[
                    add(json!({ "id": "ab", "index": "a0", "type": "artboard" })),
                    add(json!({ "id": "r1", "parent": "ab", "index": "a0", "type": "rect" })),
                ],
            )
            .unwrap();
        for x in 1..=5 {
            history
                .apply(&mut document, &[update("r1", json!({ "x": x }))])
                .unwrap();
        }
        assert_eq!(history.undo_depth(), 3);
        while history.undo(&mut document).unwrap() {}
        assert_eq!(x_of(&document, "r1"), 2.0);
    }

    #[test]
    fn a_document_changed_behind_its_back_resets_the_history() {
        let (mut document, mut history) = setup();
        history
            .apply(&mut document, &[update("r1", json!({ "x": 10 }))])
            .unwrap();
        // Not through the history: the recorded step no longer applies.
        document.apply(&[Edit::remove("r1")]).unwrap();
        assert!(history.undo(&mut document).is_err());
        assert!(!history.can_undo() && !history.can_redo());
    }
}
