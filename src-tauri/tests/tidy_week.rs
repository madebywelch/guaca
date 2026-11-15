//! A week of one agent's work with the overnight tidy on, against a real model.
//!
//! The cascade suite holds what the runtime does around a tidy. What it cannot
//! hold is whether a real model, handed a night's lines and notes, leaves the
//! page better: corrects what the week showed to be wrong, drops what stopped
//! being true, and keeps the page short. That is a model's behavior, so it is
//! printed rather than decided, night by night, with the page before and after.
//!
//! One live test, ignored, and kept out of `scripts/evals.sh`, which asks a
//! different question. It costs money:
//!
//!   cargo test --manifest-path src-tauri/Cargo.toml --test tidy_week -- --ignored --nocapture
//!
//! The days are compressed. Each day's work is real turns against the model,
//! then filed as having finished on that day, and each night is the clock's
//! sweep at three the next morning. Nothing else is moved, so the lines a tidy
//! reads are the ones the runtime would have handed it.

mod harness;

use std::sync::Arc;

use guac_lib::domain::envelope::{Envelope, Part};
use guac_lib::domain::group::{CleanGroup, Tidy};
use guac_lib::domain::now_ms;

use harness::live::{configured, live_crew, LiveAgent};

const HOUR: i64 = 3_600_000;
const DAY: i64 = 24 * HOUR;

/// What the agent already keeps when the week starts: a page written the way
/// pages get written, with one line of progress in it that belongs elsewhere.
const KEPT: &str = "Operator: Robert. Prefers short replies.\n\
                    Currently working on: offsite planning, waiting on venue options.\n\
                    Offsite date: not set yet.";

/// Seven days of what the operator asks for. Some of it is corrected later in
/// the week, and some of it stops being true.
const WEEK: [&[&str]; 7] = [
    &[
        "Our team offsite is on March 12 at the Lakeview Hotel. The venue contact is Dana \
         (dana@lakeview.example). Please remember that.",
        "Draft a two-line welcome note for the offsite.",
    ],
    &["The caterer is Green Fork, and the food budget is $4,000.", "What's the venue again?"],
    &["Change of plan: the offsite moved to the Harbor Inn, same date. Dana is not our contact \
       any more; it's Marco (marco@harborinn.example)."],
    &[
        "Write a short agenda for the offsite morning.",
        "From now on, I want every agenda as a bulleted list with times.",
    ],
    &["The food budget went up to $5,500.", "Who is our venue contact?"],
    &["The offsite is done, thanks. Next up is the Q2 board pack, due April 3."],
    &["Start an outline for the board pack."],
];

/// What this binary logged at warn and above.
fn warnings() -> String {
    struct Into(Arc<std::sync::Mutex<Vec<u8>>>);
    impl std::io::Write for Into {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    static LOG: std::sync::OnceLock<Arc<std::sync::Mutex<Vec<u8>>>> = std::sync::OnceLock::new();
    let log = LOG.get_or_init(|| {
        let log = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = log.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::WARN)
            .with_ansi(false)
            .with_writer(move || Into(sink.clone()))
            .finish();
        tracing::subscriber::set_global_default(subscriber).expect("nothing else here logs");
        log
    });
    String::from_utf8_lossy(&log.lock().unwrap()).into_owned()
}

/// Model calls, prompt tokens, of them cached, and completion tokens.
fn usage(db: &rusqlite::Connection, run: &str) -> (u32, u32, u32, u32) {
    db.query_row(
        "SELECT count(*), coalesce(sum(prompt),0), coalesce(sum(cached),0), \
                coalesce(sum(completion),0) FROM usage WHERE run_id=?1",
        [run],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )
    .unwrap()
}

fn lines(page: &str) -> Vec<&str> {
    page.lines().map(str::trim).filter(|line| !line.is_empty()).collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "live: costs money, needs a configured key"]
async fn live_a_week_of_overnight_tidies() {
    let Some(config) = configured() else {
        eprintln!("no configured model; skipping");
        return;
    };
    let _ = warnings();
    println!("model: {}", config.inference.default_model);
    let h = live_crew(config, &[LiveAgent::skilled("Assistant", &["planning", "drafting"])]);
    let me = h.id("Assistant");
    let store = h.runtime.store();
    h.runtime.workspace().write(me, "Assistant", KEPT).unwrap();
    let card = store.get_agent(me).unwrap().unwrap();
    let group = store.get_group(card.group_id).unwrap().unwrap();
    store
        .update_group(
            group.id,
            &CleanGroup {
                name: group.name,
                tidy: Some(Tidy { on: true, hour: 3 }),
                ..Default::default()
            },
        )
        .unwrap();
    let db = rusqlite::Connection::open(h._dir.path().join("guac.db")).unwrap();
    db.busy_timeout(std::time::Duration::from_secs(10)).unwrap();

    let start = now_ms();
    let mut nights = Vec::new();
    for (day, said) in WEEK.iter().enumerate() {
        // The morning after this day, far enough from now that the whole day
        // falls after the switch went on.
        let night = Tidy::last_struck(3, start + (day as i64 + 2) * DAY).unwrap();
        let worked_at = night - 15 * HOUR;
        let began = now_ms();
        let mut runs = Vec::new();
        for (i, text) in said.iter().enumerate() {
            let run = h.runtime.send_from_human(me, text).unwrap();
            assert!(h.settled_within(run, 300).await, "day {} did not settle", day + 1);
            runs.push((run, worked_at + i as i64 * HOUR));
        }
        for (run, at) in &runs {
            db.execute(
                "UPDATE outcomes SET started_at=?2, finished_at=?2 WHERE run_id=?1",
                rusqlite::params![run.to_string(), at],
            )
            .unwrap();
        }
        db.execute(
            "UPDATE working_notes SET at=?2 WHERE agent_id=?1 AND at>=?3",
            rusqlite::params![me.to_string(), worked_at + HOUR / 2, began],
        )
        .unwrap();
        let normal = runs.last().map(|(run, _)| usage(&db, &run.to_string())).unwrap();

        let before = h.runtime.workspace().read(me);
        let handed =
            store.channel_messages(me, 1000).unwrap().into_iter().filter(Envelope::is_tidy).count();
        h.runtime.sweep_tidy(night + 60_000);
        let tidies: Vec<Envelope> = store
            .channel_messages(me, 1000)
            .unwrap()
            .into_iter()
            .filter(Envelope::is_tidy)
            .collect();
        assert_eq!(tidies.len(), handed + 1, "night {} handed no tidy", day + 1);
        let tidy = tidies.last().unwrap();
        assert!(h.settled_within(tidy.run_id, 300).await, "night {} did not settle", day + 1);
        assert_eq!(store.outcome(tidy.run_id).unwrap(), None);
        let after = h.runtime.workspace().read(me);
        let (reported, notes) = match &tidy.parts[0] {
            Part::Tidy { reported, notes } => (reported.len(), notes.len()),
            _ => unreachable!(),
        };
        let spent = usage(&db, &tidy.run_id.to_string());

        println!("\n=== night {} ({} lines, {} notes) ===", day + 1, reported, notes);
        for line in lines(&before).iter().filter(|line| !lines(&after).contains(line)) {
            println!("- {line}");
        }
        for line in lines(&after).iter().filter(|line| !lines(&before).contains(line)) {
            println!("+ {line}");
        }
        println!(
            "--- page after ({} chars, was {}) ---\n{after}",
            after.chars().count(),
            before.chars().count()
        );
        println!(
            "tidy: {} calls, {} prompt ({} cached), {} completion; the day's last turn: {} calls, \
             {} prompt ({} cached), {} completion",
            spent.0, spent.1, spent.2, spent.3, normal.0, normal.1, normal.2, normal.3
        );
        nights.push((day + 1, before.chars().count(), after.chars().count(), spent, normal));
    }

    let strays = warnings()
        .lines()
        .filter(|line| {
            line.contains("a tidy named a tool it is not offered") && line.contains("Assistant")
        })
        .map(str::to_string)
        .collect::<Vec<_>>();
    println!("\n=== the week ===");
    println!("| night | page before | page after | tidy calls | tidy prompt | cached | completion | day's last turn prompt | cached |");
    println!("|---|---|---|---|---|---|---|---|---|");
    for (night, before, after, tidy, normal) in &nights {
        println!(
            "| {night} | {before} | {after} | {} | {} | {} | {} | {} | {} |",
            tidy.0, tidy.1, tidy.2, tidy.3, normal.1, normal.2
        );
    }
    println!("tools named that a tidy was not offered: {}", strays.len());
    for stray in &strays {
        println!("  {stray}");
    }
    println!("--- the page at the end of the week ---\n{}", h.runtime.workspace().read(me));
}
