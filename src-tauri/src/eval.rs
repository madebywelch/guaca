//! Reading a run's traffic and saying whether it was reasonable.
//!
//! Every cascade bug this app has had looked fine one message at a time. An
//! agent thanking a peer is polite; two agents thanking each other for four
//! rounds while the operator watches five summaries arrive is a defect, and
//! nothing about any single envelope says so. The shape is the evidence, so
//! this reads the shape.
//!
//! Deliberately not a scoring model. Every fault here is a property that can be
//! decided from the envelopes, because an eval that needs a judgement call is
//! one nobody can act on when it fails.

use std::collections::{HashMap, HashSet};

use crate::domain::envelope::{Envelope, Participant};
use crate::domain::ids::AgentId;

/// What one run's traffic looked like.
#[derive(Debug, Clone)]
pub struct Conversation {
    /// What the operator was told, in order, and by whom. This is the product.
    ///
    /// By whom matters: every agent has its own channel, so one message each
    /// from three agents is three channels with one line in them, while three
    /// from one agent is the thing operators complain about.
    pub to_operator: Vec<(String, String)>,
    /// Agent-to-agent messages. This is the cost.
    pub between_agents: usize,
    pub max_hop: u16,
    /// Messages the guard turned away, by headline.
    pub refusals: Vec<String>,
    /// Rendered for a failing assertion to print. Nobody can act on a count.
    pub script: String,
}

impl Conversation {
    /// Peer messages per thing the operator was told.
    ///
    /// The number an operator would use to decide whether a crew is working or
    /// talking to itself.
    pub fn chatter(&self) -> f32 {
        if self.to_operator.is_empty() {
            return self.between_agents as f32;
        }
        self.between_agents as f32 / self.to_operator.len() as f32
    }
}

/// Something that went wrong in the way a crew communicated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fault {
    /// The operator asked for something and was never answered.
    Silent,
    /// An agent was given work and never said what came of it.
    ///
    /// The other direction from every fault below, and the one this suite was
    /// blind to. A cascade that stops too early leaves the messages it did send
    /// looking perfectly reasonable: the operator is told something by somebody,
    /// so `Silent` does not fire, and the agent that was actually handed the
    /// job simply never appears. Both times this shipped it looked to the
    /// operator like an agent that had stopped.
    ///
    /// Decided from `intent`, which is the only thing on the wire that says an
    /// agent was given something to do. A tool trail does not count as an
    /// answer: it is the working, and the operator reads channels, not traces.
    AssignedAndSaidNothing { agent: String },
    /// Two things said to the operator that are near enough the same words.
    RepeatedToOperator { again: String },
    /// One agent, answering one instruction over and over.
    ///
    /// Counted rather than compared, because paraphrases of one summary share
    /// almost no words: the three the operator saw had only the agents' names
    /// in common. Being told three times is the defect whether or not the
    /// wording differs.
    ///
    /// Per agent, because each has its own channel. Hearing once from the
    /// manager and once from the researcher is two channels with one message
    /// each, which is not the complaint.
    AnsweredMoreThanOnce { agent: String, times: usize },
    /// A message demanding an answer, sent to a peer that had already answered.
    /// The exact shape of a cascade that will not converge.
    DemandedAnswerFromSettledPeer { from: String, to: String },
    /// An acknowledgement of an acknowledgement.
    AcknowledgedAnAcknowledgement { from: String, to: String },
    /// A peer was written to more than twice in one run without asking for
    /// anything. Politeness with a rhythm.
    Nagged { from: String, to: String, times: usize },
}

impl Fault {
    pub fn explain(&self) -> String {
        match self {
            Fault::Silent => "the operator asked for something and was never told anything".into(),
            Fault::AssignedAndSaidNothing { agent } => {
                format!("{agent} was given work and never said what came of it")
            }
            Fault::RepeatedToOperator { again } => {
                format!("said the same thing to the operator twice: {again:?}")
            }
            Fault::AnsweredMoreThanOnce { agent, times } => {
                format!("{agent} answered one instruction {times} times")
            }
            Fault::DemandedAnswerFromSettledPeer { from, to } => format!(
                "{from} demanded an answer from {to}, which had already answered: this is what \
                 does not converge"
            ),
            Fault::AcknowledgedAnAcknowledgement { from, to } => {
                format!("{from} acknowledged {to}'s acknowledgement")
            }
            Fault::Nagged { from, to, times } => {
                format!("{from} wrote to {to} {times} times without asking for anything")
            }
        }
    }
}

/// Reads a run's envelopes into something that can be asserted on.
///
/// `name_of` resolves an agent id for the rendered script and the faults; the
/// analyser never looks anything up itself, so it can be run against a store,
/// a fixture, or a live session without knowing which.
pub fn analyse(messages: &[Envelope], name_of: &dyn Fn(AgentId) -> String) -> Conversation {
    let mut to_operator = Vec::new();
    let mut between_agents = 0usize;
    let mut max_hop = 0u16;
    let mut refusals = Vec::new();
    let mut script = String::new();

    for envelope in messages {
        max_hop = max_hop.max(envelope.hop);
        let text = envelope.plain_text();

        for part in &envelope.parts {
            if let crate::domain::envelope::Part::ToolCall { outcome, .. } = part {
                match outcome {
                    crate::domain::envelope::ToolOutcome::Refused { reason } => {
                        refusals.push(reason.clone())
                    }
                    crate::domain::envelope::ToolOutcome::Partial { refused, .. } => {
                        refusals.extend(refused.iter().map(|r| r.reason.clone()))
                    }
                    _ => {}
                }
            }
        }

        match (envelope.from, envelope.to) {
            (Participant::Human, Participant::Agent { id }) => {
                script.push_str(&format!("operator -> {}: {}\n", name_of(id), brief(&text)));
            }
            (Participant::Agent { id }, Participant::Human) => {
                if text.trim().is_empty() {
                    continue;
                }
                script.push_str(&format!("{} -> operator: {}\n", name_of(id), brief(&text)));
                to_operator.push((name_of(id), text));
            }
            (Participant::Agent { id: from }, Participant::Agent { id: to }) => {
                between_agents += 1;
                script.push_str(&format!(
                    "{} -> {} (hop {}{}): {}\n",
                    name_of(from),
                    name_of(to),
                    envelope.hop,
                    if envelope.expects_reply { ", wants an answer" } else { "" },
                    brief(&text)
                ));
            }
            _ => {}
        }
    }

    Conversation { to_operator, between_agents, max_hop, refusals, script }
}

/// Everything wrong with how a run communicated, worst first.
pub fn faults(messages: &[Envelope], name_of: &dyn Fn(AgentId) -> String) -> Vec<Fault> {
    let mut faults = Vec::new();
    let convo = analyse(messages, name_of);

    let operator_asked = messages
        .iter()
        .any(|e| matches!((e.from, e.to), (Participant::Human, Participant::Agent { .. })));
    if operator_asked && convo.to_operator.is_empty() {
        faults.push(Fault::Silent);
    }

    // Given work, and never heard from. Walked over the whole run rather than
    // per batch: an agent handed something to do has the rest of the run to
    // report on it, and reporting late is not the defect.
    //
    // Speaking means text somebody reads, to the operator or to a peer. A
    // `Part::ToolCall` trail is deliberately not enough. The turn that shipped
    // this bug did call a tool; what the operator saw was a channel with no
    // words in it and an agent that appeared to have stopped.
    let mut assigned: Vec<AgentId> = Vec::new();
    let mut spoke: HashSet<AgentId> = HashSet::new();
    for envelope in messages {
        if let Participant::Agent { id } = envelope.to {
            if envelope.intent.is_work() && !assigned.contains(&id) {
                assigned.push(id);
            }
        }
        if let Participant::Agent { id } = envelope.from {
            if !envelope.plain_text().trim().is_empty() {
                spoke.insert(id);
            }
        }
    }
    let mut mute: Vec<String> =
        assigned.into_iter().filter(|id| !spoke.contains(id)).map(&name_of).collect();
    mute.sort();
    faults.extend(mute.into_iter().map(|agent| Fault::AssignedAndSaidNothing { agent }));

    // One instruction, several answers. An update followed by a result is
    // reasonable; a third is the crew narrating itself.
    let instructions = messages
        .iter()
        .filter(|e| matches!((e.from, e.to), (Participant::Human, Participant::Agent { .. })))
        .count();
    // What counts as too many scales with how much work the run did. A single
    // errand deserves an answer and at most one progress note; a two-step
    // delegation reasonably reports after each round. Each round trip is two
    // hops, which is the only measure of depth available from the envelopes.
    let allowed = 2 + (convo.max_hop / 2) as usize;
    if instructions == 1 {
        let mut answers: HashMap<AgentId, usize> = HashMap::new();
        for envelope in messages {
            if let (Participant::Agent { id }, Participant::Human) = (envelope.from, envelope.to) {
                if !envelope.plain_text().trim().is_empty() {
                    *answers.entry(id).or_default() += 1;
                }
            }
        }
        let mut noisy: Vec<_> = answers.into_iter().filter(|(_, n)| *n > allowed).collect();
        noisy.sort_by_key(|(id, _)| name_of(*id));
        for (id, times) in noisy {
            faults.push(Fault::AnsweredMoreThanOnce { agent: name_of(id), times });
        }
    }

    // Said again in nearly the same words. Kept strict: paraphrase detection
    // needs a judgement call, and a fault nobody can act on is worse than none.
    let mut said: Vec<HashSet<String>> = Vec::new();
    for (_, line) in &convo.to_operator {
        let words = significant(line);
        if words.len() >= 4 && said.iter().any(|earlier| overlap(earlier, &words) >= 0.75) {
            faults.push(Fault::RepeatedToOperator { again: brief(line) });
        }
        said.push(words);
    }

    // Who has written to whom, and whether the last thing they said wanted an
    // answer. Walked in order, because these are questions about what had
    // already happened when each message was sent.
    let mut written: HashSet<(AgentId, AgentId)> = HashSet::new();
    let mut last_wanted: HashMap<(AgentId, AgentId), bool> = HashMap::new();
    let mut courtesies: HashMap<(AgentId, AgentId), usize> = HashMap::new();

    for envelope in messages {
        let (Participant::Agent { id: from }, Participant::Agent { id: to }) =
            (envelope.from, envelope.to)
        else {
            continue;
        };

        let they_answered = written.contains(&(to, from));
        if envelope.expects_reply && they_answered {
            faults.push(Fault::DemandedAnswerFromSettledPeer {
                from: name_of(from),
                to: name_of(to),
            });
        }

        if !envelope.expects_reply && last_wanted.get(&(to, from)) == Some(&false) {
            faults.push(Fault::AcknowledgedAnAcknowledgement {
                from: name_of(from),
                to: name_of(to),
            });
        }

        if !envelope.expects_reply {
            let seen = courtesies.entry((from, to)).or_insert(0);
            *seen += 1;
            if *seen == 3 {
                faults.push(Fault::Nagged { from: name_of(from), to: name_of(to), times: *seen });
            }
        }

        written.insert((from, to));
        last_wanted.insert((from, to), envelope.expects_reply);
    }

    faults
}

/// One line of a message, enough to recognise it by.
fn brief(text: &str) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= 80 {
        return flat;
    }
    flat.chars().take(77).collect::<String>() + "..."
}

/// The words that carry the meaning.
///
/// Two summaries of the same thing rarely share phrasing but almost always
/// share nouns, so the comparison is over words worth repeating.
fn significant(text: &str) -> HashSet<String> {
    const NOISE: &[&str] = &[
        "the", "and", "for", "with", "that", "this", "have", "has", "will", "our", "your", "you",
        "are", "was", "were", "been", "into", "them", "they", "their", "from", "what", "when",
        "who", "his", "her", "its", "all", "any", "can", "not", "but", "out", "now", "one", "two",
        "three", "here", "there", "about", "over", "each", "also", "just", "only", "than", "then",
        "them", "these", "those", "some", "more", "most", "much", "very", "well", "get", "got",
    ];
    let noise: HashSet<&str> = NOISE.iter().copied().collect();
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() > 2 && !noise.contains(w))
        .map(str::to_string)
        .collect()
}

/// How much two bags of words have in common, as a fraction of the smaller.
///
/// Against the smaller rather than the union, because a long message that
/// restates a short one is still a repeat.
fn overlap(a: &HashSet<String>, b: &HashSet<String>) -> f32 {
    let smaller = a.len().min(b.len());
    if smaller == 0 {
        return 0.0;
    }
    a.intersection(b).count() as f32 / smaller as f32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::envelope::Intent;
    use crate::domain::envelope::Part;
    use crate::domain::ids::{MessageId, RunId};

    fn agents() -> (AgentId, AgentId) {
        (AgentId::new(), AgentId::new())
    }

    fn named(a: AgentId, b: AgentId) -> impl Fn(AgentId) -> String {
        move |id| {
            if id == a {
                "Manager".to_string()
            } else if id == b {
                "Chef".to_string()
            } else {
                "someone".to_string()
            }
        }
    }

    fn msg(from: Participant, to: Participant, text: &str, hop: u16, wants: bool) -> Envelope {
        Envelope {
            id: MessageId::new(),
            run_id: RunId::new(),
            channel_id: AgentId::new(),
            from,
            to,
            parts: vec![Part::text(text)],
            trust: crate::domain::envelope::Trust::Peer,
            hop,
            expects_reply: wants,
            intent: Intent::Courtesy,
            cause: None,
            created_at: 0,
        }
    }

    /// `msg`, but carrying work. The one field that says an agent was given
    /// something to do rather than merely spoken to.
    fn assign(from: Participant, to: Participant, text: &str, hop: u16) -> Envelope {
        Envelope { intent: Intent::Work, ..msg(from, to, text, hop, false) }
    }

    #[test]
    fn an_agent_given_work_that_never_says_what_came_of_it_is_a_fault() {
        // The bug that shipped twice. An instruction arrives with nobody
        // waiting on the answer, the agent spends its turn and says nothing,
        // and the operator watches an agent that appears to have stopped.
        let (a, b) = agents();
        let run = [
            msg(Participant::Human, Participant::Agent { id: a }, "get the email sent", 0, true),
            assign(
                Participant::Agent { id: a },
                Participant::Agent { id: b },
                "send the invoice to the client",
                1,
            ),
            msg(Participant::Agent { id: a }, Participant::Human, "Chef is on it", 1, false),
        ];
        assert!(
            faults(&run, &named(a, b))
                .contains(&Fault::AssignedAndSaidNothing { agent: "Chef".into() }),
            "Chef was told to send an invoice and never reported back"
        );
    }

    #[test]
    fn the_run_level_silence_check_cannot_see_this() {
        // Why the fault has to exist at all. The operator was told something by
        // somebody, so `Silent` is satisfied while the agent that was actually
        // handed the job never appears.
        let (a, b) = agents();
        let run = [
            msg(Participant::Human, Participant::Agent { id: a }, "get the email sent", 0, true),
            assign(
                Participant::Agent { id: a },
                Participant::Agent { id: b },
                "send the invoice to the client",
                1,
            ),
            msg(Participant::Agent { id: a }, Participant::Human, "Chef is on it", 1, false),
        ];
        let found = faults(&run, &named(a, b));
        assert!(
            !found.contains(&Fault::Silent),
            "the operator was told something, so this is quiet"
        );
        assert!(found.iter().any(|f| matches!(f, Fault::AssignedAndSaidNothing { .. })));
    }

    #[test]
    fn an_agent_that_does_the_work_and_reports_it_is_clean() {
        let (a, b) = agents();
        let run = [
            msg(Participant::Human, Participant::Agent { id: a }, "get the email sent", 0, true),
            assign(
                Participant::Agent { id: a },
                Participant::Agent { id: b },
                "send the invoice to the client",
                1,
            ),
            msg(
                Participant::Agent { id: b },
                Participant::Human,
                "Sent it to the client",
                2,
                false,
            ),
            msg(Participant::Agent { id: a }, Participant::Human, "Chef sent it", 1, false),
        ];
        assert!(
            !faults(&run, &named(a, b))
                .iter()
                .any(|f| matches!(f, Fault::AssignedAndSaidNothing { .. })),
            "Chef said what it did, in its own channel, which is where the operator reads"
        );
    }

    #[test]
    fn a_courtesy_nobody_answers_is_not_work_left_undone() {
        // The asymmetry that terminates cascades depends on an agent being
        // allowed to read an acknowledgement and say nothing. Flagging that
        // would make the fault fire on every well-behaved broadcast.
        let (a, b) = agents();
        let run = [
            msg(Participant::Human, Participant::Agent { id: a }, "introduce yourself", 0, true),
            msg(Participant::Agent { id: a }, Participant::Agent { id: b }, "hello", 1, true),
            msg(Participant::Agent { id: b }, Participant::Agent { id: a }, "hi back", 2, false),
            msg(Participant::Agent { id: a }, Participant::Human, "done", 1, false),
        ];
        assert!(
            !faults(&run, &named(a, b))
                .iter()
                .any(|f| matches!(f, Fault::AssignedAndSaidNothing { .. })),
            "nobody was given work here"
        );
    }

    #[test]
    fn a_tool_trail_is_not_an_answer() {
        // The turn that shipped this bug did call a tool. What the operator saw
        // was a channel with no words in it.
        let (a, b) = agents();
        let trail = Envelope {
            parts: vec![Part::ToolCall {
                name: "run_command".into(),
                arguments: serde_json::Value::Null,
                outcome: crate::domain::envelope::ToolOutcome::Ok { summary: "ran it".into() },
            }],
            ..msg(Participant::Agent { id: b }, Participant::System, "", 2, false)
        };
        let run = [
            msg(Participant::Human, Participant::Agent { id: a }, "get it done", 0, true),
            assign(Participant::Agent { id: a }, Participant::Agent { id: b }, "do the thing", 1),
            trail,
            msg(Participant::Agent { id: a }, Participant::Human, "Chef is on it", 1, false),
        ];
        assert!(
            faults(&run, &named(a, b))
                .contains(&Fault::AssignedAndSaidNothing { agent: "Chef".into() }),
            "a tool trail is the working, not the report"
        );
    }

    #[test]
    fn a_run_that_answers_once_and_stops_has_nothing_wrong_with_it() {
        let (a, b) = agents();
        let run = [
            msg(Participant::Human, Participant::Agent { id: a }, "introduce yourself", 0, true),
            msg(
                Participant::Agent { id: a },
                Participant::Agent { id: b },
                "hello, I coordinate",
                1,
                true,
            ),
            msg(
                Participant::Agent { id: b },
                Participant::Agent { id: a },
                "good to meet you",
                2,
                false,
            ),
            msg(
                Participant::Agent { id: a },
                Participant::Human,
                "I introduced myself to Chef",
                0,
                false,
            ),
        ];
        assert_eq!(faults(&run, &named(a, b)), vec![]);

        let convo = analyse(&run, &named(a, b));
        assert_eq!(convo.to_operator.len(), 1);
        assert_eq!(convo.to_operator[0].0, "Manager");
        assert_eq!(convo.between_agents, 2);
        assert_eq!(convo.max_hop, 2);
    }

    #[test]
    fn demanding_an_answer_from_someone_who_already_answered_is_the_cascade() {
        // The exact shape of the bug an operator hit twice: a manager thanks a
        // peer that has just replied, and the thanks demands a reply.
        let (a, b) = agents();
        let run = [
            msg(Participant::Human, Participant::Agent { id: a }, "introduce yourself", 0, true),
            msg(Participant::Agent { id: a }, Participant::Agent { id: b }, "hello", 1, true),
            msg(Participant::Agent { id: b }, Participant::Agent { id: a }, "hi back", 2, false),
            msg(Participant::Agent { id: a }, Participant::Agent { id: b }, "thanks!", 3, true),
            msg(Participant::Agent { id: a }, Participant::Human, "done", 0, false),
        ];
        assert_eq!(
            faults(&run, &named(a, b)),
            vec![Fault::DemandedAnswerFromSettledPeer {
                from: "Manager".into(),
                to: "Chef".into()
            }]
        );
    }

    #[test]
    fn thanking_a_thank_you_is_named_even_when_it_demands_nothing() {
        let (a, b) = agents();
        let run = [
            msg(Participant::Human, Participant::Agent { id: a }, "say hello", 0, true),
            msg(Participant::Agent { id: a }, Participant::Agent { id: b }, "hello", 1, true),
            msg(Participant::Agent { id: b }, Participant::Agent { id: a }, "hi back", 2, false),
            msg(Participant::Agent { id: a }, Participant::Agent { id: b }, "thanks", 3, false),
            msg(Participant::Agent { id: a }, Participant::Human, "done", 0, false),
        ];
        assert!(faults(&run, &named(a, b)).contains(&Fault::AcknowledgedAnAcknowledgement {
            from: "Manager".into(),
            to: "Chef".into()
        }));
    }

    #[test]
    fn a_run_that_did_more_work_is_allowed_to_report_more_often() {
        // A two-step delegation reporting after each round is not the same
        // defect as a one-step errand reporting three times, and a check that
        // cannot tell them apart gets switched off.
        let (a, b) = agents();
        let mut run = vec![
            msg(Participant::Human, Participant::Agent { id: a }, "ask both of them", 0, true),
            msg(Participant::Agent { id: a }, Participant::Agent { id: b }, "first", 1, true),
            msg(Participant::Agent { id: b }, Participant::Agent { id: a }, "one", 2, false),
            msg(Participant::Agent { id: a }, Participant::Agent { id: b }, "second", 3, true),
            msg(Participant::Agent { id: b }, Participant::Agent { id: a }, "two", 4, false),
        ];
        for note in ["asked the first", "asked the second", "here are both answers"] {
            run.push(msg(Participant::Agent { id: a }, Participant::Human, note, 0, false));
        }

        let found = faults(&run, &named(a, b));
        assert!(
            !found.iter().any(|f| matches!(f, Fault::AnsweredMoreThanOnce { .. })),
            "three notes over two round trips is reporting, not noise: {found:?}"
        );
    }

    #[test]
    fn one_instruction_answered_three_times_is_the_thing_operators_complain_about() {
        // These three went to an operator in one run, seconds apart. They are
        // paraphrases with almost no words in common, which is exactly why the
        // count is the signal and the wording is not.
        let (a, b) = agents();
        let run = [
            msg(Participant::Human, Participant::Agent { id: a }, "introduce yourself", 0, true),
            msg(
                Participant::Agent { id: a },
                Participant::Human,
                "I introduced myself to Researcher, Mathematician and Scientist and outlined my \
                 coordinating role",
                0,
                false,
            ),
            msg(
                Participant::Agent { id: a },
                Participant::Human,
                "Introductions are complete; I coordinate assignments and synthesize findings",
                0,
                false,
            ),
            msg(
                Participant::Agent { id: a },
                Participant::Human,
                "The team is assembled and ready for the first assignment",
                0,
                false,
            ),
        ];
        assert!(faults(&run, &named(a, b))
            .contains(&Fault::AnsweredMoreThanOnce { agent: "Manager".into(), times: 3 }));
    }

    #[test]
    fn saying_it_again_in_the_same_words_is_caught_on_the_second() {
        let (a, b) = agents();
        let line = "Researcher Mathematician and Scientist have confirmed they will route \
                    findings through me";
        let run = [
            msg(Participant::Human, Participant::Agent { id: a }, "tell them", 0, true),
            msg(Participant::Agent { id: a }, Participant::Human, line, 0, false),
            msg(Participant::Agent { id: a }, Participant::Human, line, 0, false),
        ];
        assert!(faults(&run, &named(a, b))
            .iter()
            .any(|f| matches!(f, Fault::RepeatedToOperator { .. })));
    }

    #[test]
    fn an_instruction_that_produced_no_answer_is_a_failure_on_its_own() {
        let (a, b) = agents();
        let run = [
            msg(Participant::Human, Participant::Agent { id: a }, "what is the time", 0, true),
            msg(
                Participant::Agent { id: a },
                Participant::Agent { id: b },
                "what time is it",
                1,
                true,
            ),
        ];
        assert!(faults(&run, &named(a, b)).contains(&Fault::Silent));
    }

    #[test]
    fn a_crew_that_says_one_thing_after_ten_is_measurably_noisy() {
        let (a, b) = agents();
        let mut run = vec![msg(Participant::Human, Participant::Agent { id: a }, "go", 0, true)];
        for i in 0..10 {
            run.push(msg(
                Participant::Agent { id: a },
                Participant::Agent { id: b },
                &format!("thing {i}"),
                1,
                true,
            ));
        }
        run.push(msg(Participant::Agent { id: a }, Participant::Human, "done", 0, false));

        let convo = analyse(&run, &named(a, b));
        assert_eq!(convo.chatter(), 10.0, "ten peer messages for one thing said");
    }
}
