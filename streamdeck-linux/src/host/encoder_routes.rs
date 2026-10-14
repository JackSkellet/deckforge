//! Route physical Galleon dials to profile Encoder positions. The recovered 7.6
//! controller defers presses only when a column has multiple assigned rows.
use super::*;
use anyhow::ensure;

const SWITCH_AFTER: Duration = Duration::from_millis(500);
#[derive(Clone)]
struct Owner {
    context: String,
    index: u8,
}
struct Hold {
    owner: Owner,
    started: Duration,
    deferred: bool,
    preview: Option<Duration>,
}
struct Pulse {
    owner: Owner,
    duration: Duration,
    release: Option<Duration>,
}
#[derive(Default)]
pub(super) struct Runtime {
    pressed: HashSet<u8>,
    holds: HashMap<u8, Hold>,
    pulses: HashMap<u8, VecDeque<Pulse>>,
    preview_due: Option<Duration>,
    frame_due: Option<Duration>,
    layer: Option<([u8; 4], DynamicImage, String)>,
}
impl Runtime {
    pub fn disconnected(&mut self) {
        self.pressed.clear();
    }
    pub fn retain(&mut self, visible: &HashSet<String>) {
        self.holds.retain(|_, h| visible.contains(&h.owner.context));
        self.pulses.retain(|_, queue| {
            queue.retain(|p| visible.contains(&p.owner.context));
            !queue.is_empty()
        });
    }
    pub fn next_due(&self) -> Option<Duration> {
        self.pulses
            .values()
            .filter_map(|q| q.front().map(|p| p.release.unwrap_or(Duration::ZERO)))
            .chain(self.preview_due.filter(|_| {
                self.holds
                    .values()
                    .any(|h| h.deferred && h.preview.is_none())
            }))
            .chain(
                self.frame_due
                    .filter(|_| self.holds.values().any(|h| h.preview.is_some())),
            )
            .min()
    }
    pub fn preview(&self) -> Option<&str> {
        self.layer.as_ref().map(|(_, _, png)| png.as_str())
    }
}
impl Host {
    /// Simulator controls submit complete operations, so window teardown cannot
    /// strand an editor-owned down event in the background host.
    pub(super) fn test_physical_dial(&mut self, column: u8, operation: &str) -> Result<()> {
        ensure!(
            self.model.encoder_rows() > 1 && column < self.model.physical_dial_count(),
            "Physical dial outside routed device geometry"
        );
        ensure!(
            matches!(operation, "click" | "next"),
            "Unknown physical dial operation"
        );
        let Some(index) = self.encoder_routes()[column as usize] else {
            return Ok(());
        };
        match operation {
            "next" => {
                self.cancel_encoder_gestures(column)?;
                let indices = self.encoder_indices(column);
                let position = indices.iter().position(|i| *i == index).unwrap();
                self.set_encoder_route(column, indices[(position + 1) % indices.len()])
            }
            "click" => {
                ensure!(
                    !self.encoder_routing.pressed.contains(&column)
                        && !self.held.contains_key(&HeldControl::Dial(index)),
                    "Release the dial before testing another press"
                );
                let down = self.input(InputEvent::DialPress {
                    index,
                    pressed: true,
                });
                let up = self.input(InputEvent::DialPress {
                    index,
                    pressed: false,
                });
                down.and(up)
            }
            _ => bail!("Unknown physical dial operation"),
        }
    }
    fn encoder_indices(&self, column: u8) -> Vec<u8> {
        let mut indices: Vec<_> = self
            .slots
            .iter()
            .filter(|s| {
                s.controller == "Encoder" && self.model.encoder_coordinates(s.index).0 == column
            })
            .map(|s| s.index)
            .collect();
        indices.sort_unstable();
        indices
    }
    pub(super) fn encoder_routes(&self) -> Vec<Option<u8>> {
        if self.model.encoder_rows() < 2 {
            return vec![];
        }
        let controllers = self
            .library
            .current()
            .ok()
            .and_then(|p| p.pages.iter().find(|p| p.id == self.library.active_page))
            .and_then(|p| p.raw["Controllers"].as_array());
        let routes = controllers
            .and_then(|c| c.iter().find(|c| c["Type"] == "Encoder"))
            .map(|c| &c["Routers"]);
        (0..self.model.physical_dial_count())
            .map(|column| {
                let available = self.encoder_indices(column);
                let selected = routes
                    .and_then(|r| r[format!("{column},0")].as_str())
                    .and_then(|s| s.split_once(','))
                    .and_then(|(c, r)| Some((c.parse::<u8>().ok()?, r.parse::<u8>().ok()?)))
                    .filter(|(c, _)| *c == column)
                    .and_then(|(c, r)| library::position_index(self.model, "Encoder", c, r));
                selected
                    .filter(|i| available.contains(i))
                    .or_else(|| available.first().copied())
            })
            .collect()
    }
    fn encoder_owner_valid(&self, owner: &Owner) -> bool {
        self.slots.iter().any(|s| {
            s.controller == "Encoder" && s.index == owner.index && s.context == owner.context
        })
    }
    fn cancel_encoder_owner(&mut self, owner: &Owner) {
        let mut visible: HashSet<_> = self
            .slots
            .iter()
            .flat_map(Slot::visible_flatten)
            .map(|s| s.context.clone())
            .collect();
        if let Some(slot) = self.slots.iter().find(|s| s.context == owner.context) {
            for child in slot.flatten() {
                visible.remove(&child.context);
            }
        }
        self.cancel_departing_triggers(&visible);
        self.cancel_departing_stacks(&visible);
        self.cancel_departing_inputs(&visible);
        if let Some(pressed) = self.dial_pressed.get_mut(owner.index as usize) {
            *pressed = false;
        }
    }
    pub(super) fn cancel_physical_dial(&mut self, column: u8) -> Result<()> {
        self.encoder_routing.pressed.remove(&column);
        self.cancel_encoder_gestures(column)?;
        self.render()
    }
    fn cancel_encoder_gestures(&mut self, column: u8) -> Result<()> {
        ensure!(
            column < self.model.physical_dial_count(),
            "Physical dial outside device geometry"
        );
        if let Some(hold) = self.encoder_routing.holds.remove(&column)
            && !hold.deferred
        {
            self.cancel_encoder_owner(&hold.owner);
        }
        if let Some(queue) = self.encoder_routing.pulses.remove(&column) {
            for pulse in queue {
                if pulse.release.is_some() {
                    self.cancel_encoder_owner(&pulse.owner);
                }
            }
        }
        Ok(())
    }
    fn set_encoder_route(&mut self, column: u8, index: u8) -> Result<()> {
        let (col, row) = self.model.encoder_coordinates(index);
        ensure!(
            col == column && self.encoder_indices(column).contains(&index),
            "Encoder route has no assigned action"
        );
        let page_id = self.library.active_page.clone();
        let page = self
            .library
            .current_mut()?
            .pages
            .iter_mut()
            .find(|p| p.id == page_id)
            .context("Active page missing")?;
        if page.raw["Controllers"].is_null() {
            page.raw["Controllers"] = json!([]);
        }
        let controllers = page.raw["Controllers"]
            .as_array_mut()
            .context("Controllers must be an array")?;
        let at = controllers
            .iter()
            .position(|c| c["Type"] == "Encoder")
            .unwrap_or_else(|| {
                controllers.push(json!({"Type":"Encoder","Actions":{}}));
                controllers.len() - 1
            });
        let controller = &mut controllers[at];
        if controller["Routers"].is_null() {
            controller["Routers"] = json!({});
        }
        let routes = controller["Routers"]
            .as_object_mut()
            .context("Imported Routers must be an object")?;
        routes.insert(format!("{column},0"), json!(format!("{col},{row}")));
        self.persist()?;
        self.render()
    }
    pub(super) fn physical_input(&mut self, event: InputEvent) -> Result<()> {
        if self.model.encoder_rows() < 2 {
            return self.input(event);
        }
        self.poll_encoder_routes();
        match event {
            InputEvent::DialRotate { index, ticks } => {
                ensure!(
                    index < self.model.physical_dial_count(),
                    "Physical dial outside device geometry"
                );
                if let Some(target) = self.encoder_routes()[index as usize] {
                    self.dial_pressed[target as usize] =
                        self.encoder_routing.pressed.contains(&index);
                    self.input(InputEvent::DialRotate {
                        index: target,
                        ticks,
                    })?;
                }
            }
            InputEvent::DialPress { index, pressed } => {
                ensure!(
                    index < self.model.physical_dial_count(),
                    "Physical dial outside device geometry"
                );
                if pressed {
                    if !self.encoder_routing.pressed.insert(index) {
                        return Ok(());
                    }
                    let Some(target) = self.encoder_routes()[index as usize] else {
                        return Ok(());
                    };
                    let slot = self
                        .slots
                        .iter()
                        .find(|s| s.controller == "Encoder" && s.index == target)
                        .unwrap();
                    let owner = Owner {
                        context: slot.context.clone(),
                        index: target,
                    };
                    let deferred = self.encoder_indices(index).len() > 1;
                    self.encoder_routing.holds.insert(
                        index,
                        Hold {
                            owner,
                            started: self.multi_clock.elapsed(),
                            deferred,
                            preview: None,
                        },
                    );
                    if deferred {
                        self.encoder_routing.preview_due =
                            Some(self.multi_clock.elapsed() + Duration::from_millis(250));
                        self.multi_notify.notify_one();
                    }
                    if !deferred {
                        self.input(InputEvent::DialPress {
                            index: target,
                            pressed: true,
                        })?;
                    }
                } else {
                    self.encoder_routing.pressed.remove(&index);
                    for logical in 0..self.model.dial_count() {
                        if self.model.encoder_coordinates(logical).0 == index
                            && !self.held.contains_key(&HeldControl::Dial(logical))
                        {
                            self.dial_pressed[logical as usize] = false;
                        }
                    }
                    let Some(hold) = self.encoder_routing.holds.remove(&index) else {
                        return Ok(());
                    };
                    if !self.encoder_owner_valid(&hold.owner)
                        || self.encoder_routes()[index as usize] != Some(hold.owner.index)
                    {
                        self.cancel_encoder_owner(&hold.owner);
                        return Ok(());
                    }
                    if !hold.deferred {
                        self.input(InputEvent::DialPress {
                            index: hold.owner.index,
                            pressed: false,
                        })?;
                    } else {
                        let duration = self.multi_clock.elapsed().saturating_sub(hold.started);
                        if duration > SWITCH_AFTER {
                            let indices = self.encoder_indices(index);
                            if let Some(position) =
                                indices.iter().position(|i| *i == hold.owner.index)
                            {
                                self.set_encoder_route(
                                    index,
                                    indices[(position + 1) % indices.len()],
                                )?;
                            }
                        } else {
                            let queue = self.encoder_routing.pulses.entry(index).or_default();
                            ensure!(queue.len() < 32, "Too many pending dial presses");
                            queue.push_back(Pulse {
                                owner: hold.owner,
                                duration,
                                release: None,
                            });
                            self.poll_encoder_routes();
                            self.multi_notify.notify_one();
                        }
                    }
                }
            }
            other => self.input(other)?,
        }
        Ok(())
    }
    pub(super) fn poll_encoder_routes(&mut self) {
        let now = self.multi_clock.elapsed();
        let mut changed = false;
        if self
            .encoder_routing
            .preview_due
            .is_some_and(|due| now >= due)
        {
            for hold in self.encoder_routing.holds.values_mut() {
                if hold.deferred
                    && hold.preview.is_none()
                    && now.saturating_sub(hold.started) > SWITCH_AFTER
                {
                    hold.preview = Some(now);
                    changed = true;
                }
            }
            self.encoder_routing.preview_due = self
                .encoder_routing
                .holds
                .values()
                .any(|h| h.deferred && h.preview.is_none())
                .then_some(now + Duration::from_millis(250));
        }
        if changed || self.encoder_routing.frame_due.is_some_and(|due| now >= due) {
            self.encoder_routing.frame_due = self
                .encoder_routing
                .holds
                .values()
                .filter_map(|h| h.preview)
                .map(|start| start + Duration::from_millis(150))
                .filter(|end| *end > now)
                .map(|end| end.min(now + Duration::from_millis(33)))
                .min();
            if let Err(error) = self.render() {
                self.log(format!("Dial row preview: {error:#}"));
            }
        }
        let columns: Vec<_> = self.encoder_routing.pulses.keys().copied().collect();
        for column in columns {
            let Some(mut queue) = self.encoder_routing.pulses.remove(&column) else {
                continue;
            };
            if let Some(pulse) =
                queue.pop_front_if(|pulse| pulse.release.is_some_and(|due| now >= due))
            {
                if self.encoder_owner_valid(&pulse.owner) {
                    if let Err(error) = self.input(InputEvent::DialPress {
                        index: pulse.owner.index,
                        pressed: false,
                    }) {
                        self.log(format!("Dial release: {error:#}"));
                    }
                } else {
                    self.cancel_encoder_owner(&pulse.owner);
                }
            }
            if let Some(pulse) = queue.front_mut()
                && pulse.release.is_none()
            {
                if !self.encoder_owner_valid(&pulse.owner) {
                    queue.clear();
                } else {
                    // Register the pulse before dispatch: navigation can cancel it.
                    pulse.release = Some(now + pulse.duration);
                    let index = pulse.owner.index;
                    self.encoder_routing.pulses.insert(column, queue);
                    if let Err(error) = self.input(InputEvent::DialPress {
                        index,
                        pressed: true,
                    }) {
                        let _ = self.cancel_physical_dial(column);
                        self.log(format!("Dial press: {error:#}"));
                    }
                    continue;
                }
            }
            if !queue.is_empty() {
                self.encoder_routing.pulses.insert(column, queue);
            }
        }
    }
}
impl Host {
    pub(super) fn encoder_marker_layer(&mut self) -> Result<Option<DynamicImage>> {
        if self.model != Model::Galleon {
            return Ok(None);
        }
        let now = self.multi_clock.elapsed();
        let mut weights = [0u8; 4];
        for (column, selected) in self.encoder_routes().into_iter().enumerate() {
            let indices = self.encoder_indices(column as u8);
            let Some(selected) = selected.filter(|_| indices.len() > 1) else {
                continue;
            };
            weights[selected as usize] = 255;
            if let Some(hold) = self.encoder_routing.holds.get(&(column as u8))
                && hold.owner.index == selected
                && self.encoder_owner_valid(&hold.owner)
                && let Some(start) = hold.preview
            {
                let next = indices
                    [(indices.iter().position(|i| *i == selected).unwrap() + 1) % indices.len()];
                let alpha = (now.saturating_sub(start).as_secs_f32() / 0.15).clamp(0., 1.);
                weights[selected as usize] = ((1. - alpha) * 255.).round() as u8;
                weights[next as usize] = (alpha * 255.).round() as u8;
            }
        }
        if self
            .encoder_routing
            .layer
            .as_ref()
            .is_none_or(|(old, _, _)| *old != weights)
        {
            let layer = marker_layer(weights)?;
            let png = display_targets::png(&layer)?;
            self.encoder_routing.layer = Some((weights, layer, png));
        }
        Ok(self
            .encoder_routing
            .layer
            .as_ref()
            .map(|(_, image, _)| image.clone()))
    }
}
fn marker_layer(weights: [u8; 4]) -> Result<DynamicImage> {
    // Native rectangle: canonical cell200x100 expanded6; radius5, white
    // border2, outline1 with RGBA30,30,30,180. The inner cutout retains radius5.
    let mut shapes = String::new();
    for (index, alpha) in weights.into_iter().enumerate().filter(|(_, a)| *a > 0) {
        let x = 9 + (index % 2) * 220;
        let y = 9 + (index / 2) * 110;
        shapes.push_str(&format!(r##"<mask id="m{index}"><rect width="450" height="240" fill="white"/><rect x="{}" y="{}" width="208" height="108" rx="5" fill="black"/></mask><g opacity="{}" mask="url(#m{index})"><rect x="{x}" y="{y}" width="212" height="112" rx="5" fill="white" stroke="#1e1e1e" stroke-opacity="0.705882" stroke-width="1"/></g>"##,x+2,y+2,f32::from(alpha)/255.));
    }
    let svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="720" height="384" viewBox="0 0 450 240">{shapes}</svg>"#
    );
    display::decode_image(svg.as_bytes(), true)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn setup() -> (tempfile::TempDir, Host, mpsc::UnboundedReceiver<Value>) {
        let root = tempfile::tempdir().unwrap();
        let mut host = super::super::tests::fixture(root.path());
        host.model = Model::Galleon;
        host.library.current_mut().unwrap().model = Model::Galleon;
        host.dial_pressed = vec![false; 4];
        host.actions[0].controllers = vec!["Keypad".into(), "Encoder".into()];
        let (tx, mut rx) = mpsc::unbounded_channel();
        host.plugins[0].sender = Some(tx);
        for index in [0, 2, 3] {
            host.assign("Encoder", index, "test.action").unwrap();
        }
        while rx.try_recv().is_ok() {}
        host.multi_clock = std::time::Instant::now();
        (root, host, rx)
    }
    #[test]
    fn long_release_switches_only_its_column_and_short_release_replays_a_press() {
        let (_root, mut host, mut rx) = setup();
        assert_eq!(host.encoder_routes(), vec![Some(0), Some(3)]);
        host.physical_input(InputEvent::DialPress {
            index: 0,
            pressed: true,
        })
        .unwrap();
        assert!(rx.try_recv().is_err());
        host.multi_clock = std::time::Instant::now() - Duration::from_millis(600);
        host.physical_input(InputEvent::DialPress {
            index: 0,
            pressed: false,
        })
        .unwrap();
        assert!(rx.try_recv().is_err());
        assert_eq!(host.encoder_routes(), vec![Some(2), Some(3)]);
        host.physical_input(InputEvent::DialRotate {
            index: 0,
            ticks: -2,
        })
        .unwrap();
        let event = rx.try_recv().unwrap();
        assert_eq!(event["payload"]["coordinates"], json!({"column":0,"row":1}));
        assert_eq!(event["payload"]["ticks"], -2);
        host.physical_input(InputEvent::DialPress {
            index: 0,
            pressed: true,
        })
        .unwrap();
        host.multi_clock = std::time::Instant::now() - Duration::from_millis(700);
        host.physical_input(InputEvent::DialPress {
            index: 0,
            pressed: false,
        })
        .unwrap();
        assert_eq!(rx.try_recv().unwrap()["event"], "dialDown");
        assert!(rx.try_recv().is_err());
        host.multi_clock = std::time::Instant::now() - Duration::from_millis(900);
        host.poll_encoder_routes();
        assert_eq!(rx.try_recv().unwrap()["event"], "dialUp");
        assert!(rx.try_recv().is_err());
        let page = host
            .library
            .current()
            .unwrap()
            .pages
            .iter()
            .find(|p| p.id == host.library.active_page)
            .unwrap();
        assert_eq!(page.raw["Controllers"][0]["Routers"]["0,0"], "0,1");
    }
    #[test]
    fn pending_marker_fades_without_switching_action_and_cancel_restores_selection() {
        use image::GenericImageView;
        let (_root, mut host, mut rx) = setup();
        host.physical_input(InputEvent::DialPress {
            index: 0,
            pressed: true,
        })
        .unwrap();
        let started = host.encoder_routing.holds[&0].started;
        let advance = |host: &mut Host, ms| {
            host.multi_clock = std::time::Instant::now() - (started + Duration::from_millis(ms));
            host.poll_encoder_routes();
        };
        advance(&mut host, 250);
        assert!(host.encoder_routing.holds[&0].preview.is_none());
        advance(&mut host, 600);
        assert!(host.encoder_routing.holds[&0].preview.is_some());
        advance(&mut host, 675);
        let weights = host.encoder_routing.layer.as_ref().unwrap().0;
        assert!(
            (120..=135).contains(&weights[0]) && (120..=135).contains(&weights[2]),
            "{weights:?}"
        );
        advance(&mut host, 800);
        assert_eq!(
            host.encoder_routing.layer.as_ref().unwrap().0,
            [0, 0, 255, 0]
        );
        assert_eq!(host.encoder_routes(), vec![Some(0), Some(3)]);
        assert!(rx.try_recv().is_err());
        host.physical_input(InputEvent::DialRotate { index: 0, ticks: 1 })
            .unwrap();
        let event = rx.try_recv().unwrap();
        assert_eq!(event["payload"]["coordinates"], json!({"column":0,"row":0}));
        assert_eq!(event["payload"]["pressed"], true);
        host.cancel_physical_dial(0).unwrap();
        assert_eq!(
            host.encoder_routing.layer.as_ref().unwrap().0,
            [255, 0, 0, 0]
        );
        assert!(host.encoder_routing.next_due().is_none());
        let marker = marker_layer([255, 0, 0, 0]).unwrap();
        assert!(marker.get_pixel(16, 100).0[3] > 200);
        assert_eq!(marker.get_pixel(100, 100).0[3], 0);
        assert_eq!(marker.get_pixel(16, 270).0[3], 0);
        assert_eq!(marker.get_pixel(24, 24).0[3], 0);
    }
    #[test]
    fn single_assignment_is_direct_and_pending_gestures_cancel_without_late_actions() {
        let (_root, mut host, mut rx) = setup();
        host.physical_input(InputEvent::DialPress {
            index: 1,
            pressed: true,
        })
        .unwrap();
        let down = rx.try_recv().unwrap();
        assert_eq!(down["event"], "dialDown");
        assert_eq!(down["payload"]["coordinates"], json!({"column":1,"row":1}));
        host.cancel_physical_dial(1).unwrap();
        assert_eq!(rx.try_recv().unwrap()["event"], "dialUp");
        host.physical_input(InputEvent::DialPress {
            index: 1,
            pressed: false,
        })
        .unwrap();
        assert!(rx.try_recv().is_err());
        host.physical_input(InputEvent::DialPress {
            index: 0,
            pressed: true,
        })
        .unwrap();
        host.release_device_holds();
        host.multi_clock = std::time::Instant::now() - Duration::from_secs(1);
        host.physical_input(InputEvent::DialPress {
            index: 0,
            pressed: false,
        })
        .unwrap();
        assert!(rx.try_recv().is_err());
        assert_eq!(host.encoder_routes(), vec![Some(0), Some(3)]);
        assert!(
            host.physical_input(InputEvent::DialPress {
                index: 2,
                pressed: true
            })
            .is_err()
        );
    }
}
