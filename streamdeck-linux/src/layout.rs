//! Independent implementation of Elgato's published 200x100 dial layout contract.
//! Sources: https://docs.elgato.com/streamdeck/sdk/guides/dials/ and
//! https://docs.elgato.com/streamdeck/sdk/references/layouts/ .
use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};
use std::collections::HashSet;

/// Built-in geometry specified by the official SDK's Dials & Touch Strip guide.
pub fn builtin(name: &str) -> Result<Value> {
    let title = json!({"key":"title","type":"text","rect":[16,10,136,24],
        "font":{"size":16,"weight":600},"alignment":"left"});
    let icon = json!({"key":"icon","type":"pixmap","rect":[16,40,48,48]});
    let value = json!({"key":"value","type":"text","rect":[76,40,108,32],
        "font":{"size":24,"weight":600},"alignment":"right"});
    let items = match name {
        "$X1" => vec![
            title,
            json!({"key":"icon","type":"pixmap","rect":[76,40,48,48]}),
        ],
        "$A0" => {
            let mut title = title;
            title["zOrder"] = json!(1);
            vec![
                json!({"key":"full-canvas","type":"pixmap","rect":[0,0,200,100]}),
                title,
                json!({"key":"canvas","type":"pixmap","rect":[16,34,136,54],"zOrder":1}),
            ]
        }
        "$A1" => vec![title, icon, value],
        "$B1" => vec![
            title,
            icon,
            value,
            json!({"key":"indicator","type":"bar",
            "rect":[76,74,108,12],"value":0,"subtype":4,"border_w":0}),
        ],
        "$B2" => vec![
            title,
            icon,
            value,
            json!({"key":"indicator","type":"gbar",
            "rect":[76,74,108,20],"value":0,"subtype":4,"bar_h":12,"border_w":0,
            "bar_bg_c":"0:#ff0000,0.33:#a6d4ec,0.66:#f4b675,1:#00ff00"}),
        ],
        "$C1" => vec![
            title,
            json!({"key":"icon1","type":"pixmap","rect":[16,40,24,24]}),
            json!({"key":"icon2","type":"pixmap","rect":[16,68,24,24]}),
            json!({"key":"indicator1","type":"bar","rect":[48,46,136,12],"value":0,"subtype":4,"border_w":0}),
            json!({"key":"indicator2","type":"bar","rect":[48,74,136,12],"value":0,"subtype":4,"border_w":0}),
        ],
        _ => bail!("Unknown built-in feedback layout {name}"),
    };
    Ok(json!({"id":name,"items":items}))
}

/// Validate geometry before allocation or drawing. Same-layer intersections and
/// out-of-bounds items are invalid in Elgato's documented contract.
pub fn validate(layout: &Value) -> Result<()> {
    ensure!(
        layout.get("id").and_then(Value::as_str).is_some(),
        "Layout needs a string id"
    );
    ensure!(
        layout.get("controller").is_none_or(|v| v == "Encoder"),
        "Only Encoder layouts are supported here"
    );
    let items = layout
        .get("items")
        .and_then(Value::as_array)
        .context("Layout needs an items array")?;
    ensure!(items.len() <= 1024, "Layout exceeds 1024 items");
    let mut keys = HashSet::new();
    let mut bounds = Vec::new();
    for item in items {
        let key = item
            .get("key")
            .and_then(Value::as_str)
            .context("Layout item needs a key")?;
        ensure!(
            !key.is_empty() && keys.insert(key),
            "Duplicate or empty layout key {key}"
        );
        ensure!(
            matches!(
                item.get("type").and_then(Value::as_str),
                Some("text" | "pixmap" | "bar" | "gbar")
            ),
            "Unsupported layout item type for {key}"
        );
        let rect = rect(item)?;
        let z = item.get("zOrder").and_then(Value::as_u64).unwrap_or(0);
        ensure!(z <= 700, "Invalid zOrder for {key}");
        ensure!(
            item.get("zOrder").is_none_or(|v| v.as_u64().is_some()),
            "Invalid zOrder for {key}"
        );
        for &(other, other_z) in &bounds {
            if z == other_z {
                let [ox, oy, ow, oh]: [u32; 4] = other;
                let [x, y, w, h] = rect;
                ensure!(
                    x >= ox + ow || ox >= x + w || y >= oy + oh || oy >= y + h,
                    "Layout item {key} overlaps another item at zOrder {z}"
                );
            }
        }
        bounds.push((rect, z));
    }
    Ok(())
}

pub fn rect(item: &Value) -> Result<[u32; 4]> {
    let a = item
        .get("rect")
        .and_then(Value::as_array)
        .context("Layout item needs a rect")?;
    ensure!(a.len() == 4, "Layout rect needs four integers");
    let mut result = [0u32; 4];
    for (out, v) in result.iter_mut().zip(a) {
        *out = v
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .context("Layout rect needs nonnegative integers")?;
    }
    let [x, y, w, h] = result;
    ensure!(
        w > 0 && h > 0 && x <= 200 && y <= 100 && w <= 200 - x && h <= 100 - y,
        "Layout rect is outside the 200x100 dial canvas"
    );
    Ok(result)
}

/// Resolve feedback into item definitions without permitting a plugin to change
/// structural properties (`key`, `type`, `rect`). Unknown feedback keys are ignored.
pub fn resolved_items(layout: &Value, feedback: &Value) -> Result<Vec<Value>> {
    validate(layout)?;
    let mut items = layout["items"].as_array().unwrap().clone();
    for item in &mut items {
        let Some(update) = feedback.get(item["key"].as_str().unwrap()) else {
            continue;
        };
        if let Some(properties) = update.as_object() {
            for (key, value) in properties {
                if matches!(key.as_str(), "key" | "type" | "rect") {
                    continue;
                }
                if matches!(key.as_str(), "font" | "range") && value.is_object() {
                    if !item[key].is_object() {
                        item[key] = json!({});
                    }
                    item[key]
                        .as_object_mut()
                        .unwrap()
                        .extend(value.as_object().unwrap().clone());
                } else {
                    item[key] = value.clone();
                }
            }
        } else if !update.is_null() {
            item["value"] = update.clone();
        }
    }
    let resolved = json!({"id":layout["id"],"items":items});
    validate(&resolved)?;
    items.sort_by_key(|v| v["zOrder"].as_u64().unwrap_or(0));
    Ok(items)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_official_builtins_have_valid_distinct_geometry() {
        for (name, count) in [
            ("$X1", 2),
            ("$A0", 3),
            ("$A1", 3),
            ("$B1", 4),
            ("$B2", 4),
            ("$C1", 5),
        ] {
            let layout = builtin(name).unwrap();
            validate(&layout).unwrap();
            assert_eq!(layout["items"].as_array().unwrap().len(), count);
        }
        assert_eq!(
            builtin("$B2").unwrap()["items"][3]["rect"],
            json!([76, 74, 108, 20])
        );
        assert!(builtin("$NoSuchLayout").is_err());
    }

    #[test]
    fn official_custom_sample_layouts_validate() {
        for input in [
            include_str!("../tests/fixtures/official-layouts/custom-layout-1.json"),
            include_str!("../tests/fixtures/official-layouts/custom-layout-2.json"),
            include_str!("../tests/fixtures/official-layouts/custom-layout-3.json"),
            include_str!("../tests/fixtures/official-layouts/custom-layout-4.json"),
        ] {
            validate(&serde_json::from_str(input).unwrap()).unwrap();
        }
    }

    #[test]
    fn rejects_bad_geometry_but_allows_explicit_layering() {
        let mut layout = json!({"id":"test","items":[
            {"key":"a","type":"pixmap","rect":[0,0,20,20]},
            {"key":"b","type":"pixmap","rect":[10,10,20,20]}
        ]});
        assert!(validate(&layout).is_err());
        layout["items"][1]["zOrder"] = json!(1);
        validate(&layout).unwrap();
        layout["items"][1]["rect"] = json!([190, 0, 20, 20]);
        assert!(validate(&layout).is_err());
        layout["items"][1]["rect"] = json!([4294967295u32, 0, 20, 20]);
        assert!(validate(&layout).is_err());
    }

    #[test]
    fn feedback_preserves_structure_and_merges_nested_defaults() {
        let layout = builtin("$B1").unwrap();
        let items = resolved_items(&layout,&json!({"title":{"rect":[0,0,1,1],"type":"bar","key":"other","font":{"size":12}},"indicator":50})).unwrap();
        assert_eq!(items[0]["font"], json!({"size":12,"weight":600}));
        assert_eq!(items[0]["type"], "text");
        assert_eq!(items[0]["rect"], json!([16, 10, 136, 24]));
        assert_eq!(items[3]["value"], 50);
    }
}
