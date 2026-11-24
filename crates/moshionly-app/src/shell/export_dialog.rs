//! The Export dialog: which artboard, in which format and how, then a
//! progress bar with Cancel, and at the end the file's size and a way to
//! find it.

use std::path::PathBuf;
use std::sync::Arc;

use egui::{Context, DragValue, ProgressBar, RichText, ScrollArea, Ui};
use moshionly_core::NodeId;
use moshionly_export::{Availability, Exported, Format, Phase, Quality, availability};

use super::Shell;
use crate::{
    export::{
        ExportRun, ExportSetup, FRAME_RATES, GifLoop, RunOutcome, SCALES, file_manager_name,
        file_size, reveal,
    },
    theme::Theme,
    widgets::{
        ButtonKind, Dropdown, ModalFrame, TextButton, Toast, modal_footer, property_row, segmented,
    },
};

/// The Export dialog while it is open.
pub(super) struct ExportDialog {
    setup: ExportSetup,
    /// Every format, and why one cannot be written right now.
    availability: Vec<Availability>,
    stage: Stage,
}

/// Where the dialog is in an export.
enum Stage {
    /// The person is choosing.
    Choosing,
    /// The file is being written.
    Running {
        run: ExportRun,
        /// What is being written, for the dialog's words.
        label: String,
    },
    /// The file was written.
    Done(Exported),
    /// It failed.
    Failed(String),
}

impl ExportDialog {
    /// What the dialog shows for the export being set up, for tests.
    pub(super) fn setup(&self) -> &ExportSetup {
        &self.setup
    }

    /// Whether the dialog's file is being written.
    pub(super) fn is_running(&self) -> bool {
        matches!(self.stage, Stage::Running { .. })
    }
}

impl Shell {
    /// Opens the Export dialog on the current artboard, with the choices
    /// left last time. Formats that need a missing program are offered
    /// with the reason they cannot be used. While a file is being written
    /// the dialog stays on that export, with its progress and Cancel.
    pub fn open_export_dialog(&mut self) {
        if self.is_exporting() {
            return;
        }
        let document = Arc::clone(self.editor.document());
        let Some(artboard) = self
            .editor
            .artboard()
            .cloned()
            .or_else(|| document.artboards().first().cloned())
        else {
            self.toasts.push(Toast::info(
                "Add an artboard first: there is nothing to export",
            ));
            return;
        };
        let availability = availability();
        let mut choices = self.settings.export.clone();
        let usable = |format: Format| {
            availability
                .iter()
                .any(|a| a.format == format && a.is_available())
        };
        if !usable(choices.format()) {
            choices.set_format(Format::Mp4);
        }
        self.export = Some(ExportDialog {
            setup: ExportSetup {
                choices,
                artboard,
                range: None,
                playhead: self.editor.playhead(),
            },
            availability,
            stage: Stage::Choosing,
        });
    }

    /// Whether the Export dialog is open.
    pub fn is_export_dialog_open(&self) -> bool {
        self.export.is_some()
    }

    /// Whether the Export dialog is writing a file.
    pub fn is_exporting(&self) -> bool {
        self.export.as_ref().is_some_and(ExportDialog::is_running)
    }

    /// The export being set up in the open Export dialog.
    pub fn export_setup(&self) -> Option<&ExportSetup> {
        self.export.as_ref().map(ExportDialog::setup)
    }

    /// The export being set up, to change its choices as the dialog does.
    pub fn export_setup_mut(&mut self) -> Option<&mut ExportSetup> {
        self.export.as_mut().map(|dialog| &mut dialog.setup)
    }

    /// Starts writing the export being set up to `destination`, off the UI
    /// thread, and remembers the choices for next time.
    pub fn start_export(&mut self, destination: PathBuf, window: Option<&Context>) {
        let document = Arc::clone(self.editor.document());
        let project = self.editor.project_file().map(ToOwned::to_owned);
        let renderer = self.editor.renderer().clone();
        let Some(dialog) = self.export.as_mut() else {
            return;
        };
        self.settings.export = dialog.setup.choices.clone();
        let mut job = dialog
            .setup
            .job(document, destination)
            .with_renderer(renderer);
        if let Some(project) = project {
            job = job.with_project_file(project);
        }
        let label = format!("{} as {}", job_name(&job.destination), job.format.label());
        dialog.stage = Stage::Running {
            run: ExportRun::start(job, window.cloned()),
            label,
        };
    }

    /// How the running export ended, once it has: the dialog moves on to
    /// say so. [`show`](Shell::show) calls this every frame.
    pub fn poll_export(&mut self) -> Option<RunOutcome> {
        let dialog = self.export.as_mut()?;
        let Stage::Running { run, .. } = &dialog.stage else {
            return None;
        };
        let outcome = run.outcome()?;
        dialog.stage = match &outcome {
            RunOutcome::Done(exported) => Stage::Done(exported.clone()),
            RunOutcome::Cancelled => {
                self.toasts.push(Toast::info("Export cancelled"));
                Stage::Choosing
            }
            RunOutcome::Failed(message) => Stage::Failed(sentence(message)),
        };
        Some(outcome)
    }

    /// Stops the running export, if there is one.
    pub fn cancel_export(&mut self) {
        if let Some(ExportDialog {
            stage: Stage::Running { run, .. },
            ..
        }) = &self.export
        {
            run.cancel();
        }
    }

    /// The Export dialog, while it is open.
    pub(super) fn export_dialog(&mut self, ctx: &Context) {
        self.poll_export();
        let document = Arc::clone(self.editor.document());
        let project_name = self.project_name();
        let Some(dialog) = self.export.as_mut() else {
            return;
        };
        let mut action = None;
        let shown = ModalFrame::new("Export").width(440.0).show(ctx, |ui| {
            action = match &mut dialog.stage {
                Stage::Choosing => choosing(ui, &mut dialog.setup, &dialog.availability, &document),
                Stage::Running { run, label } => running(ui, run, label),
                Stage::Done(exported) => done(ui, exported),
                Stage::Failed(message) => failed(ui, message),
            };
        });
        let running = matches!(dialog.stage, Stage::Running { .. });
        let format = dialog.setup.choices.format();
        let artboard = dialog.setup.artboard.clone();
        if shown.close_requested && action.is_none() {
            action = Some(if running {
                Action::Cancel
            } else {
                Action::Close
            });
        }
        match action {
            None => {}
            Some(Action::Close) => {
                self.cancel_export();
                self.export = None;
            }
            Some(Action::Cancel) => self.cancel_export(),
            Some(Action::Back) => {
                if let Some(dialog) = self.export.as_mut() {
                    dialog.stage = Stage::Choosing;
                }
            }
            Some(Action::Reveal(path)) => {
                if let Err(error) = reveal(&path) {
                    self.toasts.push(Toast::error(format!(
                        "The file could not be shown: {error}"
                    )));
                }
            }
            Some(Action::Export) => {
                let name = document
                    .get(&artboard)
                    .map(|node| node.name().trim().to_owned())
                    .filter(|name| !name.is_empty())
                    .unwrap_or(project_name);
                if let Some(destination) = self.dialogs.export_to(&name, format) {
                    self.start_export(destination, Some(ctx));
                }
            }
        }
    }
}

/// What a button in the dialog asked for.
enum Action {
    Export,
    Cancel,
    Back,
    Close,
    Reveal(PathBuf),
}

/// The choices: artboard, format, size, frame rate, quality, background,
/// time range and GIF loop, with what will be written.
fn choosing(
    ui: &mut Ui,
    setup: &mut ExportSetup,
    availability: &[Availability],
    document: &moshionly_core::Document,
) -> Option<Action> {
    let theme = Theme::of(ui);
    let c = &theme.color;
    let mut action = None;

    let artboards: Vec<NodeId> = document.artboards().to_vec();
    if artboards.len() > 1 {
        let names: Vec<String> = artboards
            .iter()
            .map(|id| document.get(id).map(|n| n.label()).unwrap_or_default())
            .collect();
        let options: Vec<(usize, &str)> = names
            .iter()
            .enumerate()
            .map(|(index, name)| (index, name.as_str()))
            .collect();
        let mut index = artboards
            .iter()
            .position(|id| *id == setup.artboard)
            .unwrap_or(0);
        property_row(ui, "Artboard", |ui| {
            if Dropdown::new(&mut index, "Artboard", &options)
                .show(ui)
                .changed()
            {
                setup.artboard = artboards[index].clone();
                setup.range = None;
            }
        });
    }

    let formats: Vec<(Format, &str)> = availability
        .iter()
        .filter(|a| a.is_available())
        .map(|a| (a.format, a.format.label()))
        .collect();
    let mut format = setup.choices.format();
    property_row(ui, "Format", |ui| {
        if Dropdown::new(&mut format, "Format", &formats)
            .show(ui)
            .changed()
        {
            setup.choices.set_format(format);
        }
    });
    for missing in availability.iter().filter_map(|a| a.missing.as_deref()) {
        ui.label(
            RichText::new(missing)
                .font(theme.small())
                .color(c.text_tertiary),
        );
    }

    let mut scale = SCALES
        .iter()
        .position(|(value, _)| (*value - setup.choices.scale()).abs() < 1e-9)
        .unwrap_or(1);
    let scale_options: Vec<(usize, &str)> = SCALES
        .iter()
        .enumerate()
        .map(|(i, (_, label))| (i, *label))
        .collect();
    property_row(ui, "Size", |ui| {
        if segmented(ui, &mut scale, &scale_options).changed() {
            setup.choices.scale = SCALES[scale].0;
        }
    });

    if format.is_animated() {
        let mut rate = FRAME_RATES
            .iter()
            .position(|(fps, _)| (*fps - setup.choices.fps).abs() < 1e-9)
            .unwrap_or(4);
        let rate_options: Vec<(usize, &str)> = FRAME_RATES
            .iter()
            .enumerate()
            .map(|(i, (_, label))| (i, *label))
            .collect();
        property_row(ui, "Frame rate", |ui| {
            if Dropdown::new(&mut rate, "Frame rate", &rate_options)
                .show(ui)
                .changed()
            {
                setup.choices.fps = FRAME_RATES[rate].0;
            }
        });
    }

    if format.draws_pixels() && format.is_animated() && format != Format::PngSequence {
        let mut quality = setup.choices.quality();
        property_row(ui, "Quality", |ui| {
            if segmented(
                ui,
                &mut quality,
                &[
                    (Quality::Draft, "Draft"),
                    (Quality::Standard, "Standard"),
                    (Quality::High, "High"),
                ],
            )
            .changed()
            {
                setup.choices.set_quality(quality);
            }
        });
    }

    if format == Format::Gif {
        let mut gif_loop = setup.choices.gif_loop;
        property_row(ui, "Plays", |ui| {
            if Dropdown::new(&mut gif_loop, "Plays", &GifLoop::OFFERED)
                .show(ui)
                .changed()
            {
                setup.choices.gif_loop = gif_loop;
            }
        });
    }

    ui.add_enabled_ui(format.supports_transparency(), |ui| {
        let mut transparent = setup.choices.transparent && format.supports_transparency();
        if ui
            .checkbox(
                &mut transparent,
                RichText::new("Transparent background").font(theme.body()),
            )
            .changed()
        {
            setup.choices.transparent = transparent;
        }
    });

    if format.is_animated() {
        let length = moshionly_core::eval::duration_of(document, &setup.artboard).unwrap_or(0.0);
        let mut part = setup.range.is_some();
        if ui
            .checkbox(
                &mut part,
                RichText::new("Only part of the scene").font(theme.body()),
            )
            .changed()
        {
            setup.range = part.then_some((0.0, length));
        }
        if let Some((start, end)) = &mut setup.range {
            let (mut from, mut to) = (*start / 1000.0, *end / 1000.0);
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("From")
                        .font(theme.body())
                        .color(c.text_secondary),
                );
                ui.add(
                    DragValue::new(&mut from)
                        .range(0.0..=(length / 1000.0))
                        .speed(0.05)
                        .suffix(" s"),
                );
                ui.label(
                    RichText::new("to")
                        .font(theme.body())
                        .color(c.text_secondary),
                );
                ui.add(
                    DragValue::new(&mut to)
                        .range(0.0..=(length / 1000.0).max(from))
                        .speed(0.05)
                        .suffix(" s"),
                );
            });
            *start = from * 1000.0;
            *end = to * 1000.0;
        }
    } else {
        ui.checkbox(
            &mut setup.choices.still_at_playhead,
            RichText::new("The moment at the playhead, rather than the design at rest")
                .font(theme.body()),
        );
    }

    let plan = setup.settings().plan(format, document, &setup.artboard);
    let ready = plan.is_ok();
    match plan {
        Ok(plan) => {
            let mut summary = format!("{} × {} px", plan.width, plan.height);
            if let Some(duration) = plan.duration_ms() {
                summary.push_str(&format!(
                    " · {} frames · {:.1} s",
                    plan.frame_count(),
                    duration / 1000.0
                ));
            }
            ui.label(
                RichText::new(summary)
                    .font(theme.body())
                    .color(c.text_secondary),
            );
        }
        Err(error) => {
            ui.label(
                RichText::new(sentence(&error.to_string()))
                    .font(theme.body())
                    .color(c.danger),
            );
        }
    }

    modal_footer(ui, |ui| {
        if ui
            .add_enabled(ready, TextButton::primary("Export…"))
            .clicked()
        {
            action = Some(Action::Export);
        }
        if ui.add(TextButton::new("Cancel")).clicked() {
            action = Some(Action::Close);
        }
    });
    action
}

/// The progress bar with Cancel.
fn running(ui: &mut Ui, run: &ExportRun, label: &str) -> Option<Action> {
    let theme = Theme::of(ui);
    let c = &theme.color;
    let mut action = None;
    ui.label(
        RichText::new(format!("Exporting {label}…"))
            .font(theme.body_strong())
            .color(c.text),
    );
    let progress = run.progress();
    let fraction = progress.map_or(0.0, |p| p.fraction()) as f32;
    let words = match progress {
        _ if run.is_cancelling() => "Stopping…".to_owned(),
        None => "Getting ready…".to_owned(),
        Some(p) if p.phase == Phase::Preparing => "Getting ready…".to_owned(),
        Some(p) if matches!(p.phase, Phase::Encoding | Phase::Finishing) => {
            "Finishing the file…".to_owned()
        }
        Some(p) => format!("Frame {} of {}", p.frames_done, p.frames_total),
    };
    ui.add(ProgressBar::new(fraction).desired_width(ui.available_width()));
    ui.label(
        RichText::new(words)
            .font(theme.small())
            .color(c.text_secondary),
    );
    modal_footer(ui, |ui| {
        if ui
            .add_enabled(!run.is_cancelling(), TextButton::new("Cancel"))
            .clicked()
        {
            action = Some(Action::Cancel);
        }
    });
    action
}

/// The file's size, where it is, and Lottie's warnings.
fn done(ui: &mut Ui, exported: &Exported) -> Option<Action> {
    let theme = Theme::of(ui);
    let c = &theme.color;
    let mut action = None;
    ui.label(
        RichText::new(format!(
            "Exported {} ({})",
            job_name(&exported.path),
            file_size(exported.bytes)
        ))
        .font(theme.body_strong())
        .color(c.text),
    );
    ui.label(
        RichText::new(exported.path.display().to_string())
            .font(theme.small())
            .color(c.text_secondary),
    );
    if !exported.warnings.is_empty() {
        ui.label(
            RichText::new("Drawn differently from the design:")
                .font(theme.body_medium())
                .color(c.text),
        );
        ScrollArea::vertical().max_height(140.0).show(ui, |ui| {
            for warning in &exported.warnings {
                ui.label(
                    RichText::new(sentence(warning))
                        .font(theme.small())
                        .color(c.text_secondary),
                );
            }
        });
    }
    modal_footer(ui, |ui| {
        if ui.add(TextButton::primary("Done")).clicked() {
            action = Some(Action::Close);
        }
        let reveal = format!("Show in {}", file_manager_name());
        if ui
            .add(TextButton::new(&reveal).kind(ButtonKind::Quiet))
            .clicked()
        {
            action = Some(Action::Reveal(exported.path.clone()));
        }
    });
    action
}

/// Why the export failed.
fn failed(ui: &mut Ui, message: &str) -> Option<Action> {
    let theme = Theme::of(ui);
    let mut action = None;
    ui.label(
        RichText::new("The export did not finish")
            .font(theme.body_strong())
            .color(theme.color.text),
    );
    ui.label(
        RichText::new(message)
            .font(theme.body())
            .color(theme.color.danger),
    );
    modal_footer(ui, |ui| {
        if ui.add(TextButton::primary("Back")).clicked() {
            action = Some(Action::Back);
        }
        if ui.add(TextButton::new("Close")).clicked() {
            action = Some(Action::Close);
        }
    });
    action
}

/// The file or folder name of an export, in quotes.
fn job_name(path: &std::path::Path) -> String {
    let name = path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    );
    format!("“{name}”")
}

/// A message as a sentence: a capital at the start and a full stop at the
/// end.
fn sentence(message: &str) -> String {
    let message = message.trim();
    let mut chars = message.chars();
    let mut text: String = match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => return String::new(),
    };
    if !text.ends_with(['.', '!', '?']) {
        text.push('.');
    }
    text
}
