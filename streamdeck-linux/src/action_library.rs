//! Native action discovery. Selection is a draft; assignment stays explicit.
use base64::Engine;
use eframe::egui::{self, RichText};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};

type IconKey = (String, String, Option<u64>);
fn icon_key(action: &Value) -> Option<IconKey> {
    let root = action["plugin_root"].as_str()?;
    let icon = action["icon"].as_str()?;
    if root.is_empty() || icon.is_empty() {
        return None;
    }
    Some((root.into(), icon.into(), action["icon_generation"].as_u64()))
}

#[derive(Clone)]
pub struct DragAction {
    pub uuid: String,
}

#[derive(Default)]
pub struct Library {
    pub query: String,
    pub show_all: bool,
    icons: HashMap<IconKey, Option<egui::TextureHandle>>,
}

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap_or("")
}

pub fn compatible(action: &Value, controller: &str) -> bool {
    action["multi_only"] != true
        && action["controllers"]
            .as_array()
            .is_some_and(|controllers| controllers.iter().any(|c| c == controller))
}

fn matches(action: &Value, query: &str) -> bool {
    let haystack = ["name", "uuid", "plugin_name", "category", "tooltip"]
        .map(|key| text(action, key))
        .join(" ")
        .to_lowercase();
    query
        .split_whitespace()
        .all(|word| haystack.contains(&word.to_lowercase()))
}

fn groups<'a>(
    actions: &'a [Value],
    query: &str,
    controller: &str,
    show_all: bool,
) -> BTreeMap<String, Vec<&'a Value>> {
    let mut groups: BTreeMap<String, Vec<&Value>> = BTreeMap::new();
    for action in actions.iter().filter(|action| {
        action["multi_only"] != true
            && action["visible"] != false
            && matches(action, query)
            && (show_all || compatible(action, controller))
    }) {
        let category = text(action, "category");
        let category = if category.is_empty() {
            text(action, "plugin_name")
        } else {
            category
        };
        let category = if category.is_empty() {
            "Other actions"
        } else {
            category
        };
        groups.entry(category.to_owned()).or_default().push(action);
    }
    for group in groups.values_mut() {
        group.sort_by_key(|action| (text(action, "name").to_lowercase(), text(action, "uuid")));
    }
    groups
}

impl Library {
    pub fn clear_images(&mut self) {
        self.icons.clear();
    }

    fn icon(
        &mut self,
        ui: &egui::Ui,
        action: &Value,
        loaded: &mut bool,
        load_icon: &dyn Fn(&str) -> Option<String>,
    ) -> Option<egui::TextureId> {
        let key = icon_key(action)?;
        if !self.icons.contains_key(&key) {
            // Spread decoding over frames; ordinary plugin input never waits on
            // catalog thumbnails or the host mutex.
            if *loaded {
                ui.ctx().request_repaint();
                return None;
            }
            *loaded = true;
            let texture = load_icon(text(action, "uuid"))
                .and_then(|uri| {
                    uri.strip_prefix("data:image/png;base64,").and_then(|data| {
                        base64::engine::general_purpose::STANDARD.decode(data).ok()
                    })
                })
                .and_then(|bytes| image::load_from_memory(&bytes).ok())
                .map(|image| {
                    let rgba = image.to_rgba8();
                    let image = egui::ColorImage::from_rgba_unmultiplied(
                        [rgba.width() as usize, rgba.height() as usize],
                        rgba.as_raw(),
                    );
                    ui.ctx().load_texture(
                        format!("action-icon:{key:?}"),
                        image,
                        egui::TextureOptions::LINEAR,
                    )
                });
            self.icons.insert(key.clone(), texture);
        }
        self.icons.get(&key)?.as_ref().map(egui::TextureHandle::id)
    }
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        actions: &[Value],
        controller: &str,
        selected: &str,
        load_icon: &dyn Fn(&str) -> Option<String>,
    ) -> Option<String> {
        let search = ui.add(
            egui::TextEdit::singleline(&mut self.query)
                .hint_text("Search actions or plugins")
                .desired_width(f32::INFINITY),
        );
        search.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::TextEdit,
                true,
                "Search actions or plugins",
            )
        });
        if ui.input_mut(|input| input.consume_key(egui::Modifiers::CTRL, egui::Key::F)) {
            search.request_focus();
        }
        if (search.has_focus() || search.lost_focus())
            && ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            self.query.clear();
        }
        ui.checkbox(&mut self.show_all, "Show key and dial actions");
        ui.weak(if controller == "Encoder" {
            "Choose an action for the selected dial."
        } else {
            "Choose an action for the selected key."
        });
        let groups = groups(actions, &self.query, controller, self.show_all);
        let count: usize = groups.values().map(Vec::len).sum();
        ui.small(format!(
            "{count} actions · drag to a control or select below"
        ));
        let mut chosen = None;
        let live: HashSet<_> = actions.iter().filter_map(icon_key).collect();
        self.icons.retain(|key, _| live.contains(key));
        let mut loaded = false;
        egui::ScrollArea::vertical()
            .id_salt("action_library")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if groups.is_empty() {
                    ui.label("No matching actions");
                    ui.weak("Try another search, select a different control, or install a plugin.");
                }
                for (category, entries) in groups {
                    egui::CollapsingHeader::new(RichText::new(&category).strong())
                        .id_salt(("action_category", &category, &self.query))
                        .default_open(true)
                        .show(ui, |ui| {
                            for action in entries {
                                let enabled = compatible(action, controller);
                                let uuid = text(action, "uuid");
                                let name = text(action, "name");
                                let button = if let Some(icon) =
                                    self.icon(ui, action, &mut loaded, load_icon)
                                {
                                    egui::Button::image_and_text(
                                        egui::Image::new((icon, egui::vec2(24.0, 24.0))),
                                        name,
                                    )
                                } else {
                                    egui::Button::new(name)
                                };
                                let button = button
                                    .min_size(egui::vec2(ui.available_width(), 30.0))
                                    .selected(uuid == selected)
                                    .sense(egui::Sense::click_and_drag());
                                let response = ui.add_enabled(enabled, button);
                                response.dnd_set_drag_payload(DragAction {
                                    uuid: uuid.to_owned(),
                                });
                                let tooltip = format!(
                                    "{}\n{}\n{}{}",
                                    text(action, "plugin_name"),
                                    text(action, "tooltip"),
                                    uuid,
                                    if enabled {
                                        ""
                                    } else {
                                        "\nChoose a compatible key or dial to use this action."
                                    }
                                );
                                if response.on_hover_text(tooltip).clicked() {
                                    chosen = Some(uuid.to_owned());
                                }
                            }
                        });
                }
            });
        chosen
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn keyboard_search_takes_focus_and_accepts_text() {
        let ctx = egui::Context::default();
        let mut library = Library::default();
        let mut frame = |events| {
            let output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(400.0, 600.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    library.show(ui, &[], "Keypad", "", &|_| None);
                },
            );
            output.drop_without_applying_deltas();
            library.query.clone()
        };
        frame(vec![]);
        frame(vec![egui::Event::Key {
            key: egui::Key::F,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::CTRL,
        }]);
        assert_eq!(frame(vec![egui::Event::Text("counter".into())]), "counter");
        assert_eq!(
            frame(vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE
            }]),
            ""
        );
    }
    #[test]
    fn search_matches_metadata_tokens_and_keeps_duplicate_names_distinct() {
        let actions = vec![
            json!({"uuid":"one","name":"Counter","plugin_name":"Official Sample","category":"Examples","controllers":["Keypad"]}),
            json!({"uuid":"two","name":"Counter","plugin_name":"Other","category":"Examples","controllers":["Keypad"]}),
        ];
        assert_eq!(
            groups(&actions, "SAMPLE counter", "Keypad", false)["Examples"].len(),
            1
        );
        assert_eq!(
            groups(&actions, "counter", "Keypad", false)["Examples"].len(),
            2
        );
        assert!(groups(&actions, "absent", "Keypad", false).is_empty());
    }
    #[test]
    fn controller_filter_and_multi_only_prevent_invalid_assignments() {
        let actions = vec![
            json!({"uuid":"key","name":"Key","controllers":["Keypad"]}),
            json!({"uuid":"dial","name":"Dial","controllers":["Encoder"]}),
            json!({"uuid":"delay","controllers":["Keypad"],"multi_only":true}),
            json!({"uuid":"hidden","controllers":["Keypad"],"visible":false}),
        ];
        assert_eq!(
            groups(&actions, "", "Encoder", false)["Other actions"][0]["uuid"],
            "dial"
        );
        assert_eq!(
            groups(&actions, "", "Encoder", true)["Other actions"].len(),
            2
        );
        assert!(!compatible(&actions[0], "Encoder"));
        assert!(!compatible(&actions[2], "Keypad"));
    }
}
