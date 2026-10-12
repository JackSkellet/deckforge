//! Persist canvas layout independently of audio configuration and unsaved actions.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap, fs::OpenOptions, io::Read, os::unix::fs::OpenOptionsExt, path::Path,
};
const FILE: &str = "audio-canvas.json";
#[derive(Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
struct Point {
    x: f64,
    y: f64,
}
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
struct Layout {
    positions: BTreeMap<String, Point>,
    split: BTreeMap<String, bool>,
    zoom: f64,
}
impl Default for Layout {
    fn default() -> Self {
        Self {
            positions: BTreeMap::new(),
            split: BTreeMap::new(),
            zoom: 1.,
        }
    }
}
impl Layout {
    fn validate(&self) -> Result<()> {
        ensure!(
            self.zoom.is_finite() && (0.25..=2.).contains(&self.zoom),
            "Invalid canvas zoom"
        );
        ensure!(
            self.positions.len() <= 4096 && self.split.len() <= 4096,
            "Canvas contains too many nodes"
        );
        for key in self.positions.keys().chain(self.split.keys()) {
            ensure!(
                key.len() <= 4096 && !key.chars().any(char::is_control),
                "Invalid canvas node identity"
            );
        }
        for point in self.positions.values() {
            ensure!(
                point.x.is_finite()
                    && point.y.is_finite()
                    && (0.0..=100_000.).contains(&point.x)
                    && (0.0..=100_000.).contains(&point.y),
                "Invalid canvas node position"
            );
        }
        Ok(())
    }
}
pub(super) fn load(state: &Path) -> Value {
    (|| -> Result<Value> {
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(state.join(FILE))?;
        ensure!(
            file.metadata()?.is_file() && file.metadata()?.len() <= 1024 * 1024,
            "Invalid canvas layout file"
        );
        let layout: Layout = serde_json::from_reader(file.take(1024 * 1024 + 1))?;
        layout.validate()?;
        Ok(json!({"formDrafts":{"audio-mixer-canvas":layout}}))
    })()
    .unwrap_or(Value::Null)
}
pub(super) fn save(state: &Path, previous: &Value, next: &Value) -> Result<()> {
    let Some(value) = next
        .get("formDrafts")
        .and_then(|v| v.get("audio-mixer-canvas"))
    else {
        return Ok(());
    };
    if previous
        .get("formDrafts")
        .and_then(|v| v.get("audio-mixer-canvas"))
        == Some(value)
    {
        return Ok(());
    }
    let layout: Layout = serde_json::from_value(value.clone())?;
    layout.validate()?;
    let mut file = tempfile::NamedTempFile::new_in(state)?;
    serde_json::to_writer(file.as_file_mut(), &layout)?;
    file.as_file().sync_all()?;
    file.persist(state.join(FILE))?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn restores_only_layout_and_rejects_invalid_updates_atomically() {
        let root = tempfile::tempdir().unwrap();
        let state = json!({"formDrafts":{"audio-mixer-canvas":{"positions":{"mix:a":{"x":180,"y":94}},"split":{"mix:a":true},"zoom":0.8},"private-action-draft":{"secret":"not a layout"}}});
        save(root.path(), &Value::Null, &state).unwrap();
        let restored = load(root.path());
        assert_eq!(
            serde_json::from_value::<Layout>(restored["formDrafts"]["audio-mixer-canvas"].clone())
                .unwrap(),
            serde_json::from_value::<Layout>(state["formDrafts"]["audio-mixer-canvas"].clone())
                .unwrap()
        );
        assert!(restored["formDrafts"].get("private-action-draft").is_none());
        let mut invalid = state.clone();
        invalid["formDrafts"]["audio-mixer-canvas"]["zoom"] = json!(100);
        assert!(save(root.path(), &state, &invalid).is_err());
        assert_eq!(load(root.path()), restored);
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(root.path().join(FILE))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
}
