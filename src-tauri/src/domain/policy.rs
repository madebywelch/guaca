use std::cmp::Ordering;

use serde::{Deserialize, Serialize};

use super::ids::{AgentId, GroupId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Effect {
    Allow,
    Block,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Class {
    ActOnBehalf,
    CreateAgent,
    ChangeSettings,
    ActOnSite,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Field {
    Recipients,
    Domain,
    Amount,
    AgentName,
    SettingKeys,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase")]
pub enum Predicate {
    Is { value: String },
    OneOf { values: Vec<String> },
    Under { value: String },
    OutsideDomain { value: String },
    AtMost { value: String, currency: String },
    GreaterThan { value: String, currency: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Condition {
    pub field: Field,
    pub predicate: Predicate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleDraft {
    pub group_id: Option<GroupId>,
    pub effect: Effect,
    pub class: Class,
    pub tool: Option<String>,
    pub agents: Vec<AgentId>,
    pub conditions: Vec<Condition>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Rule {
    #[serde(flatten)]
    pub draft: RuleDraft,
    pub id: String,
    pub wrote: String,
    pub created_at: i64,
    pub version: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Amount {
    pub value: String,
    pub currency: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Facts {
    pub recipients: Option<Vec<String>>,
    pub domain: Option<String>,
    pub amount: Option<Amount>,
    pub agent_name: Option<String>,
    pub setting_keys: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Request {
    pub group_id: GroupId,
    pub agent_id: AgentId,
    pub class: Class,
    pub tool: Option<String>,
    pub facts: Facts,
    pub page_read: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Decision {
    Block { rule_id: String, wrote: String },
    Allow { rule_id: String, wrote: String },
    Ask { blocks_grant: bool },
}

fn text_ok(s: &str) -> bool {
    !s.is_empty() && s == s.trim() && s.len() <= 512 && !s.chars().any(char::is_control)
}

fn tool_ok(s: &str) -> bool {
    text_ok(s)
        && !s.chars().any(char::is_whitespace)
        && s.split_once("__")
            .is_some_and(|(connector, tool)| !connector.is_empty() && !tool.is_empty())
}

fn domain(s: &str) -> Option<String> {
    let host = if s.contains("://") {
        let url = reqwest::Url::parse(s).ok()?;
        if !matches!(url.scheme(), "http" | "https") {
            return None;
        }
        url.host_str()?.to_owned()
    } else {
        let h = s.rsplit('@').next()?;
        if s.matches('@').count() > 1 || h != h.trim() {
            return None;
        }
        h.to_owned()
    };
    let host = host.to_ascii_lowercase();
    if host.len() > 253
        || !host.contains('.')
        || host.split('.').any(|label| {
            label.is_empty()
                || label.len() > 63
                || label.starts_with('-')
                || label.ends_with('-')
                || !label.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
    {
        return None;
    }
    Some(host)
}

fn decimal(s: &str) -> Option<(&str, &str)> {
    if s.len() > 128 {
        return None;
    }
    let (whole, fraction) = s.split_once('.').unwrap_or((s, ""));
    if whole.is_empty()
        || !whole.bytes().all(|c| c.is_ascii_digit())
        || !fraction.bytes().all(|c| c.is_ascii_digit())
        || (s.contains('.') && fraction.is_empty())
    {
        return None;
    }
    let whole = whole.trim_start_matches('0');
    Some((if whole.is_empty() { "0" } else { whole }, fraction.trim_end_matches('0')))
}

fn compare(a: &str, b: &str) -> Option<Ordering> {
    let (a, af) = decimal(a)?;
    let (b, bf) = decimal(b)?;
    Some(a.len().cmp(&b.len()).then_with(|| a.cmp(b)).then_with(|| {
        af.bytes()
            .chain(std::iter::repeat(b'0'))
            .zip(bf.bytes().chain(std::iter::repeat(b'0')))
            .take(af.len().max(bf.len()))
            .find_map(|(a, b)| (a != b).then(|| a.cmp(&b)))
            .unwrap_or(Ordering::Equal)
    }))
}

fn currency(s: &str) -> bool {
    s.len() == 3 && s.bytes().all(|b| b.is_ascii_alphabetic())
}

fn valid_field(field: Field, value: &str) -> bool {
    if !text_ok(value) {
        return false;
    }
    match field {
        Field::Recipients => value.split_once('@').is_some_and(|(local, host)| {
            !local.is_empty()
                && !local.chars().any(char::is_whitespace)
                && domain(host).as_deref() == Some(host.to_ascii_lowercase().as_str())
        }),
        Field::Domain => domain(value).is_some(),
        Field::Amount => false,
        _ => true,
    }
}

impl RuleDraft {
    pub fn validate(&self) -> Result<(), String> {
        if self.effect == Effect::Allow
            && matches!(self.class, Class::ChangeSettings | Class::ActOnSite)
        {
            return Err("This class cannot be allowed by a rule".into());
        }
        if self.tool.as_ref().is_some_and(|v| self.class != Class::ActOnBehalf || !tool_ok(v)) {
            return Err("Invalid connector tool".into());
        }
        if self.agents.len() > 100 || self.conditions.len() > 20 {
            return Err("Too many agents or conditions".into());
        }
        for c in &self.conditions {
            let field_ok = matches!(
                (self.class, c.field),
                (Class::ActOnBehalf, Field::Recipients | Field::Domain | Field::Amount)
                    | (Class::CreateAgent, Field::AgentName)
                    | (Class::ChangeSettings, Field::SettingKeys)
                    | (Class::ActOnSite, Field::Domain)
            );
            let pred_ok = match &c.predicate {
                Predicate::Is { value } => valid_field(c.field, value),
                Predicate::OneOf { values } => {
                    c.field != Field::Amount
                        && !values.is_empty()
                        && values.len() <= 20
                        && values.iter().all(|v| valid_field(c.field, v))
                }
                Predicate::Under { value } | Predicate::OutsideDomain { value } => {
                    matches!(c.field, Field::Domain | Field::Recipients)
                        && domain(value).as_deref() == Some(value.to_ascii_lowercase().as_str())
                }
                Predicate::AtMost { value, currency: unit }
                | Predicate::GreaterThan { value, currency: unit } => {
                    c.field == Field::Amount && decimal(value).is_some() && currency(unit)
                }
            };
            if !field_ok || !pred_ok {
                return Err("Invalid policy condition".into());
            }
        }
        Ok(())
    }

    pub fn wrote(&self) -> String {
        let effect = match self.effect {
            Effect::Allow => "Allow",
            Effect::Block => "Never",
        };
        let class = match self.class {
            Class::ActOnBehalf => "act in your name",
            Class::CreateAgent => "create an agent",
            Class::ChangeSettings => "change settings",
            Class::ActOnSite => "act on a site",
        };
        let mut line = format!("{effect} {class}");
        if let Some(tool) = &self.tool {
            line.push_str(&format!(" · tool {tool}"));
        }
        if !self.agents.is_empty() {
            line.push_str(&format!(
                " · agents {}",
                self.agents.iter().map(ToString::to_string).collect::<Vec<_>>().join(", ")
            ));
        }
        for c in &self.conditions {
            let field = match c.field {
                Field::Recipients => "recipients",
                Field::Domain => "domain",
                Field::Amount => "amount",
                Field::AgentName => "agent name",
                Field::SettingKeys => "setting keys",
            };
            let condition = match &c.predicate {
                Predicate::Is { value } => format!("is {value}"),
                Predicate::OneOf { values } => format!("is one of {}", values.join(", ")),
                Predicate::Under { value } => format!("is under {value}"),
                Predicate::OutsideDomain { value } => format!("is outside {value}"),
                Predicate::AtMost { value, currency } => format!("is at most {value} {currency}"),
                Predicate::GreaterThan { value, currency } => {
                    format!("is greater than {value} {currency}")
                }
            };
            line.push_str(&format!(" · {field} {condition}"));
        }
        line
    }
}

fn matches_condition(c: &Condition, facts: &Facts, effect: Effect) -> Option<bool> {
    if let Predicate::AtMost { value, currency: unit }
    | Predicate::GreaterThan { value, currency: unit } = &c.predicate
    {
        let amount = facts.amount.as_ref()?;
        if !currency(&amount.currency) || !amount.currency.eq_ignore_ascii_case(unit) {
            return None;
        }
        let order = compare(&amount.value, value)?;
        return Some(match c.predicate {
            Predicate::AtMost { .. } => order != Ordering::Greater,
            _ => order == Ordering::Greater,
        });
    }
    let values: Vec<&str> = match c.field {
        Field::Recipients => facts.recipients.as_ref()?.iter().map(String::as_str).collect(),
        Field::Domain => vec![facts.domain.as_deref()?],
        Field::AgentName => vec![facts.agent_name.as_deref()?],
        Field::SettingKeys => facts.setting_keys.as_ref()?.iter().map(String::as_str).collect(),
        Field::Amount => return None,
    };
    if values.is_empty() || values.iter().any(|v| !valid_field(c.field, v)) {
        return None;
    }
    let results: Option<Vec<bool>> = values
        .iter()
        .map(|v| {
            Some(match &c.predicate {
                Predicate::Is { value } => v.eq_ignore_ascii_case(value),
                Predicate::OneOf { values } => values.iter().any(|x| v.eq_ignore_ascii_case(x)),
                Predicate::Under { value } | Predicate::OutsideDomain { value } => {
                    let host = domain(v)?;
                    let inside = super::signin::on_domain(&host, &value.to_ascii_lowercase());
                    if matches!(c.predicate, Predicate::OutsideDomain { .. }) {
                        !inside
                    } else {
                        inside
                    }
                }
                _ => return None,
            })
        })
        .collect();
    let results = results?;
    Some(if effect == Effect::Allow {
        results.iter().all(|v| *v)
    } else {
        results.iter().any(|v| *v)
    })
}

fn matches_rule(rule: &Rule, request: &Request) -> Option<bool> {
    let d = &rule.draft;
    if d.group_id.is_some_and(|g| g != request.group_id)
        || d.class != request.class
        || (!d.agents.is_empty() && !d.agents.contains(&request.agent_id))
    {
        return Some(false);
    }
    if d.validate().is_err() {
        return None;
    }
    let mut unknown = false;
    if let Some(tool) = &d.tool {
        match request.tool.as_ref().filter(|v| tool_ok(v)) {
            Some(actual) if actual != tool => return Some(false),
            None => unknown = true,
            _ => {}
        }
    }
    for condition in &d.conditions {
        match matches_condition(condition, &request.facts, d.effect) {
            Some(false) => return Some(false),
            None => unknown = true,
            Some(true) => {}
        }
    }
    if unknown {
        None
    } else {
        Some(true)
    }
}

pub fn evaluate(rules: &[Rule], request: &Request) -> Decision {
    let mut uncertain_block = false;
    for rule in rules.iter().filter(|r| r.draft.effect == Effect::Block) {
        match matches_rule(rule, request) {
            Some(true) => {
                return Decision::Block { rule_id: rule.id.clone(), wrote: rule.wrote.clone() }
            }
            None => uncertain_block = true,
            _ => {}
        }
    }
    if uncertain_block {
        return Decision::Ask { blocks_grant: true };
    }
    if !request.page_read && !matches!(request.class, Class::ChangeSettings | Class::ActOnSite) {
        if let Some(rule) = rules
            .iter()
            .find(|r| r.draft.effect == Effect::Allow && matches_rule(r, request) == Some(true))
        {
            return Decision::Allow { rule_id: rule.id.clone(), wrote: rule.wrote.clone() };
        }
    }
    Decision::Ask { blocks_grant: false }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(effect: Effect, condition: Condition) -> Rule {
        let draft = RuleDraft {
            group_id: None,
            effect,
            class: Class::ActOnBehalf,
            tool: None,
            agents: vec![],
            conditions: vec![condition],
        };
        Rule { id: "rule".into(), wrote: draft.wrote(), draft, created_at: 1, version: 1 }
    }

    fn request() -> Request {
        Request {
            group_id: GroupId::new(),
            agent_id: AgentId::new(),
            class: Class::ActOnBehalf,
            tool: None,
            facts: Facts::default(),
            page_read: false,
        }
    }

    #[test]
    fn unknown_block_prevents_allow_and_standing_grant() {
        let block = rule(
            Effect::Block,
            Condition {
                field: Field::Amount,
                predicate: Predicate::GreaterThan { value: "50".into(), currency: "USD".into() },
            },
        );
        let mut allow = block.clone();
        allow.draft.effect = Effect::Allow;
        allow.draft.conditions.clear();
        assert_eq!(
            evaluate(&[block.clone(), allow.clone()], &request()),
            Decision::Ask { blocks_grant: true }
        );
        let mut req = request();
        req.facts.amount = Some(Amount { value: "51".into(), currency: "eur".into() });
        assert_eq!(
            evaluate(&[block.clone(), allow.clone()], &req),
            Decision::Ask { blocks_grant: true }
        );
        req.facts.amount =
            Some(Amount { value: "50.0000000000000000001".into(), currency: "usd".into() });
        assert!(matches!(evaluate(&[allow, block], &req), Decision::Block { .. }));
    }

    #[test]
    fn domains_and_scope_are_not_substring_matches() {
        let block = rule(
            Effect::Block,
            Condition {
                field: Field::Recipients,
                predicate: Predicate::OutsideDomain { value: "example.com".into() },
            },
        );
        let mut req = request();
        req.facts.recipients =
            Some(vec!["a@sub.example.com".into(), "example.com@evil.com".into()]);
        assert!(matches!(evaluate(std::slice::from_ref(&block), &req), Decision::Block { .. }));
        req.facts.recipients = Some(vec!["a@notexample.com".into()]);
        assert!(matches!(evaluate(std::slice::from_ref(&block), &req), Decision::Block { .. }));
        let mut other = block;
        other.draft.group_id = Some(GroupId::new());
        req.facts.recipients = None;
        assert_eq!(evaluate(&[other], &req), Decision::Ask { blocks_grant: false });
    }

    #[test]
    fn allows_are_suspended_after_page_read() {
        let allow = rule(
            Effect::Allow,
            Condition {
                field: Field::Domain,
                predicate: Predicate::Under { value: "example.com".into() },
            },
        );
        let mut req = request();
        req.facts.domain = Some("www.example.com".into());
        assert!(matches!(evaluate(std::slice::from_ref(&allow), &req), Decision::Allow { .. }));
        req.page_read = true;
        assert_eq!(evaluate(&[allow], &req), Decision::Ask { blocks_grant: false });
    }

    #[test]
    fn malformed_connector_tool_is_unknown_for_a_block() {
        let mut block = rule(
            Effect::Block,
            Condition {
                field: Field::Domain,
                predicate: Predicate::Under { value: "example.com".into() },
            },
        );
        block.draft.tool = Some("gmail__send".into());
        let mut req = request();
        req.tool = Some("send".into());
        req.facts.domain = Some("example.com".into());
        assert_eq!(
            evaluate(std::slice::from_ref(&block), &req),
            Decision::Ask { blocks_grant: true }
        );
        req.tool = Some("gmail__SEND".into());
        assert_eq!(evaluate(&[block], &req), Decision::Ask { blocks_grant: false });
    }
}
