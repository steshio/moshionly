//! PNG export: the pixels are the renderer's own, and the job behaves.

mod common;

use std::fs;

use moshionly_export::{
    Background, CancelToken, ExportError, Format, Moment, Phase, Settings, SettingsError,
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
