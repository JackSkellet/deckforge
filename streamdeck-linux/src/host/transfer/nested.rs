//! Container paste shares the same clipboard, provenance and publication path as
//! canvas paste. It never serializes private resource bindings into action JSON.
use super::*;

pub(super) fn address(root: &Slot, context: &str) -> Option<ChildAnchor> {
    for (branch, group) in root.children.iter().enumerate() {
        for (index, child) in group.iter().enumerate() {
            if child.context == context {
                return Some(ChildAnchor {
                    parent: root.context.clone(),
                    branch,
                    index,
                });
            }
            if let Some(found) = address(child, context) {
                return Some(found);
            }
        }
    }
    None
}
fn positional(parent: &Slot) -> bool {
    matches!(
        parent.uuid.as_str(),
        crate::action_trigger::UUID | crate::key_logic::UUID
    )
}
fn selection(parent: &Slot) -> Result<Option<usize>> {
    let mut raw = parent.raw.clone();
    raw["Settings"] = parent.settings.clone();
    match parent.uuid.as_str() {
        crate::dial_stack::UUID => Ok(Some(crate::dial_stack::parse(&raw)?.current)),
        crate::action_wheel::UUID => Ok(Some(crate::action_wheel::parse(&raw)?.current)),
        _ => Ok(None),
    }
}
fn select(parent: &mut Slot, index: usize) {
    parent.settings["CurrentIdx"] = json!(index);
    parent.raw["Settings"] = parent.settings.clone();
}
pub(super) fn position_in(parent: &Slot, child: &ChildAnchor) -> (usize, usize) {
    if positional(parent) {
        (0, child.branch)
    } else {
        (child.branch, child.index)
    }
}

// Parser-generated IDs can be positional in imported records without ActionID.
// Bind the already-edited child order to the parser's new IDs, preserving the
// actual instances/media rather than accidentally reusing the old occupant.
fn rehydrate(parent: &mut Slot, actions: &[Action]) -> Result<()> {
    let previous = parent.children.clone();
    parent.hydrate_children(actions)?;
    ensure!(
        previous.len() == parent.children.len(),
        "Container branches changed unexpectedly"
    );
    for (group, old) in parent.children.iter_mut().zip(previous) {
        ensure!(
            group.len() == old.len(),
            "Container children changed unexpectedly"
        );
        for (parsed, mut owned) in group.iter_mut().zip(old) {
            ensure!(
                parsed.uuid == owned.uuid,
                "Container child order is inconsistent"
            );
            owned.child_id = parsed.child_id.clone();
            owned.raw = parsed.raw.clone();
            if owned.controller != parsed.controller {
                owned.layout = parsed.layout.clone();
            }
            owned.controller = parsed.controller.clone();
            owned.index = parsed.index;
            owned.event_coordinates = parsed.event_coordinates;
            owned.in_multi_action = parsed.in_multi_action;
            library::set_slot_model(&mut owned, parsed.model);
            *parsed = owned;
        }
    }
    parent.hydrate_children(actions)
}

pub(super) fn remove_source(
    library: &mut Library,
    anchor: &Anchor,
    actions: &[Action],
) -> Result<()> {
    let Some(child) = &anchor.child else {
        library.remove(&anchor.context);
        return Ok(());
    };
    let mut parent = library
        .slot(&child.parent)
        .context("Source container missing")?
        .clone();
    ensure!(
        parent
            .children
            .get(child.branch)
            .and_then(|g| g.get(child.index))
            .is_some_and(|s| s.context == anchor.context),
        "Cut source changed; cut the action again"
    );
    let positional = positional(&parent);
    let multi = multiaction::container(&parent.uuid);
    let selected = selection(&parent)?;
    parent.sync_children_raw();
    if positional {
        parent.raw["Actions"][child.branch] = json!({"Actions":[]});
        parent.children[child.branch].clear();
    } else {
        let list = if multi {
            &mut parent.raw["Actions"][child.branch]["Actions"]
        } else {
            &mut parent.raw["Actions"]
        };
        list.as_array_mut()
            .context("Source container list missing")?
            .remove(child.index);
        parent.children[child.branch].remove(child.index);
        if !multi {
            let current = selected.unwrap_or(0);
            let index = current
                .saturating_sub(usize::from(child.index < current))
                .min(parent.children[0].len().saturating_sub(1));
            if parent.settings["CurrentIdx"] != json!(index) {
                select(&mut parent, index);
            }
        }
    }
    rehydrate(&mut parent, actions)?;
    ensure!(library.update(&parent), "Source container missing");
    finish(library, actions)
}

pub(super) fn finish(library: &mut Library, actions: &[Action]) -> Result<()> {
    for root in library
        .profiles
        .iter_mut()
        .flat_map(|p| &mut p.pages)
        .flat_map(|p| &mut p.slots)
    {
        root.sync_children_raw();
        root.hydrate_children(actions)?;
    }
    library.validate()
}

fn allowed(parent: &Slot, incoming: &Slot, actions: &[Action]) -> Result<()> {
    let multi = multiaction::container(&parent.uuid);
    let stack = parent.uuid == crate::dial_stack::UUID;
    let logic = parent.uuid == crate::key_logic::UUID;
    ensure!(
        multi
            || stack
            || logic
            || matches!(
                parent.uuid.as_str(),
                crate::action_trigger::UUID | crate::action_wheel::UUID
            ),
        "Destination is not a container"
    );
    compatible(incoming, if stack { "Encoder" } else { "Keypad" }, actions)?;
    let action = actions.iter().find(|a| a.uuid == incoming.uuid);
    if multi {
        ensure!(
            !multiaction::container(&incoming.uuid)
                && !matches!(
                    incoming.uuid.as_str(),
                    crate::dial_stack::UUID
                        | crate::action_wheel::UUID
                        | crate::action_trigger::UUID
                        | crate::key_logic::UUID
                )
                && action.is_none_or(|a| a.raw["SupportedInMultiActions"] != false),
            "Action cannot be used in a Multi Action"
        );
    } else {
        ensure!(
            incoming.uuid != multiaction::DELAY,
            "Delay can only run inside a Multi Action"
        );
    }
    if logic {
        ensure!(
            action.is_none_or(|a| a.raw["SupportedInKeyLogicActions"] != false),
            "Action cannot be used in Key Logic"
        );
    }
    Ok(())
}

pub(super) fn insert(
    parent: &Slot,
    mut incoming: Slot,
    branch: usize,
    index: usize,
    actions: &[Action],
) -> Result<Slot> {
    allowed(parent, &incoming, actions)?;
    let multi = multiaction::container(&parent.uuid);
    let positional = matches!(
        parent.uuid.as_str(),
        crate::action_trigger::UUID | crate::key_logic::UUID
    );
    ensure!(
        if multi {
            branch
                < if parent.uuid == multiaction::SWITCH {
                    2
                } else {
                    1
                }
        } else {
            branch == 0
        },
        "Invalid container branch"
    );
    // Local canvas actions may not yet have a native record. Imported unknown
    // fields and explicit null Settings remain intact.
    if !incoming.raw.is_object() {
        incoming.raw = json!({});
    }
    incoming.raw["UUID"] = json!(incoming.uuid);
    incoming.raw["ActionID"] = json!(incoming.context);
    if incoming.raw.get("Name").is_none() {
        incoming.raw["Name"] = json!(incoming.name);
    }
    if incoming.raw["Settings"] != Value::Null || incoming.settings != json!({}) {
        incoming.raw["Settings"] = incoming.settings.clone();
    }
    incoming.raw["States"] = incoming.states.clone();
    if incoming.title_changed
        && let Some(state) = incoming.raw["States"].get_mut(incoming.state as usize)
        && state.is_object()
    {
        state["Title"] = json!(incoming.title);
        incoming.states = incoming.raw["States"].clone();
    }
    incoming.raw["State"] = json!(incoming.state);
    incoming.sync_children_raw();
    let mut candidate = parent.clone();
    candidate.sync_children_raw();
    // Hydration also materializes default empty groups without losing metadata.
    candidate.hydrate_children(actions)?;
    let selected = selection(&candidate)?;
    let has_children = candidate.children.iter().any(|g| !g.is_empty());
    if multi {
        let groups = candidate.raw["Actions"]
            .as_array_mut()
            .context("Container groups missing")?;
        while groups.len() <= branch {
            groups.push(json!({"Actions":[]}));
            candidate.children.push(vec![]);
        }
    }
    let list = if multi {
        candidate.raw["Actions"][branch]["Actions"].as_array_mut()
    } else {
        candidate.raw["Actions"].as_array_mut()
    }
    .context("Container action list is invalid")?;
    if positional {
        ensure!(index < 3, "Invalid positional child");
        while list.len() <= index {
            list.push(json!({"Actions":[]}));
        }
        list[index] = incoming.raw.clone();
    } else {
        ensure!(
            index <= list.len(),
            "Container position changed; select it again"
        );
        list.insert(index, incoming.raw.clone());
    }
    incoming.pinned = false;
    if positional {
        candidate.children[index] = vec![incoming];
    } else {
        candidate.children[branch].insert(index, incoming);
    }
    if let Some(selected) = selected
        && has_children
        && index <= selected
    {
        select(&mut candidate, selected + 1);
    }
    rehydrate(&mut candidate, actions)?;
    Ok(candidate)
}

impl Host {
    pub(super) fn paste_child(&mut self, request: &Value) -> Result<()> {
        let context = request["context"]
            .as_str()
            .context("Container context missing")?;
        let current = self.library.current()?;
        ensure!(!current.readonly, "Cannot edit a read-only profile");
        let (p, g, _) = locate(&self.library, context)?;
        ensure!(
            self.library.profiles[p].id == current.id
                && self
                    .library
                    .visible()?
                    .iter()
                    .any(|s| s.find(context).is_some()),
            "Destination container is no longer visible"
        );
        let parent = self.library.slot(context).context("Container missing")?;
        ensure!(
            request.get("expected") == Some(&parent.raw),
            "Container changed; save or reload before pasting"
        );
        let index = usize::try_from(
            request["index"]
                .as_u64()
                .context("Paste position missing")?,
        )?;
        let branch = usize::try_from(request["branch"].as_u64().context("Paste branch missing")?)?;
        let source = self
            .clipboard
            .as_ref()
            .context("Action clipboard is empty")?;
        let source = if source.cut {
            let latest = capture(&self.library, &source.anchor.context, true)?;
            ensure!(
                !stale(&source.anchor, &latest.anchor),
                "Cut source changed; cut the action again"
            );
            latest
        } else {
            source.clone()
        };
        ensure!(
            source.pages.is_empty() || source.slot.model.profile_compatible(current.model),
            "Folder geometry must match the destination device model"
        );
        if source.cut {
            ensure!(
                source.slot.find(context).is_none(),
                "Cannot move an action into itself"
            );
            let page = &self.library.profiles[p].pages[g].id;
            ensure!(
                !source.pages.iter().any(|p| &p.id == page),
                "Cannot move a folder inside itself"
            );
        }
        let (incoming, pages) = materialize(
            &source,
            !source.cut || source.anchor.profile != current.id,
            &self.actions,
        )?;
        let selected = selection(parent)?;
        let moving_selected = source.cut
            && source
                .anchor
                .child
                .as_ref()
                .is_some_and(|child| child.parent == context && selected == Some(child.index));
        let mut next = self.library.clone();
        let mut index = index;
        if source.cut {
            if let Some(child) = &source.anchor.child
                && child.parent == context
                && !positional(parent)
                && child.branch == branch
                && child.index < index
            {
                index -= 1;
            }
            remove_source(&mut next, &source.anchor, &self.actions)?;
        }
        let parent = next
            .slot(context)
            .context("Destination container disappeared")?;
        let mut candidate = insert(parent, incoming, branch, index, &self.actions)?;
        if moving_selected {
            select(&mut candidate, index);
        }
        ensure!(next.update(&candidate), "Destination container disappeared");
        next.current_mut()?.pages.extend(pages);
        finish(&mut next, &self.actions)?;
        self.publish_transfer(next, &source, "child_paste")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(root: &Path) -> Host {
        let mut host = super::super::super::tests::fixture(root);
        host.actions.extend(multiaction::definitions());
        host.actions[0].controllers.push("Encoder".into());
        host
    }
    fn paste(host: &mut Host, parent: &str, branch: usize, index: usize) -> Result<()> {
        let expected = host.slot_any(parent).unwrap().raw.clone();
        host.transfer_command(&json!({"command":"child_paste","context":parent,
            "branch":branch,"index":index,"expected":expected}))
    }
    #[test]
    fn copy_nested_step_to_canvas_preserves_bindings_and_resets_parent_flags() {
        let dir = tempfile::tempdir().unwrap();
        let mut host = fixture(dir.path());
        host.assign("Keypad", 0, multiaction::ROUTINE).unwrap();
        let parent = host.slots[0].context.clone();
        host.save_multi(
            &parent,
            json!({"Actions":[{"Actions":[{
            "UUID":"test.action","ActionID":"step","Settings":{"value":42},
            "States":[{"Title":"child"}],"Future":7}]}]}),
        )
        .unwrap();
        let child = host.slots[0].children[0][0].clone();
        let resource = dir.path().join("resource.txt");
        fs::write(&resource, b"portable resource").unwrap();
        host.set_resources(&child, json!({"file":resource}))
            .unwrap();
        host.transfer_command(&json!({"command":"slot_copy","context":child.context}))
            .unwrap();
        host.transfer_command(
            &json!({"command":"slot_paste","controller":"Keypad","index":1,"expected_target":null}),
        )
        .unwrap();
        let pasted = host.slots.iter().find(|s| s.index == 1).unwrap();
        assert_ne!(pasted.context, child.context);
        assert_eq!(pasted.settings, json!({"value":42}));
        assert_eq!(pasted.raw["Future"], 7);
        assert!(!pasted.in_multi_action);
        assert!(pasted.event_coordinates.is_none());
        assert!(pasted.child_id.is_empty());
        assert_eq!(pasted.coordinates(), (1, 0));
        assert_eq!(
            pasted.resources.as_ref().unwrap()["file"],
            resource.to_str().unwrap()
        );
        assert!(host.slots[0].children[0][0].in_multi_action);
        host.transfer_command(&json!({"command":"slot_cut","context":child.context}))
            .unwrap();
        assert!(host.slot_any(&child.context).is_some());
    }
    #[test]
    fn paste_into_switch_branch_keeps_resources_and_independent_identities() {
        let dir = tempfile::tempdir().unwrap();
        let mut host = fixture(dir.path());
        let source = host.slots[0].context.clone();
        host.slots[0].settings = json!({"shared":false});
        host.slots[0].raw = json!({"Future":"keep"});
        host.slots[0].title = "Custom title".into();
        host.slots[0].title_changed = true;
        host.slots[0].image_changed = true;
        host.slots[0].background = Some(DynamicImage::new_rgba8(2, 2));
        let resource = dir.path().join("file.txt");
        fs::write(&resource, b"nested").unwrap();
        let slot = host.slots[0].clone();
        host.set_resources(&slot, json!({"file":resource})).unwrap();
        host.assign("Keypad", 1, multiaction::SWITCH).unwrap();
        let parent = host
            .slots
            .iter()
            .find(|s| s.index == 1)
            .unwrap()
            .context
            .clone();
        host.transfer_command(&json!({"command":"slot_copy","context":source}))
            .unwrap();
        paste(&mut host, &parent, 1, 0).unwrap();
        paste(&mut host, &parent, 1, 1).unwrap();
        let container = host.slot_any(&parent).unwrap();
        assert!(container.children[0].is_empty());
        let children = &container.children[1];
        assert_eq!(children.len(), 2);
        assert_ne!(children[0].context, children[1].context);
        assert_ne!(children[0].raw["ActionID"], children[1].raw["ActionID"]);
        assert_eq!(children[0].raw["Future"], "keep");
        assert_eq!(children[0].settings, json!({"shared":false}));
        assert_eq!(children[0].title, "Custom title");
        assert!(children[0].image_changed && children[0].background.is_some());
        assert!(children[0].in_multi_action);
        assert_eq!(children[0].coordinates(), (1, 0));
        assert_eq!(
            children[0].resources.as_ref().unwrap()["file"],
            resource.to_str().unwrap()
        );
        let output = dir.path().join("nested.streamDeckProfile");
        host.export_profile(&output).unwrap();
        let bundle = profile::load_bundle(&output).unwrap();
        let root = bundle
            .page(&bundle.current_page)
            .unwrap()
            .slots
            .iter()
            .find(|s| s.uuid == multiaction::SWITCH)
            .unwrap();
        let id = root.raw["Actions"][1]["Actions"][0]["Resources"]["file"]
            .as_str()
            .unwrap();
        assert_eq!(bundle.resource(id).unwrap().1, b"nested");
    }
    #[test]
    fn paste_into_nested_sequence_and_positional_trigger_preserves_parent_tree() {
        let dir = tempfile::tempdir().unwrap();
        let mut host = fixture(dir.path());
        host.actions.push(trigger::action());
        host.actions.push(stack::action());
        let source = host.slots[0].context.clone();
        host.assign("Encoder", 2, crate::action_trigger::UUID)
            .unwrap();
        let root = host
            .slots
            .iter()
            .find(|s| s.controller == "Encoder")
            .unwrap()
            .context
            .clone();
        host.save_trigger(
            &root,
            json!({"FutureRoot":1,"Actions":[{}, {
            "UUID":multiaction::ROUTINE,"ActionID":"nested","FutureParent":2,
            "Actions":[{"Actions":[]},{"Actions":[]}]},{}]}),
        )
        .unwrap();
        let parent = host.slot_any(&root).unwrap().children[1][0].context.clone();
        host.transfer_command(&json!({"command":"slot_copy","context":source}))
            .unwrap();
        paste(&mut host, &parent, 0, 0).unwrap();
        let root_slot = host.slot_any(&root).unwrap();
        assert_eq!(root_slot.raw["FutureRoot"], 1);
        assert_eq!(root_slot.raw["Actions"][1]["FutureParent"], 2);
        assert_eq!(
            root_slot.raw["Actions"][1]["Actions"][0]["Actions"][0]["UUID"],
            "test.action"
        );
        let child = &host.slot_any(&parent).unwrap().children[0][0];
        assert_eq!(child.coordinates(), (2, 0));
        assert!(child.in_multi_action);
        paste(&mut host, &root, 0, 2).unwrap();
        assert!(!host.slot_any(&root).unwrap().children[2][0].in_multi_action);
        host.assign("Encoder", 0, crate::dial_stack::UUID).unwrap();
        let stack = host
            .slots
            .iter()
            .find(|s| s.uuid == crate::dial_stack::UUID)
            .unwrap()
            .context
            .clone();
        paste(&mut host, &stack, 0, 0).unwrap();
        let child = &host.slot_any(&stack).unwrap().children[0][0];
        assert_eq!(child.controller, "Encoder");
        assert!(!child.in_multi_action);
        assert_eq!(child.coordinates(), (0, 0));
    }
    #[test]
    fn cut_legacy_step_moves_only_on_paste_and_keeps_neighbor_bindings() {
        let dir = tempfile::tempdir().unwrap();
        let mut host = fixture(dir.path());
        host.assign("Keypad", 0, multiaction::ROUTINE).unwrap();
        let parent = host.slots[0].context.clone();
        host.save_multi(
            &parent,
            json!({"Actions":[{"Actions":[
            {"UUID":"test.action","Settings":{"value":"first"}},
            {"UUID":"test.action","Settings":{"value":"second"}},
            {"UUID":"test.action","Settings":{"value":"third"}}]}]}),
        )
        .unwrap();
        let original = host.slots[0].children[0].clone();
        for (i, child) in original.iter().enumerate() {
            let path = dir.path().join(format!("{i}.txt"));
            fs::write(&path, i.to_string()).unwrap();
            host.set_resources(child, json!({"file":path})).unwrap();
        }
        let (tx, mut rx) = mpsc::unbounded_channel();
        host.plugins[0].sender = Some(tx);
        let entry = crate::multi::Sequence::parse(multiaction::ROUTINE, &host.slots[0].raw)
            .unwrap()
            .groups[0][0]
            .clone();
        host.multi_owned.insert(parent.clone());
        host.dispatch_multi(
            crate::multi::Event::Press {
                parent: parent.clone(),
                entry,
            },
            false,
        )
        .unwrap();
        while rx.try_recv().is_ok() {}
        host.transfer_command(&json!({"command":"slot_cut","context":original[0].context}))
            .unwrap();
        assert!(host.slot_any(&original[0].context).is_some());
        assert!(host.multi_running(&parent));
        assert_eq!(host.clipboard_snapshot()["root_context"], parent);
        paste(&mut host, &parent, 0, 3).unwrap();
        assert!(!host.multi_running(&parent));
        assert!(host.clipboard.is_none());
        let children = &host.slot_any(&parent).unwrap().children[0];
        assert_eq!(
            children
                .iter()
                .map(|c| c.settings["value"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["second", "third", "first"]
        );
        assert_eq!(children[0].context, original[1].context);
        assert_eq!(children[1].context, original[2].context);
        for (child, number) in children.iter().zip([1, 2, 0]) {
            assert_eq!(
                fs::read_to_string(&child.resources.as_ref().unwrap()["file"]).unwrap(),
                number.to_string()
            );
        }
        let events: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        let released = events
            .iter()
            .position(|v| v["event"] == "keyUp" && v["context"] == original[0].context)
            .unwrap();
        let disappeared = events
            .iter()
            .position(|v| v["event"] == "willDisappear" && v["context"] == original[0].context)
            .unwrap();
        assert!(released < disappeared);
    }
    #[test]
    fn cut_child_swaps_with_canvas_and_rejects_ancestor_or_incompatible_swap() {
        let dir = tempfile::tempdir().unwrap();
        let mut host = fixture(dir.path());
        host.assign("Keypad", 0, multiaction::ROUTINE).unwrap();
        let parent = host.slots[0].context.clone();
        host.save_multi(
            &parent,
            json!({"Actions":[{"Actions":[{"UUID":"test.action","Settings":{"from":"child"}}]}]}),
        )
        .unwrap();
        host.assign("Keypad", 1, "test.action").unwrap();
        let mut target = host.slots.iter().find(|s| s.index == 1).unwrap().clone();
        target.settings = json!({"from":"canvas"});
        host.store_slot(target.clone());
        let child = host.slot_any(&parent).unwrap().children[0][0]
            .context
            .clone();
        host.transfer_command(&json!({"command":"slot_cut","context":child}))
            .unwrap();
        let before = serde_json::to_value(&host.library).unwrap();
        assert!(host.transfer_command(&json!({"command":"slot_paste","controller":"Keypad","index":0,"expected_target":parent})).is_err());
        assert_eq!(serde_json::to_value(&host.library).unwrap(), before);
        host.transfer_command(&json!({"command":"slot_paste","controller":"Keypad","index":1,"expected_target":target.context})).unwrap();
        assert_eq!(
            host.slot_any(&parent).unwrap().children[0][0].settings,
            json!({"from":"canvas"})
        );
        assert_eq!(
            host.slots.iter().find(|s| s.index == 1).unwrap().settings,
            json!({"from":"child"})
        );
        assert!(host.slot_any(&child).is_none());
        host.assign("Keypad", 2, multiaction::ROUTINE).unwrap();
        let incompatible = host
            .slots
            .iter()
            .find(|s| s.index == 2)
            .unwrap()
            .context
            .clone();
        let child = host.slot_any(&parent).unwrap().children[0][0]
            .context
            .clone();
        host.transfer_command(&json!({"command":"slot_cut","context":child}))
            .unwrap();
        let before = serde_json::to_value(&host.library).unwrap();
        assert!(host.transfer_command(&json!({"command":"slot_paste","controller":"Keypad","index":2,"expected_target":incompatible})).is_err());
        assert_eq!(serde_json::to_value(&host.library).unwrap(), before);
    }
    #[test]
    fn positional_cut_leaves_blank_and_stale_or_failed_moves_keep_source() {
        let dir = tempfile::tempdir().unwrap();
        let mut host = fixture(dir.path());
        host.actions.push(trigger::logic_action());
        host.assign("Keypad", 0, crate::key_logic::UUID).unwrap();
        let parent = host.slots[0].context.clone();
        host.save_trigger(&parent,json!({"Actions":[{}, {"UUID":"test.action","ActionID":"held","Settings":{"keep":1}}, {}]})).unwrap();
        let child = host.slot_any(&parent).unwrap().children[1][0]
            .context
            .clone();
        host.transfer_command(&json!({"command":"slot_cut","context":child}))
            .unwrap();
        let before = serde_json::to_value(&host.library).unwrap();
        let state = host.state_path.clone();
        host.state_path = dir.path().join("directory");
        fs::create_dir(&host.state_path).unwrap();
        assert!(host.transfer_command(&json!({"command":"slot_paste","controller":"Keypad","index":1,"expected_target":null})).is_err());
        assert_eq!(serde_json::to_value(&host.library).unwrap(), before);
        assert!(host.clipboard.as_ref().unwrap().cut);
        host.state_path = state;
        host.transfer_command(
            &json!({"command":"slot_paste","controller":"Keypad","index":1,"expected_target":null}),
        )
        .unwrap();
        assert_eq!(
            host.slot_any(&parent).unwrap().raw["Actions"][1],
            json!({"Actions":[]})
        );
        assert_eq!(host.slot_any(&parent).unwrap().children.len(), 3);
        assert!(host.slot_any(&parent).unwrap().children[1].is_empty());
    }
    #[test]
    fn cut_rejects_reordered_source_and_stack_move_tracks_selected_child() {
        let dir = tempfile::tempdir().unwrap();
        let mut host = fixture(dir.path());
        host.actions.push(stack::action());
        host.assign("Encoder", 0, crate::dial_stack::UUID).unwrap();
        let parent = host
            .slots
            .iter()
            .find(|s| s.controller == "Encoder")
            .unwrap()
            .context
            .clone();
        host.save_stack(
            &parent,
            json!({"Settings":{"CurrentIdx":1},"Actions":[
            {"UUID":"test.action","ActionID":"a","Settings":{"label":"a"}},
            {"UUID":"test.action","ActionID":"b","Settings":{"label":"b"}},
            {"UUID":"test.action","ActionID":"c","Settings":{"label":"c"}}]}),
        )
        .unwrap();
        let child = host.slot_any(&parent).unwrap().children[0][1]
            .context
            .clone();
        host.transfer_command(&json!({"command":"slot_cut","context":child}))
            .unwrap();
        paste(&mut host, &parent, 0, 0).unwrap();
        let stack = host.slot_any(&parent).unwrap();
        assert_eq!(stack.settings["CurrentIdx"], 0);
        assert_eq!(stack.children[0][0].settings["label"], "b");
        let child = stack.children[0][0].context.clone();
        host.transfer_command(&json!({"command":"slot_cut","context":child}))
            .unwrap();
        let mut raw = host.slot_any(&parent).unwrap().raw.clone();
        raw["Actions"].as_array_mut().unwrap().swap(0, 1);
        host.save_stack(&parent, raw).unwrap();
        assert!(
            paste(&mut host, &parent, 0, 3)
                .unwrap_err()
                .to_string()
                .contains("Cut source changed")
        );
        assert!(host.clipboard.as_ref().unwrap().cut);
        // Missing ActionIDs also fail closed when their positional occupant changes.
        let raw = json!({"Actions":[{"UUID":"test.action","Settings":{"label":"old"}}]});
        host.save_stack(&parent, raw).unwrap();
        let child = host.slot_any(&parent).unwrap().children[0][0]
            .context
            .clone();
        host.transfer_command(&json!({"command":"slot_cut","context":child}))
            .unwrap();
        host.save_stack(
            &parent,
            json!({"Actions":[{"UUID":"test.action","Settings":{"label":"new"}}]}),
        )
        .unwrap();
        assert!(
            paste(&mut host, &parent, 0, 1)
                .unwrap_err()
                .to_string()
                .contains("Cut source changed")
        );
    }
    #[test]
    fn nested_cut_moves_across_devices_without_disturbing_the_source_parent() {
        let dir = tempfile::tempdir().unwrap();
        let mut host = fixture(dir.path());
        host.assign("Keypad", 0, multiaction::ROUTINE).unwrap();
        let parent = host.slots[0].context.clone();
        host.save_multi(&parent,json!({"Actions":[{"Actions":[{"UUID":"test.action","ActionID":"leaf","Settings":{"from":"A"}}]}]})).unwrap();
        let child = host.slot_any(&parent).unwrap().children[0][0]
            .context
            .clone();
        host.add_simulated_device("B", Model::Mini).unwrap();
        let target = host
            .with_device("B", |host| {
                host.assign("Keypad", 0, multiaction::ROUTINE)?;
                Ok(host.slots[0].context.clone())
            })
            .unwrap();
        host.transfer_command(&json!({"command":"slot_cut","context":child}))
            .unwrap();
        host.with_device("B", |host| paste(host, &target, 0, 0))
            .unwrap();
        assert!(host.slot_any(&parent).unwrap().children[0].is_empty());
        let child = &host.slot_any(&target).unwrap().children[0][0];
        assert_eq!(child.settings, json!({"from":"A"}));
        assert_eq!(child.model, Model::Mini);
        assert_eq!(child.coordinates(), (0, 0));
        assert!(host.clipboard.is_none());
    }
    #[test]
    fn stale_incompatible_and_failed_paste_leave_cut_source_intact() {
        let dir = tempfile::tempdir().unwrap();
        let mut host = fixture(dir.path());
        let source = host.slots[0].context.clone();
        host.assign("Keypad", 1, multiaction::ROUTINE).unwrap();
        let parent = host
            .slots
            .iter()
            .find(|s| s.index == 1)
            .unwrap()
            .context
            .clone();
        host.transfer_command(&json!({"command":"slot_cut","context":source}))
            .unwrap();
        let before = serde_json::to_value(&host.library).unwrap();
        assert!(
            host.transfer_command(&json!({"command":"child_paste","context":parent,
            "branch":0,"index":0,"expected":{}}))
                .is_err()
        );
        host.actions[0].raw["SupportedInMultiActions"] = json!(false);
        assert!(paste(&mut host, &parent, 0, 0).is_err());
        host.actions[0].raw["SupportedInMultiActions"] = json!(true);
        let original = host.state_path.clone();
        host.state_path = dir.path().join("directory");
        fs::create_dir(&host.state_path).unwrap();
        assert!(paste(&mut host, &parent, 0, 0).is_err());
        assert_eq!(serde_json::to_value(&host.library).unwrap(), before);
        assert!(host.clipboard.as_ref().unwrap().cut);
        host.state_path = original;
        paste(&mut host, &parent, 0, 0).unwrap();
        assert!(host.clipboard.is_none());
        assert!(host.slot_any(&source).is_none());
        assert_eq!(host.slot_any(&parent).unwrap().children[0].len(), 1);
    }
}
