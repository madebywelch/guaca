//! Versioned group copies. Explicit field allowlists keep stored credentials out.
//! Imports never execute SQL from an archive, reach a service, or start a routine.
use crate::{
    db::Store,
    domain::ids::{AgentId, GroupId},
    files::FileStore,
    workspace::Workspace,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use rusqlite::{params, types::Value as SqlValue, Connection};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, HashMap, HashSet};

type Row = Map<String, Value>;
type Result<T> = std::result::Result<T, String>;
pub const MAX_ARCHIVE: usize = 64 * 1024 * 1024;

// Order is also insertion order. Nothing from repository or plugin credential stores.
const TABLES: &[(&str, &str, &str)] = &[
    ("groups", "id,name,created_at,base_url,default_model,provider,subscription_model,request_timeout_secs,max_hops,max_steps_per_run,max_fanout_per_call,max_sends_per_pair,max_tool_rounds", "id = ?1"),
    ("agents", "id,group_id,name,avatar,color,model,system_prompt,skills,lifecycle,version,created_at,updated_at,pinned,rail_order,discarded_at,browser_consent", "group_id = ?1"),
    ("routines", "id,agent_id,name,what,fires,active,next_run_at,last_run_at,created_at,skip_if_working", "agent_id IN (SELECT id FROM agents WHERE group_id = ?1)"),
    ("messages", "id,run_id,channel_id,from_kind,from_agent,to_kind,to_agent,parts,trust,hop,expects_reply,cause,created_at,intent", "channel_id IN (SELECT id FROM agents WHERE group_id = ?1)"),
    ("occasions", "id,group_id,agent_id,title,detail,place,starts_at,minutes,all_day,created_at,updated_at", "group_id = ?1"),
    ("working_notes", "agent_id,at,body", "agent_id IN (SELECT id FROM agents WHERE group_id = ?1)"),
    ("usage", "agent_id,group_id,run_id,model,prompt,completion,cost,created_at", "group_id = ?1"),
    ("routine_runs", "routine_id,run_id,kind,at", "routine_id IN (SELECT id FROM routines WHERE agent_id IN (SELECT id FROM agents WHERE group_id = ?1))"),
];

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Reconnect {
    pub kind: String,
    pub name: String,
    pub details: Row,
    pub agents: Vec<String>,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Archive {
    pub format: String,
    pub version: u32,
    pub tables: BTreeMap<String, Vec<Row>>,
    pub memories: BTreeMap<String, String>,
    pub files: BTreeMap<String, String>,
    pub reconnect: Vec<Reconnect>,
}

fn sql<T>(r: rusqlite::Result<T>) -> Result<T> {
    r.map_err(|e| e.to_string())
}
fn columns(conn: &Connection, table: &str) -> Result<HashSet<String>> {
    let mut query = sql(conn.prepare(&format!("PRAGMA table_info({table})")))?;
    let rows = sql(query.query_map([], |row| row.get(1)))?;
    sql(rows.collect())
}

fn read_rows(
    conn: &Connection,
    table: &str,
    allowed: &str,
    filter: &str,
    group: &str,
) -> Result<Vec<Row>> {
    let exists = columns(conn, table)?;
    if exists.is_empty() {
        return Ok(Vec::new());
    }
    let selected: Vec<_> = allowed.split(',').filter(|c| exists.contains(*c)).collect();
    let mut query =
        sql(conn.prepare(&format!("SELECT {} FROM {table} WHERE {filter}", selected.join(","))))?;
    let mut cursor = sql(query.query([group]))?;
    let mut rows = Vec::new();
    while let Some(row) = sql(cursor.next())? {
        let mut fields = Map::new();
        for (index, name) in selected.iter().enumerate() {
            let v: SqlValue = sql(row.get(index))?;
            let v = match v {
                SqlValue::Null => Value::Null,
                SqlValue::Integer(n) => n.into(),
                SqlValue::Real(n) => serde_json::json!(n),
                SqlValue::Text(s) => s.into(),
                SqlValue::Blob(_) => return Err("Unexpected binary database field.".into()),
            };
            fields.insert(name.to_string(), v);
        }
        rows.push(fields);
    }
    Ok(rows)
}

fn clean_address(value: &mut Value) {
    if let Some(text) = value.as_str() {
        if let Ok(mut url) = reqwest::Url::parse(text) {
            let _ = url.set_username("");
            let _ = url.set_password(None);
            url.set_query(None);
            url.set_fragment(None);
            *value = url.to_string().into();
        }
    }
}

/// A read transaction keeps all database tables at the same point in time.
/// The source can be opened read-only, including by the new desktop shell.
pub fn export(
    conn: &mut Connection,
    group: GroupId,
    workspace: &Workspace,
    files: &FileStore,
) -> Result<Archive> {
    let tx = sql(conn.transaction())?;
    let group = group.to_string();
    let mut archive = Archive {
        format: "guaca-group".into(),
        version: 1,
        tables: BTreeMap::new(),
        memories: BTreeMap::new(),
        files: BTreeMap::new(),
        reconnect: Vec::new(),
    };
    for (table, fields, filter) in TABLES {
        archive.tables.insert(table.to_string(), read_rows(&tx, table, fields, filter, &group)?);
    }
    if archive.tables["groups"].len() != 1 {
        return Err("This group no longer exists.".into());
    }
    if let Some(url) = archive
        .tables
        .get_mut("groups")
        .and_then(|g| g.first_mut())
        .and_then(|g| g.get_mut("base_url"))
    {
        clean_address(url);
    }
    for agent in &archive.tables["agents"] {
        let id = text(agent, "id")?;
        archive.memories.insert(
            id.into(),
            workspace.read(id.parse().map_err(|_| "Invalid agent identifier.")?),
        );
    }
    // Preserve references to services, not their grants, cookies or running machines.
    for (table, fields, kind) in [
        ("repositories", "id,name,path,remote,note,harness,gate,bench", "repository"),
        ("plugins", "id,kind,endpoint,access", "plugin"),
        ("connectors", "id,service,account,env_var,note", "credential"),
    ] {
        for mut details in read_rows(&tx, table, fields, "group_id = ?1", &group)? {
            let id = text(&details, "id")?.to_string();
            let name = details
                .get("name")
                .or_else(|| details.get("kind"))
                .or_else(|| details.get("service"))
                .and_then(Value::as_str)
                .unwrap_or(kind)
                .to_string();
            let agents =
                if kind == "repository" && columns(&tx, "agents")?.contains("repository_id") {
                    let mut query = sql(tx.prepare(
                        "SELECT name FROM agents WHERE repository_id = ?1 AND group_id = ?2",
                    ))?;
                    let rows = sql(query.query_map(params![id, group], |r| r.get(0)))?;
                    sql(rows.collect())?
                } else {
                    Vec::new()
                };
            details.remove("id");
            for key in ["remote", "endpoint"] {
                if let Some(value) = details.get_mut(key) {
                    clean_address(value);
                }
            }
            archive.reconnect.push(Reconnect { kind: kind.into(), name, details, agents });
        }
    }
    if !columns(&tx, "group_imports")?.is_empty() {
        let mut query = sql(tx.prepare("SELECT reconnect FROM group_imports WHERE group_id = ?1"))?;
        let values = sql(sql(query.query_map([&group], |r| r.get::<_, String>(0)))?
            .collect::<rusqlite::Result<Vec<_>>>())?;
        for value in values {
            archive
                .reconnect
                .extend(serde_json::from_str::<Vec<Reconnect>>(&value).map_err(|e| e.to_string())?);
        }
    }
    let mut digests = HashSet::new();
    for row in &archive.tables["messages"] {
        let parts: Value = serde_json::from_str(text(row, "parts")?).map_err(|e| e.to_string())?;
        collect_files(&parts, &mut digests);
    }
    let mut size = 0;
    for digest in digests {
        let bytes = files.read(&digest).map_err(|_| {
            format!("An attachment ({digest}) is missing. Restore it before exporting this group.")
        })?;
        size += bytes.len();
        if size > MAX_ARCHIVE / 2 {
            return Err("This group's attachments exceed the 32 MB export limit.".into());
        }
        archive.files.insert(digest, STANDARD.encode(bytes));
    }
    sql(tx.commit())?;
    if serde_json::to_vec(&archive).map_err(|e| e.to_string())?.len() > MAX_ARCHIVE {
        return Err("This group exceeds the 64 MB export limit.".into());
    }
    Ok(archive)
}

fn collect_files(value: &Value, digests: &mut HashSet<String>) {
    // Only attachment parts address this store. A tool's arguments may contain
    // an unrelated hash under the same key and must not make export fail.
    if let Some(parts) = value.as_array() {
        for part in parts {
            if part.get("type").and_then(Value::as_str) == Some("file") {
                if let Some(digest) = part.get("digest").and_then(Value::as_str) {
                    digests.insert(digest.to_string());
                }
            }
        }
    }
}

fn text<'a>(row: &'a Row, key: &str) -> Result<&'a str> {
    row.get(key).and_then(Value::as_str).ok_or_else(|| format!("The group file is missing {key}."))
}
fn fresh(map: &mut HashMap<String, String>, old: &str) -> Result<String> {
    uuid::Uuid::parse_str(old).map_err(|_| "The group file contains an invalid identifier.")?;
    Ok(map.entry(old.into()).or_insert_with(|| uuid::Uuid::new_v4().to_string()).clone())
}
fn remap_json(value: &mut Value, ids: &HashMap<String, String>) {
    match value {
        Value::String(s) => {
            if let Some(new) = ids.get(s) {
                *s = new.clone();
            }
        }
        Value::Array(a) => {
            for v in a {
                remap_json(v, ids);
            }
        }
        Value::Object(o) => {
            for v in o.values_mut() {
                remap_json(v, ids);
            }
        }
        _ => {}
    }
}

fn validate_row(table: &str, row: &Row) -> Result<()> {
    if table == "agents" {
        if crate::domain::agent::Lifecycle::parse(text(row, "lifecycle")?).is_none() {
            return Err("An agent has an unknown state.".into());
        }
        serde_json::from_str::<Vec<String>>(text(row, "skills")?)
            .map_err(|_| "An agent has invalid skills.")?;
        if text(row, "name")?.trim().is_empty() {
            return Err("An agent has no name.".into());
        }
    }
    if table == "routines" && crate::domain::routine::Trigger::parse(text(row, "fires")?).is_none()
    {
        return Err("A routine has an unsupported schedule.".into());
    }
    if table == "messages" {
        serde_json::from_str::<Vec<crate::domain::envelope::Part>>(text(row, "parts")?)
            .map_err(|_| "A conversation contains unreadable content.")?;
        for key in ["from_kind", "to_kind"] {
            if !["agent", "operator", "system"].contains(&text(row, key)?) {
                return Err("A conversation has an unknown participant.".into());
            }
        }
    }
    if table == "messages" {
        if crate::domain::envelope::Trust::parse(text(row, "trust")?).is_none() {
            return Err("A conversation has an unknown trust level.".into());
        }
        for (kind, agent) in [("from_kind", "from_agent"), ("to_kind", "to_agent")] {
            if text(row, kind)? == "agent" {
                text(row, agent)?;
            }
        }
    }
    for (key, value) in row {
        if [
            "version",
            "pinned",
            "rail_order",
            "has_computer",
            "has_browser",
            "hop",
            "prompt",
            "completion",
            "max_hops",
            "max_steps_per_run",
            "max_fanout_per_call",
            "max_sends_per_pair",
            "max_tool_rounds",
            "request_timeout_secs",
        ]
        .contains(&key.as_str())
            && !value.is_null()
            && value.as_u64().is_none_or(|v| v > u32::MAX as u64)
        {
            return Err("The group file contains an invalid numeric setting.".into());
        }
    }
    Ok(())
}

pub fn import(
    store: &Store,
    mut archive: Archive,
    name: String,
    workspace: &Workspace,
    files: &FileStore,
) -> Result<GroupId> {
    if archive.format != "guaca-group" || archive.version != 1 {
        return Err("This group file needs a different version of Guaca.".into());
    }
    if serde_json::to_vec(&archive).map_err(|e| e.to_string())?.len() > MAX_ARCHIVE {
        return Err("The group file exceeds 64 MB.".into());
    }
    let clean =
        crate::domain::group::GroupDraft { name, inference: None, api_key: None, limits: None }
            .validate()
            .map_err(|e| e.to_string())?;
    if archive.tables.get("groups").map(Vec::len) != Some(1) {
        return Err("A group file must contain exactly one group.".into());
    }
    let source = text(&archive.tables["groups"][0], "id")?.to_string();
    let mut ids = HashMap::new();
    let mut members: HashMap<&str, HashSet<String>> = HashMap::new();
    for (table, rows) in &archive.tables {
        let allowed = TABLES
            .iter()
            .find(|t| t.0 == table)
            .ok_or("The group file contains an unknown section.")?
            .1
            .split(',')
            .collect::<HashSet<_>>();
        for row in rows {
            if row.keys().any(|k| !allowed.contains(k.as_str())) {
                return Err("The group file contains an unexpected field.".into());
            }
            validate_row(table, row)?;
            if ["groups", "agents", "routines", "messages", "occasions"].contains(&table.as_str()) {
                let id = text(row, "id")?;
                if ids.contains_key(id) {
                    return Err("The group file contains a duplicate identifier.".into());
                }
                fresh(&mut ids, id)?;
                members.entry(table).or_default().insert(id.into());
            }
        }
    }
    let belongs = |table: &str, value: &str| members.get(table).is_some_and(|s| s.contains(value));
    for rows in archive.tables.values() {
        for row in rows {
            for (field, target) in [
                ("group_id", "groups"),
                ("agent_id", "agents"),
                ("channel_id", "agents"),
                ("from_agent", "agents"),
                ("to_agent", "agents"),
                ("routine_id", "routines"),
            ] {
                if let Some(value) = row.get(field).filter(|v| !v.is_null()) {
                    if !value.as_str().is_some_and(|v| belongs(target, v)) {
                        return Err("The group file refers to data outside its group.".into());
                    }
                }
            }
            if let Some(run) = row.get("run_id").and_then(Value::as_str) {
                fresh(&mut ids, run)?;
            }
        }
    }
    for key in archive.memories.keys() {
        if !belongs("agents", key) {
            return Err("A memory belongs to an unknown agent.".into());
        }
    }
    let group: GroupId = ids[&source].parse().map_err(|_| "Invalid group identifier.")?;
    archive.tables.get_mut("groups").unwrap()[0].insert("name".into(), clean.name.into());
    let mut conn = store.conn().map_err(|e| e.to_string())?;
    let tx = sql(conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate))?;
    for (table, _, _) in TABLES {
        for original in archive.tables.get(*table).into_iter().flatten() {
            let mut row = original.clone();
            for (key, value) in &mut row {
                if key == "parts" {
                    let mut parts: Value =
                        serde_json::from_str(value.as_str().ok_or("Invalid message content.")?)
                            .map_err(|e| e.to_string())?;
                    remap_json(&mut parts, &ids);
                    *value = serde_json::to_string(&parts).map_err(|e| e.to_string())?.into();
                } else if key == "id"
                    || key.ends_with("_id")
                    || ["from_agent", "to_agent", "cause"].contains(&key.as_str())
                {
                    if key == "cause" && value.as_str().is_some_and(|s| !ids.contains_key(s)) {
                        *value = Value::Null;
                    } else {
                        remap_json(value, &ids);
                    }
                }
            }
            if *table == "routines" {
                row.insert("active".into(), 0.into());
            }
            let keys = row.keys().map(String::as_str).collect::<Vec<_>>();
            let values = row
                .values()
                .map(|v| match v {
                    Value::Null => Ok(SqlValue::Null),
                    Value::String(s) => Ok(SqlValue::Text(s.clone())),
                    Value::Number(n) => n
                        .as_i64()
                        .map(SqlValue::Integer)
                        .or_else(|| n.as_f64().map(SqlValue::Real))
                        .ok_or("Invalid number."),
                    _ => Err("The group file contains an invalid database value."),
                })
                .collect::<std::result::Result<Vec<_>, _>>()?;
            let placeholders = vec!["?"; keys.len()].join(",");
            sql(tx.execute(
                &format!("INSERT INTO {table} ({}) VALUES ({placeholders})", keys.join(",")),
                rusqlite::params_from_iter(values),
            ))?;
        }
    }
    sql(tx.execute(
        "INSERT INTO group_imports (group_id,reconnect) VALUES (?1,?2)",
        params![
            group.to_string(),
            serde_json::to_string(&archive.reconnect).map_err(|e| e.to_string())?
        ],
    ))?;
    // Validate attachment hashes before any file is installed. The content store cannot overwrite unrelated files.
    let mut decoded = Vec::new();
    for (digest, encoded) in archive.files {
        use sha2::{Digest, Sha256};
        let bytes = STANDARD.decode(encoded).map_err(|_| "An attachment is damaged.")?;
        if bytes.len() > crate::domain::attachment::MAX_FILE_BYTES as usize
            || format!("{:x}", Sha256::digest(&bytes)) != digest
        {
            return Err("An attachment failed its integrity check.".into());
        }
        decoded.push(bytes);
    }
    let mut required = HashSet::new();
    for row in archive.tables.get("messages").into_iter().flatten() {
        collect_files(
            &serde_json::from_str::<Value>(text(row, "parts")?).map_err(|e| e.to_string())?,
            &mut required,
        );
    }
    for bytes in &decoded {
        files.put("imported-file", bytes).map_err(|e| e.to_string())?;
    }
    for digest in required {
        files.read(&digest).map_err(|_| "The group file is missing an attachment.")?;
    }
    let mut memories_written = Vec::new();
    let finish = || -> Result<()> {
        for (old, content) in &archive.memories {
            let agent: AgentId = ids[old].parse().map_err(|_| "Invalid agent identifier.")?;
            let name = archive.tables["agents"]
                .iter()
                .find(|r| r.get("id").and_then(Value::as_str) == Some(old))
                .and_then(|r| r.get("name"))
                .and_then(Value::as_str)
                .ok_or("A memory has no agent.")?;
            let stored = workspace.write(agent, name, content).map_err(|e| e.to_string())?;
            memories_written.push(workspace.preferred_path(agent, name));
            if stored.truncated {
                return Err("A memory is too large for this version of Guaca.".into());
            }
        }
        sql(tx.commit())
    };
    if let Err(error) = finish() {
        for path in memories_written {
            let _ = std::fs::remove_file(path);
        }
        return Err(error);
    }
    tracing::info!(%group, "imported a group with routines paused and services disconnected");
    Ok(group)
}

pub fn reconnect(store: &Store, group: GroupId) -> Result<Vec<Reconnect>> {
    use rusqlite::OptionalExtension;
    let conn = store.conn().map_err(|e| e.to_string())?;
    let value: Option<String> = sql(conn
        .query_row(
            "SELECT reconnect FROM group_imports WHERE group_id = ?1",
            [group.to_string()],
            |r| r.get(0),
        )
        .optional())?;
    value
        .map(|v| serde_json::from_str(&v).map_err(|e| e.to_string()))
        .transpose()
        .map(Option::unwrap_or_default)
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture {
        _dir: tempfile::TempDir,
        store: Store,
        workspace: Workspace,
        files: FileStore,
        group: GroupId,
        agent: AgentId,
    }
    impl Fixture {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let store = Store::open(&dir.path().join("guac.db")).unwrap();
            let workspace = Workspace::new(dir.path().join("workspace"));
            let files = FileStore::new(dir.path().join("files"));
            let group = GroupId::new();
            let agent = AgentId::new();
            let conn = store.conn().unwrap();
            conn.execute("INSERT INTO groups (id,name,created_at,api_key) VALUES (?1,'Crew',1,'provider-secret')", [group.to_string()]).unwrap();
            conn.execute("INSERT INTO agents (id,group_id,name,avatar,color,model,system_prompt,skills,lifecycle,created_at,updated_at,sandbox_id,sandbox_envd_token,has_computer) VALUES (?1,?2,'Engineer','avocado','#ffffff','small','Write carefully','[]','active',1,1,'rented-machine','machine-secret',1)", params![agent.to_string(),group.to_string()]).unwrap();
            workspace.write(agent, "Engineer", "Remember the tests.").unwrap();
            let attached = files.put("hello.md", b"hello").unwrap();
            let parts = serde_json::to_string(&vec![crate::domain::envelope::Part::File(attached)])
                .unwrap();
            conn.execute("INSERT INTO messages (id,run_id,channel_id,from_kind,to_kind,to_agent,parts,trust,created_at) VALUES (?1,?2,?3,'operator','agent',?3,?4,'operator',1)", params![uuid::Uuid::new_v4().to_string(),uuid::Uuid::new_v4().to_string(),agent.to_string(),parts]).unwrap();
            conn.execute("INSERT INTO routines (id,agent_id,what,fires,created_at,active) VALUES (?1,?2,'Check tests','once',1,1)", params![uuid::Uuid::new_v4().to_string(),agent.to_string()]).unwrap();
            conn.execute("INSERT INTO occasions (id,group_id,agent_id,title,starts_at,created_at,updated_at) VALUES (?1,?2,?3,'Review',1000,1,1)", params![uuid::Uuid::new_v4().to_string(),group.to_string(),agent.to_string()]).unwrap();
            conn.execute("INSERT INTO repositories (id,group_id,name,path,harness,created_at,updated_at) VALUES (?1,?2,'Code','/old/code','codex',1,1)",params![uuid::Uuid::new_v4().to_string(),group.to_string()]).unwrap();
            Self { _dir: dir, store, workspace, files, group, agent }
        }
        fn export(&self) -> Archive {
            export(&mut self.store.conn().unwrap(), self.group, &self.workspace, &self.files)
                .unwrap()
        }
        fn import(&self, archive: Archive) -> Result<GroupId> {
            import(&self.store, archive, "Copy".into(), &self.workspace, &self.files)
        }
    }
    #[test]
    fn stored_secrets_and_other_groups_never_leave() {
        let f = Fixture::new();
        let archive = f.export();
        let text = serde_json::to_string(&archive).unwrap();
        assert!(!text.contains("provider-secret"));
        assert!(!text.contains("machine-secret"));
        assert!(!text.contains("rented-machine"));
        assert_eq!(archive.tables["groups"].len(), 1);
        assert_eq!(archive.tables["agents"].len(), 1);
        assert_eq!(archive.reconnect[0].details["harness"], "codex");
    }
    #[test]
    fn round_trip_preserves_history_and_memory_without_reusing_identity_or_firing() {
        let f = Fixture::new();
        let imported = f.import(f.export()).unwrap();
        assert_ne!(imported, f.group);
        let agents = f.store.list_agents().unwrap();
        let copied = agents.iter().find(|a| a.group_id == imported).unwrap();
        assert_ne!(copied.id, f.agent);
        assert!(!copied.has_computer);
        assert!(copied.sandbox_id.is_none());
        assert_eq!(f.workspace.read(copied.id), f.workspace.read(f.agent));
        let conn = f.store.conn().unwrap();
        assert_eq!(
            conn.query_row(
                "SELECT count(*) FROM messages WHERE channel_id = ?1",
                [copied.id.to_string()],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
        assert_eq!(
            conn.query_row(
                "SELECT active FROM routines WHERE agent_id = ?1",
                [copied.id.to_string()],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        assert_eq!(
            conn.query_row(
                "SELECT active FROM routines WHERE agent_id = ?1",
                [f.agent.to_string()],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
        assert_eq!(
            conn.query_row(
                "SELECT agent_id FROM occasions WHERE group_id = ?1",
                [imported.to_string()],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            copied.id.to_string()
        );
        assert_eq!(reconnect(&f.store, imported).unwrap().len(), 1);
        assert!(!f.store.get_group(imported).unwrap().unwrap().api_key_set);
    }
    #[test]
    fn unknown_fields_and_cross_group_references_are_refused_without_partial_groups() {
        let f = Fixture::new();
        let before = f.store.list_groups().unwrap().len();
        let mut archive = f.export();
        archive.tables.get_mut("agents").unwrap()[0]
            .insert("sandbox_id".into(), "other-machine".into());
        assert!(f.import(archive).is_err());
        let mut archive = f.export();
        archive.tables.get_mut("messages").unwrap()[0]
            .insert("channel_id".into(), AgentId::new().to_string().into());
        assert!(f.import(archive).is_err());
        assert_eq!(f.store.list_groups().unwrap().len(), before);
    }
    #[test]
    fn damaged_files_and_truncated_memories_roll_back_the_entire_group() {
        let f = Fixture::new();
        let before = f.store.list_groups().unwrap().len();
        let mut archive = f.export();
        *archive.files.values_mut().next().unwrap() = STANDARD.encode(b"wrong");
        assert!(f.import(archive).is_err());
        let mut archive = f.export();
        *archive.memories.values_mut().next().unwrap() =
            "x".repeat(crate::workspace::MAX_MEMORY + 1);
        assert!(f.import(archive).is_err());
        assert_eq!(f.store.list_groups().unwrap().len(), before);
        assert_eq!(std::fs::read_dir(f.workspace.root()).unwrap().count(), 1);
    }
    #[test]
    fn duplicate_names_do_not_replace_existing_groups() {
        let f = Fixture::new();
        f.import(f.export()).unwrap();
        assert!(f.import(f.export()).is_err());
        assert_eq!(f.store.list_groups().unwrap().len(), 3);
    }
    #[test]
    fn legacy_export_does_not_migrate_or_write_its_database() {
        let f = Fixture::new();
        let conn = f.store.conn().unwrap();
        conn.execute("DROP TABLE group_imports", []).unwrap();
        conn.pragma_update(None, "user_version", 49).unwrap();
        drop(conn);
        let mut conn = Connection::open_with_flags(
            f._dir.path().join("guac.db"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        assert!(export(&mut conn, f.group, &f.workspace, &f.files).is_ok());
        assert_eq!(
            conn.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0)).unwrap(),
            49
        );
    }
}
