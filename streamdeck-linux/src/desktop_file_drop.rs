//! Normalize native GTK file drag events. GTK coordinates are widget logical
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

/// WebKit uses GTK target-info 2 for its serialized custom drag data too. Wry's
/// generic file handler treats every such payload as URIs and finishes the drag,
/// stealing internal DOM moves. Claim only external, explicitly typed URI lists.
pub fn connect(view: &webkit2gtk::WebView, handler: impl Fn(DragDropEvent) + 'static) {
    use gtk::{gdk, prelude::*};
    use std::{cell::RefCell, path::PathBuf, rc::Rc};
    #[derive(Default)]
    struct Drag {
        context: Option<gdk::DragContext>,
        paths: Option<Vec<PathBuf>>,
        position: (i32, i32),
        drop_time: Option<u32>,
    }
    fn external(view: &webkit2gtk::WebView, ctx: &gdk::DragContext) -> bool {
        ctx.drag_get_source_widget().as_ref() != Some(view.upcast_ref::<gtk::Widget>())
            && ctx
                .list_targets()
                .contains(&gdk::Atom::intern("text/uri-list"))
    }
    let drag = Rc::new(RefCell::new(Drag::default()));
    let handler: Rc<dyn Fn(DragDropEvent)> = Rc::new(handler);
    let state = drag.clone();
    let notify = handler.clone();
    view.connect_drag_motion(move |view, ctx, x, y, time| {
        if !external(view, ctx) {
            return false;
        }
        let mut state = state.borrow_mut();
        let request = state.context.as_ref() != Some(ctx);
        if request {
            *state = Drag {
                context: Some(ctx.clone()),
                ..Default::default()
            };
        }
        state.position = (x, y);
        let entered = state.paths.is_some();
        drop(state);
        ctx.drag_status(gdk::DragAction::COPY, time);
        if request {
            view.drag_get_data(ctx, &gdk::Atom::intern("text/uri-list"), time);
        }
        if entered {
            notify(DragDropEvent::Over { position: (x, y) });
        }
        true
    });
    let state = drag.clone();
    let notify = handler.clone();
    view.connect_drag_drop(move |view, ctx, x, y, time| {
        if !external(view, ctx) {
            return false;
        }
        let mut state = state.borrow_mut();
        if state.context.as_ref() != Some(ctx) {
            *state = Drag {
                context: Some(ctx.clone()),
                ..Default::default()
            };
        }
        state.position = (x, y);
        if let Some(paths) = state.paths.take() {
            *state = Drag::default();
            drop(state);
            ctx.drag_finish(true, false, time);
            notify(DragDropEvent::Drop {
                paths,
                position: (x, y),
            });
        } else {
            state.drop_time = Some(time);
            drop(state);
            view.drag_get_data(ctx, &gdk::Atom::intern("text/uri-list"), time);
        }
        true
    });
    let state = drag.clone();
    let notify = handler.clone();
    view.connect_drag_data_received(move |view, ctx, _, _, data, _, _| {
        if !external(view, ctx) || data.target() != gdk::Atom::intern("text/uri-list") {
            return;
        }
        // This response belongs to our URI request, not WebKit's DOM handling.
        view.stop_signal_emission_by_name("drag-data-received");
        let mut state = state.borrow_mut();
        if state.context.as_ref() != Some(ctx) {
            return;
        }
        let paths = data
            .uris()
            .iter()
            .map(|uri| {
                url::Url::parse(uri)
                    .ok()
                    .and_then(|url| url.to_file_path().ok())
                    .unwrap_or_else(|| PathBuf::from(uri.as_str()))
            })
            .collect::<Vec<_>>();
        let position = state.position;
        if let Some(time) = state.drop_time.take() {
            *state = Drag::default();
            drop(state);
            ctx.drag_finish(true, false, time);
            notify(DragDropEvent::Drop { paths, position });
        } else {
            state.paths = Some(paths.clone());
            drop(state);
            notify(DragDropEvent::Enter { paths, position });
        }
    });
    view.connect_drag_leave(move |_, ctx, _| {
        // GTK emits leave immediately before drop. Give drop its turn before
        // clearing the cached external file, including asynchronous URI replies.
        let state = drag.clone();
        let notify = handler.clone();
        let ctx = ctx.clone();
        gtk::glib::idle_add_local_once(move || {
            let mut state = state.borrow_mut();
            if state.context.as_ref() == Some(&ctx) && state.drop_time.is_none() {
                *state = Drag::default();
                drop(state);
                notify(DragDropEvent::Leave);
            }
        });
    });
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
