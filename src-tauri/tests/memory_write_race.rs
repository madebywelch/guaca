use guac_lib::domain::ids::AgentId;
use guac_lib::workspace::{version, Workspace, WriteResult};
use std::sync::{Arc, Barrier};

#[test]
fn agent_write_between_read_and_save_must_not_be_lost() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = Workspace::new(dir.path().to_path_buf());
    let id = AgentId::new();
    workspace.write(id, "Agent", "original").unwrap();
    let original = workspace.read(id);
    let barrier = Arc::new(Barrier::new(2));
    let agent = workspace.clone();
    let ready = barrier.clone();
    let thread = std::thread::spawn(move || {
        agent.write(id, "Agent", "agent rewrite").unwrap();
        ready.wait();
    });
    barrier.wait();
    let result = workspace
        .write_versioned(id, "Agent", "operator draft", Some(&version(&original)))
        .unwrap();
    thread.join().unwrap();
    assert_eq!(
        result,
        WriteResult::Conflict { current: "agent rewrite".into(), draft: "operator draft".into() }
    );
    assert_eq!(workspace.read(id), "agent rewrite");
    let result = workspace
        .write_versioned(id, "Agent", "operator draft", Some(&version("agent rewrite")))
        .unwrap();
    assert!(matches!(result, WriteResult::Stored(_)));
    assert_eq!(workspace.read(id), "operator draft");
}

#[test]
fn agent_write_after_save_must_not_be_returned_as_save_result() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = Workspace::new(dir.path().to_path_buf());
    let id = AgentId::new();
    let saved = workspace.write(id, "Agent", "operator draft").unwrap();
    let barrier = Arc::new(Barrier::new(2));
    let agent = workspace.clone();
    let ready = barrier.clone();
    let thread = std::thread::spawn(move || {
        agent.write(id, "Agent", "agent rewrite").unwrap();
        ready.wait();
    });
    barrier.wait();
    let result = saved.content;
    thread.join().unwrap();
    assert_eq!(result, "operator draft");
}

#[test]
fn unchanged_save_keeps_the_same_content() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = Workspace::new(dir.path().to_path_buf());
    let id = AgentId::new();
    workspace.write(id, "Agent", "same").unwrap();
    let saved = workspace.write_versioned(id, "Agent", "same", Some(&version("same"))).unwrap();
    assert!(matches!(saved, WriteResult::Stored(_)));
    assert_eq!(workspace.read(id), "same");
}

#[test]
fn versioned_read_matches_the_save_for_unicode_content() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = Workspace::new(dir.path().to_path_buf());
    let id = AgentId::new();
    workspace.write(id, "Agent", "café 🥑").unwrap();
    let snapshot = workspace.read_versioned(id);
    assert_eq!(snapshot.content, "café 🥑");
    assert!(matches!(
        workspace.write_versioned(id, "Agent", "updated", Some(&snapshot.version)).unwrap(),
        WriteResult::Stored(_)
    ));
}
