use rusqlite::{params, OptionalExtension, Transaction, TransactionBehavior};

use super::{Store, StoreError};
use crate::domain::ids::GroupId;
use crate::domain::now_ms;
use crate::domain::policy::{Rule, RuleDraft};

fn encode<T: serde::Serialize>(value: &T) -> Result<String, StoreError> {
    serde_json::to_string(value).map_err(|e| StoreError::Corrupt(e.to_string()))
}

fn decode<T: serde::de::DeserializeOwned>(value: &str) -> Result<T, StoreError> {
    serde_json::from_str(value).map_err(|e| StoreError::Corrupt(format!("invalid policy: {e}")))
}

fn read(row: &rusqlite::Row<'_>) -> rusqlite::Result<Result<Rule, StoreError>> {
    let id: String = row.get(0)?;
    let group: Option<String> = row.get(1)?;
    let effect: String = row.get(2)?;
    let class: String = row.get(3)?;
    let tool: Option<String> = row.get(4)?;
    let agents: String = row.get(5)?;
    let conditions: String = row.get(6)?;
    let wrote: String = row.get(7)?;
    let created_at: i64 = row.get(8)?;
    let version: u64 = row.get(9)?;
    Ok((|| {
        let draft = RuleDraft {
            group_id: group
                .map(|g| {
                    g.parse().map_err(|e| StoreError::Corrupt(format!("invalid group id: {e}")))
                })
                .transpose()?,
            effect: decode(&effect)?,
            class: decode(&class)?,
            tool,
            agents: decode(&agents)?,
            conditions: decode(&conditions)?,
        };
        draft.validate().map_err(StoreError::Corrupt)?;
        if version == 0 {
            return Err(StoreError::Corrupt("invalid policy version".into()));
        }
        Ok(Rule { id, draft, wrote, created_at, version })
    })())
}

fn validate(tx: &Transaction<'_>, draft: &RuleDraft) -> Result<(), StoreError> {
    draft.validate().map_err(StoreError::Decision)?;
    if let Some(group) = draft.group_id {
        if !tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM groups WHERE id = ?1)",
            [group.to_string()],
            |r| r.get::<_, bool>(0),
        )? {
            return Err(StoreError::GroupNotFound(group));
        }
    }
    for agent in &draft.agents {
        let group: Option<String> = tx
            .query_row(
                "SELECT group_id FROM agents WHERE id = ?1 AND lifecycle != 'terminated'",
                [agent.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        if group.is_none() {
            return Err(StoreError::AgentNotFound(*agent));
        }
        if draft.group_id.is_some_and(|g| group.as_deref() != Some(g.to_string().as_str())) {
            return Err(StoreError::Decision("Agent is outside the policy's group".into()));
        }
    }
    Ok(())
}

const COLUMNS: &str =
    "id, group_id, effect, class, tool, agents, conditions, wrote, created_at, version";

impl Store {
    pub fn create_policy(&self, draft: RuleDraft) -> Result<Rule, StoreError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        validate(&tx, &draft)?;
        let rule = Rule {
            id: uuid::Uuid::new_v4().to_string(),
            wrote: draft.wrote(),
            draft,
            created_at: now_ms(),
            version: 1,
        };
        tx.execute("INSERT INTO approval_policies (id, group_id, effect, class, tool, agents, conditions, wrote, created_at, version) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)", params![rule.id, rule.draft.group_id.map(|g| g.to_string()), encode(&rule.draft.effect)?, encode(&rule.draft.class)?, rule.draft.tool, encode(&rule.draft.agents)?, encode(&rule.draft.conditions)?, rule.wrote, rule.created_at, rule.version])?;
        tx.commit()?;
        Ok(rule)
    }

    pub fn update_policy(
        &self,
        id: &str,
        scope: Option<GroupId>,
        version: u64,
        draft: RuleDraft,
    ) -> Result<Rule, StoreError> {
        if draft.group_id != scope {
            return Err(StoreError::Decision("A policy cannot move between scopes".into()));
        }
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        validate(&tx, &draft)?;
        let changed = tx.execute("UPDATE approval_policies SET effect = ?1, class = ?2, tool = ?3, agents = ?4, conditions = ?5, wrote = ?6, version = version + 1 WHERE id = ?7 AND group_id IS ?8 AND version = ?9", params![encode(&draft.effect)?, encode(&draft.class)?, draft.tool, encode(&draft.agents)?, encode(&draft.conditions)?, draft.wrote(), id, scope.map(|g| g.to_string()), version])?;
        if changed != 1 {
            return Err(StoreError::Decision("Policy is missing or has changed".into()));
        }
        let rule = tx.query_row(
            &format!("SELECT {COLUMNS} FROM approval_policies WHERE id = ?1"),
            [id],
            read,
        )??;
        tx.commit()?;
        Ok(rule)
    }

    pub fn delete_policy(
        &self,
        id: &str,
        scope: Option<GroupId>,
        version: u64,
    ) -> Result<bool, StoreError> {
        let conn = self.conn()?;
        Ok(conn.execute(
            "DELETE FROM approval_policies WHERE id = ?1 AND group_id IS ?2 AND version = ?3",
            params![id, scope.map(|g| g.to_string()), version],
        )? == 1)
    }

    pub fn list_policies(&self, scope: Option<GroupId>) -> Result<Vec<Rule>, StoreError> {
        let conn = self.conn()?;
        let mut statement = conn.prepare(&format!(
            "SELECT {COLUMNS} FROM approval_policies WHERE group_id IS ?1 ORDER BY created_at, id"
        ))?;
        let rows = statement.query_map([scope.map(|g| g.to_string())], read)?;
        rows.map(|row| row?).collect()
    }

    pub fn policies_for(&self, group: GroupId) -> Result<Vec<Rule>, StoreError> {
        let conn = self.conn()?;
        let mut statement = conn.prepare(&format!("SELECT {COLUMNS} FROM approval_policies WHERE group_id IS NULL OR group_id = ?1 ORDER BY created_at, id"))?;
        let rows = statement.query_map([group.to_string()], read)?;
        rows.map(|row| row?).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::agent::CleanDraft;
    use crate::domain::group::CleanGroup;
    use crate::domain::policy::{Class, Effect};

    #[test]
    fn policies_are_scoped_versioned_and_cascade_with_their_group() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("policies.db");
        let store = Store::open(&path).unwrap();
        let group = store
            .create_group(&CleanGroup { name: "Sales".into(), ..Default::default() })
            .unwrap()
            .id;
        let other = store
            .create_group(&CleanGroup { name: "Other".into(), ..Default::default() })
            .unwrap()
            .id;
        let agent = store
            .create_agent(&CleanDraft {
                group_id: Some(other),
                name: "Rae".into(),
                avatar: "orb".into(),
                color: "#7fb069".into(),
                model: "test".into(),
                subscription_model: None,
                reasoning_effort: None,
                system_prompt: String::new(),
                skills: vec![],
            })
            .unwrap();
        let mut draft = RuleDraft {
            group_id: Some(group),
            effect: Effect::Block,
            class: Class::ActOnBehalf,
            tool: None,
            agents: vec![agent.id],
            conditions: vec![],
        };
        assert!(store.create_policy(draft.clone()).is_err());
        draft.agents.clear();
        let first = store.create_policy(draft.clone()).unwrap();
        let mut global = draft.clone();
        global.group_id = None;
        store.create_policy(global).unwrap();
        assert_eq!(store.list_policies(Some(group)).unwrap(), vec![first.clone()]);
        assert_eq!(store.policies_for(group).unwrap().len(), 2);
        assert_eq!(store.policies_for(other).unwrap().len(), 1);
        assert!(!store.delete_policy(&first.id, Some(other), first.version).unwrap());
        draft.tool = Some("gmail__send".into());
        let second =
            store.update_policy(&first.id, Some(group), first.version, draft.clone()).unwrap();
        assert_eq!(second.version, 2);
        assert!(store.update_policy(&first.id, Some(group), first.version, draft).is_err());
        assert!(!store.delete_policy(&first.id, Some(group), first.version).unwrap());
        drop(store);
        let store = Store::open(&path).unwrap();
        assert_eq!(store.list_policies(Some(group)).unwrap(), vec![second]);
        store
            .conn()
            .unwrap()
            .execute("DELETE FROM groups WHERE id = ?1", [group.to_string()])
            .unwrap();
        assert!(store.list_policies(Some(group)).unwrap().is_empty());
        assert_eq!(store.list_policies(None).unwrap().len(), 1);
    }
}
