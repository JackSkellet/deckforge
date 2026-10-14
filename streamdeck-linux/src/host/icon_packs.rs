//! Pack IO is outside the host mutex; choosing an asset uses normal appearance Save.
use super::*;
impl Controller {
    pub fn icon_packs(&self, request: &Value) -> Result<Value> {
        let (destination, reference) = {
            let host = self.shared.lock().unwrap();
            anyhow::ensure!(host.running, "Deckforge is shutting down");
            (
                host.state_path
                    .parent()
                    .context("State directory missing")?
                    .join("icon-packs"),
                host.reference_app.clone(),
            )
        };
        let text = |key: &str| {
            request[key]
                .as_str()
                .with_context(|| format!("Icon pack {key} missing"))
        };
        match text("operation")? {
            "list" => Ok(serde_json::to_value(crate::icon_pack::catalog(
                &destination,
                reference.as_deref(),
            )?)?),
            "install" => Ok(serde_json::to_value(crate::icon_pack::install(
                Path::new(text("path")?),
                &destination,
                reference.as_deref(),
            )?)?),
            "asset" => Ok(json!(crate::icon_pack::selected_asset(
                &destination,
                text("id")?,
                text("path")?,
                text("revision")?,
                reference.as_deref()
            )?)),
            "thumbnail" => Ok(json!(crate::icon_pack::thumbnail(
                &destination,
                text("id")?,
                text("path")?,
                reference.as_deref(),
                Some(text("revision")?)
            )?)),
            "hide" => {
                crate::icon_pack::hide(
                    &destination,
                    text("id")?,
                    text("revision")?,
                    request["expected_hidden"]
                        .as_bool()
                        .context("Previous visibility missing")?,
                    request["hidden"].as_bool().context("Visibility missing")?,
                    reference.as_deref(),
                )?;
                Ok(Value::Null)
            }
            "remove" => Ok(json!(crate::icon_pack::remove(
                &destination,
                text("id")?,
                text("revision")?,
                reference.as_deref()
            )?)),
            "review" => Ok(json!(crate::icon_pack::review(
                &destination,
                text("id")?,
                Path::new(text("path")?),
                reference.as_deref()
            )?)),
            "review_restore" => Ok(json!(crate::icon_pack::review_restore(
                &destination,
                text("token")?,
                reference.as_deref()
            )?)),
            "apply" => {
                let review: crate::icon_pack::Review =
                    serde_json::from_value(request["review"].clone())?;
                Ok(json!(crate::icon_pack::apply(
                    &destination,
                    &review,
                    reference.as_deref()
                )?))
            }
            _ => bail!("Unknown icon pack operation"),
        }
    }
}
