//! An export running on a thread of its own, so the window stays
//! responsive: its progress, a way to cancel it, and how it ended.

use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;

use moshionly_export::{CancelToken, ExportError, ExportJob, Exported, Progress};

/// How an export ended.
#[derive(Clone, Debug, PartialEq)]
pub enum RunOutcome {
    /// The file was written.
    Done(Exported),
    /// The person cancelled; nothing was left behind.
    Cancelled,
    /// It failed, for this reason.
    Failed(String),
}

#[derive(Default)]
struct Shared {
    progress: Option<Progress>,
    outcome: Option<RunOutcome>,
}

/// An export running in the background. Dropping it cancels the export
/// and waits for it to stop, so nothing half-written is left behind.
pub struct ExportRun {
    cancel: CancelToken,
    shared: Arc<Mutex<Shared>>,
    /// The export's thread, until it is joined.
    thread: Option<JoinHandle<()>>,
}

impl ExportRun {
    /// Starts `job` on a thread of its own. `window`, when given, is asked
    /// to repaint as the export moves on and when it ends.
    pub fn start(job: ExportJob, window: Option<egui::Context>) -> Self {
        let cancel = CancelToken::new();
        let shared = Arc::new(Mutex::new(Shared::default()));
        let mut run = Self {
            cancel: cancel.clone(),
            shared: Arc::clone(&shared),
            thread: None,
        };
        let repaint = move || {
            if let Some(window) = &window {
                window.request_repaint();
            }
        };
        let spawned = std::thread::Builder::new()
            .name("moshionly-export".into())
            .spawn({
                let shared = Arc::clone(&shared);
                move || {
                    let result = job.run(
                        |progress| {
                            lock(&shared).progress = Some(progress);
                            repaint();
                        },
                        &cancel,
                    );
                    lock(&shared).outcome = Some(match result {
                        Ok(exported) => RunOutcome::Done(exported),
                        Err(ExportError::Cancelled) => RunOutcome::Cancelled,
                        Err(error) => RunOutcome::Failed(error.to_string()),
                    });
                    repaint();
                }
            });
        match spawned {
            Ok(thread) => run.thread = Some(thread),
            Err(error) => {
                lock(&shared).outcome = Some(RunOutcome::Failed(format!(
                    "the export could not start: {error}"
                )));
            }
        }
        run
    }

    /// Asks the export to stop. It stops within a frame or so and leaves
    /// nothing behind.
    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    /// Whether Cancel was asked for.
    pub fn is_cancelling(&self) -> bool {
        self.cancel.is_cancelled()
    }

    /// How far the export has come, once it reported.
    pub fn progress(&self) -> Option<Progress> {
        lock(&self.shared).progress
    }

    /// How the export ended, once it has; `None` while it runs.
    pub fn outcome(&self) -> Option<RunOutcome> {
        lock(&self.shared).outcome.clone()
    }

    /// Waits for the export to end, for tests and the command line.
    pub fn wait(&self, limit: std::time::Duration) -> Option<RunOutcome> {
        let deadline = std::time::Instant::now() + limit;
        loop {
            if let Some(outcome) = self.outcome() {
                return Some(outcome);
            }
            if std::time::Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
}

impl Drop for ExportRun {
    /// Cancels the export if it still runs, and waits the frame or so it
    /// takes to stop and delete its unfinished file.
    fn drop(&mut self) {
        self.cancel.cancel();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn lock(shared: &Mutex<Shared>) -> std::sync::MutexGuard<'_, Shared> {
    shared.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::Duration;

    use moshionly_core::{NodeId, io};
    use moshionly_export::Format;
    use moshionly_render::{AssetLoader, FontLibrary, Renderer};

    use super::*;
    use crate::export::{ExportChoices, ExportSetup};

    fn folder(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "moshionly-app-{label}-{}-{}",
            std::process::id(),
            NodeId::generate()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn job(format: Format, scale: f64, destination: PathBuf) -> ExportJob {
        let document = std::sync::Arc::new(
            io::load(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../moshionly-core/examples/demo.moshi"
            ))
            .unwrap(),
        );
        let mut choices = ExportChoices {
            scale,
            fps: 12.0,
            ..ExportChoices::default()
        };
        choices.set_format(format);
        ExportSetup {
            choices,
            artboard: NodeId::from("scene"),
            range: None,
            playhead: 0.0,
        }
        .job(document, destination)
        .with_renderer(Renderer::new(
            FontLibrary::bundled(),
            AssetLoader::default(),
        ))
    }

    #[test]
    fn an_export_runs_in_the_background_and_reports_its_file() {
        let dir = folder("run");
        let run = ExportRun::start(job(Format::Gif, 0.25, dir.join("demo.gif")), None);
        let outcome = run.wait(Duration::from_secs(60)).expect("it finishes");
        let RunOutcome::Done(exported) = outcome else {
            panic!("expected a file, got {outcome:?}");
        };
        assert!(exported.bytes > 0);
        assert!(
            std::fs::read(&exported.path)
                .unwrap()
                .starts_with(b"GIF89a")
        );
        let progress = run.progress().expect("progress was reported");
        assert_eq!(progress.frames_done, progress.frames_total);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_cancelled_export_leaves_nothing_behind() {
        let dir = folder("cancel");
        let destination = dir.join("demo.mp4");
        let run = ExportRun::start(job(Format::Mp4, 2.0, destination.clone()), None);
        run.cancel();
        assert!(run.is_cancelling());
        let outcome = run.wait(Duration::from_secs(60)).expect("it stops");
        assert_eq!(outcome, RunOutcome::Cancelled);
        assert!(!destination.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn dropping_a_running_export_stops_it_and_leaves_nothing_behind() {
        let dir = folder("drop");
        let run = ExportRun::start(job(Format::Mp4, 2.0, dir.join("demo.mp4")), None);
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        while run.progress().is_none_or(|p| p.frames_done == 0) {
            assert!(std::time::Instant::now() < deadline, "it never started");
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(run.outcome(), None, "still running when dropped");
        drop(run);
        let left: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|entry| entry.file_name())
            .collect();
        assert!(left.is_empty(), "nothing is left behind: {left:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
