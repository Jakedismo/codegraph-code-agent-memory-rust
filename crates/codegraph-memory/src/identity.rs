// ABOUTME: Stable memory identities across repository moves and Git worktrees.
// ABOUTME: Memory identities never replace or change existing code graph node identities.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Command,
};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectIdentity {
    pub id: String,
    pub store_root: PathBuf,
}
pub fn project_identity(root: &Path) -> Result<ProjectIdentity> {
    let root = root.canonicalize()?;
    let common = Command::new("git")
        .args(["rev-parse", "--path-format=absolute", "--git-common-dir"])
        .current_dir(&root)
        .output()
        .ok()
        .filter(|v| v.status.success());
    let main_root = common
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .map(|path| PathBuf::from(path.trim()))
        .filter(|p| p.file_name().is_some_and(|n| n == ".git"))
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .unwrap_or(root);
    let dir = main_root.join(".codegraph");
    std::fs::create_dir_all(&dir)?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(dir.join("memory-identity.lock"))?;
    lock.lock()?;
    let ignore = dir.join(".gitignore");
    if ignore.exists() {
        ensure!(
            !std::fs::symlink_metadata(&ignore)?.file_type().is_symlink(),
            "Memory ignore file cannot be a symlink"
        );
        let content = std::fs::read_to_string(&ignore)?;
        if !content.lines().any(|line| line.trim() == "*") {
            let mut file = OpenOptions::new().append(true).open(&ignore)?;
            if !content.is_empty() && !content.ends_with('\n') {
                file.write_all(b"\n")?;
            }
            for entry in [
                "memory-db/",
                "memory-db.identity",
                "memory-project.json",
                "memory-identity.lock",
            ] {
                if !content.lines().any(|line| line.trim() == entry) {
                    writeln!(file, "{entry}")?;
                }
            }
            file.sync_all()?;
        }
    } else {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(ignore)?;
        file.write_all(b"*\n")?;
        file.sync_all()?;
    }
    let path = dir.join("memory-project.json");
    let id = if path.exists() {
        let mut content = String::new();
        File::open(&path)?.read_to_string(&mut content)?;
        let value: serde_json::Value = serde_json::from_str(&content)?;
        ensure!(
            value["version"] == 1,
            "Unsupported memory project identity version"
        );
        value["id"]
            .as_str()
            .context("Invalid memory project identity")?
            .to_string()
    } else {
        let id = Uuid::new_v4().to_string();
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)?;
        file.write_all(
            serde_json::to_string(&serde_json::json!({"version":1,"id":id}))?.as_bytes(),
        )?;
        file.sync_all()?;
        id
    };
    Uuid::parse_str(&id).context("Invalid memory project UUID")?;
    Ok(ProjectIdentity {
        id,
        store_root: dir.join("memory-db"),
    })
}
