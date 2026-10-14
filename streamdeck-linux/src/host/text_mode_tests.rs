use super::*;

fn host(root: &Path) -> Host {
    let mut host = tests::fixture(root);
    host.actions.push(Action {
        uuid: crate::system_actions::TEXT.into(),
        name: "Text".into(),
        plugin: "deckforge.system".into(),
        controllers: vec!["Keypad".into()],
        raw: json!({"States":[{"Title":"Text"}]}),
    });
    host.actions.extend(multiaction::definitions());
    host
}
fn text(name: &str, settings: Value) -> Value {
    json!({"UUID":crate::system_actions::TEXT,"Name":name,"ActionID":name,"Settings":settings,"States":[{"Title":name}],"Future":7})
}
fn fixture(path: &Path, os: Option<&str>, shared: bool) {
    let id = uuid::Uuid::new_v4().to_string();
    let main = uuid::Uuid::new_v4().to_string();
    let pins = uuid::Uuid::new_v4().to_string();
    let folder = uuid::Uuid::new_v4().to_string();
    let root = format!(
        "{}{id}.sdProfile/",
        if os.is_some() || shared {
            "Profiles/"
        } else {
            ""
        }
    );
    let legacy = text(
        "Legacy",
        json!({"pastedText":"legacy 🌍","FutureSetting":9}),
    );
    let mut actions = json!({"0,0":legacy.clone()});
    if !shared {
        actions["1,0"] = text(
            "Paste",
            json!({"pastedText":"explicit paste","isTypingMode":false}),
        );
        actions["2,0"] = text(
            "Type",
            json!({"pastedText":"explicit type","isTypingMode":true}),
        );
        actions["3,0"] = text("Empty", json!({"pastedText":"","isSendingEnter":true}));
        actions["0,1"] = json!({"UUID":multiaction::ROUTINE,"Name":"Sequence","Settings":{},"Actions":[{"Actions":[text("Nested",json!({"pastedText":"nested"}))]}],"Future":5});
        actions["1,1"] = json!({"UUID":"unknown.container","Name":"Opaque","ActionID":"opaque","Settings":{"Future":1},"Actions":[legacy]});
        actions["2,1"] = json!({"UUID":library::OPEN_FOLDER,"Name":"Folder","Settings":{"ProfileUUID":folder},"States":[{}]});
    }
    let mut files = BTreeMap::from([
        (format!("{root}manifest.json"), serde_json::to_vec(&json!({"Name":"Text defaults","Version":"2.0","Device":{"Model":if shared {"Fake/Storage"} else {Model::Plus.profile_model()}},"Pages":{"Current":main,"Default":pins,"Pages":[main]},"FutureRoot":3})).unwrap()),
        (format!("{root}Profiles/{main}/manifest.json"), serde_json::to_vec(&json!({"Controllers":[{"Type":"Keypad","Actions":actions}],"FuturePage":4})).unwrap()),
        (format!("{root}Profiles/{pins}/manifest.json"), serde_json::to_vec(&json!({"Controllers":[]})).unwrap()),
        (format!("{root}Profiles/{folder}/manifest.json"), serde_json::to_vec(&json!({"Controllers":[{"Type":"Keypad","Actions":{"0,0":text("Hidden",json!({"pastedText":"hidden"}))}}]})).unwrap()),
        (format!("{root}retained-asset.bin"), b"unchanged asset bytes".to_vec()),
    ]);
    if os.is_some() || shared {
        let mut package = json!({"FormatVersion":1,"RequiredPlugins":[]});
        if let Some(os) = os {
            package["OSType"] = json!(os);
        }
        files.insert("package.json".into(), serde_json::to_vec(&package).unwrap());
    }
    export::write_archive(path, &files).unwrap();
}
fn named<'a>(profile: &'a library::Profile, name: &str) -> &'a Slot {
    profile
        .pages
        .iter()
        .flat_map(|p| &p.slots)
        .find(|s| s.name == name)
        .unwrap()
}

#[test]
fn fresh_native_and_linux_imports_materialize_only_missing_known_text_modes() {
    for os in [None, Some("Windows"), Some("Linux")] {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source.streamDeckProfile");
        fixture(&source, os, false);
        let bytes = fs::read(&source).unwrap();
        let mut host = host(root.path());
        let (profile, _) = host
            .prepare_bundle(profile::load_bundle(&source).unwrap(), None, false)
            .unwrap();
        let native = os != Some("Linux");
        for name in ["Legacy", "Hidden"] {
            let slot = named(&profile, name);
            assert_eq!(slot.settings["isTypingMode"], native);
            assert_eq!(slot.raw["Settings"], slot.settings);
        }
        assert_eq!(named(&profile, "Paste").settings["isTypingMode"], false);
        assert_eq!(named(&profile, "Type").settings["isTypingMode"], true);
        assert_eq!(named(&profile, "Empty").settings["isTypingMode"], false);
        assert_eq!(
            named(&profile, "Sequence").raw["Actions"][0]["Actions"][0]["Settings"]["isTypingMode"],
            native
        );
        assert!(
            named(&profile, "Opaque").raw["Actions"][0]["Settings"]
                .get("isTypingMode")
                .is_none()
        );
        assert_eq!(named(&profile, "Legacy").settings["FutureSetting"], 9);
        assert!(host.system_actions.is_none());
        assert_eq!(fs::read(&source).unwrap(), bytes);
        let cached = profile::load_bundle(profile.source.as_ref().unwrap()).unwrap();
        let original = profile::load_bundle(&source).unwrap();
        assert_eq!(
            cached.files().collect::<BTreeMap<_, _>>(),
            original.files().collect::<BTreeMap<_, _>>()
        );
    }
}

#[test]
fn export_preserves_legacy_runtime_behavior_without_mutating_saved_or_unvisited_data() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("original.streamDeckProfile");
    fixture(&source, None, false);
    let mut host = host(root.path());
    let (mut profile, current) = host
        .prepare_bundle(profile::load_bundle(&source).unwrap(), None, false)
        .unwrap();
    // An older saved session, including raw-only children on unvisited pages.
    for slot in profile.pages.iter_mut().flat_map(|p| &mut p.slots) {
        if slot.uuid == crate::system_actions::TEXT && slot.name != "Type" && slot.name != "Paste" {
            slot.settings
                .as_object_mut()
                .unwrap()
                .remove("isTypingMode");
            slot.raw["Settings"]
                .as_object_mut()
                .unwrap()
                .remove("isTypingMode");
        }
        if slot.uuid == multiaction::ROUTINE {
            slot.raw["Actions"][0]["Actions"][0]["Settings"]
                .as_object_mut()
                .unwrap()
                .remove("isTypingMode");
            assert!(slot.children.is_empty());
        }
    }
    let before = serde_json::to_value(&profile).unwrap();
    let files = export::build_profile_files(&profile, &current, &HashMap::new()).unwrap();
    let output = root.path().join("export.streamDeckProfile");
    export::write_archive(&output, &files).unwrap();
    assert_eq!(serde_json::to_value(&profile).unwrap(), before);
    let (reimported, _) = host
        .prepare_bundle(profile::load_bundle(&output).unwrap(), None, false)
        .unwrap();
    for name in ["Legacy", "Hidden", "Empty", "Paste"] {
        assert_eq!(named(&reimported, name).settings["isTypingMode"], false);
    }
    assert_eq!(named(&reimported, "Type").settings["isTypingMode"], true);
    assert_eq!(
        named(&reimported, "Sequence").raw["Actions"][0]["Actions"][0]["Settings"]["isTypingMode"],
        false
    );
    assert_eq!(
        named(&reimported, "Opaque").raw,
        named(&profile, "Opaque").raw
    );
    assert!(matches!(
        crate::system_actions::plan(
            crate::system_actions::TEXT,
            &named(&profile, "Legacy").settings
        )
        .unwrap(),
        crate::system_actions::Plan::Paste { .. }
    ));
}

#[test]
fn shared_action_import_uses_the_same_defaults_without_requesting_keyboard_access() {
    for os in [None, Some("Linux")] {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("shared.streamDeckAction");
        fixture(&source, os, true);
        let mut host = host(root.path());
        let count = host.library.profiles.len();
        host.load_action_clipboard(&source).unwrap();
        assert_eq!(host.library.profiles.len(), count);
        host.transfer_command(
            &json!({"command":"slot_paste","controller":"Keypad","index":1,"expected_target":null}),
        )
        .unwrap();
        assert_eq!(
            host.slots.iter().find(|s| s.index == 1).unwrap().settings["isTypingMode"],
            os != Some("Linux")
        );
        assert!(host.system_actions.is_none());
    }
}

#[test]
fn failed_import_keeps_existing_profile_modes_and_cleans_staged_source() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source.streamDeckProfile");
    fixture(&source, None, false);
    let mut host = host(root.path());
    let before = serde_json::to_value(&host.library).unwrap();
    host.state_path = root.path().join("directory-instead-of-file");
    fs::create_dir(&host.state_path).unwrap();
    assert!(host.import_profile(&source).is_err());
    assert_eq!(serde_json::to_value(&host.library).unwrap(), before);
    assert_eq!(
        fs::read_dir(root.path().join("sources")).unwrap().count(),
        0
    );
    assert!(host.system_actions.is_none());
}
