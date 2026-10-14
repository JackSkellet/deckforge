//! Deckforge-only profile metadata; native AppIdentifier remains untouched.
use super::*;

const FILE: &str = "Deckforge/SmartProfile.json";
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Metadata {
    version: u32,
    binding: Binding,
}

pub(in crate::host) fn read(bundle: &profile::ProfileBundle) -> Result<Option<Binding>> {
    let manifest = bundle.manifest_path();
    let path = format!("{}{FILE}", manifest.trim_end_matches("manifest.json"));
    let Some((_, bytes)) = bundle.files().find(|(name, _)| *name == path) else {
        return Ok(None);
    };
    ensure!(
        bytes.len() <= 16 * 1024,
        "Smart Profile metadata is too large"
    );
    let metadata: Metadata =
        serde_json::from_slice(bytes).context("Invalid Deckforge Smart Profile metadata")?;
    ensure!(
        metadata.version == 1,
        "Unsupported Smart Profile metadata version"
    );
    metadata.binding.validate()?;
    Ok(Some(metadata.binding))
}

pub(in crate::host) fn write(
    files: &mut BTreeMap<String, Vec<u8>>,
    profile_id: &str,
    binding: Option<&Binding>,
) -> Result<()> {
    let root = [
        format!("{profile_id}.sdProfile/"),
        format!("Profiles/{profile_id}.sdProfile/"),
    ]
    .into_iter()
    .find(|root| files.contains_key(&format!("{root}manifest.json")))
    .context("Exported profile manifest missing")?;
    let path = format!("{root}{FILE}");
    // A cleared binding must not reappear from retained source archive bytes.
    files.remove(&path);
    if let Some(binding) = binding {
        binding.validate()?;
        files.insert(
            path,
            serde_json::to_vec_pretty(&Metadata {
                version: 1,
                binding: binding.clone(),
            })?,
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn source(root: &Path) -> (PathBuf, Binding) {
        let mut host = crate::host::tests::fixture(root);
        let id = host.library.active_profile.clone();
        host.library.profiles[0].raw["AppIdentifier"] = json!("C:\\Original.exe");
        let binding = Binding {
            kind: Kind::DesktopId,
            value: "org.example.Editor".into(),
            name: "Editor".into(),
        };
        host.smart_profiles.insert(
            host.scope.clone(),
            Config {
                enabled: true,
                fallback: Some(id.clone()),
                bindings: BTreeMap::from([(id, binding.clone())]),
            },
        );
        let archive = root.join("source.streamDeckProfile");
        host.export_profile(&archive).unwrap();
        (archive, binding)
    }
    fn entries(path: &Path) -> BTreeMap<String, Vec<u8>> {
        profile::load_bundle(path)
            .unwrap()
            .files()
            .map(|(k, v)| (k.into(), v.to_vec()))
            .collect()
    }
    fn metadata_path(files: &BTreeMap<String, Vec<u8>>) -> String {
        files.keys().find(|p| p.ends_with(FILE)).unwrap().clone()
    }
    #[test]
    fn bindings_roundtrip_plain_and_wrapped_without_enabling_or_replacing_destination_rules() {
        for wrapped in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let (path, binding) = source(dir.path());
            if wrapped {
                let mut files: BTreeMap<_, _> = entries(&path)
                    .into_iter()
                    .map(|(k, v)| (format!("Profiles/{k}"), v))
                    .collect();
                files.insert(
                    "package.json".into(),
                    serde_json::to_vec(
                        &json!({"FormatVersion":1,"DeviceModel":"20GBD9901","RequiredPlugins":[]}),
                    )
                    .unwrap(),
                );
                export::write_archive(&path, &files).unwrap();
            }
            let other = tempfile::tempdir().unwrap();
            let mut host = crate::host::tests::fixture(other.path());
            let base = host.library.active_profile.clone();
            let existing = Binding {
                kind: Kind::WindowClass,
                value: "Existing".into(),
                name: "Existing".into(),
            };
            host.smart_profiles.insert(
                host.scope.clone(),
                Config {
                    enabled: false,
                    fallback: Some(base.clone()),
                    bindings: BTreeMap::from([(base.clone(), existing.clone())]),
                },
            );
            host.import_profile(&path).unwrap();
            let id = host.library.active_profile.clone();
            let config = host.smart_config();
            assert!(!config.enabled);
            assert_eq!(config.fallback, Some(base.clone()));
            assert_eq!(config.bindings[&base], existing);
            assert_eq!(config.bindings[&id], binding);
            assert_eq!(
                host.library.current().unwrap().raw["AppIdentifier"],
                "C:\\Original.exe"
            );
            let saved: Saved =
                serde_json::from_slice(&fs::read(&host.state_path).unwrap()).unwrap();
            assert_eq!(saved.smart_profiles[&host.scope], config);
            fs::remove_file(&path).unwrap();
            let output = other.path().join("roundtrip.streamDeckProfile");
            host.export_profile(&output).unwrap();
            assert_eq!(
                read(&profile::load_bundle(&output).unwrap()).unwrap(),
                Some(binding)
            );
            host.smart_profiles
                .get_mut(&host.scope)
                .unwrap()
                .bindings
                .remove(&id);
            host.export_profile(&output).unwrap();
            assert_eq!(read(&profile::load_bundle(&output).unwrap()).unwrap(), None);
            let unbound = tempfile::tempdir().unwrap();
            let mut clean = crate::host::tests::fixture(unbound.path());
            clean.import_profile(&output).unwrap();
            assert!(clean.smart_config().bindings.is_empty());
            assert!(!clean.smart_config().enabled);
        }
    }
    #[test]
    fn malformed_metadata_is_rejected_before_materializing_or_publishing() {
        let dir = tempfile::tempdir().unwrap();
        let (path, _) = source(dir.path());
        let original = entries(&path);
        let key = metadata_path(&original);
        for bad in [
            b"{".to_vec(),
            vec![b' '; 16 * 1024 + 1],
            serde_json::to_vec(&json!({"version":2,"binding":{"kind":"desktop_id","value":"app"}}))
                .unwrap(),
            serde_json::to_vec(
                &json!({"version":1,"binding":{"kind":"executable","value":"relative/path"}}),
            )
            .unwrap(),
            serde_json::to_vec(
                &json!({"version":1,"binding":{"kind":"window_class","value":"bad\nvalue"}}),
            )
            .unwrap(),
            serde_json::to_vec(
                &json!({"version":1,"binding":{"kind":"desktop_id","value":"app"},"enabled":true}),
            )
            .unwrap(),
        ] {
            let mut files = original.clone();
            files.insert(key.clone(), bad);
            export::write_archive(&path, &files).unwrap();
            let other = tempfile::tempdir().unwrap();
            let mut host = crate::host::tests::fixture(other.path());
            let before = serde_json::to_value(&host.library).unwrap();
            assert!(host.import_profile(&path).is_err());
            assert_eq!(serde_json::to_value(&host.library).unwrap(), before);
            assert!(host.smart_profiles.is_empty());
            assert!(!other.path().join("sources").exists());
        }
    }
    #[test]
    fn failed_import_rolls_back_rules_graph_and_owned_source_without_releasing_keys() {
        let dir = tempfile::tempdir().unwrap();
        let (path, _) = source(dir.path());
        let other = tempfile::tempdir().unwrap();
        let mut host = crate::host::tests::fixture(other.path());
        host.assign("Keypad", 0, "test.action").unwrap();
        let context = host.slots[0].context.clone();
        let (tx, mut rx) = mpsc::unbounded_channel();
        host.plugins[0].sender = Some(tx);
        host.input(InputEvent::Key {
            index: 0,
            pressed: true,
        })
        .unwrap();
        while rx.try_recv().is_ok() {}
        host.sync_all_devices();
        let before = serde_json::to_value(&host.library).unwrap();
        let state = host.state_path.clone();
        let saved = fs::read(&state).unwrap();
        host.state_path = other.path().join("directory");
        fs::create_dir(&host.state_path).unwrap();
        assert!(host.import_profile(&path).is_err());
        assert_eq!(serde_json::to_value(&host.library).unwrap(), before);
        assert!(host.smart_profiles.is_empty());
        assert_eq!(fs::read(state).unwrap(), saved);
        assert_eq!(
            fs::read_dir(other.path().join("sources")).unwrap().count(),
            0
        );
        assert_eq!(host.held.get(&HeldControl::Key(0)), Some(&context));
        assert!(rx.try_recv().is_err());
    }
    #[test]
    fn incompatible_layout_import_keeps_active_controls_and_stores_binding_for_later() {
        let dir = tempfile::tempdir().unwrap();
        let (path, binding) = source(dir.path());
        let mut files = entries(&path);
        let manifest = profile::load_bundle(&path).unwrap().manifest_path();
        let mut raw: Value = serde_json::from_slice(&files[&manifest]).unwrap();
        raw["Device"]["Model"] = json!(Model::Mini.profile_model());
        files.insert(manifest, serde_json::to_vec(&raw).unwrap());
        export::write_archive(&path, &files).unwrap();
        let other = tempfile::tempdir().unwrap();
        let mut host = crate::host::tests::fixture(other.path());
        host.assign("Keypad", 0, "test.action").unwrap();
        let base = host.library.active_profile.clone();
        let context = host.slots[0].context.clone();
        let (tx, mut rx) = mpsc::unbounded_channel();
        host.plugins[0].sender = Some(tx);
        host.input(InputEvent::Key {
            index: 0,
            pressed: true,
        })
        .unwrap();
        while rx.try_recv().is_ok() {}
        host.import_profile(&path).unwrap();
        assert_eq!(host.library.active_profile, base);
        assert_eq!(host.held.get(&HeldControl::Key(0)), Some(&context));
        assert!(rx.try_recv().is_err());
        let imported = host.library.profiles.last().unwrap();
        assert_eq!(imported.model, Model::Mini);
        assert_eq!(host.smart_config().bindings[&imported.id], binding);
        assert!(!host.smart_config().enabled);
    }
}
