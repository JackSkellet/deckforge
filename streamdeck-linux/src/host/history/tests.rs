use super::*;
fn fixture(root: &Path) -> Host {
    crate::host::tests::fixture(root)
}
fn command(host: &mut Host, value: Value) {
    apply_control(host, value).unwrap();
}
fn reverse(host: &mut Host, undo: bool) -> Result<()> {
    host.history_command(&json!({"command":if undo{"history_undo"}else{"history_redo"},"expected_revision":host.history.revision}))
}
#[test]
fn grouped_settings_preserve_plugin_updates_and_notify_surviving_contexts() {
    let dir = tempfile::tempdir().unwrap();
    let mut host = fixture(dir.path());
    let context = host.slots[0].context.clone();
    let original = host.slots[0].settings.clone();
    command(
        &mut host,
        json!({"command":"settings_patch","context":context,"base":original,"settings":{"user":1},"history_group":"save"}),
    );
    let base = host.slots[0].settings.clone();
    command(
        &mut host,
        json!({"command":"settings_patch","context":context,"base":base,"settings":{"user":1,"second":2},"history_group":"save"}),
    );
    assert_eq!(host.history.undo.len(), 1);
    let mut current = host.slots[0].clone();
    current.settings["live"] = json!(73);
    host.store_slot(current);
    let (tx, mut rx) = mpsc::unbounded_channel();
    host.plugins[0].sender = Some(tx);
    reverse(&mut host, true).unwrap();
    assert_eq!(host.slots[0].settings, json!({"live":73}));
    assert!(
        std::iter::from_fn(|| rx.try_recv().ok())
            .any(|e| e["event"] == "didReceiveSettings" && e["context"] == context)
    );
    reverse(&mut host, false).unwrap();
    assert_eq!(
        host.slots[0].settings,
        json!({"user":1,"second":2,"live":73})
    );
}
#[test]
fn assigned_action_can_undo_after_plugin_updates_and_redo_latest_payload() {
    let dir = tempfile::tempdir().unwrap();
    let mut host = fixture(dir.path());
    let original = host.slots[0].context.clone();
    command(
        &mut host,
        json!({"command":"assign","controller":"Keypad","index":1,"uuid":"test.action"}),
    );
    let mut created = host.slots.iter().find(|s| s.index == 1).unwrap().clone();
    created.settings = json!({"plugin":51});
    created.feedback = json!({"value":88});
    let id = created.context.clone();
    host.store_slot(created);
    host.input(InputEvent::Key {
        index: 1,
        pressed: true,
    })
    .unwrap();
    reverse(&mut host, true).unwrap();
    assert!(host.library.slot(&id).is_none());
    assert!(host.held.is_empty());
    assert!(host.library.slot(&original).is_some());
    reverse(&mut host, false).unwrap();
    let restored = host.library.slot(&id).unwrap();
    assert_eq!(restored.settings["plugin"], 51);
    assert_eq!(restored.feedback["value"], 88);
}
#[test]
fn conflict_and_failed_save_keep_history_graph_and_held_controls() {
    let dir = tempfile::tempdir().unwrap();
    let mut host = fixture(dir.path());
    let context = host.slots[0].context.clone();
    command(
        &mut host,
        json!({"command":"settings","context":context,"settings":{"v":1}}),
    );
    let mut current = host.slots[0].clone();
    current.settings = json!({"v":2});
    host.store_slot(current);
    let revision = host.history.revision;
    assert!(reverse(&mut host, true).is_err());
    assert_eq!(host.history.revision, revision);
    assert_eq!(host.slots[0].settings["v"], 2);
    let mut current = host.slots[0].clone();
    current.settings = json!({"v":1});
    host.store_slot(current);
    host.input(InputEvent::Key {
        index: 0,
        pressed: true,
    })
    .unwrap();
    host.sync_all_devices();
    let before = serde_json::to_value(&host.library).unwrap();
    let (tx, mut rx) = mpsc::unbounded_channel();
    host.plugins[0].sender = Some(tx);
    host.state_path = dir.path().join("directory");
    fs::create_dir(&host.state_path).unwrap();
    assert!(reverse(&mut host, true).is_err());
    assert_eq!(host.history.revision, revision);
    assert_eq!(host.history.undo.len(), 1);
    assert!(host.history.redo.is_empty());
    assert_eq!(serde_json::to_value(&host.library).unwrap(), before);
    assert_eq!(host.held.get(&HeldControl::Key(0)), Some(&context));
    assert!(rx.try_recv().is_err());
}
#[test]
fn cross_device_cut_reverses_together_without_releasing_unaffected_key() {
    let dir = tempfile::tempdir().unwrap();
    let mut host = fixture(dir.path());
    let a = host.device.clone();
    let source = host.slots[0].context.clone();
    host.add_simulated_device("B", Model::Plus).unwrap();
    let held = host
        .with_device("B", |host| {
            host.assign("Keypad", 0, "test.action")?;
            let c = host.slots[0].context.clone();
            host.input(InputEvent::Key {
                index: 0,
                pressed: true,
            })?;
            Ok(c)
        })
        .unwrap();
    command(
        &mut host,
        json!({"command":"slot_cut","device":a,"context":source}),
    );
    command(
        &mut host,
        json!({"command":"slot_paste","device":"B","controller":"Keypad","index":1,"expected_target":null}),
    );
    assert!(host.library.slot(&source).is_none());
    assert_eq!(host.history.undo.len(), 1);
    reverse(&mut host, true).unwrap();
    assert!(host.library.slot(&source).is_some());
    assert_eq!(host.device, a);
    host.with_device("B", |host| {
        assert_eq!(host.slots.len(), 1);
        assert_eq!(host.held.get(&HeldControl::Key(0)), Some(&held));
        Ok(())
    })
    .unwrap();
    reverse(&mut host, false).unwrap();
    assert!(host.library.slot(&source).is_none());
    host.with_device("B", |host| {
        assert_eq!(host.slots.len(), 2);
        assert_eq!(host.held.get(&HeldControl::Key(0)), Some(&held));
        Ok(())
    })
    .unwrap();
}
#[test]
fn profile_navigation_rules_redo_invalidation_and_stale_requests() {
    let dir = tempfile::tempdir().unwrap();
    let mut host = fixture(dir.path());
    let base = host.library.active_profile.clone();
    command(
        &mut host,
        json!({"command":"profile_create","name":"Created"}),
    );
    let created = host.library.active_profile.clone();
    let revision = host.history.revision;
    reverse(&mut host, true).unwrap();
    assert_eq!(host.library.active_profile, base);
    assert_eq!(host.library.profiles.len(), 1);
    assert!(
        host.history_command(&json!({"command":"history_redo","expected_revision":revision}))
            .is_err()
    );
    reverse(&mut host, false).unwrap();
    assert_eq!(host.library.active_profile, created);
    let expected = host.smart_config();
    command(
        &mut host,
        json!({"command":"smart_config","expected":expected,"settings":{"enabled":false,"bindings":{created.clone():{"kind":"window_class","value":"app","name":"App"}},"fallback":base}}),
    );
    reverse(&mut host, true).unwrap();
    assert!(host.smart_config().bindings.is_empty());
    command(
        &mut host,
        json!({"command":"profile_rename","id":created,"name":"Different"}),
    );
    assert!(host.history.redo.is_empty());
    let saved: Saved = serde_json::from_slice(&fs::read(&host.state_path).unwrap()).unwrap();
    assert_eq!(saved.library.unwrap().profiles.len(), 2);
}

#[test]
fn page_artwork_sources_survive_replacement_undo_and_export() {
    let dir = tempfile::tempdir().unwrap();
    let mut host = fixture(dir.path());
    let profile = host.library.active_profile.clone();
    let page = host.library.active_page.clone();
    let first = dir.path().join("first.png");
    let second = dir.path().join("second.png");
    image::RgbImage::from_pixel(800, 100, image::Rgb([200, 30, 40]))
        .save(&first)
        .unwrap();
    image::RgbImage::from_pixel(800, 100, image::Rgb([10, 80, 190]))
        .save(&second)
        .unwrap();
    command(
        &mut host,
        json!({"command":"page_background","profile":profile,"id":page,"path":first}),
    );
    let source = host
        .library
        .current()
        .unwrap()
        .pages
        .iter()
        .find(|p| p.id == page)
        .unwrap()
        .background_source
        .clone()
        .unwrap()
        .source;
    command(
        &mut host,
        json!({"command":"page_background","profile":profile,"id":page,"path":second}),
    );
    fs::remove_file(first).unwrap();
    fs::remove_file(second).unwrap();
    assert!(source.exists());
    reverse(&mut host, true).unwrap();
    assert_eq!(
        host.library
            .current()
            .unwrap()
            .pages
            .iter()
            .find(|p| p.id == page)
            .unwrap()
            .background_source
            .as_ref()
            .unwrap()
            .source,
        source
    );
    let output = dir.path().join("restored.streamDeckProfile");
    host.export_profile(&output).unwrap();
    let bundle = profile::load_bundle(&output).unwrap();
    assert!(bundle.files().any(|(_, b)| {
        image::load_from_memory(b)
            .ok()
            .is_some_and(|im| im.to_rgb8().get_pixel(0, 0).0 == [200, 30, 40])
    }));
    reverse(&mut host, false).unwrap();
    host.export_profile(&output).unwrap();
    assert!(
        profile::load_bundle(&output)
            .unwrap()
            .files()
            .any(|(_, b)| image::load_from_memory(b).ok().is_some_and(|im| im
                .to_rgb8()
                .get_pixel(0, 0)
                .0
                == [10, 80, 190]))
    );
}
#[test]
fn inspector_resource_edits_are_recorded_and_both_sides_receive_restoration() {
    let dir = tempfile::tempdir().unwrap();
    let mut host = fixture(dir.path());
    let context = host.slots[0].context.clone();
    let (tx, mut plugin) = mpsc::unbounded_channel();
    host.plugins[0].sender = Some(tx);
    let (tx, mut inspector) = mpsc::unbounded_channel();
    host.inspectors.insert(
        "pi".into(),
        Inspector {
            context: context.clone(),
            plugin: 0,
            child: None,
            sender: Some(tx),
        },
    );
    let file = dir.path().join("resource.bin");
    fs::write(&file, b"resource").unwrap();
    host.inspector_message(
        "pi",
        json!({"event":"setResources","context":"pi","payload":{"file":file}}),
    )
    .unwrap();
    assert_eq!(host.history.undo.len(), 1);
    while plugin.try_recv().is_ok() {}
    reverse(&mut host, true).unwrap();
    assert!(
        host.slots[0]
            .resources
            .as_ref()
            .is_none_or(BTreeMap::is_empty)
    );
    assert!(
        std::iter::from_fn(|| plugin.try_recv().ok()).any(|e| e["event"] == "didReceiveResources")
    );
    assert!(
        std::iter::from_fn(|| inspector.try_recv().ok())
            .any(|e| e["event"] == "didReceiveResources")
    );
    reverse(&mut host, false).unwrap();
    assert_eq!(
        host.slots[0].resources.as_ref().unwrap()["file"],
        file.to_str().unwrap()
    );
}
#[test]
fn nested_reorder_retains_later_child_settings_and_owned_resources() {
    let dir = tempfile::tempdir().unwrap();
    let mut host = fixture(dir.path());
    host.actions.extend(multiaction::definitions());
    host.assign("Keypad", 0, multiaction::ROUTINE).unwrap();
    let context = host.slots[0].context.clone();
    let a = json!({"UUID":"test.action","ActionID":"A","Settings":{"n":1},"States":[{}]});
    let b = json!({"UUID":"test.action","ActionID":"B","Settings":{"n":2},"States":[{}]});
    host.save_multi(
        &context,
        json!({"Actions":[{"Actions":[a,b]},{"Actions":[]}]}),
    )
    .unwrap();
    let a = host.slots[0].children[0][0].raw.clone();
    let b = host.slots[0].children[0][1].raw.clone();
    command(
        &mut host,
        json!({"command":"multi_save","context":context,"raw":{"Actions":[{"Actions":[b,a]},{"Actions":[]}]}}),
    );
    let mut child = host.slots[0].children[0][0].clone();
    let child_context = child.context.clone();
    child.settings["live"] = json!(90);
    child.resources = Some(BTreeMap::from([(
        "file".into(),
        "/tmp/owned-test-resource".into(),
    )]));
    host.store_slot(child);
    reverse(&mut host, true).unwrap();
    assert_eq!(host.slots[0].children[0][1].context, child_context);
    assert_eq!(host.slots[0].children[0][1].settings["live"], 90);
    assert_eq!(
        host.slots[0].children[0][1].resources.as_ref().unwrap()["file"],
        "/tmp/owned-test-resource"
    );
    reverse(&mut host, false).unwrap();
    assert_eq!(host.slots[0].children[0][0].context, child_context);
    assert_eq!(host.slots[0].children[0][0].settings["live"], 90);
}

#[test]
fn history_normalization_keeps_opaque_container_records_without_guessing() {
    let mut graph = json!({"profiles":[{"pages":[{"slots":[{
        "context":"opaque","uuid":crate::multi::ROUTINE,"children":[],
        "raw":{"Actions":{"unknown-native-structure":[1,2,3]},"unknown":73}
    }]}]}]});
    let before = graph.clone();
    normalize_graph(&mut graph, true).unwrap();
    normalize_graph(&mut graph, false).unwrap();
    assert_eq!(graph, before);
}

#[test]
fn session_history_is_bounded_and_noop_edits_do_not_invalidate_redo() {
    let dir = tempfile::tempdir().unwrap();
    let mut host = fixture(dir.path());
    let id = host.library.active_profile.clone();
    for n in 0..55 {
        command(
            &mut host,
            json!({"command":"profile_rename","id":id,"name":format!("Name {n}")}),
        );
    }
    assert_eq!(host.history.undo.len(), MAX_ENTRIES);
    assert!(host.history.note.is_some());
    reverse(&mut host, true).unwrap();
    let revision = host.history.revision;
    let name = host.library.current().unwrap().name.clone();
    command(
        &mut host,
        json!({"command":"profile_rename","id":id,"name":name}),
    );
    assert_eq!(host.history.revision, revision);
    assert_eq!(host.history.redo.len(), 1);
}

#[test]
fn appearance_restore_notifies_the_edited_state_even_when_another_state_is_visible() {
    let dir = tempfile::tempdir().unwrap();
    let mut host = fixture(dir.path());
    host.history_edit("Edit action", None, |host| {
        let mut slot = host.slots[0].clone();
        slot.states[1]["FontSize"] = json!(24);
        host.store_slot(slot);
        host.persist()
    })
    .unwrap();
    let (tx, mut rx) = mpsc::unbounded_channel();
    host.plugins[0].sender = Some(tx);
    reverse(&mut host, true).unwrap();
    let changes: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok())
        .filter(|e| e["event"] == "titleParametersDidChange")
        .collect();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0]["payload"]["state"], 1);
    assert_eq!(host.slots[0].state, 0);
}
