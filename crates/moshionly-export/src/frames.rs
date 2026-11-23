//! Turning moments into rendered frames.

use moshionly_core::{Document, NodeId, Scene, evaluate, evaluate_at_rest};
use moshionly_render::{Frame, RenderOptions, Renderer};

use crate::error::ExportError;
use crate::settings::Moment;

/// Renders frames of one artboard. Cheap to copy and safe to share between
/// threads: each frame is a pure function of the document and its moment.
#[derive(Clone, Copy)]
pub(crate) struct FrameSource<'a> {
    /// The document.
    pub document: &'a Document,
    /// The artboard drawn.
    pub artboard: &'a NodeId,
    /// Draws the scenes; its fonts also lay text out.
    pub renderer: &'a Renderer,
}

impl FrameSource<'_> {
    /// The artboard at `moment`.
    pub fn scene(&self, moment: Moment) -> Result<Scene, ExportError> {
        let fonts = self.renderer.fonts();
        Ok(match moment {
            Moment::AtRest => evaluate_at_rest(self.document, self.artboard, fonts)?,
            Moment::At(time_ms) => evaluate(self.document, self.artboard, time_ms, fonts)?,
        })
    }

    /// Frame `index`, which shows the artboard at `moment`.
    pub fn render(
        &self,
        index: u64,
        moment: Moment,
        options: &RenderOptions,
    ) -> Result<Frame, ExportError> {
        let scene = self.scene(moment)?;
        self.renderer
            .render(&scene, options)
            .map_err(|source| ExportError::Render {
                frame: index,
                source,
            })
    }
}
