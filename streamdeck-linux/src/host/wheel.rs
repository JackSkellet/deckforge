//! Action Wheel: key-action children controlled by an encoder.
use super::*;
use crate::action_wheel::{self, Hit, Indicators, Rotation};
use anyhow::ensure;
use image::Pixel;

pub(super) fn action() -> Action {
    Action {
        uuid: action_wheel::UUID.into(),
        name: "Action Wheel".into(),
        plugin: "deckforge.wheels".into(),
        controllers: vec!["Encoder".into()],
        raw: json!({"SupportedInMultiActions":false,"Encoder":{"layout":"$X1","TriggerDescription":{"Rotate":"Select action","Push":"Activate selected action","Touch":"Select or activate an action"}},"States":[{"Title":"Action Wheel"}]}),
    }
}
pub(super) fn defaults() -> Value {
    json!({"CurrentIdx":0,"ControlOffset":0,"ExtTitles":false,"InvertRotation":false})
}
fn model(slot: &Slot) -> Result<action_wheel::Wheel> {
    let mut raw = slot.raw.clone();
    raw["Settings"] = slot.settings.clone();
    action_wheel::parse(&raw)
}
pub(super) fn current(slot: &Slot) -> usize {
    let last = slot.children.first().map_or(0, Vec::len).saturating_sub(1);
    let value = slot.settings["CurrentIdx"].as_f64().unwrap_or(0.).trunc();
    if value < 0. {
        last
    } else {
        (value as usize).min(last)
    }
}
struct IndicatorState {
    count: usize,
    offset: usize,
    bar: Indicators,
}
#[derive(Default)]
pub(super) struct Runtime {
    rotations: BTreeMap<String, Rotation>,
    indicators: HashMap<String, IndicatorState>,
}
impl Runtime {
    pub(super) fn next_due(&self) -> Option<Duration> {
        self.rotations.values().filter_map(Rotation::next_due).min()
    }
    pub(super) fn stop_turns(&mut self) {
        self.rotations.clear();
    }
    pub(super) fn sync(&mut self, slots: &[Slot]) {
        let mut visible = HashSet::new();
        for slot in slots
            .iter()
            .flat_map(Slot::visible_flatten)
            .filter(|s| s.uuid == action_wheel::UUID)
        {
            visible.insert(slot.context.clone());
            let count = slot.children.first().map_or(0, Vec::len);
            let offset = (slot.settings["ControlOffset"].as_f64().unwrap_or(0.) as usize)
                .min(count.saturating_sub(1));
            let current = current(slot);
            let state = self
                .indicators
                .entry(slot.context.clone())
                .or_insert_with(|| IndicatorState {
                    count,
                    offset,
                    bar: Indicators::new(count, current, offset),
                });
            if state.count != count || state.offset != offset {
                *state = IndicatorState {
                    count,
                    offset,
                    bar: Indicators::new(count, current, offset),
                };
            }
            state.bar.select(current);
        }
        self.rotations.retain(|id, _| visible.contains(id));
        self.indicators.retain(|id, _| visible.contains(id));
    }
}
impl Host {
    pub(super) fn save_wheel_draft(
        &mut self,
        context: &str,
        raw: Value,
        base: Option<&Value>,
    ) -> Result<()> {
        let raw = if let Some(base) = base {
            self.sync_visible();
            let current = &self
                .visible_slot(context)
                .context("Wheel is no longer visible")?
                .raw;
            merge_draft(current, base, &raw)?
        } else {
            raw
        };
        self.save_wheel(context, raw)
    }
    pub(super) fn wheel_button(&mut self, slot: &Slot, pressed: bool) -> Result<()> {
        let index = model(slot)?.current;
        let result = self.wheel_key_button(slot, index, pressed);
        if pressed
            && result.is_err()
            && let Some(child) = slot.children.first().and_then(|g| g.get(index))
        {
            self.show_feedback(child, crate::feedback::Kind::Alert)?;
        }
        result
    }
    pub(super) fn wheel_input(&mut self, slot: &Slot, event: &str, payload: Value) -> Result<()> {
        let wheel = model(slot)?;
        if wheel.children.is_empty() {
            return Ok(());
        }
        let (ticks, pressed) = if event == "dialRotate" {
            (
                i8::try_from(payload["ticks"].as_i64().context("Missing dial ticks")?)?,
                payload["pressed"].as_bool().unwrap_or(false),
            )
        } else {
            let x = u16::try_from(payload["tapPos"][0].as_u64().context("Missing tap x")?)?;
            let y = u16::try_from(payload["tapPos"][1].as_u64().context("Missing tap y")?)?;
            match action_wheel::hit(wheel.children.len(), wheel.current, x, y) {
                Some(Hit::Activate) => {
                    let result = self.adapter_pulse(slot, wheel.current, 1);
                    if result.is_err()
                        && let Some(child) =
                            slot.children.first().and_then(|g| g.get(wheel.current))
                    {
                        self.show_feedback(child, crate::feedback::Kind::Alert)?;
                    }
                    return result;
                }
                Some(Hit::Previous) => (-1, false),
                Some(Hit::Next) => (1, false),
                None => return Ok(()),
            }
        };
        self.wheels
            .rotations
            .entry(slot.context.clone())
            .or_default()
            .request(
                ticks,
                pressed,
                wheel.invert_rotation,
                wheel.children.len(),
                self.multi_clock.elapsed(),
            );
        self.poll_wheels();
        self.multi_notify.notify_one();
        Ok(())
    }
    pub(super) fn poll_wheels(&mut self) {
        let now = self.multi_clock.elapsed();
        let contexts: Vec<_> = self.wheels.rotations.keys().cloned().collect();
        for context in contexts {
            let Some(slot) = self.visible_slot(&context).cloned() else {
                self.wheels.rotations.remove(&context);
                continue;
            };
            let count = slot.children.first().map_or(0, Vec::len);
            let next = self.wheels.rotations.get_mut(&context).unwrap().advance(
                current(&slot),
                count,
                now,
            );
            if let Some(index) = next
                && let Err(error) = self.select_wheel(&context, index, false)
            {
                self.log(format!("Action Wheel selection: {error:#}"));
                self.wheels.rotations.remove(&context);
            }
        }
        self.wheels.rotations.retain(|_, r| r.next_due().is_some());
    }
    pub(super) fn select_wheel(
        &mut self,
        context: &str,
        index: usize,
        cancel_turns: bool,
    ) -> Result<()> {
        let mut slot = self
            .visible_slot(context)
            .cloned()
            .context("Wheel is no longer visible")?;
        ensure!(
            slot.uuid == action_wheel::UUID,
            "This control is not an Action Wheel"
        );
        let count = slot.children.first().map_or(0, Vec::len);
        ensure!(index < count, "Wheel selection is out of range");
        if cancel_turns {
            self.wheels.rotations.remove(context);
        }
        self.wheels.sync(&self.slots);
        let offset = if let Some(state) = self.wheels.indicators.get_mut(context) {
            state.bar.select(index);
            state.offset = state.bar.offset();
            state.offset
        } else {
            0
        };
        slot.settings["CurrentIdx"] = json!(index);
        slot.settings["ControlOffset"] = json!(offset);
        slot.raw["Settings"] = slot.settings.clone();
        // Every wheel child remains live; selection does not change SDK lifecycle.
        self.store_slot(slot);
        self.persist()?;
        self.render()
    }
    pub(super) fn save_wheel(&mut self, context: &str, raw: Value) -> Result<()> {
        self.editable()?;
        let mut parent = self
            .visible_slot(context)
            .cloned()
            .context("Wheel is no longer visible")?;
        ensure!(
            parent.uuid == action_wheel::UUID,
            "This control is not an Action Wheel"
        );
        action_wheel::parse(&raw)?;
        let old = parent.clone();
        parent.raw = raw;
        parent.settings = parent
            .raw
            .get("Settings")
            .filter(|s| s.is_object())
            .cloned()
            .unwrap_or(json!({}));
        parent.hydrate_children(&self.actions)?;
        for child in parent.children.iter().flatten() {
            if let Some(action) = self.actions.iter().find(|a| a.uuid == child.uuid) {
                ensure!(
                    child.uuid != multiaction::DELAY,
                    "Delay can only run inside a Multi Action"
                );
                ensure!(
                    action.controllers.iter().any(|c| c == "Keypad"),
                    "{} does not support keys",
                    child.name
                );
            }
        }
        if old.raw["Actions"] != parent.raw["Actions"] {
            self.cancel_trigger(context);
            self.wheels.rotations.remove(context);
            for child in old.flatten() {
                if multiaction::container(&child.uuid) {
                    self.cancel_multi(&child.context);
                }
            }
        }
        self.sync_visible();
        self.library.update(&parent);
        self.refresh_page()?;
        for child in parent.flatten().into_iter().skip(1) {
            if old
                .find(&child.context)
                .is_some_and(|s| s.settings != child.settings)
            {
                self.emit(child, "didReceiveSettings", json!({}));
                self.notify_inspectors(&child.context, self.settings_event(child, None), None);
            }
        }
        Ok(())
    }
}

fn merge_draft(current: &Value, base: &Value, edited: &Value) -> Result<Value> {
    let mut result = settings::patch(current, base, edited)?;
    if base["Actions"] != edited["Actions"] {
        let old = base["Actions"]
            .as_array()
            .context("Wheel draft baseline is invalid")?;
        let live = current["Actions"]
            .as_array()
            .context("Wheel actions changed; reload before saving")?;
        let desired = edited["Actions"]
            .as_array()
            .context("Wheel actions must be an array")?;
        let locate = |records: &Vec<Value>, entry: &Value, index: usize| {
            if let Some(id) = entry["ActionID"].as_str() {
                records.iter().position(|candidate| {
                    candidate["ActionID"] == id && candidate["UUID"] == entry["UUID"]
                })
            } else {
                records
                    .get(index)
                    .filter(|candidate| candidate["UUID"] == entry["UUID"])
                    .map(|_| index)
            }
        };
        let mut merged = Vec::with_capacity(desired.len());
        for (index, entry) in desired.iter().enumerate() {
            if let Some(old_index) = locate(old, entry, index) {
                let live_index = locate(live, entry, index)
                    .context("Wheel child changed; reload before saving")?;
                merged.push(settings::patch(&live[live_index], &old[old_index], entry)?);
            } else {
                merged.push(entry.clone());
            }
        }
        result["Actions"] = Value::Array(merged);
    }
    Ok(result)
}

/// Independent raster artwork using recovered card bounds and layering. Smooth
/// carousel interpolation is separate from the recovered selection timer.
pub(super) fn picture(display: &display_targets::Display<'_>, slot: &Slot) -> Result<DynamicImage> {
    let children = slot.children.first().map(Vec::as_slice).unwrap_or(&[]);
    let selected = current(slot);
    let extended = slot.settings["ExtTitles"] == true;
    let pressed = display.wheel_pressed.contains(&slot.context);
    let mut canvas = image::RgbaImage::from_pixel(200, 100, image::Rgba([0, 0, 0, 255]));
    for card in action_wheel::cards(children.len(), selected) {
        let child = &children[card.index];
        let mut style = appearance::style(child, display.actions);
        if extended {
            style.show_title = false;
        }
        let mut key = if display.actions.iter().any(|a| a.uuid == child.uuid) {
            display.renderer.key_styled_size(
                &child.title,
                child.background.as_ref(),
                &style,
                (72, 72),
            )
        } else {
            appearance::unavailable_picture(display.renderer, (72, 72))
        };
        feedback::paint_key(
            display.badges,
            &mut key,
            child,
            display.now,
            display.progress.get(&child.context).copied(),
        );
        let [mut x, mut y, mut w, mut h] = card.rect;
        if pressed && card.hit == Hit::Activate {
            let nw = w * 4 / 5;
            let nh = h * 4 / 5;
            x += (w - nw) / 2;
            y += (h - nh) / 2;
            w = nw;
            h = nh;
        }
        let mut layer = key
            .resize_exact(w, h, image::imageops::FilterType::Triangle)
            .to_rgba8();
        round(&mut layer, card.radius.min(h / 2), card.opacity, true);
        image::imageops::overlay(&mut canvas, &layer, i64::from(x), i64::from(y));
    }
    let fallback = Indicators::new(
        children.len(),
        selected,
        slot.settings["ControlOffset"].as_u64().unwrap_or(0) as usize,
    );
    let bar = display
        .wheels
        .indicators
        .get(&slot.context)
        .map(|s| &s.bar)
        .unwrap_or(&fallback);
    let dots = bar.items();
    let width = dots
        .iter()
        .map(|d| d.width + 4)
        .sum::<u32>()
        .saturating_sub(4);
    let mut x = (200 - width.min(200)) / 2;
    for dot in dots {
        let color = if dot.active { 255 } else { 128 };
        let mut layer = image::RgbaImage::from_pixel(
            dot.width,
            dot.height,
            image::Rgba([color, color, color, 255]),
        );
        round(&mut layer, dot.height / 2, 1., false);
        image::imageops::overlay(
            &mut canvas,
            &layer,
            i64::from(x),
            i64::from(85 + (8 - dot.height) / 2),
        );
        x += dot.width + 4;
    }
    let mut items = vec![json!({"key":"canvas","type":"pixmap","rect":[0,0,200,100]})];
    let mut style = None;
    let title = if let Some(child) = children.get(selected).filter(|_| extended) {
        let mut selected_style = appearance::style(child, display.actions);
        selected_style.font_style = if selected_style.italic() {
            "Bold Italic"
        } else {
            "Bold"
        }
        .into();
        style = Some(selected_style);
        items.push(json!({"key":"title","type":"text","zOrder":1,"rect":[16,10,168,72]}));
        child.title.as_str()
    } else if children.is_empty() {
        items.push(json!({"key":"title","type":"text","zOrder":1,"rect":[16,32,168,40],"font":{"size":16}}));
        "Add key actions"
    } else {
        ""
    };
    display.renderer.dial_layout_styled(
        title,
        &json!({}),
        &json!({"id":"deckforge-wheel","items":items}),
        &HashMap::from([("canvas".into(), DynamicImage::ImageRgba8(canvas))]),
        style.as_ref(),
    )
}
fn round(layer: &mut image::RgbaImage, radius: u32, opacity: f32, border: bool) {
    let (w, h) = layer.dimensions();
    let r = radius as f32;
    for (x, y, pixel) in layer.enumerate_pixels_mut() {
        let px = x as f32 + 0.5;
        let py = y as f32 + 0.5;
        let cx = px.clamp(r, w as f32 - r);
        let cy = py.clamp(r, h as f32 - r);
        let distance = (px - cx).hypot(py - cy);
        let coverage = (r + 0.5 - distance).clamp(0., 1.);
        if border
            && (distance > r - 1. || px < 1. || py < 1. || px > w as f32 - 1. || py > h as f32 - 1.)
        {
            pixel.blend(&image::Rgba([38, 38, 38, 255]));
        }
        pixel[3] = (f32::from(pixel[3]) * opacity * coverage).round() as u8;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn option_and_child_edits_preserve_live_selection_counts_and_states() {
        let base = json!({"Settings":{"CurrentIdx":1,"InvertRotation":false},"Actions":[
            {"UUID":"counter","ActionID":"a","Settings":{"count":3,"incrementBy":1},"State":0},
            {"UUID":"counter","ActionID":"b","Settings":{"count":7},"State":0}
        ]});
        let mut live = base.clone();
        live["Settings"]["CurrentIdx"] = json!(0);
        live["Actions"][0]["Settings"]["count"] = json!(4);
        live["Actions"][0]["State"] = json!(1);
        let mut edited = base.clone();
        edited["Settings"]["InvertRotation"] = json!(true);
        edited["Actions"][0]["Settings"]["incrementBy"] = json!(5);
        edited["Actions"].as_array_mut().unwrap().swap(0, 1);
        let merged = merge_draft(&live, &base, &edited).unwrap();
        assert_eq!(
            merged["Settings"],
            json!({"CurrentIdx":0,"InvertRotation":true})
        );
        assert_eq!(
            merged["Actions"][1]["Settings"],
            json!({"count":4,"incrementBy":5})
        );
        assert_eq!(merged["Actions"][1]["State"], 1);
    }
    fn fixture(path: &Path, count: usize) -> (Host, mpsc::UnboundedReceiver<Value>) {
        let mut host = super::super::tests::fixture(path);
        let (sender, receiver) = mpsc::unbounded_channel();
        host.plugins[0].sender = Some(sender);
        host.actions.push(action());
        host.assign("Encoder", 0, action_wheel::UUID).unwrap();
        let context = host
            .slots
            .iter()
            .find(|s| s.uuid == action_wheel::UUID)
            .unwrap()
            .context
            .clone();
        host.save_wheel(&context,json!({"Settings":defaults(),"Actions":(0..count).map(|i|json!({"UUID":"test.action","ActionID":format!("child-{i}"),"States":[{}]})).collect::<Vec<_>>()})).unwrap();
        (host, receiver)
    }
    fn parent(host: &Host) -> Slot {
        host.slots
            .iter()
            .find(|s| s.uuid == action_wheel::UUID)
            .unwrap()
            .clone()
    }
    #[test]
    fn empty_wheel_and_two_card_layers_render_in_the_real_host() {
        use image::GenericImageView;
        let root = tempfile::tempdir().unwrap();
        let (mut host, _receiver) = fixture(root.path(), 0);
        assert!(!parent(&host).preview.is_empty());
        let context = parent(&host).context;
        host.save_wheel(&context,json!({"Settings":defaults(),"Actions":[{"UUID":"test.action","ActionID":"red"},{"UUID":"test.action","ActionID":"blue"}]})).unwrap();
        let index = host
            .slots
            .iter()
            .position(|s| s.context == context)
            .unwrap();
        for (child, rgb) in host.slots[index].children[0]
            .iter_mut()
            .zip([[255, 0, 0], [0, 0, 255]])
        {
            child.background = Some(DynamicImage::ImageRgb8(RgbImage::from_pixel(
                72,
                72,
                Rgb(rgb),
            )));
        }
        host.render().unwrap();
        let decode =
            |s: &Slot| image::load_from_memory(&STANDARD.decode(&s.preview).unwrap()).unwrap();
        let first = decode(&parent(&host));
        assert_eq!((first.width(), first.height()), (200, 100));
        assert_eq!(first.get_pixel(80, 40).0, [255, 0, 0, 255]);
        assert_eq!(first.get_pixel(145, 40).0, [0, 0, 153, 255]);
        host.select_wheel(&context, 1, true).unwrap();
        let second = decode(&parent(&host));
        assert_eq!(second.get_pixel(120, 40).0, [0, 0, 255, 255]);
        assert_eq!(second.get_pixel(50, 40).0, [153, 0, 0, 255]);
    }
    #[test]
    fn changing_wheel_options_keeps_an_owned_press_until_release() {
        let root = tempfile::tempdir().unwrap();
        let (mut host, mut receiver) = fixture(root.path(), 2);
        let slot = parent(&host);
        while receiver.try_recv().is_ok() {}
        host.wheel_button(&slot, true).unwrap();
        let mut edited = parent(&host).raw;
        edited["Settings"]["InvertRotation"] = json!(true);
        host.save_wheel(&slot.context, edited).unwrap();
        assert!(host.key_pulse_active(&slot.context));
        host.wheel_button(&parent(&host), false).unwrap();
        let mut edges = vec![];
        while let Ok(event) = receiver.try_recv() {
            if matches!(event["event"].as_str(), Some("keyDown" | "keyUp")) {
                edges.push(event);
            }
        }
        assert_eq!(edges.len(), 2);
        assert_eq!(edges[0]["event"], "keyDown");
        assert_eq!(edges[1]["event"], "keyUp");
        assert_eq!(edges[0]["context"], edges[1]["context"]);
    }
    #[test]
    fn selection_and_navigation_state_stay_with_their_device() {
        let root = tempfile::tempdir().unwrap();
        let (mut host, _receiver) = fixture(root.path(), 2);
        let first = parent(&host).context;
        host.add_simulated_device("other-wheel", Model::Plus)
            .unwrap();
        host.with_device("other-wheel",|host| {
            host.assign("Encoder",0,action_wheel::UUID)?;
            let context=parent(host).context;
            host.save_wheel(&context,json!({"Settings":defaults(),"Actions":[{"UUID":"test.action","ActionID":"a"},{"UUID":"test.action","ActionID":"b"}]}))?;
            host.select_wheel(&context,1,true)?;
            assert_eq!(current(&parent(host)),1);
            Ok(())
        }).unwrap();
        assert_eq!(parent(&host).context, first);
        assert_eq!(current(&parent(&host)), 0);
        host.select_wheel(&first, 1, true).unwrap();
        host.with_device("other-wheel", |host| {
            assert_eq!(current(&parent(host)), 1);
            Ok(())
        })
        .unwrap();
    }
    #[test]
    fn selection_during_a_hold_releases_the_original_child() {
        let root = tempfile::tempdir().unwrap();
        let (mut host, mut receiver) = fixture(root.path(), 2);
        let slot = parent(&host);
        let first = slot.children[0][0].context.clone();
        let second = slot.children[0][1].context.clone();
        while receiver.try_recv().is_ok() {}
        host.wheel_button(&slot, true).unwrap();
        host.select_wheel(&slot.context, 1, true).unwrap();
        host.wheel_button(&parent(&host), false).unwrap();
        let mut events = vec![];
        while let Ok(event) = receiver.try_recv() {
            if matches!(event["event"].as_str(), Some("keyDown" | "keyUp")) {
                events.push(event);
            }
        }
        assert_eq!(events.len(), 2);
        assert_eq!(events[0]["context"], first);
        assert_eq!(events[1]["context"], first);
        assert_ne!(events[1]["context"], second);
        assert!(!host.key_pulse_active(&slot.context));
    }
}
