use guac_lib::db::Store;
use guac_lib::domain::agent::CleanDraft;
use guac_lib::domain::envelope::{Envelope, Intent, Part, Participant, Trust};
use guac_lib::domain::ids::{MessageId, RunId};
use guac_lib::domain::routine::{Cadence, Routine, RunKind, Trigger};

fn fixture() -> (tempfile::TempDir, Store, Routine, Envelope) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("guac.db")).unwrap();
    let agent = store
        .create_agent(&CleanDraft {
            group_id: None,
            name: "Scout".into(),
            avatar: "orb".into(),
            color: "#7fb069".into(),
            model: "test".into(),
            subscription_model: None,
            reasoning_effort: None,
            system_prompt: "test".into(),
            skills: vec![],
        })
        .unwrap();
    let routine = store
        .create_routine(
            agent.id,
            "Check",
            "Check the inventory",
            Trigger::Clock(Cadence::Daily),
            Some(1000),
            false,
        )
        .unwrap();
    let envelope = Envelope {
        id: MessageId::new(),
        run_id: RunId::new(),
        channel_id: agent.id,
        from: Participant::System,
        to: Participant::Agent { id: agent.id },
        parts: vec![Part::Routine {
            routine_id: routine.id,
            name: routine.name.clone(),
            what: routine.what.clone(),
            payload: None,
        }],
        trust: Trust::Operator,
        hop: 0,
        expects_reply: true,
        intent: Intent::Work,
        cause: None,
        created_at: 1000,
    };
    (dir, store, routine, envelope)
}

#[test]
fn transaction_failures_never_advance_a_slot_or_leave_partial_work() {
    for table in ["routine_occurrences", "messages", "pending_runs", "routine_runs"] {
        let (_dir, store, routine, envelope) = fixture();
        store.conn().unwrap().execute_batch(&format!("CREATE TRIGGER inject BEFORE INSERT ON {table} BEGIN SELECT RAISE(ABORT, 'injected'); END;")).unwrap();
        assert!(store
            .claim_routine(
                &routine,
                "scheduled:1000",
                Some(&envelope),
                RunKind::Scheduled,
                None,
                1000,
                true
            )
            .is_err());
        assert_eq!(store.get_routine(routine.id).unwrap().unwrap().next_run_at, Some(1000));
        assert!(store.routine_runs(routine.id, 20).unwrap().is_empty());
        assert!(store.get_message(envelope.id).unwrap().is_none());
        let count: i64 = store
            .conn()
            .unwrap()
            .query_row("SELECT count(*) FROM routine_occurrences", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0, "failure at {table}");
    }
    let (_dir, store, routine, envelope) = fixture();
    store.conn().unwrap().execute_batch("CREATE TRIGGER inject BEFORE UPDATE ON routines BEGIN SELECT RAISE(ABORT, 'injected'); END;").unwrap();
    assert!(store
        .claim_routine(
            &routine,
            "scheduled:1000",
            Some(&envelope),
            RunKind::Scheduled,
            None,
            1000,
            true
        )
        .is_err());
    assert!(store.get_message(envelope.id).unwrap().is_none());
    assert!(store.routine_runs(routine.id, 20).unwrap().is_empty());
}

#[test]
fn unstarted_claim_survives_repeated_crashes_and_is_not_claimed_twice() {
    let (dir, store, routine, envelope) = fixture();
    assert!(store
        .claim_routine(
            &routine,
            "scheduled:1000",
            Some(&envelope),
            RunKind::Scheduled,
            None,
            1000,
            true
        )
        .unwrap());
    assert!(!store
        .claim_routine(
            &routine,
            "scheduled:1000",
            Some(&envelope),
            RunKind::Scheduled,
            None,
            1000,
            true
        )
        .unwrap());
    drop(store);
    for _ in 0..2 {
        let store = Store::open(&dir.path().join("guac.db")).unwrap();
        let recovered = store.recover().unwrap();
        assert_eq!(recovered.resumed.len(), 1);
        assert_eq!(recovered.resumed[0].envelope, envelope);
        assert_eq!(recovered.interrupted, 0);
    }
}

#[test]
fn started_absorbed_firing_is_reported_interrupted_instead_of_replayed() {
    let (_dir, store, routine, envelope) = fixture();
    store
        .claim_routine(
            &routine,
            "scheduled:1000",
            Some(&envelope),
            RunKind::Scheduled,
            None,
            1000,
            true,
        )
        .unwrap();
    store.start_routine_deliveries(&[envelope.id], RunId::new()).unwrap();
    store.settle_run(envelope.run_id).unwrap();
    let recovered = store.recover().unwrap();
    assert!(recovered.resumed.is_empty());
    assert_eq!(recovered.interrupted, 1);
    assert_eq!(store.recover().unwrap().interrupted, 0);
}

#[test]
fn a_stopped_queued_firing_never_returns_after_restart() {
    let (_dir, store, routine, envelope) = fixture();
    store
        .claim_routine(
            &routine,
            "scheduled:1000",
            Some(&envelope),
            RunKind::Scheduled,
            None,
            1000,
            true,
        )
        .unwrap();
    store.stop_routine_run(envelope.run_id).unwrap();
    let recovered = store.recover().unwrap();
    assert!(recovered.resumed.is_empty());
    assert_eq!(recovered.interrupted, 0);
}

#[test]
fn completed_firing_does_not_become_a_false_interruption() {
    let (_dir, store, routine, envelope) = fixture();
    store
        .claim_routine(
            &routine,
            "scheduled:1000",
            Some(&envelope),
            RunKind::Scheduled,
            None,
            1000,
            true,
        )
        .unwrap();
    store.start_routine_deliveries(&[envelope.id], envelope.run_id).unwrap();
    store.settle_run(envelope.run_id).unwrap();
    let recovered = store.recover().unwrap();
    assert!(recovered.resumed.is_empty());
    assert_eq!(recovered.interrupted, 0);
}

#[test]
fn one_shot_completion_keeps_its_history_after_the_schedule_is_removed() {
    let (_dir, store, routine, envelope) = fixture();
    let once = store
        .create_routine(
            routine.agent_id,
            "Once",
            "Do it once",
            Trigger::Clock(Cadence::Once),
            Some(1000),
            false,
        )
        .unwrap();
    store
        .claim_routine(
            &once,
            "scheduled:1000",
            Some(&envelope),
            RunKind::Scheduled,
            None,
            1000,
            true,
        )
        .unwrap();
    assert!(store.get_routine(once.id).unwrap().is_none());
    assert_eq!(store.routine_runs(once.id, 20).unwrap().len(), 1);
}
