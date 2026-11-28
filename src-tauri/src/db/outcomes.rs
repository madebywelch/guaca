use rusqlite::{params, Connection, OptionalExtension, Row, TransactionBehavior};

use super::store::row_to_envelope;
use super::{Store, StoreError};
use crate::domain::envelope::{Envelope, NoticeKind, Part, Participant};
use crate::domain::ids::{AgentId, GroupId, MessageId, RunId};
use crate::domain::outcome::{
    first_sentence, headline_of, Need, Outcome, Running, Settled, Status, TaskRecord, Trigger,
    NO_NOTE, STOPPED,
};

const ENVELOPE: &str = "m.id,m.run_id,m.channel_id,m.from_kind,m.from_agent,m.to_kind,m.to_agent,\
     m.parts,m.trust,m.hop,m.expects_reply,m.intent,m.cause,m.created_at";

const COLUMNS: &str = "run_id,agent_id,group_id,trigger,trigger_ref,status,headline,note_id,needs,\
     steps,started_at,finished_at,seen_at";

pub const NOTHING_NEW: &str = "Ran and found nothing new.";

const GUARD_STOP: &str = "EXISTS (SELECT 1 FROM json_each(m.parts) \
     WHERE json_extract(value,'$.type')='notice' AND json_extract(value,'$.kind')='guardStop')";

pub const LIST_MAX: u32 = 500;

const FAILED: &str = "EXISTS (SELECT 1 FROM json_each(m.parts) \
     WHERE json_extract(value,'$.type')='notice' AND json_extract(value,'$.kind')='upstreamError')";

const CUT_SHORT: &str = "EXISTS (SELECT 1 FROM json_each(m.parts) \
     WHERE json_extract(value,'$.type')='notice' AND json_extract(value,'$.kind')='interrupted')";

pub(super) fn record(
    tx: &Connection,
    run: RunId,
    settled: Settled,
    steps: u32,
    now: i64,
) -> Result<Option<Outcome>, StoreError> {
    if settled == Settled::Absorbed {
        return Ok(None);
    }
    let Some(start) = first(tx, "1=1", "ASC", params![run.to_string()])? else {
        return Ok(None);
    };
    let Some(Owner { agent: owner, group, trigger, trigger_ref }) = owned(tx, &start)? else {
        return Ok(None);
    };

    let note = first(
        tx,
        "m.from_kind='agent' AND m.from_agent=?2 AND m.to_kind='human'",
        "DESC",
        params![run.to_string(), owner.to_string()],
    )?;
    let cut_short = match settled {
        Settled::Interrupted => first(tx, CUT_SHORT, "DESC", params![run.to_string()])?,
        _ => None,
    };
    let failure = first(tx, FAILED, "DESC", params![run.to_string()])?
        .filter(|failure| note.as_ref().is_none_or(|note| failure.created_at > note.created_at));
    let needs = left_open(tx, run)?;
    let quiet = note.is_none()
        && needs.is_empty()
        && matches!(trigger, Trigger::Routine | Trigger::Event)
        && first(
            tx,
            &format!("m.channel_id=?2 AND {GUARD_STOP}"),
            "DESC",
            params![run.to_string(), owner.to_string()],
        )?
        .is_none();

    let quoted = note.as_ref().and_then(headline_of);
    let (status, headline, note_id) = match (settled, failure) {
        (Settled::Interrupted, _) => (
            Status::Interrupted,
            cut_short
                .as_ref()
                .map(|notice| first_sentence(&notice_text(notice)))
                .unwrap_or_default(),
            cut_short.as_ref().map(|notice| notice.id),
        ),
        (Settled::Stopped, _) => {
            (Status::Stopped, quoted.unwrap_or_else(|| STOPPED.into()), note.as_ref().map(|n| n.id))
        }
        (_, Some(failure)) => {
            (Status::Failed, first_sentence(&notice_text(&failure)), Some(failure.id))
        }
        _ if quiet => (Status::NothingNew, NOTHING_NEW.into(), None),
        _ => (
            if needs.is_empty() { Status::Done } else { Status::NeedsYou },
            quoted.unwrap_or_else(|| NO_NOTE.into()),
            note.as_ref().map(|n| n.id),
        ),
    };

    let outcome = Outcome {
        run_id: run,
        agent_id: owner,
        group_id: group,
        trigger,
        trigger_ref,
        status,
        headline,
        note_id,
        colleagues: colleagues(tx, run, owner)?,
        needs,
        steps,
        started_at: start.created_at,
        finished_at: now,
        seen_at: None,
    };
    if !insert(tx, &outcome)? {
        return Ok(None);
    }
    announce(tx, &outcome);
    Ok(Some(outcome))
}

fn announce(tx: &Connection, outcome: &Outcome) {
    let written = super::outbox::name_of(tx, outcome.agent_id).and_then(|name| {
        match crate::domain::outbox::for_outcome(outcome, &name.unwrap_or_default()) {
            Some(draft) => super::outbox::enqueue(tx, &draft, outcome.finished_at).map(drop),
            None => Ok(()),
        }
    });
    if let Err(err) = written {
        tracing::error!(%err, run = %outcome.run_id, "could not write what announces a finished run");
    }
}

pub(super) fn record_interrupted(
    tx: &Connection,
    original: &Envelope,
    notice: &Envelope,
    now: i64,
) -> Result<(), StoreError> {
    let Some(Owner { agent: owner, group, trigger, trigger_ref }) = owned(tx, original)? else {
        return Ok(());
    };
    let run = original.run_id.to_string();
    let steps: u32 = tx.query_row(
        &format!(
            "SELECT (SELECT count(*) FROM usage WHERE run_id=?1)
                  + (SELECT count(*) FROM messages m WHERE m.run_id=?1 AND {FAILED})"
        ),
        [&run],
        |row| row.get(0),
    )?;
    let outcome = Outcome {
        run_id: original.run_id,
        agent_id: owner,
        group_id: group,
        trigger,
        trigger_ref,
        status: Status::Interrupted,
        headline: first_sentence(&notice_text(notice)),
        note_id: Some(notice.id),
        colleagues: colleagues(tx, original.run_id, owner)?,
        needs: left_open(tx, original.run_id)?,
        steps,
        started_at: original.created_at,
        finished_at: now,
        seen_at: None,
    };
    if insert(tx, &outcome)? {
        announce(tx, &outcome);
        if let Err(err) = super::recall::file(tx, original.run_id) {
            tracing::error!(%err, run = %original.run_id, "could not file what the operator said for recall");
        }
    }
    Ok(())
}

struct Owner {
    agent: AgentId,
    group: GroupId,
    trigger: Trigger,
    trigger_ref: Option<String>,
}

fn owned(tx: &Connection, start: &Envelope) -> Result<Option<Owner>, StoreError> {
    let Participant::Agent { id: owner } = start.to else { return Ok(None) };
    let Some((trigger, trigger_ref)) = Trigger::of(start) else { return Ok(None) };
    let group: Option<String> = tx
        .query_row(
            "SELECT group_id FROM agents WHERE id=?1 AND lifecycle<>'terminated'",
            [owner.to_string()],
            |row| row.get(0),
        )
        .optional()?;
    let Some(group) = group else { return Ok(None) };
    let group = group.parse().map_err(|e| StoreError::Corrupt(format!("bad group id: {e}")))?;
    Ok(Some(Owner { agent: owner, group, trigger, trigger_ref }))
}

fn first(
    tx: &Connection,
    filter: &str,
    order: &str,
    args: &[&dyn rusqlite::ToSql],
) -> Result<Option<Envelope>, StoreError> {
    let found = tx
        .query_row(
            &format!(
                "SELECT {ENVELOPE} FROM messages m WHERE m.run_id=?1 AND {filter}
                  ORDER BY m.created_at {order}, m.id {order} LIMIT 1"
            ),
            args,
            row_to_envelope,
        )
        .optional()?;
    found.transpose()
}

fn colleagues(tx: &Connection, run: RunId, owner: AgentId) -> Result<Vec<AgentId>, StoreError> {
    let mut stmt = tx.prepare_cached(
        "SELECT m.from_agent FROM messages m
          WHERE m.run_id=?1 AND m.from_kind='agent' AND m.to_kind='agent'
            AND m.to_agent=?2 AND m.from_agent<>?2
          GROUP BY m.from_agent
          ORDER BY min(m.created_at), m.from_agent",
    )?;
    let rows =
        stmt.query_map(params![run.to_string(), owner.to_string()], |row| row.get::<_, String>(0))?;
    let mut found = Vec::new();
    for id in rows {
        found.push(id?.parse().map_err(|e| StoreError::Corrupt(format!("bad agent id: {e}")))?);
    }
    Ok(found)
}

fn with_colleagues(tx: &Connection, mut outcome: Outcome) -> Result<Outcome, StoreError> {
    outcome.colleagues = colleagues(tx, outcome.run_id, outcome.agent_id)?;
    Ok(outcome)
}

fn left_open(tx: &Connection, run: RunId) -> Result<Vec<Need>, StoreError> {
    let run = run.to_string();
    let mut needs = Vec::new();
    let mut decisions = tx.prepare(
        "SELECT id FROM decisions WHERE filed_run=?1 AND status='pending' ORDER BY created_at,id",
    )?;
    for id in decisions.query_map([&run], |row| row.get::<_, String>(0))? {
        let id = id?.parse().map_err(|e| StoreError::Corrupt(format!("bad decision id: {e}")))?;
        needs.push(Need::Decision { id });
    }
    let mut escalations =
        tx.prepare("SELECT id FROM escalations WHERE run_id=?1 AND cleared_at IS NULL")?;
    for id in escalations.query_map([&run], |row| row.get::<_, String>(0))? {
        let id = id?.parse().map_err(|e| StoreError::Corrupt(format!("bad escalation id: {e}")))?;
        needs.push(Need::Escalation { id });
    }
    Ok(needs)
}

fn notice_text(message: &Envelope) -> String {
    message
        .parts
        .iter()
        .find_map(|part| match part {
            Part::Notice {
                kind: NoticeKind::UpstreamError | NoticeKind::Interrupted,
                text,
                ..
            } => Some(text.clone()),
            _ => None,
        })
        .unwrap_or_default()
}

fn insert(tx: &Connection, outcome: &Outcome) -> Result<bool, StoreError> {
    let needs = (!outcome.needs.is_empty())
        .then(|| serde_json::to_string(&outcome.needs).expect("a list of ids serializes"));
    let written = tx.execute(
        &format!("INSERT OR IGNORE INTO outcomes ({COLUMNS}) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)"),
        params![
            outcome.run_id.to_string(),
            outcome.agent_id.to_string(),
            outcome.group_id.to_string(),
            outcome.trigger.as_str(),
            outcome.trigger_ref,
            outcome.status.as_str(),
            outcome.headline,
            outcome.note_id.map(|id| id.to_string()),
            needs,
            outcome.steps,
            outcome.started_at,
            outcome.finished_at,
            outcome.seen_at,
        ],
    )?;
    Ok(written == 1)
}

fn read(row: &Row<'_>) -> rusqlite::Result<Result<Outcome, StoreError>> {
    let raw: [String; 3] = [row.get(0)?, row.get(1)?, row.get(2)?];
    let trigger: String = row.get(3)?;
    let trigger_ref: Option<String> = row.get(4)?;
    let status: String = row.get(5)?;
    let headline: String = row.get(6)?;
    let note: Option<String> = row.get(7)?;
    let needs: Option<String> = row.get(8)?;
    let steps: u32 = row.get(9)?;
    let started_at: i64 = row.get(10)?;
    let finished_at: i64 = row.get(11)?;
    let seen_at: Option<i64> = row.get(12)?;
    let corrupt =
        |what: &str, e: &dyn std::fmt::Display| StoreError::Corrupt(format!("{what}: {e}"));
    Ok((|| {
        Ok(Outcome {
            run_id: raw[0].parse().map_err(|e| corrupt("bad run id", &e))?,
            agent_id: raw[1].parse().map_err(|e| corrupt("bad agent id", &e))?,
            group_id: raw[2].parse().map_err(|e| corrupt("bad group id", &e))?,
            trigger: Trigger::parse(&trigger)
                .ok_or_else(|| StoreError::Corrupt(format!("unknown trigger {trigger:?}")))?,
            trigger_ref,
            status: Status::parse(&status)
                .ok_or_else(|| StoreError::Corrupt(format!("unknown status {status:?}")))?,
            headline,
            note_id: note
                .map(|id| id.parse())
                .transpose()
                .map_err(|e| corrupt("bad note id", &e))?,
            colleagues: Vec::new(),
            needs: needs
                .map(|raw| serde_json::from_str(&raw))
                .transpose()
                .map_err(|e| corrupt("unreadable needs", &e))?
                .unwrap_or_default(),
            steps,
            started_at,
            finished_at,
            seen_at,
        })
    })())
}

fn closes<'a>(run: &'a [Envelope], agent: AgentId, batch: &[MessageId]) -> Option<&'a Envelope> {
    let start = run.first()?;
    let me = Participant::Agent { id: agent };
    if start.to != me || Trigger::of(start).is_none() {
        return None;
    }
    let mut asked: Vec<(AgentId, usize)> = Vec::new();
    for (at, envelope) in run.iter().enumerate() {
        if let (true, Participant::Agent { id: peer }) = (envelope.from == me, envelope.to) {
            if peer != agent && envelope.intent.is_work() {
                asked.retain(|(who, _)| *who != peer);
                asked.push((peer, at));
            }
        }
    }
    let answers =
        |peer: AgentId, e: &Envelope| e.from == Participant::Agent { id: peer } && e.to == me;
    let answered = |peer: AgentId, at: usize| run[at + 1..].iter().any(|e| answers(peer, e));
    if asked.is_empty() || !asked.iter().all(|(peer, at)| answered(*peer, *at)) {
        return None;
    }
    let closing = run.iter().enumerate().any(|(i, envelope)| {
        let Participant::Agent { id: peer } = envelope.from else { return false };
        batch.contains(&envelope.id)
            && envelope.to == me
            && asked.iter().any(|(who, at)| {
                *who == peer && *at < i && !run[at + 1..i].iter().any(|e| answers(peer, e))
            })
    });
    closing.then_some(start)
}

impl Store {
    pub fn task_record(
        &self,
        channel: AgentId,
        run: RunId,
        limit: usize,
    ) -> Result<Option<TaskRecord>, StoreError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let run_id = run.to_string();
        let recorded: Option<String> = tx.query_row(
            "SELECT coalesce(
                (SELECT m.to_agent FROM pending_runs p JOIN messages m ON m.id=p.message_id
                  WHERE p.run_id=?1),
                (SELECT agent_id FROM outcomes WHERE run_id=?1)
             )",
            [&run_id],
            |row| row.get(0),
        )?;
        let mut messages = tx.prepare(&format!(
            "SELECT {ENVELOPE} FROM messages m INDEXED BY messages_run
              WHERE m.run_id=?1 ORDER BY m.created_at, m.rowid LIMIT ?2"
        ))?;
        let belongs = match recorded {
            Some(id) => id == channel.to_string(),
            None => messages
                .query_row(params![run_id, 1], row_to_envelope)
                .optional()?
                .transpose()?
                .is_some_and(|start| {
                    start.to == Participant::Agent { id: channel }
                        && (start.from == Participant::Human
                            || matches!(
                                Trigger::of(&start),
                                Some((Trigger::Routine | Trigger::Event | Trigger::Decision, _))
                            ))
                }),
        };
        if !belongs {
            return Ok(None);
        }
        let result = tx
            .query_row(
                &format!(
                    "SELECT {ENVELOPE} FROM outcomes o JOIN messages m ON m.id=o.note_id
                  WHERE o.run_id=?1 AND m.run_id=o.run_id AND o.status IN ('done','needs_you')"
                ),
                [&run_id],
                row_to_envelope,
            )
            .optional()?
            .transpose()?;
        let result_id = result
            .filter(|message| {
                !message.parts.is_empty()
                    && message
                        .parts
                        .iter()
                        .all(|part| matches!(part, Part::Text { .. } | Part::File { .. }))
            })
            .map(|message| message.id);
        let mut found = Vec::new();
        for message in messages.query_map(params![run_id, limit + 1], row_to_envelope)? {
            if found.len() == limit {
                return Ok(Some(TaskRecord { messages: None, result_id }));
            }
            found.push(message??);
        }
        Ok(Some(TaskRecord { messages: Some(found), result_id }))
    }

    pub fn closing_start(
        &self,
        run: RunId,
        agent: AgentId,
        batch: &[MessageId],
    ) -> Result<Option<Envelope>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(&format!(
            "SELECT {ENVELOPE} FROM messages m WHERE m.run_id=?1 ORDER BY m.created_at, m.id"
        ))?;
        let mut messages = Vec::new();
        for row in stmt.query_map([run.to_string()], row_to_envelope)? {
            messages.push(row??);
        }
        Ok(closes(&messages, agent, batch).cloned())
    }

    pub fn outcomes(
        &self,
        since: Option<i64>,
        group: Option<GroupId>,
        unseen_only: bool,
        limit: u32,
    ) -> Result<Vec<Outcome>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(&format!(
            "SELECT {COLUMNS} FROM outcomes
              WHERE (?1 IS NULL OR finished_at >= ?1)
                AND (?2 IS NULL OR group_id = ?2)
                AND (?3 = 0 OR seen_at IS NULL)
              ORDER BY finished_at DESC, run_id DESC
              LIMIT ?4"
        ))?;
        let rows = stmt.query_map(
            params![since, group.map(|g| g.to_string()), unseen_only, limit.min(LIST_MAX)],
            read,
        )?;
        let mut outcomes = Vec::new();
        for row in rows {
            outcomes.push(with_colleagues(&conn, row??)?);
        }
        Ok(outcomes)
    }

    pub fn running(&self) -> Result<Vec<Running>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT run_id,started_at FROM pending_runs
              WHERE started_at IS NOT NULL ORDER BY started_at,run_id",
        )?;
        let started = stmt
            .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        let mut running = Vec::new();
        for (run, started_at) in started {
            let Some(start) = first(&conn, "1=1", "ASC", params![run])? else { continue };
            let Some(owner) = owned(&conn, &start)? else { continue };
            let note = conn
                .query_row(
                    "SELECT body FROM working_notes WHERE agent_id=?1 AND at>=?2
                      ORDER BY id DESC LIMIT 1",
                    params![owner.agent.to_string(), started_at],
                    |row| row.get(0),
                )
                .optional()?;
            running.push(Running {
                run_id: start.run_id,
                agent_id: owner.agent,
                group_id: owner.group,
                started_at,
                note,
            });
        }
        Ok(running)
    }

    pub fn reported(
        &self,
        agent: AgentId,
        group: GroupId,
        before: i64,
        limit: u32,
    ) -> Result<Vec<Outcome>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(&format!(
            "SELECT {COLUMNS} FROM outcomes o
              WHERE o.agent_id=?1 AND o.group_id=?2 AND o.started_at<?3
                AND o.status<>'{}'
                AND EXISTS (SELECT 1 FROM messages m
                             WHERE m.run_id=o.run_id AND m.channel_id=o.agent_id)
              ORDER BY o.finished_at DESC, o.run_id DESC
              LIMIT ?4",
            Status::NothingNew.as_str()
        ))?;
        let rows = stmt.query_map(
            params![agent.to_string(), group.to_string(), before, limit.min(LIST_MAX)],
            read,
        )?;
        let mut outcomes = Vec::new();
        for row in rows {
            outcomes.push(with_colleagues(&conn, row??)?);
        }
        Ok(outcomes)
    }

    pub fn outcome(&self, run: RunId) -> Result<Option<Outcome>, StoreError> {
        let conn = self.conn()?;
        let found = conn
            .query_row(
                &format!("SELECT {COLUMNS} FROM outcomes WHERE run_id=?1"),
                [run.to_string()],
                read,
            )
            .optional()?;
        found.transpose()?.map(|outcome| with_colleagues(&conn, outcome)).transpose()
    }

    pub fn mark_outcomes_seen(
        &self,
        runs: Option<&[RunId]>,
        before: Option<i64>,
        now: i64,
    ) -> Result<Vec<RunId>, StoreError> {
        let names = runs.map(|runs| {
            serde_json::to_string(&runs.iter().map(ToString::to_string).collect::<Vec<_>>())
                .expect("a list of ids serializes")
        });
        self.mark_seen_where(
            "(?1 IS NULL OR run_id IN (SELECT value FROM json_each(?1)))
              AND (?2 IS NULL OR finished_at <= ?2)",
            params![names, before],
            now,
        )
    }

    pub fn mark_channel_seen(&self, agent: AgentId, now: i64) -> Result<Vec<RunId>, StoreError> {
        self.mark_seen_where("agent_id=?1", params![agent.to_string()], now)
    }

    fn mark_seen_where(
        &self,
        filter: &str,
        args: &[&dyn rusqlite::ToSql],
        now: i64,
    ) -> Result<Vec<RunId>, StoreError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let marked: Vec<String> = {
            let mut stmt = tx.prepare(&format!(
                "SELECT run_id FROM outcomes WHERE seen_at IS NULL AND {filter} ORDER BY finished_at"
            ))?;
            let rows = stmt.query_map(args, |row| row.get(0))?;
            rows.collect::<Result<_, _>>()?
        };
        if marked.is_empty() {
            return Ok(Vec::new());
        }
        let marked_json = serde_json::to_string(&marked).expect("a list of ids serializes");
        tx.execute(
            "UPDATE outcomes SET seen_at=?2 WHERE seen_at IS NULL
                AND run_id IN (SELECT value FROM json_each(?1))",
            params![marked_json, now],
        )?;
        super::outbox::withdraw_runs(&tx, &marked_json)?;
        tx.commit()?;
        marked
            .iter()
            .map(|run| run.parse().map_err(|e| StoreError::Corrupt(format!("bad run id: {e}"))))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::agent::CleanDraft;
    use crate::domain::envelope::{Intent, Trust};

    fn hired(store: &Store, name: &str) -> (AgentId, GroupId) {
        let card = store
            .create_agent(&CleanDraft {
                group_id: None,
                name: name.into(),
                avatar: "avocado".into(),
                color: "#7fb069".into(),
                model: "test/model".into(),
                subscription_model: None,
                reasoning_effort: None,
                system_prompt: format!("You are the {name}."),
                skills: Vec::new(),
            })
            .unwrap();
        (card.id, card.group_id)
    }

    #[allow(clippy::too_many_arguments)]
    fn ran(
        store: &Store,
        agent: AgentId,
        group: GroupId,
        headline: &str,
        status: Status,
        started: i64,
        finished: i64,
        kept: bool,
    ) {
        let run = RunId::new();
        if kept {
            let mut start = say(Participant::Human, Participant::Agent { id: agent }, Intent::Work);
            start.run_id = run;
            start.channel_id = agent;
            start.created_at = started;
            store.append(&start).unwrap();
        }
        let outcome = Outcome {
            run_id: run,
            agent_id: agent,
            group_id: group,
            trigger: Trigger::Operator,
            trigger_ref: None,
            status,
            headline: headline.into(),
            note_id: None,
            colleagues: Vec::new(),
            needs: Vec::new(),
            steps: 1,
            started_at: started,
            finished_at: finished,
            seen_at: None,
        };
        assert!(insert(&store.conn().unwrap(), &outcome).unwrap());
    }

    #[test]
    fn an_agent_is_handed_the_ten_newest_lines_of_its_own_that_predate_its_window() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("guac.db")).unwrap();
        let (me, group) = hired(&store, "Manager");
        let (other, _) = hired(&store, "Chef");
        let window = 10_000;

        for i in 1..=12 {
            let at = i * 100;
            let status = if i == 12 { Status::NeedsYou } else { Status::Done };
            ran(&store, me, group, &format!("Line {i}."), status, at, at + 50, true);
        }
        for i in 1..=3 {
            let at = 2_000 + i * 100;
            ran(&store, me, group, NOTHING_NEW, Status::NothingNew, at, at + 50, true);
        }
        ran(&store, other, group, "Chef's line.", Status::Done, 3_000, 3_050, true);
        ran(&store, me, GroupId::new(), "In the old group.", Status::Done, 3_100, 3_150, true);
        ran(&store, me, group, "Cleared.", Status::Done, 3_200, 3_250, false);
        ran(&store, me, group, "Still on screen.", Status::Done, window, window + 50, true);
        ran(&store, me, group, "Newer still.", Status::Done, window + 100, window + 150, true);

        let lines: Vec<String> = store
            .reported(me, group, window, 10)
            .unwrap()
            .into_iter()
            .map(|outcome| outcome.headline)
            .collect();
        let expected: Vec<String> = (3..=12).rev().map(|i| format!("Line {i}.")).collect();
        assert_eq!(lines, expected);

        assert!(store.reported(me, group, 100, 10).unwrap().is_empty(), "nothing predates it");
    }

    fn say(from: Participant, to: Participant, intent: Intent) -> Envelope {
        Envelope {
            id: MessageId::new(),
            run_id: RunId::new(),
            channel_id: AgentId::new(),
            from,
            to,
            parts: vec![Part::text("words")],
            trust: Trust::Peer,
            hop: 0,
            expects_reply: intent.is_work(),
            intent,
            cause: None,
            created_at: 0,
        }
    }

    #[test]
    fn the_first_answer_to_the_last_ask_closes_a_delegation_and_a_thank_you_after_it_does_not() {
        let (a, b, c) = (AgentId::new(), AgentId::new(), AgentId::new());
        let (me, chef, bo) = (
            Participant::Agent { id: a },
            Participant::Agent { id: b },
            Participant::Agent { id: c },
        );
        let start = say(Participant::Human, me, Intent::Work);
        let to_chef = say(me, chef, Intent::Work);
        let to_bo = say(me, bo, Intent::Work);
        let from_chef = say(chef, me, Intent::Courtesy);
        let from_bo = say(bo, me, Intent::Courtesy);
        let thanks = say(chef, me, Intent::Courtesy);

        let mut run = vec![start.clone(), to_chef, to_bo, from_chef.clone()];
        assert!(closes(&run, a, &[from_chef.id]).is_none());

        run.push(from_bo.clone());
        assert_eq!(closes(&run, a, &[from_bo.id]).map(|e| e.id), Some(start.id));
        assert_eq!(closes(&run, a, &[from_chef.id, from_bo.id]).map(|e| e.id), Some(start.id));

        run.push(thanks.clone());
        assert!(closes(&run, a, &[thanks.id]).is_none());

        assert!(closes(&run, b, &[from_bo.id]).is_none());
        let chat = vec![start, say(me, chef, Intent::Courtesy), thanks.clone()];
        assert!(closes(&chat, a, &[thanks.id]).is_none());
    }
}
