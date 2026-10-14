//! Normalize Wry's GTK file drag events. GTK coordinates are widget logical
//! coordinates; only WebKit page zoom needs conversion to CSS coordinates.
use serde_json::{Value, json};
use wry::DragDropEvent;

pub fn payload(event: &DragDropEvent, zoom: f64) -> Value {
    let (phase, paths, position) = match event {
        DragDropEvent::Enter { paths, position } => ("enter", Some(paths), Some(*position)),
        DragDropEvent::Over { position } => ("over", None, Some(*position)),
        DragDropEvent::Drop { paths, position } => ("drop", Some(paths), Some(*position)),
        _ => ("leave", None, None),
    };
    let mut result = json!({"phase":phase});
    if let Some((x, y)) = position {
        let zoom = if zoom.is_finite() && zoom > 0.0 {
            zoom
        } else {
            1.0
        };
        result["x"] = json!(f64::from(x) / zoom);
        result["y"] = json!(f64::from(y) / zoom);
    }
    if let Some(paths) = paths {
        if paths.len() != 1 {
            result["error"] = json!("Drop one Stream Deck file at a time.");
            return result;
        }
        let Some(path) = paths[0]
            .to_str()
            .filter(|p| p.len() <= 8192 && !p.chars().any(char::is_control))
        else {
            result["error"] = json!("The dropped file path is not supported.");
            return result;
        };
        if !paths[0].is_absolute() {
            result["error"] = json!("Drop a local file from your file manager.");
            return result;
        }
        match crate::file_open::kind(&paths[0]) {
            Ok(kind) => {
                result["path"] = json!(path);
                result["kind"] = json!(match kind {
                    crate::file_open::Kind::Action => "action",
                    crate::file_open::Kind::Profile => "profile",
                    crate::file_open::Kind::Plugin => "plugin",
                    crate::file_open::Kind::IconPack => "icon_pack",
                });
            }
            Err(_) => {
                result["error"] =
                    json!("Drop a Stream Deck action, profile, plugin or icon-pack file.")
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_paths_stay_literal_and_gtk_positions_follow_page_zoom() {
        let event = DragDropEvent::Drop {
            paths: vec!["/tmp/name %20 #.streamDeckAction".into()],
            position: (200, 80),
        };
        assert_eq!(
            payload(&event, 2.0),
            json!({"phase":"drop","path":"/tmp/name %20 #.streamDeckAction","kind":"action","x":100.0,"y":40.0})
        );
        assert_eq!(
            payload(&DragDropEvent::Leave, 1.0),
            json!({"phase":"leave"})
        );
        for (path, kind) in [
            ("/tmp/p.streamDeckPlugin", "plugin"),
            ("/tmp/i.streamDeckIconPack", "icon_pack"),
        ] {
            let value = payload(
                &DragDropEvent::Drop {
                    paths: vec![path.into()],
                    position: (1, 2),
                },
                1.0,
            );
            assert_eq!(value["kind"], kind);
            assert_eq!(value["path"], path);
        }
        for paths in [
            vec![],
            vec!["/a.streamDeckAction".into(), "/b.streamDeckAction".into()],
            vec!["http://remote/a.streamDeckAction".into()],
            vec!["/a.png".into()],
        ] {
            let value = payload(
                &DragDropEvent::Drop {
                    paths,
                    position: (0, 0),
                },
                1.0,
            );
            assert!(value["error"].is_string());
            assert!(value.get("path").is_none());
        }
    }
}
