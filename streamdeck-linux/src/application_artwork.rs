//! Derived Open Application icons and running feedback. Never saved in profiles.
use crate::{application_monitor::Snapshot, system_app::Config};
use gtk::gio::{self, prelude::*};
use image::{DynamicImage, GenericImage, Rgba};
use serde_json::Value;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

pub type Requests = Vec<(String, Value)>;
pub type Artwork = HashMap<String, Arc<Visual>>;
#[derive(Debug)]
pub struct Visual {
    pub settings: Value,
    pub running: Option<bool>,
    pub icon: Option<Arc<DynamicImage>>,
}
impl Visual {
    pub fn same(&self, other: &Self) -> bool {
        self.settings == other.settings
            && self.running == other.running
            && match (&self.icon, &other.icon) {
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                (None, None) => true,
                _ => false,
            }
    }
}
struct Entry {
    settings: Value,
    config: Option<Config>,
    icon: Option<Arc<DynamicImage>>,
}
#[derive(Default)]
pub struct Cache {
    entries: HashMap<String, Entry>,
}
impl Cache {
    pub fn sample(&mut self, requests: &Requests, snapshot: &Snapshot, root: &Path) -> Artwork {
        self.entries
            .retain(|context, _| requests.iter().any(|(id, _)| id == context));
        requests
            .iter()
            .take(512)
            .map(|(context, settings)| {
                let entry = self
                    .entries
                    .entry(context.clone())
                    .or_insert_with(|| build(settings));
                if entry.settings != *settings {
                    *entry = build(settings);
                }
                let running = entry.config.as_ref().and_then(|config| {
                    // The shared observer retains unreadable known identities. An unrelated
                    // unreadable process must not suppress every application indicator.
                    let mut unknown = false;
                    for process in &snapshot.processes {
                        if process.executable != config.executable {
                            continue;
                        }
                        match crate::application_monitor::matches_arguments(
                            root,
                            process,
                            &config.process_args,
                        ) {
                            Some(true) => return Some(true),
                            Some(false) => {}
                            None => unknown = true,
                        }
                    }
                    (!unknown).then_some(false)
                });
                (
                    context.clone(),
                    Arc::new(Visual {
                        settings: settings.clone(),
                        running,
                        icon: entry.icon.clone(),
                    }),
                )
            })
            .collect()
    }
}
fn build(settings: &Value) -> Entry {
    let config = Config::parse(settings).ok().and_then(|mut c| {
        c.executable = c.executable.canonicalize().ok()?;
        Some(c)
    });
    let icon = config.as_ref().and_then(icon_path).and_then(|path| {
        let path = path.canonicalize().ok()?;
        let bytes = crate::action_icons::png(path.parent()?, path.file_name()?.to_str()?).ok()?;
        image::load_from_memory(&bytes).ok().map(Arc::new)
    });
    Entry {
        settings: settings.clone(),
        config,
        icon,
    }
}
fn icon_path(config: &Config) -> Option<PathBuf> {
    let app = if let Some(path) = &config.desktop {
        gio::DesktopAppInfo::from_filename(path)?.upcast::<gio::AppInfo>()
    } else {
        let mut apps = gio::AppInfo::all().into_iter().filter(|app| {
            app.should_show()
                && crate::system_app::desktop_command(app)
                    .ok()
                    .and_then(|(exe, _)| crate::system_actions::find_program(&exe).ok())
                    .and_then(|path| path.canonicalize().ok())
                    .as_ref()
                    == Some(&config.executable)
        });
        let app = apps.next()?;
        // Shared interpreters must not pick an unrelated application's icon.
        if apps.next().is_some() {
            return None;
        }
        app
    };
    let icon = app.icon()?;
    if let Some(file) = icon.downcast_ref::<gio::FileIcon>() {
        return file.file().path();
    }
    icon.downcast_ref::<gio::ThemedIcon>()?
        .names()
        .iter()
        .find_map(|name| freedesktop_icons::lookup(name).with_size(128).find())
}
/// Deckforge's own running indicator, not a redistributed vendor overlay.
pub fn mark(image: &mut DynamicImage) {
    let radius = (image.width().min(image.height()) / 24).max(2) as i32;
    let center = radius * 2 + 1;
    for y in -radius - 1..=radius + 1 {
        for x in -radius - 1..=radius + 1 {
            let distance = x * x + y * y;
            let color = if distance <= radius * radius {
                Rgba([141, 216, 188, 255])
            } else if distance <= (radius + 1) * (radius + 1) {
                Rgba([14, 21, 29, 255])
            } else {
                continue;
            };
            let (px, py) = ((center + x) as u32, (center + y) as u32);
            if px < image.width() && py < image.height() {
                image.put_pixel(px, py, color);
            }
        }
    }
}
