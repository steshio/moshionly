//! `list_fonts`: the font families text can be set in.

use serde_json::{Value, json};

use super::args::Args;
use super::overview::fonts_in_use;
use super::{Context, ToolDef, input_schema};
use crate::tool::{ToolError, ToolOutput};

pub(crate) static LIST_FONTS: ToolDef = ToolDef {
    name: "list_fonts",
    title: "List fonts",
    description: "Lists the font families text layers can use, with their weights and whether \
        they have italics, and the families the file already uses. A family that is not \
        available is drawn with a fallback, so pick from this list when setting \"font\" on a \
        text layer. Narrow a long list with \"query\".",
    read_only: true,
    schema: || {
        input_schema(
            json!({
                "query": {
                    "type": "string",
                    "description": "Only families whose name contains this, ignoring case, for example \"mono\" or \"serif\"."
                },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": MAX_LIMIT,
                    "description": "The most families to list. Default 40."
                }
            }),
            &[],
        )
    },
    run,
};

const DEFAULT_LIMIT: i64 = 40;
const MAX_LIMIT: i64 = 500;

fn run(context: &Context<'_>, args: &Args) -> Result<ToolOutput, ToolError> {
    let query = args
        .string("query")?
        .map(|query| query.trim().to_lowercase());
    let limit = args
        .integer("limit", 1..=MAX_LIMIT)?
        .unwrap_or(DEFAULT_LIMIT) as usize;
    let document = context.session.document();
    let in_use = fonts_in_use(&document);
    let mut result = json!({ "in_use": in_use });

    let Some(source) = context.host.fonts.as_ref() else {
        result["note"] = json!(
            "This host cannot list the fonts it has. The families the file already uses are \
             above; Inter, the default family, is the safe choice."
        );
        return Ok(ToolOutput::json(&result));
    };
    let families = source.families();
    let matching: Vec<_> = families
        .iter()
        .filter(|font| {
            query
                .as_deref()
                .is_none_or(|query| font.family.to_lowercase().contains(query))
        })
        .collect();
    let listed: Vec<Value> = matching
        .iter()
        .take(limit)
        .map(
            |font| json!({ "family": font.family, "weights": font.weights, "italic": font.italic }),
        )
        .collect();
    result["matched"] = json!(matching.len());
    result["available"] = json!(listed);
    if matching.len() > limit {
        result["note"] = json!(format!(
            "The first {limit} of {} families. Narrow the list with \"query\".",
            matching.len()
        ));
    }
    let missing: Vec<&str> = in_use
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|font| font["family"].as_str())
        .filter(|family| {
            !families
                .iter()
                .any(|font| font.family.eq_ignore_ascii_case(family))
        })
        .collect();
    if !missing.is_empty() {
        result["missing"] = json!(missing);
        result["missing_note"] = json!(
            "The file uses these families, but this machine does not have them, so their text \
             is drawn with a fallback."
        );
    }
    Ok(ToolOutput::json(&result))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::json;

    use crate::host::{FontFamily, FontSource};
    use crate::tools::testing::{call, demo_session, demo_tools};
    use crate::{Host, Tools};

    struct Fonts(Vec<&'static str>);

    impl FontSource for Fonts {
        fn families(&self) -> Vec<FontFamily> {
            self.0
                .iter()
                .map(|family| FontFamily {
                    family: (*family).to_owned(),
                    weights: vec![400, 700],
                    italic: false,
                })
                .collect()
        }
    }

    fn tools(families: Vec<&'static str>) -> Tools {
        Tools::new(
            demo_session(),
            Host::new().with_fonts(Arc::new(Fonts(families))),
        )
    }

    #[test]
    fn lists_and_filters_the_host_fonts() {
        let tools = tools(vec!["Inter", "JetBrains Mono", "Space Mono", "Lora"]);
        let result = call(&tools, "list_fonts", json!({ "query": "MONO" }));
        assert_eq!(result["matched"], 2);
        assert_eq!(
            result["available"],
            json!([
                { "family": "JetBrains Mono", "weights": [400, 700], "italic": false },
                { "family": "Space Mono", "weights": [400, 700], "italic": false },
            ])
        );
        assert_eq!(result["in_use"][0]["family"], "Inter");
        assert!(result.get("missing").is_none());

        let result = call(&tools, "list_fonts", json!({ "limit": 1 }));
        assert_eq!(result["available"].as_array().unwrap().len(), 1);
        assert!(result["note"].as_str().unwrap().contains("first 1 of 4"));
    }

    #[test]
    fn fonts_the_file_uses_but_the_host_lacks_are_named() {
        let result = call(&tools(vec!["Lora"]), "list_fonts", json!({}));
        assert_eq!(result["missing"], json!(["Inter"]));
    }

    #[test]
    fn without_a_font_source_the_fonts_in_use_still_come() {
        let result = call(&demo_tools(), "list_fonts", json!({}));
        assert_eq!(result["in_use"][0]["family"], "Inter");
        assert!(result.get("available").is_none());
        assert!(result["note"].as_str().unwrap().contains("cannot list"));
    }
}
