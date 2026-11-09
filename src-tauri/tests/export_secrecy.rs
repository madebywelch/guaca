use guac_lib::{
    boot::Paths,
    db::Store,
    domain::{ids::GroupId, plugin::PluginKind},
    files::FileStore,
    transfer,
    workspace::Workspace,
};
use serde_json::json;

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
    assert!(!bytes
        .windows(b"recognizable-pasted-key".len())
        .any(|w| w == b"recognizable-pasted-key"));
}

#[test]
fn workspace_secrets_stay_out_of_the_archive_and_import() {
    let source = tempfile::tempdir().unwrap();
    let paths = Paths::under(source.path());
    std::fs::create_dir_all(&paths.data).unwrap();
    std::fs::create_dir_all(&paths.config).unwrap();
    let store = Store::open(&paths.db()).unwrap();
    let workspace = Workspace::new(paths.data.join("workspace"));
    let files = FileStore::new(paths.data.join("files"));
    let group = GroupId::new();
    let agent = uuid::Uuid::new_v4().to_string();
    let secrets = [
        "group-provider-key-unique",
        "global-provider-key-unique",
        "chatgpt-access-unique",
        "chatgpt-refresh-unique",
        "claude-signin-unique",
        "plugin-access-unique",
        "plugin-refresh-unique",
        "plugin-client-secret-unique",
        "plugin-header-unique",
        "plugin-env-unique",
        "plugin-argument-unique",
        "plugin-url-user-unique",
        "plugin-url-password-unique",
        "plugin-url-query-unique",
        "group-url-user-unique",
        "group-url-password-unique",
        "group-url-query-unique",
        "connector-secret-unique",
        "browser-id-unique",
        "browser-signin-unique",
        "computer-id-unique",
        "computer-envd-unique",
        "computer-traffic-unique",
        "account-access-unique",
        "account-refresh-unique",
        "host-token-unique",
        "home-directory-unique",
    ];
    std::fs::write(
        paths.config.join("config.json"),
        json!({"inference":{"api_key":secrets[1]}}).to_string(),
    )
    .unwrap();
    std::fs::write(
        paths.config.join("subscription.json"),
        json!({"access_token":secrets[2],"refresh_token":secrets[3]}).to_string(),
    )
    .unwrap();
    std::fs::write(
        paths.config.join("account.json"),
        json!({"access_token":secrets[23],"refresh_token":secrets[24]}).to_string(),
    )
    .unwrap();
    std::fs::write(paths.config.join("token"), secrets[25]).unwrap();
    std::fs::write(source.path().join("claude-credentials.json"), secrets[4]).unwrap();
    let mut conn = store.conn().unwrap();
    conn.execute("INSERT INTO groups (id,name,created_at,api_key,base_url) VALUES (?1,'Secret fixture',1,?2,?3)", rusqlite::params![group.to_string(), secrets[0], format!("https://{}:{}@example.com/v1?key={}", secrets[14], secrets[15], secrets[16])]).unwrap();
    conn.execute("INSERT INTO agents (id,group_id,name,avatar,color,model,system_prompt,skills,lifecycle,created_at,updated_at,browser_id,sandbox_id,sandbox_envd_token,sandbox_traffic_token,has_computer,has_browser) VALUES (?1,?2,'Agent','avocado','#ffffff','small','Work','[]','active',1,1,?3,?4,?5,?6,1,1)", rusqlite::params![agent, group.to_string(), secrets[18], secrets[20], secrets[21], secrets[22]]).unwrap();
    conn.execute("INSERT INTO signins (agent_id,surface,domain,service,first_seen_at,last_seen_at) VALUES (?1,'browser',?2,'Browser',1,1)", rusqlite::params![agent, secrets[19]]).unwrap();
    conn.execute("INSERT INTO connectors (id,group_id,service,account,env_var,secret,created_at,updated_at) VALUES (?1,?2,'Service','Account','SERVICE_KEY',?3,1,1)", rusqlite::params![uuid::Uuid::new_v4().to_string(),group.to_string(),secrets[17]]).unwrap();
    conn.execute("INSERT INTO plugins (id,group_id,kind,connected_at,endpoint,access_token,refresh_token,client_secret,headers) VALUES (?1,?2,'custom_stdio',1,?3,?4,?5,?6,?7)", rusqlite::params![uuid::Uuid::new_v4().to_string(),group.to_string(),format!("stdio:/home/{}/bin/server --key {}",secrets[26],secrets[10]),secrets[5],secrets[6],secrets[7],json!({"env":[{"name":"KEY","value":secrets[9]}]}).to_string()]).unwrap();
    conn.execute("INSERT INTO plugins (id,group_id,kind,connected_at,endpoint,headers) VALUES (?1,?2,'custom_http',1,?3,?4)", rusqlite::params![uuid::Uuid::new_v4().to_string(),group.to_string(),format!("https://{}:{}@example.com/mcp?key={}",secrets[11],secrets[12],secrets[13]),json!([{"name":"x-api-key","value":secrets[8]}]).to_string()]).unwrap();
    let archive = transfer::export(&mut conn, group, &workspace, &files).unwrap();
    let bytes = serde_json::to_vec(&archive).unwrap();
    for secret in secrets {
        assert!(
            !bytes.windows(secret.len()).any(|w| w == secret.as_bytes()),
            "export contains {secret}"
        );
    }
    let stdio = archive.reconnect.iter().find(|r| r.name == "custom_stdio").unwrap();
    assert_eq!(stdio.details["endpoint"], "stdio:server");
    assert_eq!(stdio.details["note"], "Re-enter the full command and environment.");
    let target = tempfile::tempdir().unwrap();
    let target_store = Store::open(&target.path().join("guac.db")).unwrap();
    let target_workspace = Workspace::new(target.path().join("workspace"));
    let target_files = FileStore::new(target.path().join("files"));
    let imported = transfer::import(
        &target_store,
        serde_json::from_slice(&bytes).unwrap(),
        "Imported".into(),
        &target_workspace,
        &target_files,
    )
    .unwrap();
    let target_conn = target_store.conn().unwrap();
    let count: i64 = target_conn
        .query_row("SELECT count(*) FROM plugins WHERE group_id=?1", [imported.to_string()], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(count, 0);
    let count: i64 = target_conn
        .query_row(
            "SELECT count(*) FROM connectors WHERE group_id=?1",
            [imported.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
    let group_key: Option<String> = target_conn
        .query_row("SELECT api_key FROM groups WHERE id=?1", [imported.to_string()], |r| r.get(0))
        .unwrap();
    assert!(group_key.as_deref().unwrap_or_default().is_empty());
    let copied_agent: (Option<String>, Option<String>, Option<String>, Option<String>) = target_conn
        .query_row("SELECT browser_id,sandbox_id,sandbox_envd_token,sandbox_traffic_token FROM agents WHERE group_id=?1", [imported.to_string()], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .unwrap();
    assert_eq!(copied_agent, (None, None, None, None));
    let count: i64 =
        target_conn.query_row("SELECT count(*) FROM signins", [], |r| r.get(0)).unwrap();
    assert_eq!(count, 0);
    drop(target_conn);
    let exported_again = transfer::export(
        &mut target_store.conn().unwrap(),
        imported,
        &target_workspace,
        &target_files,
    )
    .unwrap();
    let imported_bytes = serde_json::to_vec(&exported_again).unwrap();
    for secret in secrets {
        assert!(
            !imported_bytes.windows(secret.len()).any(|w| w == secret.as_bytes()),
            "import restored {secret}"
        );
    }
    assert!(transfer::reconnect(&target_store, imported).unwrap().iter().any(|r| r
        .details
        .get("note")
        .is_some_and(|n| n == "Re-enter the full command and environment.")));
}

#[test]
fn old_reconnect_details_are_cleaned_on_import_and_reexport() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("guac.db")).unwrap();
    let workspace = Workspace::new(dir.path().join("workspace"));
    let files = FileStore::new(dir.path().join("files"));
    let group = GroupId::new();
    let mut conn = store.conn().unwrap();
    conn.execute(
        "INSERT INTO groups (id,name,created_at) VALUES (?1,'Old',1)",
        [group.to_string()],
    )
    .unwrap();
    let mut archive = transfer::export(&mut conn, group, &workspace, &files).unwrap();
    archive.tables.get_mut("groups").unwrap()[0].insert(
        "base_url".into(),
        "https://old-user:old-password@example.com/v1?key=old-query".into(),
    );
    archive.reconnect.push(transfer::Reconnect {
        kind: "plugin".into(),
        name: "old server".into(),
        details: json!({"endpoint":"stdio:/home/private-name/server --token old-secret", "headers":{"authorization":"old-header"}, "env":{"TOKEN":"old-env"}}).as_object().unwrap().clone(),
        agents: vec![],
    });
    drop(conn);
    let imported = transfer::import(&store, archive, "Copy".into(), &workspace, &files).unwrap();
    let reconnect = transfer::reconnect(&store, imported).unwrap();
    assert_eq!(reconnect[0].details["endpoint"], "stdio:server");
    let stored = serde_json::to_vec(&reconnect).unwrap();
    for secret in ["private-name", "old-secret", "old-header", "old-env"] {
        assert!(!stored.windows(secret.len()).any(|w| w == secret.as_bytes()));
    }
    let base_url: String = store
        .conn()
        .unwrap()
        .query_row("SELECT base_url FROM groups WHERE id=?1", [imported.to_string()], |r| r.get(0))
        .unwrap();
    assert_eq!(base_url, "https://example.com/v1");
    let exported =
        transfer::export(&mut store.conn().unwrap(), imported, &workspace, &files).unwrap();
    assert_eq!(exported.reconnect[0].details["note"], "Re-enter the full command and environment.");
}
