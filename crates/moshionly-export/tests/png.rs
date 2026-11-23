//! PNG export: the pixels are the renderer's own, and the job behaves.

mod common;

use std::fs;

use moshionly_export::{
    Background, CancelToken, ExportError, Format, FrameRate, Moment, Phase, Settings, SettingsError,
};
use moshionly_render::RenderOptions;

#[test]
fn a_still_is_the_renderers_own_frame() {
    let dir = tempfile::tempdir().unwrap();
    for (moment, still) in [(Moment::AtRest, None), (Moment::At(600.0), Some(600.0))] {
        let path = dir.path().join("still.png");
        let settings = Settings::default().with_still(still);
        let (result, _) = common::run(&common::job(Format::Png, &path, settings));
        let exported = result.unwrap();
        assert_eq!(
            (exported.width, exported.height, exported.frames),
            (1080, 1080, 1)
        );
        assert_eq!(exported.duration_ms, None);
        assert_eq!(exported.bytes, fs::metadata(&path).unwrap().len());
        assert!(exported.warnings.is_empty(), "{:?}", exported.warnings);

        let (width, height, pixels) = common::decode_png(&fs::read(&path).unwrap());
        let expected = common::reference(moment, &RenderOptions::new());
        assert_eq!((width, height), (expected.width(), expected.height()));
        assert!(
            pixels == expected.to_unpremultiplied(),
            "the PNG at {moment:?} differs from the renderer's frame"
        );
    }
}

#[test]
fn a_still_can_be_scaled_and_transparent() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("half.png");
    let settings = Settings::default()
        .with_scale(0.5)
        .with_background(Background::Transparent);
    let (result, _) = common::run(&common::job(Format::Png, &path, settings));
    let exported = result.unwrap();
    assert_eq!((exported.width, exported.height), (540, 540));
    let (_, _, pixels) = common::decode_png(&fs::read(&path).unwrap());
    let expected = common::reference(
        Moment::AtRest,
        &RenderOptions::new()
            .with_scale(0.5)
            .with_background(Background::Transparent),
    );
    assert!(pixels == expected.to_unpremultiplied());
    // The corner has no background behind it.
    assert_eq!(pixels[3], 0, "the top-left pixel is transparent");
}

#[test]
fn exporting_twice_gives_the_same_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let settings = Settings::default().with_still(Some(1200.0));
    let first = dir.path().join("first.png");
    let second = dir.path().join("second.png");
    common::run(&common::job(Format::Png, &first, settings))
        .0
        .unwrap();
    common::run(&common::job(Format::Png, &second, settings))
        .0
        .unwrap();
    assert!(fs::read(first).unwrap() == fs::read(second).unwrap());
}

#[test]
fn progress_runs_from_preparing_to_finishing() {
    let dir = tempfile::tempdir().unwrap();
    let job = common::job(Format::Png, dir.path().join("p.png"), Settings::default());
    let (result, reports) = common::run(&job);
    result.unwrap();
    let phases: Vec<Phase> = reports.iter().map(|report| report.phase).collect();
    assert_eq!(phases.first(), Some(&Phase::Preparing));
    assert_eq!(phases.last(), Some(&Phase::Finishing));
    assert!(phases.contains(&Phase::Rendering));
    assert!(reports.iter().all(|report| report.frames_total == 1));
    assert_eq!(reports.last().unwrap().fraction(), 1.0);
}

#[test]
fn a_cancelled_or_refused_export_leaves_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let job = common::job(Format::Png, dir.path().join("c.png"), Settings::default());
    let cancel = CancelToken::new();
    cancel.cancel();
    let error = job.run(|_| {}, &cancel).unwrap_err();
    assert!(matches!(error, ExportError::Cancelled), "{error}");
    assert!(common::names_in(dir.path()).is_empty());

    let late = Settings::default().with_still(Some(99_000.0));
    let job = common::job(Format::Png, dir.path().join("late.png"), late);
    assert!(matches!(
        job.plan(),
        Err(ExportError::Settings(SettingsError::OutsideArtboard { .. }))
    ));
    let (result, reports) = common::run(&job);
    assert!(matches!(result, Err(ExportError::Settings(_))));
    assert!(reports.is_empty(), "nothing starts with bad settings");
    assert!(common::names_in(dir.path()).is_empty());
}

#[test]
fn a_sequence_holds_every_frame_in_order() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("frames");
    let settings = Settings::default()
        .with_scale(0.25)
        .with_frame_rate(FrameRate::new(10, 1).unwrap())
        .with_range(Some(300.0), Some(1300.0));
    let job = common::job(Format::PngSequence, &folder, settings);
    let (result, reports) = common::run(&job);
    let exported = result.unwrap();
    assert_eq!(exported.frames, 10);
    assert_eq!(exported.duration_ms, Some(1000.0));
    assert_eq!(exported.path, folder);
    let names = common::names_in(&folder);
    assert_eq!(names.len(), 10);
    assert_eq!(names[0], "frame_0000.png");
    assert_eq!(names[9], "frame_0009.png");
    assert_eq!(
        common::names_in(dir.path()),
        ["frames"],
        "no temporary folder is left"
    );

    // Progress counts every frame once, in order.
    let rendered: Vec<u64> = reports
        .iter()
        .filter(|report| report.phase == Phase::Rendering)
        .map(|report| report.frames_done)
        .collect();
    assert_eq!(rendered, (0..=10).collect::<Vec<_>>());

    let options = RenderOptions::new().with_scale(0.25);
    let mut bytes = 0;
    for (index, name) in names.iter().enumerate() {
        let file = fs::read(folder.join(name)).unwrap();
        bytes += file.len() as u64;
        let (width, height, pixels) = common::decode_png(&file);
        assert_eq!((width, height), (270, 270));
        let time = 300.0 + 100.0 * index as f64;
        let expected = common::reference(Moment::At(time), &options);
        assert!(
            pixels == expected.to_unpremultiplied(),
            "frame {index} at {time} ms"
        );
    }
    assert_eq!(exported.bytes, bytes);
}

#[test]
fn a_sequence_stops_when_cancelled_and_leaves_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("frames");
    let settings = Settings::default().with_scale(0.25);
    let job = common::job(Format::PngSequence, &folder, settings);
    let cancel = CancelToken::new();
    let mut last = 0;
    let started = std::time::Instant::now();
    let error = job
        .run(
            |progress| {
                last = progress.frames_done;
                if progress.frames_done == 3 {
                    cancel.cancel();
                }
            },
            &cancel,
        )
        .unwrap_err();
    assert!(matches!(error, ExportError::Cancelled), "{error}");
    assert!(last < 120, "stopped after {last} of 120 frames");
    assert!(started.elapsed().as_secs() < 30);
    assert!(
        common::names_in(dir.path()).is_empty(),
        "nothing is left behind"
    );
}

#[test]
fn a_sequence_never_writes_into_a_folder_with_files() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("keep.txt"), b"mine").unwrap();
    let job = common::job(Format::PngSequence, dir.path(), Settings::default());
    let (result, _) = common::run(&job);
    assert!(matches!(result, Err(ExportError::FolderNotEmpty(_))));
    assert_eq!(common::names_in(dir.path()), ["keep.txt"]);
}

#[test]
fn jobs_and_their_results_can_move_to_another_thread() {
    fn movable<T: Send + Sync + 'static>() {}
    movable::<moshionly_export::ExportJob>();
    movable::<moshionly_export::Exported>();
    movable::<ExportError>();
    movable::<CancelToken>();

    // Run on a worker thread, cancelled from this one.
    let dir = tempfile::tempdir().unwrap();
    let job = common::job(Format::Gif, dir.path().join("t.gif"), Settings::default());
    let cancel = CancelToken::new();
    let worker = {
        let cancel = cancel.clone();
        std::thread::spawn(move || job.run(|_| {}, &cancel))
    };
    cancel.cancel();
    let result = worker.join().unwrap();
    assert!(matches!(result, Err(ExportError::Cancelled)), "{result:?}");
    assert!(common::names_in(dir.path()).is_empty());
}
