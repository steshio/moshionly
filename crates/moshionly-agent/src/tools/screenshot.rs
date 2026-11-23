//! `screenshot`: a picture of an artboard or a layer, so a model can see
//! its work.

use moshionly_core::eval::duration_of;
use moshionly_core::{NodeId, NodeType, Rect};
use serde_json::json;

use super::args::Args;
use super::common::{self, number, rect};
use super::{Context, ToolDef, input_schema};
use crate::host::FrameRequest;
use crate::tool::{ToolError, ToolOutput};

pub(crate) static SCREENSHOT: ToolDef = ToolDef {
    name: "screenshot",
    title: "Take a screenshot",
    description: "Renders an artboard, or the part of it a layer covers, as a PNG image you can \
        look at. Without a time it shows the design at rest, with no animation applied; with \
        \"time\" it shows that moment of the animation, and \"times\" shows up to four moments \
        to check motion. Take one after a visual change to confirm it looks right: text that \
        fits, nothing cut off, colours that read well. Images are small by default (768 px on \
        the long edge); crop or name a layer to see detail.",
    read_only: true,
    destructive: false,
    schema: || {
        input_schema(
            json!({
                "artboard": {
                    "type": "string",
                    "description": "The artboard to render. May be left out when a layer is given or the file has one artboard."
                },
                "layer": {
                    "type": "string",
                    "description": "Show only the part of the artboard this layer covers at that moment, with a little room around it."
                },
                "time": {
                    "type": "number",
                    "minimum": 0,
                    "description": "The moment to render, in milliseconds from the start. Leave out for the design at rest."
                },
                "times": {
                    "type": "array",
                    "items": { "type": "number", "minimum": 0 },
                    "maxItems": MAX_TIMES,
                    "description": "Up to four moments in milliseconds, one image each, to see an animation unfold."
                },
                "crop": {
                    "type": "object",
                    "properties": {
                        "x": { "type": "number" },
                        "y": { "type": "number" },
                        "width": { "type": "number", "exclusiveMinimum": 0 },
                        "height": { "type": "number", "exclusiveMinimum": 0 }
                    },
                    "required": ["x", "y", "width", "height"],
                    "additionalProperties": false,
                    "description": "A region of the artboard to render, in artboard coordinates."
                },
                "max_size": {
                    "type": "integer",
                    "minimum": MIN_SIZE,
                    "maximum": MAX_SIZE,
                    "description": "The longest the image's longer side may be, in pixels. Default 768."
                }
            }),
            &[],
        )
    },
    run,
};

const DEFAULT_SIZE: i64 = 768;
const MIN_SIZE: i64 = 64;
const MAX_SIZE: i64 = 2048;
const MAX_TIMES: usize = 4;

fn run(context: &Context<'_>, args: &Args) -> Result<ToolOutput, ToolError> {
    let Some(frames) = context.host.frames.as_ref() else {
        return Err(ToolError::NotAvailable {
            what: "Screenshots are",
            why: "this host has no renderer attached.",
            instead: "Use evaluate to check positions, sizes, opacity and colours as numbers, and \
                      get_node for the box of a layer.",
        });
    };
    let (document, project_file) = context
        .session
        .read(|session| (session.snapshot(), session.path().map(ToOwned::to_owned)));

    let layer = match args.string("layer")? {
        Some(id) => Some(common::layer(&document, &id, "layer")?.id.clone()),
        None => None,
    };
    let artboard = artboard(&document, args, layer.as_ref())?;
    if layer.as_ref() == Some(&artboard) {
        return Err(ToolError::argument(
            "layer",
            "\"layer\" is the artboard itself. Leave \"layer\" out to see the whole artboard.",
        ));
    }
    let times = times(args)?;
    let crop = crop(args)?;
    let max_size = args
        .integer("max_size", MIN_SIZE..=MAX_SIZE)?
        .unwrap_or(DEFAULT_SIZE) as u32;
    let duration = duration_of(&document, &artboard).unwrap_or_default();

    let mut output = ToolOutput::default();
    let mut shots = Vec::new();
    for time in &times {
        let frame = frames.render(&FrameRequest {
            document: &document,
            project_file: project_file.as_deref(),
            artboard: &artboard,
            layer: layer.as_ref(),
            time_ms: *time,
            crop,
            max_size,
        })?;
        let mut shot = json!({
            "time": time.map_or(json!("at rest"), number),
            "width": frame.width,
            "height": frame.height,
            "region": rect(&frame.region),
        });
        if layer.is_some() {
            shot["layer_box"] = match &frame.layer_box {
                Some(bounds) => rect(bounds),
                None => json!(null),
            };
            if frame.layer_box.is_none() {
                shot["note"] = json!(
                    "The layer is not drawn at this moment: it is hidden, fully transparent, \
                     scaled to nothing or outside the picture."
                );
            }
        }
        shots.push(shot);
        output = output.with_png(frame.png);
    }
    let mut info = json!({
        "artboard": artboard,
        "duration": number(duration),
        "shots": shots,
    });
    if let Some(layer) = &layer {
        info["layer"] = json!(layer);
    }
    if times.iter().flatten().any(|time| *time > duration) {
        info["note"] = json!(format!(
            "A time is past the end of the {duration} ms timeline; the picture shows the \
             animation as it would continue."
        ));
    }
    output.content.insert(
        0,
        crate::tool::ToolContent::Text {
            text: info.to_string(),
        },
    );
    Ok(output)
}

/// The artboard to render: the one named, the one the layer is in, or the
/// only one there is.
fn artboard(
    document: &moshionly_core::Document,
    args: &Args,
    layer: Option<&NodeId>,
) -> Result<NodeId, ToolError> {
    let of_layer = layer
        .and_then(|layer| document.artboard_of(layer))
        .map(|artboard| artboard.id.clone());
    match args.string("artboard")? {
        Some(id) => {
            let artboard = common::artboard(document, &id, "artboard")?.id.clone();
            if let (Some(owner), Some(layer)) = (&of_layer, layer)
                && *owner != artboard
            {
                return Err(ToolError::argument(
                    "artboard",
                    format!(
                        "The layer \"{layer}\" is in the artboard \"{owner}\", not in \
                         \"{artboard}\". Leave \"artboard\" out when giving a layer."
                    ),
                ));
            }
            Ok(artboard)
        }
        None => match (of_layer, document.artboards()) {
            (Some(artboard), _) => Ok(artboard),
            (None, [only]) => Ok(only.clone()),
            (None, []) => Err(ToolError::Rejected(
                "The file has no artboards, so there is nothing to render.".to_owned(),
            )),
            (None, several) => {
                let list: Vec<String> = several
                    .iter()
                    .filter_map(|id| document.get(id))
                    .filter(|node| node.node_type() == NodeType::Artboard)
                    .map(|node| node.label())
                    .collect();
                Err(ToolError::argument(
                    "artboard",
                    format!(
                        "\"artboard\" is required when the file has several: {}.",
                        list.join(", ")
                    ),
                ))
            }
        },
    }
}

/// The moments to render: `None` is the design at rest.
fn times(args: &Args) -> Result<Vec<Option<f64>>, ToolError> {
    let times = match (args.number("time")?, args.numbers("times")?) {
        (Some(_), Some(_)) => {
            return Err(ToolError::argument(
                "times",
                "Give \"time\" for one moment or \"times\" for several, not both.",
            ));
        }
        (Some(time), None) => vec![time],
        (None, Some(times)) if !times.is_empty() => times,
        _ => return Ok(vec![None]),
    };
    if times.len() > MAX_TIMES {
        return Err(ToolError::argument(
            "times",
            format!(
                "\"times\" has {} moments; one call renders at most {MAX_TIMES}. Use evaluate to \
                 check many moments as numbers.",
                times.len()
            ),
        ));
    }
    if let Some(negative) = times.iter().find(|time| **time < 0.0) {
        return Err(ToolError::argument(
            "time",
            format!("Times start at 0 ms, but {negative} was given."),
        ));
    }
    Ok(times.into_iter().map(Some).collect())
}

/// The region to render, when one is given.
fn crop(args: &Args) -> Result<Option<Rect>, ToolError> {
    let Some(value) = args.get("crop") else {
        return Ok(None);
    };
    let crop = Args::nested("crop", value, &["x", "y", "width", "height"])?;
    let mut parts = [0.0; 4];
    for (part, name) in parts.iter_mut().zip(["x", "y", "width", "height"]) {
        *part = crop
            .number(name)?
            .ok_or_else(|| crop.missing(name, "a number in artboard pixels"))?;
    }
    let [x, y, width, height] = parts;
    if width <= 0.0 || height <= 0.0 {
        return Err(ToolError::argument(
            "crop",
            "\"crop\" needs a width and a height above 0.",
        ));
    }
    Ok(Some(Rect::new(x, y, width, height)))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::{Value, json};

    use crate::tools::testing::{StubFrames, call_error, demo_session, demo_tools};
    use crate::{Host, Tools};

    fn tools() -> (Tools, Arc<StubFrames>) {
        let frames = Arc::new(StubFrames::default());
        let tools = Tools::new(demo_session(), Host::new().with_frames(frames.clone()));
        (tools, frames)
    }

    fn info(output: &crate::ToolOutput) -> Value {
        serde_json::from_str(&output.joined_text()).unwrap()
    }

    #[test]
    fn without_a_renderer_it_says_what_to_use_instead() {
        let error = call_error(&demo_tools(), "screenshot", json!({ "artboard": "scene" }));
        assert!(
            error.starts_with("Screenshots are not available in this host"),
            "{error}"
        );
        assert!(error.contains("Use evaluate"), "{error}");
    }

    #[test]
    fn an_artboard_at_rest_with_the_default_size() {
        let (tools, frames) = tools();
        let output = tools.call("screenshot", json!({ "artboard": "scene" }));
        assert!(!output.is_error, "{}", output.joined_text());
        assert_eq!(output.images().count(), 1);
        let info = info(&output);
        assert_eq!(info["artboard"], "scene");
        assert_eq!(info["shots"][0]["time"], "at rest");
        assert_eq!(info["shots"][0]["width"], 768);
        let requests = frames.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].time_ms, None);
        assert_eq!(requests[0].max_size, 768);
        assert_eq!(requests[0].layer, None);
    }

    #[test]
    fn a_layer_at_several_times_finds_its_artboard() {
        let (tools, frames) = tools();
        let output = tools.call(
            "screenshot",
            json!({ "layer": "dot", "times": [600, 900, 1200], "max_size": 256 }),
        );
        assert!(!output.is_error, "{}", output.joined_text());
        assert_eq!(output.images().count(), 3);
        let info = info(&output);
        assert_eq!(info["layer"], "dot");
        assert_eq!(info["shots"][1]["time"], 900);
        assert_eq!(
            info["shots"][0]["layer_box"],
            json!({ "x": 740, "y": 640, "width": 52, "height": 52 })
        );
        let requests = frames.requests();
        let times: Vec<Option<f64>> = requests.iter().map(|request| request.time_ms).collect();
        assert_eq!(times, [Some(600.0), Some(900.0), Some(1200.0)]);
        assert_eq!(requests[0].artboard, "scene");
        assert_eq!(requests[0].layer.as_deref(), Some("dot"));
    }

    #[test]
    fn a_crop_is_passed_on() {
        let (tools, frames) = tools();
        let output = tools.call(
            "screenshot",
            json!({ "artboard": "scene", "time": 5000,
                    "crop": { "x": 0, "y": 0, "width": 540, "height": 540 } }),
        );
        assert!(!output.is_error, "{}", output.joined_text());
        let info = info(&output);
        assert!(info["note"].as_str().unwrap().contains("past the end"));
        let crop = frames.requests()[0].crop.unwrap();
        assert_eq!((crop.width, crop.height), (540.0, 540.0));
    }

    #[test]
    fn mistakes_are_explained() {
        let (tools, _) = tools();
        let error = call_error(&tools, "screenshot", json!({}));
        assert!(
            error.contains("required when the file has several"),
            "{error}"
        );
        let error = call_error(
            &tools,
            "screenshot",
            json!({ "artboard": "second", "layer": "dot" }),
        );
        assert!(error.contains("is in the artboard \"scene\""), "{error}");
        let error = call_error(&tools, "screenshot", json!({ "layer": "dotmove" }));
        assert!(error.contains("It animates the layer \"dot\""), "{error}");
        let error = call_error(
            &tools,
            "screenshot",
            json!({ "artboard": "scene", "times": [0, 1, 2, 3, 4] }),
        );
        assert!(error.contains("at most 4"), "{error}");
        let error = call_error(
            &tools,
            "screenshot",
            json!({ "artboard": "scene", "crop": { "x": 0, "y": 0, "width": 0, "height": 5 } }),
        );
        assert!(error.contains("above 0"), "{error}");
        let error = call_error(
            &tools,
            "screenshot",
            json!({ "artboard": "scene", "max_size": 10 }),
        );
        assert!(error.contains("from 64 to 2048"), "{error}");
    }

    #[test]
    fn a_renderer_failure_is_reported() {
        let (tools, frames) = tools();
        frames.fail_with("the GPU is on fire");
        let error = call_error(&tools, "screenshot", json!({ "artboard": "scene" }));
        assert_eq!(error, "the GPU is on fire");
    }
}
