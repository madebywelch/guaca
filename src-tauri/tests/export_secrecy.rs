use guac_lib::{
    db::Store,
    domain::{ids::GroupId, plugin::PluginKind},
    files::FileStore,
    transfer,
    workspace::Workspace,
};

#[test]
fn custom_plugin_command_does_not_export_a_pasted_key() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("guac.db")).unwrap();
    let workspace = Workspace::new(dir.path().join("workspace"));
    let files = FileStore::new(dir.path().join("files"));
    let group = GroupId::new();
    let endpoint = "stdio:npx server --api-key recognizable-pasted-key";
    assert!(PluginKind::custom("Custom server", endpoint).is_ok());
    let mut conn = store.conn().unwrap();
    conn.execute(
        "INSERT INTO groups (id,name,created_at) VALUES (?1,'Export test',1)",
        [group.to_string()],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO plugins (id,group_id,kind,connected_at,endpoint) VALUES (?1,?2,'custom-server',1,?3)",
        rusqlite::params![uuid::Uuid::new_v4().to_string(), group.to_string(), endpoint],
    )
    .unwrap();
    let archive = transfer::export(&mut conn, group, &workspace, &files).unwrap();
    let bytes = serde_json::to_vec(&archive).unwrap();
    assert!(!bytes.windows(b"recognizable-pasted-key".len()).any(|w| w == b"recognizable-pasted-key"));
}
