//! Native action discovery. Selection is a draft; assignment stays explicit.
use eframe::egui::{self, RichText};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Clone)]
pub struct DragAction {
    pub uuid: String,
}

#[derive(Default)]
pub struct Library {
    pub query: String,
    pub show_all: bool,
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
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        actions: &[Value],
        controller: &str,
        selected: &str,
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
                                let button = egui::Button::new(name)
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
                    library.show(ui, &[], "Keypad", "");
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
