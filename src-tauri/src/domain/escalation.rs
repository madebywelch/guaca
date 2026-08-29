//! An escalation: work that has stopped, and that only the operator can move.
//!
//! The third thing an agent can do about a person, and the first one that does
//! not park a turn. [`super::approval`] holds the other two, and both of them
//! are a turn stopped inside a tool call waiting ten minutes for an answer:
//! `request_permission` asks to be let off something, `ask_operator` asks which
//! way to go. Both are right for a decision the turn needs *now*.
//!
//! Neither is right for the case they kept being reached for anyway, which is
//! an agent that cannot go on at all. A coding harness that will not start, a
//! sign-in that has expired, a machine only the operator can touch: none of
//! that is answerable inside a turn, and none of it stops being true because
//! ten minutes passed. So the agent did the one thing left and wrote it into
//! its channel, in a good clear paragraph, addressed to somebody who was not
//! reading it. That paragraph is the thing this exists to replace.
//!
//! What makes it a different shape rather than a longer approval:
//!
//! - **Nothing parks.** The turn ends. No run booking is held, so there is no
//!   window, no expiry, and no cost to it staying open for two days.
//! - **Nothing is answered.** There is no verdict and no value: clearing is the
//!   operator saying they have dealt with it. What actually unblocks the agent
//!   is a message in its channel, which is what the row on the desk opens.
//! - **It is one per agent, and it counts.** An agent that hits the same wall
//!   on six turns raises six times and the desk holds one row, which says it
//!   has been true for two days and that six turns have hit it. That pair is
//!   the whole signal, and it is the one a message in a channel cannot carry.

use serde::{Deserialize, Serialize};

use super::cut_to;
use super::ids::{AgentId, EscalationId, GroupId, RunId};

/// How much of what the agent wrote the operator reads on the desk.
///
/// A headline, not the report. The escalation is raised from a turn that also
/// writes a reply, and the reply is where the detail belongs: the desk card is
/// two lines in the corner of a screen and the way out of it is the channel.
/// Given room for a page an agent writes one, and a desk holding three pages is
/// a desk that gets collapsed.
pub const MAX_SUMMARY: usize = 400;

/// One escalation, as it is stored and as it is read back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Escalation {
    pub id: EscalationId,
    pub agent_id: AgentId,
    pub group_id: GroupId,
    /// The run of the turn that raised it, so it can be traced back to whatever
    /// set that turn off. Not the run of the latest raise: the first one is the
    /// one that has the beginning of the story in it.
    pub run_id: RunId,
    /// The agent's own words, which is why every surface draws them as text
    /// under a heading Guaca wrote. Same rule a question's options follow.
    pub summary: String,
    /// When this first went up. Never moved, and the reason the row is worth
    /// more than the message: an escalation is not news, it is a duration.
    pub raised_at: i64,
    /// When it was last restated. The difference between this and `raised_at`
    /// is what says whether an agent is still walking into the wall or has gone
    /// quiet in front of it, which are different problems.
    pub said_at: i64,
    /// How many turns have hit it. One when it goes up.
    pub times: u32,
    pub cleared_at: Option<i64>,
}

/// What became of a raise.
///
/// The two are worded differently to the agent and only the second one can be,
/// because only the second one knows how long the operator has had this. An
/// agent told "raised" for the sixth time learns that raising is how you say
/// something is still true; told "you raised this two days ago and this is the
/// sixth turn to hit it" it has what it needs to stop, work around it, or say
/// something the operator has not already been told.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Raised {
    /// Nothing was open for this agent. It is on the desk now.
    First(Escalation),
    /// One was already open. Restated and counted; the stamp did not move.
    Again(Escalation),
}

impl Raised {
    pub fn escalation(&self) -> &Escalation {
        match self {
            Raised::First(one) | Raised::Again(one) => one,
        }
    }
}

/// What is actually stored, which is the input trimmed and cut.
///
/// Handed back rather than applied silently, for the reason every other cut in
/// this app is: an agent that believes it said something it did not will not
/// say it again.
pub fn store_as(summary: &str) -> (String, bool) {
    cut_to(summary, MAX_SUMMARY)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_summary_within_the_cap_is_stored_whole() {
        let (body, cut) = store_as("  the deploy needs a key only you have  ");
        assert_eq!(body, "the deploy needs a key only you have");
        assert!(!cut);
    }

    #[test]
    fn an_oversized_summary_is_cut_and_says_so() {
        let (body, cut) = store_as(&"word ".repeat(200));
        assert!(cut);
        assert!(body.chars().count() <= MAX_SUMMARY);
    }

    #[test]
    fn both_outcomes_carry_the_row() {
        // Every caller words its answer from the row rather than from what it
        // passed in, so a raise that was cut reports what was kept.
        let one = Escalation {
            id: EscalationId::new(),
            agent_id: AgentId::new(),
            group_id: GroupId::new(),
            run_id: RunId::new(),
            summary: "stuck".into(),
            raised_at: 1,
            said_at: 1,
            times: 1,
            cleared_at: None,
        };
        assert_eq!(Raised::First(one.clone()).escalation(), &one);
        assert_eq!(Raised::Again(one.clone()).escalation(), &one);
    }
}
