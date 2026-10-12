//! Installer extraction runs outside the host mutex so device input continues.
use super::*;

impl Controller {
    pub fn import_reference(
        &self,
        input: &Path,
        expected: Option<&Path>,
    ) -> Result<crate::reference_import::Imported> {
        let state = {
            let host = self.shared.lock().unwrap();
            anyhow::ensure!(
                host.reference_app.as_deref() == expected,
                "Reference changed; refresh before importing"
            );
            host.state_path
                .parent()
                .context("State directory missing")?
                .to_owned()
        };
        let imported = crate::reference_import::import(input, &state)?;
        let mut host = self.shared.lock().unwrap();
        anyhow::ensure!(host.running, "Deckforge is shutting down");
        anyhow::ensure!(
            host.reference_app.as_deref() == expected,
            "Reference changed during import; the previous selection was preserved"
        );
        let previous = host.reference_app.replace(imported.path.clone());
        if let Err(error) = host.persist() {
            host.reference_app = previous;
            return Err(error);
        }
        host.log(format!(
            "Imported Stream Deck {} {} verification reference",
            imported.version, imported.platform
        ));
        Ok(imported)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_import_and_bad_input_preserve_the_reference() {
        let root = tempfile::tempdir().unwrap();
        let mut host = super::super::tests::fixture(root.path());
        let previous = root.path().join("previous");
        host.reference_app = Some(previous.clone());
        let controller = Controller {
            shared: Arc::new(Mutex::new(host)),
        };
        let missing = root.path().join("missing.pkg");
        assert!(
            controller
                .import_reference(&missing, None)
                .err()
                .unwrap()
                .to_string()
                .contains("Reference changed")
        );
        assert!(
            controller
                .import_reference(&missing, Some(&previous))
                .is_err()
        );
        assert_eq!(
            controller.shared.lock().unwrap().reference_app.as_ref(),
            Some(&previous)
        );
        assert!(!root.path().join("references").exists());
    }

    #[test]
    #[ignore = "requires the user-supplied Elgato macOS installer; extracts static data only"]
    fn save_failure_after_successful_extraction_preserves_reference() {
        let source = std::env::var_os("DECKFORGE_TEST_ELGATO_PKG").expect("set fixture path");
        let root = tempfile::tempdir().unwrap();
        let mut host = super::super::tests::fixture(root.path());
        let previous = root.path().join("previous");
        host.reference_app = Some(previous.clone());
        // A directory at the state-file destination makes atomic replacement fail.
        host.state_path = root.path().join("state-directory");
        std::fs::create_dir(&host.state_path).unwrap();
        let controller = Controller {
            shared: Arc::new(Mutex::new(host)),
        };
        assert!(
            controller
                .import_reference(Path::new(&source), Some(&previous))
                .is_err()
        );
        assert_eq!(
            controller.shared.lock().unwrap().reference_app.as_ref(),
            Some(&previous)
        );
        assert_eq!(
            std::fs::read_dir(root.path().join("references"))
                .unwrap()
                .count(),
            1
        );
    }
}
