//! What reaches a person, and why most of what happens does not.
//!
//! Guaca works while nobody is looking, so the question is never whether
//! something happened but whether it is worth an interruption. The answer used
//! to be worked out in each window, from events that are dropped when no
//! window listens: a reminder due while every window was closed was spent and
//! never shown, and two windows raised every banner twice. Now the Host keeps
//! one row per thing to deliver, written in the commit that caused it, and
//! [`decide`] is the one place that says what earns a banner.
//!
//! Start quiet. What needs the operator and what failed
//! interrupts them. Finished work interrupts only an operator who is away,
//! gathered over [`GATHER_MS`] into one item, and is otherwise only listed in
//! For you. A run the operator stopped, or a check that found nothing, is
//! never a row at all: nobody needs to be told about either.
//!
//! Pure: no clock and no store. The dispatcher in `runtime/outbox.rs` reads
//! the rows and the operator's presence and applies the verdict.

use serde::{Deserialize, Serialize};

use super::ids::{AgentId, ApprovalId, EscalationId, GroupId, RunId};
use super::outcome::{Outcome, Status as Ended};
use super::view::Presence;

/// How long finished work is gathered before it is offered as one item.
///
/// Long enough that a morning's routines, which tend to finish together,
/// become one banner rather than four; short enough that the banner still
/// describes the morning.
pub const GATHER_MS: i64 = 10 * 60 * 1000;

/// How long an offered item waits for a window to claim it. After that it is
/// news about yesterday, and For you already lists what it said.
pub const EXPIRE_MS: i64 = 24 * 60 * 60 * 1000;

/// The most lines a gathered item lists before it says how many more.
const GATHERED_LINES: usize = 5;

/// What a row is about. Snake case in SQLite, camel case over IPC.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    /// A run finished, and it is not a failure.
    Outcome,
    /// A turn is parked on the operator, or an agent cannot go on without them.
    NeedsYou,
    /// Decisions are waiting, at a briefing hour or before a deadline.
    Reminder,
    /// A run ended in a failed call, or a restart cut it off.
    Failure,
}

impl Kind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Kind::Outcome => "outcome",
            Kind::NeedsYou => "needs_you",
            Kind::Reminder => "reminder",
            Kind::Failure => "failure",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "outcome" => Kind::Outcome,
            "needs_you" => Kind::NeedsYou,
            "reminder" => Kind::Reminder,
            "failure" => Kind::Failure,
            _ => return None,
        })
    }
}

/// Where a row goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Destination {
    /// Every connected window is offered it, and the first to claim it shows it.
    Clients,
    /// The address in the Host's `delivery` setting, which is how a person is
    /// reached with every window closed. Written beside a window's row when
    /// that row is offered while the operator is away, and sent rather than
    /// offered: tried, and tried again, until the address takes it or a day
    /// has gone by. See `push.rs`.
    Endpoint,
}

impl Destination {
    pub const fn as_str(self) -> &'static str {
        match self {
            Destination::Clients => "clients",
            Destination::Endpoint => "endpoint",
        }
    }
}

/// Where a row stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Status {
    /// Not yet decided, held, gathering, or offered and not yet claimed.
    Pending,
    /// A window claimed it and showed it.
    Delivered,
    /// Decided against. `last_error` says why, in a word or two.
    Suppressed,
    /// Offered for a day and claimed by nobody.
    Expired,
    /// Could not be delivered. Nothing writes this for windows, which are
    /// offered a row rather than sent one; it is where an address that keeps
    /// refusing ends.
    Failed,
}

impl Status {
    pub const fn as_str(self) -> &'static str {
        match self {
            Status::Pending => "pending",
            Status::Delivered => "delivered",
            Status::Suppressed => "suppressed",
            Status::Expired => "expired",
            Status::Failed => "failed",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "pending" => Status::Pending,
            "delivered" => Status::Delivered,
            "suppressed" => Status::Suppressed,
            "expired" => Status::Expired,
            "failed" => Status::Failed,
            _ => return None,
        })
    }
}

/// What a click on the banner opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Target {
    /// The agent's channel, where the note, the notice or the card is.
    Channel { agent_id: AgentId },
    /// For you, where the decisions and the list of finished work are.
    ForYou,
}

/// The three things an operator can be interrupted for. One setting on the
/// Host for every window, where it used to be six switches per browser.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Attention {
    /// Permission requests, questions, an agent that is stuck, and the
    /// reminders about decisions.
    pub needs_you: bool,
    /// A run that failed or was cut off by a restart.
    pub failures: bool,
    /// Finished work, gathered into one item, while the operator is away.
    pub finished: bool,
}

impl Default for Attention {
    fn default() -> Self {
        Self { needs_you: true, failures: true, finished: true }
    }
}

/// One item as a window is offered it and asked to claim it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Delivery {
    pub id: String,
    pub kind: Kind,
    pub title: String,
    pub body: String,
    pub target: Target,
    pub created_at: i64,
}

/// A row a producer writes, in the commit of the thing it is about.
#[derive(Debug, Clone, PartialEq)]
pub struct Draft {
    pub kind: Kind,
    /// One row per key and destination: settling twice, raising an
    /// escalation twice in a day or claiming one reminder slot twice writes
    /// nothing the second time.
    pub dedup_key: String,
    pub group_id: Option<GroupId>,
    pub agent_id: Option<AgentId>,
    pub run_id: Option<RunId>,
    /// The approval or escalation it is about, so answering or clearing it
    /// withdraws what was waiting to announce it.
    pub ref_id: Option<String>,
    pub title: String,
    pub body: String,
    pub target: Target,
}

/// The row a finished run gives, or none.
///
/// Stopped is the operator's own act and nothing new is a check that found
/// nothing, so neither is news to anyone. Every other ending is written, and
/// [`decide`] says later whether it interrupts. The headline is the body: it is
/// the agent's own first line, and the one thing a banner about a run is for.
pub fn for_outcome(outcome: &Outcome, agent: &str) -> Option<Draft> {
    let (kind, title) = match outcome.status {
        Ended::Stopped | Ended::NothingNew => return None,
        Ended::Done | Ended::NeedsYou => (Kind::Outcome, agent.to_string()),
        Ended::Failed => (Kind::Failure, format!("{agent} could not finish")),
        Ended::Interrupted => (Kind::Failure, format!("{agent} was interrupted")),
    };
    Some(Draft {
        kind,
        dedup_key: format!("outcome:{}", outcome.run_id),
        group_id: Some(outcome.group_id),
        agent_id: Some(outcome.agent_id),
        run_id: Some(outcome.run_id),
        ref_id: None,
        title,
        body: outcome.headline.clone(),
        target: Target::Channel { agent_id: outcome.agent_id },
    })
}

/// The row a parked turn gives.
///
/// Nothing the agent wrote is on it: the request is the agent's words, and a
/// banner is read on a lock screen. A question is not called a permission,
/// which is what the window used to say for both.
pub fn for_request(
    approval: ApprovalId,
    question: bool,
    agent: AgentId,
    group: GroupId,
    run: RunId,
    name: &str,
) -> Draft {
    let title = if question {
        format!("{name} has a question")
    } else {
        format!("{name} needs your permission")
    };
    Draft {
        kind: Kind::NeedsYou,
        dedup_key: format!("approval:{approval}"),
        group_id: Some(group),
        agent_id: Some(agent),
        run_id: Some(run),
        ref_id: Some(approval.to_string()),
        title,
        body: "Its turn is parked until you answer, and gives up after ten minutes.".into(),
        target: Target::Channel { agent_id: agent },
    }
}

/// The row an escalation gives on `day`, the operator's local date.
///
/// Keyed by day, so a wall an agent walks into six times this afternoon is
/// one interruption, and the same wall still standing tomorrow is one more.
pub fn for_escalation(
    escalation: EscalationId,
    agent: AgentId,
    group: GroupId,
    run: RunId,
    name: &str,
    day: &str,
) -> Draft {
    Draft {
        kind: Kind::NeedsYou,
        dedup_key: format!("escalation:{escalation}:{day}"),
        group_id: Some(group),
        agent_id: Some(agent),
        run_id: Some(run),
        ref_id: Some(escalation.to_string()),
        title: format!("{name} is stuck"),
        body: "It cannot go on without you. In For you until you clear it.".into(),
        target: Target::Channel { agent_id: agent },
    }
}

/// The row a claimed batch of decision reminders gives. `slot` is the
/// earliest reminder time in the batch, which no later claim can share,
/// because a claim moves every reminder it takes past now.
pub fn for_reminder(count: usize, slot: i64) -> Draft {
    let needs = if count == 1 { "decision needs" } else { "decisions need" };
    Draft {
        kind: Kind::Reminder,
        dedup_key: format!("reminder:{slot}"),
        group_id: None,
        agent_id: None,
        run_id: None,
        ref_id: None,
        title: "For you".into(),
        body: format!("{count} {needs} a review. Open For you to answer or follow up."),
        target: Target::ForYou,
    }
}

/// Where a row is in its life when the dispatcher looks at it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Look {
    /// Written since the last pass.
    New,
    /// Held for the operator to be away, and the time has come.
    Held,
    /// Gathered with others, and the gathering is over.
    Closing,
}

/// Why a row was not delivered, as `last_error` keeps it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Why {
    /// The operator turned this kind off.
    Setting,
    /// The channel it is about has the operator's focus.
    OnScreen,
    /// The operator was here when it finished, and was still here later.
    Present,
    /// The operator saw it before it was offered.
    Seen,
    /// What it asked about was answered, cleared or closed first.
    Answered,
    /// An ending that never interrupts anyone.
    Never,
}

impl Why {
    pub const fn as_str(self) -> &'static str {
        match self {
            Why::Setting => "turned off",
            Why::OnScreen => "on screen",
            Why::Present => "operator present",
            Why::Seen => "seen",
            Why::Answered => "answered",
            Why::Never => "never delivered",
        }
    }
}

/// What to do with one row now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Offer it to every window now.
    Deliver,
    /// Look again at `until`, when the operator will count as away if nothing
    /// changes.
    Hold { until: i64 },
    /// Gather it with other finished work, and offer them as one.
    Gather,
    /// Never offer it.
    Suppress(Why),
}

/// The one rule about interrupting. The table in docs/ATTENTION.md is this
/// function in words, and the tests below are the table.
///
/// `agent` is the agent a row is about, which is what "on screen" is measured
/// against, and `ended` is how the run ended for a row about a run.
pub fn decide(
    kind: Kind,
    agent: Option<AgentId>,
    ended: Option<Ended>,
    look: Look,
    attention: &Attention,
    presence: &Presence,
) -> Verdict {
    let on_screen = agent.is_some() && presence.on_screen() == agent;
    match kind {
        Kind::Reminder if !attention.needs_you => return Verdict::Suppress(Why::Setting),
        Kind::Reminder => return Verdict::Deliver,
        Kind::NeedsYou if !attention.needs_you => return Verdict::Suppress(Why::Setting),
        Kind::Failure if !attention.failures => return Verdict::Suppress(Why::Setting),
        Kind::NeedsYou | Kind::Failure if on_screen => return Verdict::Suppress(Why::OnScreen),
        Kind::NeedsYou | Kind::Failure => return Verdict::Deliver,
        Kind::Outcome => {}
    }
    if !matches!(ended, Some(Ended::Done | Ended::NeedsYou)) {
        return Verdict::Suppress(Why::Never);
    }
    if !attention.finished {
        return Verdict::Suppress(Why::Setting);
    }
    if on_screen {
        return Verdict::Suppress(Why::OnScreen);
    }
    // Finished while the operator was here: they may have left without
    // looking, so it is looked at again once they would count as away. If
    // they are still here then, it waits in For you and interrupts nobody.
    match (look, presence.away_at()) {
        (Look::New, Some(until)) => Verdict::Hold { until },
        (Look::Held | Look::Closing, Some(_)) => Verdict::Suppress(Why::Present),
        (Look::New | Look::Held, None) => Verdict::Gather,
        (Look::Closing, None) => Verdict::Deliver,
    }
}

/// One gathered row, as the item that carries them all reads it.
#[derive(Debug, Clone, PartialEq)]
pub struct Gathered {
    pub title: String,
    pub body: String,
    pub target: Target,
}

/// One item made of every gathered row, oldest first. A gathering of one is
/// that row unchanged: "4 finished" is for four.
pub fn gather(rows: &[Gathered]) -> Option<Gathered> {
    match rows {
        [] => None,
        [one] => Some(one.clone()),
        many => {
            let mut lines: Vec<String> = many
                .iter()
                .take(GATHERED_LINES)
                .map(|row| format!("{}: {}", row.title, row.body))
                .collect();
            if many.len() > GATHERED_LINES {
                lines.push(format!("And {} more in For you.", many.len() - GATHERED_LINES));
            }
            Some(Gathered {
                title: format!("{} finished", many.len()),
                body: lines.join("\n"),
                target: Target::ForYou,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::outcome::Trigger;

    const NOW: i64 = 1_800_000_000_000;
    const AWAY_AT: i64 = NOW + 60_000;

    fn away() -> Presence {
        Presence::Away
    }

    fn here(on_screen: Option<AgentId>) -> Presence {
        Presence::Here { on_screen, away_at: AWAY_AT }
    }

    fn about(
        agent: AgentId,
        kind: Kind,
        ended: Option<Ended>,
        look: Look,
        p: &Presence,
    ) -> Verdict {
        decide(kind, Some(agent), ended, look, &Attention::default(), p)
    }

    #[test]
    fn what_needs_the_operator_interrupts_them_unless_they_are_looking_at_it() {
        let agent = AgentId::new();
        for (kind, ended) in [
            (Kind::NeedsYou, None),
            (Kind::Failure, Some(Ended::Failed)),
            (Kind::Failure, Some(Ended::Interrupted)),
        ] {
            let verdict = |p: &Presence| about(agent, kind, ended, Look::New, p);
            assert_eq!(verdict(&away()), Verdict::Deliver, "{kind:?} away");
            assert_eq!(verdict(&here(None)), Verdict::Deliver, "{kind:?} here");
            assert_eq!(
                verdict(&here(Some(AgentId::new()))),
                Verdict::Deliver,
                "{kind:?} with another channel on screen"
            );
            assert_eq!(
                verdict(&here(Some(agent))),
                Verdict::Suppress(Why::OnScreen),
                "{kind:?} with its own channel on screen"
            );
        }
    }

    #[test]
    fn a_reminder_is_about_no_channel_and_always_reaches_them() {
        for presence in [away(), here(None), here(Some(AgentId::new()))] {
            let verdict =
                decide(Kind::Reminder, None, None, Look::New, &Attention::default(), &presence);
            assert_eq!(verdict, Verdict::Deliver, "{presence:?}");
        }
    }

    #[test]
    fn finished_work_is_gathered_while_away_and_held_while_here() {
        let agent = AgentId::new();
        for ended in [Ended::Done, Ended::NeedsYou] {
            let at = |look: Look, p: &Presence| about(agent, Kind::Outcome, Some(ended), look, p);

            assert_eq!(at(Look::New, &away()), Verdict::Gather);
            assert_eq!(at(Look::New, &here(None)), Verdict::Hold { until: AWAY_AT });
            assert_eq!(at(Look::New, &here(Some(agent))), Verdict::Suppress(Why::OnScreen));

            // Held while the operator was here: it goes out only if they left.
            assert_eq!(at(Look::Held, &away()), Verdict::Gather);
            assert_eq!(at(Look::Held, &here(None)), Verdict::Suppress(Why::Present));

            // A gathering closes: offered only to an operator who is still away.
            assert_eq!(at(Look::Closing, &away()), Verdict::Deliver);
            assert_eq!(at(Look::Closing, &here(None)), Verdict::Suppress(Why::Present));
            assert_eq!(at(Look::Closing, &here(Some(agent))), Verdict::Suppress(Why::OnScreen));
        }
    }

    #[test]
    fn a_stop_or_a_check_that_found_nothing_is_never_news() {
        let agent = AgentId::new();
        for ended in [Ended::Stopped, Ended::NothingNew] {
            for look in [Look::New, Look::Held, Look::Closing] {
                assert_eq!(
                    about(agent, Kind::Outcome, Some(ended), look, &away()),
                    Verdict::Suppress(Why::Never)
                );
            }
            assert_eq!(for_outcome(&outcome(ended), "Digest"), None, "{ended:?} writes no row");
        }
    }

    #[test]
    fn each_switch_silences_its_own_kind_and_no_other() {
        let agent = AgentId::new();
        let verdict = |attention: Attention, kind: Kind, ended: Option<Ended>| {
            decide(kind, Some(agent), ended, Look::Closing, &attention, &away())
        };
        let no_needs = Attention { needs_you: false, ..Default::default() };
        let no_failures = Attention { failures: false, ..Default::default() };
        let no_finished = Attention { finished: false, ..Default::default() };
        let failed = Some(Ended::Failed);
        let done = Some(Ended::Done);

        assert_eq!(verdict(no_needs, Kind::NeedsYou, None), Verdict::Suppress(Why::Setting));
        assert_eq!(verdict(no_needs, Kind::Reminder, None), Verdict::Suppress(Why::Setting));
        assert_eq!(verdict(no_needs, Kind::Failure, failed), Verdict::Deliver);
        assert_eq!(verdict(no_needs, Kind::Outcome, done), Verdict::Deliver);

        assert_eq!(verdict(no_failures, Kind::Failure, failed), Verdict::Suppress(Why::Setting));
        assert_eq!(verdict(no_failures, Kind::NeedsYou, None), Verdict::Deliver);
        assert_eq!(verdict(no_failures, Kind::Outcome, done), Verdict::Deliver);

        assert_eq!(verdict(no_finished, Kind::Outcome, done), Verdict::Suppress(Why::Setting));
        assert_eq!(verdict(no_finished, Kind::Failure, failed), Verdict::Deliver);
        assert_eq!(verdict(no_finished, Kind::Reminder, None), Verdict::Deliver);
    }

    #[test]
    fn the_defaults_start_quiet() {
        // Every switch on means what needs you, failures, and finished
        // work only while away: being here is what keeps finished work quiet.
        let attention = Attention::default();
        assert!(attention.needs_you && attention.failures && attention.finished);
        let parsed: Attention = serde_json::from_str("{}").unwrap();
        assert_eq!(parsed, attention, "a settings file older than these gets the defaults");
        let wire = serde_json::to_value(attention).unwrap();
        assert_eq!(wire["needsYou"], true);
    }

    fn outcome(status: Ended) -> Outcome {
        Outcome {
            run_id: RunId::new(),
            agent_id: AgentId::new(),
            group_id: GroupId::new(),
            trigger: Trigger::Routine,
            trigger_ref: None,
            status,
            headline: "Sent the digest to 12 people.".into(),
            note_id: None,
            needs: Vec::new(),
            steps: 3,
            started_at: NOW - 1_000,
            finished_at: NOW,
            seen_at: None,
        }
    }

    #[test]
    fn a_finished_run_says_who_and_its_own_first_line() {
        let done = outcome(Ended::Done);
        let draft = for_outcome(&done, "Digest").unwrap();
        assert_eq!(draft.kind, Kind::Outcome);
        assert_eq!(draft.title, "Digest");
        assert_eq!(draft.body, "Sent the digest to 12 people.");
        assert_eq!(draft.dedup_key, format!("outcome:{}", done.run_id));
        assert_eq!(draft.run_id, Some(done.run_id));
        assert_eq!(draft.target, Target::Channel { agent_id: done.agent_id });

        let needs = for_outcome(&outcome(Ended::NeedsYou), "Digest").unwrap();
        assert_eq!(needs.kind, Kind::Outcome, "finished, even though it left something open");

        let failed = for_outcome(&outcome(Ended::Failed), "Digest").unwrap();
        assert_eq!(
            (failed.kind, failed.title.as_str()),
            (Kind::Failure, "Digest could not finish")
        );
        let cut = for_outcome(&outcome(Ended::Interrupted), "Digest").unwrap();
        assert_eq!((cut.kind, cut.title.as_str()), (Kind::Failure, "Digest was interrupted"));
    }

    #[test]
    fn a_question_is_not_called_a_permission() {
        let (id, agent, group, run) =
            (ApprovalId::new(), AgentId::new(), GroupId::new(), RunId::new());
        let permission = for_request(id, false, agent, group, run, "Pip");
        let question = for_request(id, true, agent, group, run, "Pip");
        assert_eq!(permission.title, "Pip needs your permission");
        assert_eq!(question.title, "Pip has a question");
        assert_eq!(question.ref_id, Some(id.to_string()));
        assert_eq!(question.dedup_key, format!("approval:{id}"));
    }

    #[test]
    fn an_escalation_interrupts_once_a_day() {
        let (id, agent, group, run) =
            (EscalationId::new(), AgentId::new(), GroupId::new(), RunId::new());
        let monday = for_escalation(id, agent, group, run, "Pip", "2026-11-16");
        let again = for_escalation(id, agent, group, run, "Pip", "2026-11-16");
        let tuesday = for_escalation(id, agent, group, run, "Pip", "2026-11-17");
        assert_eq!(monday.dedup_key, again.dedup_key);
        assert_ne!(monday.dedup_key, tuesday.dedup_key);
        assert_eq!(monday.title, "Pip is stuck");
    }

    #[test]
    fn a_reminder_says_how_many_and_nothing_anyone_wrote() {
        assert_eq!(
            for_reminder(1, NOW).body,
            "1 decision needs a review. Open For you to answer or follow up."
        );
        assert_eq!(
            for_reminder(3, NOW).body,
            "3 decisions need a review. Open For you to answer or follow up."
        );
        assert_eq!(for_reminder(3, NOW).target, Target::ForYou);
        assert_eq!(for_reminder(3, NOW).dedup_key, format!("reminder:{NOW}"));
    }

    #[test]
    fn several_finished_runs_are_one_item_and_one_is_itself() {
        let row = |name: &str, line: &str| Gathered {
            title: name.into(),
            body: line.into(),
            target: Target::Channel { agent_id: AgentId::new() },
        };
        let one = row("Digest", "Sent the digest to 12 people.");
        assert_eq!(gather(std::slice::from_ref(&one)), Some(one.clone()));
        assert_eq!(gather(&[]), None);

        let three = [one, row("Triage", "2 replies drafted."), row("Nightly", "All green.")];
        let item = gather(&three).unwrap();
        assert_eq!(item.title, "3 finished");
        assert_eq!(
            item.body,
            "Digest: Sent the digest to 12 people.\nTriage: 2 replies drafted.\nNightly: All green."
        );
        assert_eq!(item.target, Target::ForYou);

        let many: Vec<Gathered> = (0..7).map(|n| row(&format!("A{n}"), "Done.")).collect();
        let item = gather(&many).unwrap();
        assert_eq!(item.title, "7 finished");
        assert_eq!(item.body.lines().count(), 6);
        assert_eq!(item.body.lines().last(), Some("And 2 more in For you."));
    }

    #[test]
    fn kinds_and_states_are_spelled_one_way_in_sqlite_and_another_on_the_wire() {
        for kind in [Kind::Outcome, Kind::NeedsYou, Kind::Reminder, Kind::Failure] {
            assert_eq!(Kind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(Kind::NeedsYou.as_str(), "needs_you");
        assert_eq!(serde_json::to_value(Kind::NeedsYou).unwrap(), "needsYou");
        for status in [
            Status::Pending,
            Status::Delivered,
            Status::Suppressed,
            Status::Expired,
            Status::Failed,
        ] {
            assert_eq!(Status::parse(status.as_str()), Some(status));
        }
        let target = serde_json::to_value(Target::Channel { agent_id: AgentId::new() }).unwrap();
        assert_eq!(target["kind"], "channel");
        assert!(target.get("agentId").is_some());
        assert_eq!(serde_json::to_value(Target::ForYou).unwrap()["kind"], "forYou");
    }

    #[test]
    fn nothing_a_person_reads_has_an_em_dash() {
        let (agent, group, run) = (AgentId::new(), GroupId::new(), RunId::new());
        let mut said = vec![
            for_reminder(2, NOW).body,
            for_request(ApprovalId::new(), true, agent, group, run, "P").body,
            for_escalation(EscalationId::new(), agent, group, run, "P", "d").body,
        ];
        for ended in [Ended::Done, Ended::Failed, Ended::Interrupted] {
            said.extend(for_outcome(&outcome(ended), "P").map(|draft| draft.title));
        }
        for line in said {
            assert!(!line.contains('\u{2014}'), "{line}");
        }
    }
}
