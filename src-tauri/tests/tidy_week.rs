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

/// The page an agent has after months of work with no tidy: long, with a
/// block of progress that belongs in working notes, and several statements
/// the week's work will show to be wrong. About three thousand characters.
const LONG: &str = "# Memory\n\
\n\
Operator: Robert. Prefers short replies, with the answer in the first line.\n\
Robert works from New York and usually reads messages before 9am and after 5pm.\n\
Robert likes agendas laid out as a table with columns for time, topic and owner.\n\
\n\
## People\n\
- Priya runs finance. Budget questions go to her, and she wants numbers in a spreadsheet, not in prose.\n\
- Tom is the office manager. He books rooms and handles travel for the team.\n\
- Dana at the Lakeview Hotel is our venue contact for the offsite (dana@lakeview.example).\n\
- Lee joined in February and is hiring manager for the two engineering roles; Lee wants candidate notes as short bullet points.\n\
- Sam in design makes slides and wants a week's notice for anything over ten pages.\n\
\n\
## The team offsite\n\
- The offsite is on March 19, at the Lakeview Hotel.\n\
- Food budget for the offsite: $3,000, set by Priya.\n\
- Thirty people are expected. Two are vegetarian and one has a nut allergy.\n\
- Robert wants the morning for strategy and the afternoon free.\n\
\n\
## Current status\n\
- Waiting on quotes from three caterers (sent Monday, chasing Thursday).\n\
- Drafted the invitation; sending it once Robert confirms the date.\n\
- Asked Tom about a coach from the office; no answer yet.\n\
- Started a seating plan in the notebook; half done.\n\
- Need to remind Robert about the AV check on the 10th.\n\
- Waiting on Sam for the welcome slides, promised by Friday.\n\
- Booked a call with Dana for Wednesday at 2pm to walk the rooms.\n\
- Still to do: name badges, dietary list, parking passes.\n\
\n\
## How things are done here\n\
- Travel is booked through Tom, never directly: he holds the company card and everyone's loyalty numbers.\n\
- Expense claims go through Priya by the 25th of each month.\n\
- Anything sent outside the company is sent by Robert, never by me: I draft, he sends.\n\
- Calendar entries for team events go on the group calendar with the place in the title.\n\
- Robert reviews documents in the morning, so drafts are best ready by 8am.\n\
- The board meets quarterly. The board pack is due a week before each meeting.\n\
- Last quarter's board pack lived in the shared drive under Board/2026-Q4; reuse its structure.\n\
\n\
## Past work\n\
- Organised the January all-hands: forty people, the Riverside room, catering by Green Fork, which went well.\n\
- Wrote the December newsletter; Robert cut it to half its length, so keep newsletters under 400 words.\n\
- Prepared the Q4 board pack with Priya; the board asked for fewer slides and more numbers.\n\
- Helped Tom plan the October trip to Boston; the hotel was too far from the venue, so check distances next time.\n\
- Collected feedback after the all-hands; people wanted shorter talks and more time to talk to each other.\n\
- Ran the November customer dinner for twelve at Osteria Nove; book the back room, because the front room is too loud to talk in.\n\
- Built the onboarding checklist for new starters, kept in the notebook as onboarding.md; Tom updates the equipment part.\n\
- Drafted the hiring plan for two engineers; it is waiting on budget approval and nothing has moved since.\n";

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
    week(KEPT).await;
}

/// The same week, starting from a page that needs the tidy most: long, partly
/// stale, and holding progress that is not memory. What it is for is the page
/// coming out shorter and right, and the week's lines say what right is.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "live: costs money, needs a configured key"]
async fn live_a_week_of_overnight_tidies_from_a_long_stale_page() {
    week(LONG).await;
}

/// Runs the week from a starting page and prints what each night did to it.
async fn week(kept: &str) {
    let Some(config) = configured() else {
        eprintln!("no configured model; skipping");
        return;
    };
    let _ = warnings();
    println!("model: {}", config.inference.default_model);
    let h = live_crew(config, &[LiveAgent::skilled("Assistant", &["planning", "drafting"])]);
    let me = h.id("Assistant");
    let store = h.runtime.store();
    h.runtime.workspace().write(me, "Assistant", kept).unwrap();
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

    println!("--- the page the week starts from ({} chars) ---\n{kept}", kept.chars().count());
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
