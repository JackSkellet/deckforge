//! Inspect shared-action roots without publishing a profile or executing actions.
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};
#[derive(Clone, Serialize)]
pub struct Root {
    pub index: usize,
    pub name: String,
    pub uuid: String,
    pub controller: String,
    pub page: String,
    pub page_name: String,
    pub column: u8,
    pub row: u8,
    #[serde(skip)]
    pub page_index: usize,
    #[serde(skip)]
    pub slot_index: usize,
}
#[derive(Clone, Serialize)]
pub struct Info {
    pub path: PathBuf,
    pub source_hash: String,
    pub roots: Vec<Root>,
}
fn label(value: &str, limit: usize) -> String {
    let mut result: String = value.chars().take(limit).collect();
    if value.chars().count() > limit {
        result.push('…');
    }
    result
}
pub fn load(path: &Path) -> Result<(crate::profile::ProfileBundle, Vec<Root>)> {
    ensure!(
        std::fs::metadata(path)?.is_file(),
        "Action source must be a regular file"
    );
    let bundle = crate::profile::load_bundle(path)?;
    ensure!(
        bundle.raw["Device"]["Model"] == "Fake/Storage",
        "File is not a shared action bundle"
    );
    if let Some((_, bytes)) = bundle.files().find(|(name, _)| *name == "package.json") {
        let package: serde_json::Value = serde_json::from_slice(bytes)?;
        ensure!(
            package["FormatVersion"] == 1,
            "Unsupported action bundle format version"
        );
    }
    ensure!(
        bundle.pages.iter().all(|p| p.warnings.is_empty()),
        "Action bundle contains unsupported data: {}",
        bundle.warnings.join("; ")
    );
    let pages: HashSet<_> = bundle
        .page_order
        .iter()
        .chain([&bundle.default_page, &bundle.current_page])
        .collect();
    let mut roots = Vec::new();
    for (page_index, page) in bundle
        .pages
        .iter()
        .enumerate()
        .filter(|(_, p)| pages.contains(&p.page))
    {
        for (slot_index, item) in page.slots.iter().enumerate() {
            ensure!(
                roots.len() < 1024,
                "Shared action file exceeds 1024 root choices"
            );
            roots.push(Root {
                index: roots.len(),
                name: label(item.raw["Name"].as_str().unwrap_or(&item.uuid), 256),
                uuid: label(&item.uuid, 512),
                controller: item.controller.clone(),
                page: page.page.clone(),
                page_name: label(
                    page.page_raw["Name"]
                        .as_str()
                        .filter(|n| !n.is_empty())
                        .unwrap_or(""),
                    128,
                ),
                column: item.column,
                row: item.row,
                page_index,
                slot_index,
            });
        }
    }
    for root in &mut roots {
        if root.page_name.is_empty() {
            root.page_name = if let Some(i) = bundle.page_order.iter().position(|p| p == &root.page)
            {
                format!("Page {}", i + 1)
            } else if root.page == bundle.default_page {
                "Pinned actions".into()
            } else {
                "Current page".into()
            };
        }
    }
    ensure!(!roots.is_empty(), "Shared file contains no root actions");
    Ok((bundle, roots))
}
pub fn digest(bundle: &crate::profile::ProfileBundle) -> String {
    let mut hash = Sha256::new();
    for (path, bytes) in bundle.files() {
        hash.update((path.len() as u64).to_le_bytes());
        hash.update(path.as_bytes());
        hash.update((bytes.len() as u64).to_le_bytes());
        hash.update(bytes);
    }
    format!("{:x}", hash.finalize())
}
pub fn inspect(path: &Path) -> Result<Info> {
    let path = path.canonicalize().context("Open shared action file")?;
    let (bundle, roots) = load(&path)?;
    Ok(Info {
        path,
        source_hash: digest(&bundle),
        roots,
    })
}
