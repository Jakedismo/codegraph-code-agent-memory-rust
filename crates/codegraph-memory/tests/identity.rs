// ABOUTME: Memory identities survive moves and share Git worktree ownership without touching graph IDs.
// ABOUTME: All repository and ignore-file fixtures are temporary and provider-independent.
use codegraph_memory::identity::project_identity;
use std::{path::Path, process::Command};

#[test]
fn identity_is_stable_after_move_and_ignore_updates_are_idempotent() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("project");
    std::fs::create_dir_all(root.join(".codegraph")).unwrap();
    std::fs::write(root.join(".codegraph/.gitignore"), "db/\n").unwrap();
    let before = project_identity(&root).unwrap();
    let ignore = std::fs::read(root.join(".codegraph/.gitignore")).unwrap();
    assert!(String::from_utf8_lossy(&ignore).contains("memory-db/"));
    assert_eq!(project_identity(&root).unwrap().id, before.id);
    assert_eq!(
        std::fs::read(root.join(".codegraph/.gitignore")).unwrap(),
        ignore
    );
    let moved = temporary.path().join("moved");
    std::fs::rename(&root, &moved).unwrap();
    let after = project_identity(&moved).unwrap();
    assert_eq!(after.id, before.id);
    assert_eq!(
        after.store_root,
        moved.canonicalize().unwrap().join(".codegraph/memory-db")
    );
    assert!(!moved.join(".codegraph/db").exists());
}

fn git(root: &Path, arguments: &[&str]) {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
#[test]
fn git_worktrees_share_the_main_repository_memory_identity() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("project");
    std::fs::create_dir(&root).unwrap();
    git(&root, &["init", "--quiet"]);
    git(
        &root,
        &[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--quiet",
            "--allow-empty",
            "-m",
            "fixture",
        ],
    );
    let worktree = temporary.path().join("worktree");
    git(
        &root,
        &[
            "worktree",
            "add",
            "--quiet",
            "--detach",
            worktree.to_str().unwrap(),
        ],
    );
    let main = project_identity(&root).unwrap();
    let related = project_identity(&worktree).unwrap();
    assert_eq!(main.id, related.id);
    assert_eq!(main.store_root, related.store_root);
    assert!(!worktree.join(".codegraph/memory-project.json").exists());
}
