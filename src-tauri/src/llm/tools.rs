//! The tools an agent can call.
//!
//! Two, deliberately. `directory` is A2A's Agent Card discovery reduced to what
//! a local app can use; `send_message` is the whole point of the product. Every
//! additional tool is surface a model can get wrong, so the bar for a third one
//! is high.
//!
//! Schemas are tight (`additionalProperties: false`, `minItems`, explicit
//! enums) because a precise interface is what makes correct usage the default.
//! Parsing is deliberately looser than the schema: models routinely send a bare
//! string where an array is specified, and refusing that produces a retry loop
//! rather than a working app.

use serde::{Deserialize, Serialize};

use crate::llm::openrouter::{ToolCall, ToolSpec};

pub const DIRECTORY: &str = "directory";
pub const SEND_MESSAGE: &str = "send_message";
pub const UPDATE_NOTES: &str = "update_notes";
pub const RUN_COMMAND: &str = "run_command";
pub const OPEN_ON_DESKTOP: &str = "open_on_desktop";
pub const USE_SCREEN: &str = "use_screen";
pub const BROWSE: &str = "browse";
pub const SCHEDULE: &str = "schedule";
pub const CREATE_AGENT: &str = "create_agent";

/// Tool definitions offered on every agent turn.
pub fn specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: DIRECTORY.to_string(),
            // Framed as the routing decision, not a spelling check. Described
            // as a name lookup, it got used as one: a coordinator called it,
            // read three names, and sent the same research task to all three.
            description: "List the agents you can reach, with what each one is for and its \
                          current status. Call this to decide who should do a piece of work: \
                          the skills are how you tell which agent the task belongs to, not \
                          decoration on a list of addresses."
                .to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
        },
        ToolSpec {
            name: UPDATE_NOTES.to_string(),
            // The description is the whole design. It has to make selective
            // writing and consolidation the obvious reading, because the model
            // has no other signal about what belongs in a durable file.
            description: "Replace your memory. Your memory is a short markdown file, also called \
                          your notes, shown to you at the start of every turn, so anything kept \
                          there you will always know. This is the tool for anything asked of your \
                          memory, in whatever words: remember this, update your memory, make a \
                          note of that, forget that. Record only what will still matter in a week: \
                          who you are and how you work, the operator's standing preferences, \
                          decisions that hold across conversations, and durable facts. Do not \
                          record the conversation itself, task-by-task progress, or anything \
                          already in the messages above. This REPLACES the file entirely, so write \
                          out everything you want to keep and leave behind what no longer holds; \
                          if something you believed turned out to be wrong, correct it here rather \
                          than adding a contradiction. Space is limited, so choose."
                .to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "content": {
                        "type": "string",
                        "description": "The complete new contents of your memory, in markdown."
                    }
                },
                "required": ["content"],
                "additionalProperties": false
            }),
        },
        ToolSpec {
            name: RUN_COMMAND.to_string(),
            description: "Run a shell command on your own computer: a Linux machine with a \
                          terminal, a filesystem and internet access, kept between turns. Use it \
                          to look things up (`curl`), read and write files, install packages, \
                          and run code. This is how you reach anything you do not already know. \
                          The first call may take a few seconds while the machine starts."
                .to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "minLength": 1,
                        "description": "A bash command, e.g. `curl -s wttr.in/Charleston?format=3`."
                    }
                },
                "required": ["command"],
                "additionalProperties": false
            }),
        },
        ToolSpec {
            name: OPEN_ON_DESKTOP.to_string(),
            description: "Open a program on your computer's screen, where the operator can watch \
                          it and take over. Your machine runs a full Linux desktop with \
                          google-chrome, firefox-esr, a file manager and an editor installed. \
                          Use this whenever you are asked to visit a site, look at a page, or do \
                          anything a person would do in a window: `run_command` fetches text, \
                          this shows the real thing on screen. The program keeps running after \
                          this returns."
                .to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "minLength": 1,
                        "description": "The program and its arguments, e.g. \
                                        `google-chrome https://cnn.com`."
                    }
                },
                "required": ["command"],
                "additionalProperties": false
            }),
        },
        ToolSpec {
            name: SCHEDULE.to_string(),
            // The prohibition is here because a fired routine is a fresh run
            // with a fresh budget: polling for a reply is the one use of this
            // tool that routes around every limit on what a run may spend.
            description: "Keep your own schedule. Use this to do something later, or to keep \
                          doing it: `add` with `every_secs` repeats, `add` with only `in_secs` \
                          fires once. When it fires you get the instruction back as a new \
                          message and work as usual, so write it as something you will be able \
                          to act on with no other context. Nothing is running while you wait, \
                          and a routine outlives restarts. Never schedule a check for a reply, a \
                          result, or anything else you are waiting on: those arrive as new \
                          messages on their own, so a routine that fires to look for one only \
                          spends a turn finding nothing."
                .to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["list", "add", "cancel"] },
                    "what": {
                        "type": "string",
                        "description": "The instruction to give yourself when it fires."
                    },
                    "every_secs": {
                        "type": "integer",
                        "description": "Repeat this often. 18000 is every five hours. \
                                        Omit for something that happens once."
                    },
                    "in_secs": {
                        "type": "integer",
                        "description": "How long until the first run. Defaults to one interval \
                                        away, or immediately for a one-off."
                    },
                    "id": { "type": "string", "description": "For `cancel`, from `list`." }
                },
                "required": ["action"],
                "additionalProperties": false
            }),
        },
        ToolSpec {
            name: BROWSE.to_string(),
            description: "Use the web browser on your computer. This is the right tool for \
                          anything on the web: the browser tells you exactly where every link, \
                          button and field is, so you never have to guess at a position. `read` \
                          gives you the page's text and a numbered list of everything you can \
                          use; `click` and `type` take one of those numbers. Read again after \
                          anything that changes the page, because the numbers are renumbered \
                          each time. The operator watches this happen on screen."
                .to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["open", "read", "click", "type", "scroll", "back"],
                        "description": "`open` a url, `read` the current page, then act on it."
                    },
                    "url": { "type": "string", "description": "For `open`." },
                    "id": {
                        "type": "integer",
                        "description": "For `click` and `type`: the number `read` gave that element."
                    },
                    "text": { "type": "string", "description": "For `type`: what to enter." },
                    "submit": {
                        "type": "boolean",
                        "description": "For `type`: press Enter afterwards, to search or submit."
                    },
                    "direction": { "type": "string", "enum": ["up", "down"] },
                    "amount": { "type": "integer", "description": "For `scroll`: screenfuls." }
                },
                "required": ["action"],
                "additionalProperties": false
            }),
        },
        ToolSpec {
            name: USE_SCREEN.to_string(),
            description: "Look at your computer's screen and use it: click, type, press keys and \
                          scroll, exactly as a person would. Start with `look`, which returns a \
                          picture of the screen; every coordinate you give afterwards is in that \
                          picture's pixels, measured from the top left. Look again after anything \
                          that changes the screen, because you are working from the last picture \
                          you took, not from what is there now. This is how you read a page, fill \
                          a form, follow a link, or work in an app you have opened."
                .to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["look", "click", "double_click", "right_click", "move",
                                 "type", "key", "scroll"],
                        "description": "What to do. `look` first, then act on what you saw."
                    },
                    "x": { "type": "integer", "description": "Pixels from the left edge." },
                    "y": { "type": "integer", "description": "Pixels from the top edge." },
                    "text": { "type": "string", "description": "For `type`: the text to enter." },
                    "keys": {
                        "type": "string",
                        "description": "For `key`: an xdotool key name or chord, such as \
                                        `Return`, `ctrl+t`, `alt+F4`, `ctrl+shift+Tab`."
                    },
                    "direction": {
                        "type": "string",
                        "enum": ["up", "down"],
                        "description": "For `scroll`."
                    },
                    "amount": {
                        "type": "integer",
                        "description": "For `scroll`: how many notches. Three is about a screenful."
                    }
                },
                "required": ["action"],
                "additionalProperties": false
            }),
        },
        ToolSpec {
            name: CREATE_AGENT.to_string(),
            // Three failures this description exists to prevent: an agent made
            // for one afternoon's task, a refusal treated as an obstacle to
            // route around, and a crew created and then left waiting because
            // nobody realised a new agent does nothing until it is spoken to.
            description: "Add an agent to this workspace: a new colleague with its own \
                          instructions, its own computer and its own memory, which you and \
                          everyone else can then reach by name with `send_message`. It joins your \
                          own group and can only ever talk to the agents you can. Create one for a \
                          role the operator will still need next week; work that ends with this \
                          conversation belongs to you or to an agent that already exists. The \
                          operator has to approve it, so this waits for their answer, and their \
                          answer is final: if they decline, say what you would have created and \
                          carry on without it. A new agent starts idle and does nothing at all \
                          until somebody messages it, so send it its first piece of work yourself."
                .to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "minLength": 1,
                        "description": "What it is called, and how it is addressed. Name it for \
                                        the role, e.g. `Chief of Product`."
                    },
                    "instructions": {
                        "type": "string",
                        "minLength": 1,
                        "description": "Its standing instructions, written as if speaking to it: \
                                        who it is, what it owns, and how it should work. This is \
                                        all it will know about its job, so write the whole brief \
                                        rather than a job title."
                    },
                    "skills": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Short capability lines. This is what the rest of the crew \
                                        reads when deciding whether a task is this agent's, so \
                                        write what it does, not what it is."
                    },
                    "notes": {
                        "type": "string",
                        "description": "Optional. Seeds its memory, the file it is shown every \
                                        turn: facts it should start out knowing, in markdown. It \
                                        maintains this itself afterwards."
                    }
                },
                "required": ["name", "instructions"],
                "additionalProperties": false
            }),
        },
        ToolSpec {
            name: SEND_MESSAGE.to_string(),
            description: "Send a message to the other agents a piece of work belongs to. Choose \
                          them by fit: the agents whose skills cover this task, and no others. \
                          Reaching every agent in the directory is not thoroughness, it is \
                          skipping the decision, and cutting the task into a piece each is the \
                          same thing with a plan attached: both buy answers from agents the work \
                          was never for. Address several only when the content is genuinely for \
                          all of them. Delivery is asynchronous and non-blocking: this returns as \
                          soon as the messages are queued. Replies, if any, arrive later as new \
                          messages addressed to you. Do not wait for a reply and do not call \
                          this again to check for one."
                .to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "to": {
                        "type": "array",
                        "items": { "type": "string" },
                        "minItems": 1,
                        "description": "Exact agent names, as returned by directory. Only the \
                                        agents this particular message is for."
                    },
                    "text": {
                        "type": "string",
                        "minLength": 1,
                        "description": "The message body, written as if speaking directly to the \
                                        recipient. Do not address several agents in one body; \
                                        send the same text to each instead."
                    }
                },
                "required": ["to", "text"],
                "additionalProperties": false
            }),
        },
    ]
}

#[derive(Debug, Clone, PartialEq)]
pub enum ToolInvocation {
    Directory,
    SendMessage { to: Vec<String>, text: String },
    UpdateNotes { content: String },
    RunCommand { command: String },
    OpenOnDesktop { command: String },
    UseScreen { action: ScreenAction },
    Browse { action: String, args: serde_json::Value },
    Schedule { action: ScheduleAction },
    CreateAgent { draft: NewAgent },
}

/// The agent an agent asked for. Not yet validated, and not yet approved.
///
/// No model field: what a new agent costs to run is the operator's decision,
/// so it inherits its group's model the way an agent created in the UI does.
#[derive(Debug, Clone, PartialEq)]
pub struct NewAgent {
    pub name: String,
    pub instructions: String,
    pub skills: Vec<String>,
    pub notes: String,
}

/// What an agent can do to its own schedule.
#[derive(Debug, Clone, PartialEq)]
pub enum ScheduleAction {
    List,
    /// `every_secs` absent fires once, after `in_secs`.
    Add {
        what: String,
        every_secs: Option<u32>,
        in_secs: Option<u32>,
    },
    Cancel {
        id: String,
    },
}

/// What an agent can do to the screen it is looking at.
#[derive(Debug, Clone, PartialEq)]
pub enum ScreenAction {
    Look,
    Click { x: i32, y: i32, button: u8, count: u8 },
    Move { x: i32, y: i32 },
    Type { text: String },
    Key { keys: String },
    Scroll { down: bool, amount: u8 },
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ToolParseError {
    #[error(
        "unknown tool {name:?}. Available tools: directory, send_message, update_notes, \
         run_command, open_on_desktop, use_screen, browse, schedule, create_agent."
    )]
    UnknownTool { name: String },
    #[error("arguments for {name} were not valid JSON: {detail}")]
    BadJson { name: String, detail: String },
    #[error("send_message needs a non-empty `to` list of agent names")]
    MissingRecipients,
    #[error("send_message needs a non-empty `text`")]
    MissingText,
    #[error("update_notes needs a `content` string")]
    MissingContent,
    #[error("run_command needs a non-empty `command` string")]
    MissingCommand,
    #[error("open_on_desktop needs a non-empty `command` string")]
    MissingDesktopCommand,
    #[error("use_screen needs a known `action`")]
    UnknownScreenAction,
    #[error("browse needs a known `action`")]
    UnknownBrowseAction,
    #[error("schedule needs a known `action`")]
    UnknownScheduleAction,
    #[error("schedule add needs {needs}")]
    IncompleteSchedule { needs: String },
    #[error("use_screen {action} needs {needs}")]
    IncompleteScreenAction { action: String, needs: String },
    #[error("create_agent needs {needs}")]
    IncompleteAgent { needs: String },
}

impl ToolParseError {
    /// What gets handed back to the model. Says what was wrong and what a
    /// correct call looks like, so the next attempt can succeed.
    pub fn guidance(&self) -> String {
        match self {
            ToolParseError::UnknownTool { name } => {
                format!(
                    "Error: no tool named {name:?}. You can call `directory`, `send_message`, \
                     `update_notes` (your memory), or `run_command`."
                )
            }
            ToolParseError::BadJson { name, detail } => format!(
                "Error: the arguments to `{name}` were not valid JSON ({detail}). Send a single \
                 well-formed JSON object."
            ),
            ToolParseError::UnknownScheduleAction => {
                "Error: `action` must be list, add or cancel. Use \
                 {\"action\": \"list\"} to see what you have already set."
                    .to_string()
            }
            ToolParseError::IncompleteSchedule { needs } => format!(
                "Error: to add a routine you need {needs}. For example \
                 {{\"action\": \"add\", \"what\": \"check the listings\", \"every_secs\": 18000}}."
            ),
            ToolParseError::UnknownBrowseAction => {
                "Error: `action` must be one of open, read, click, type, scroll or back. \
                 Call it with {\"action\": \"read\"} to see the page you are on."
                    .to_string()
            }
            ToolParseError::UnknownScreenAction => {
                "Error: `action` must be one of look, click, double_click, right_click, move, \
                 type, key or scroll. Start with {\"action\": \"look\"} to see the screen."
                    .to_string()
            }
            ToolParseError::IncompleteScreenAction { action, needs } => format!(
                "Error: `{action}` needs {needs}. Take a look first if you are not sure where \
                 things are."
            ),
            ToolParseError::MissingDesktopCommand => {
                "Error: `command` must name a graphical program to start, for example \
                 {\"command\": \"google-chrome https://cnn.com\"}."
                    .to_string()
            }
            ToolParseError::MissingCommand => {
                "Error: `command` must be a non-empty string, for example \
                 {\"command\": \"curl -s wttr.in/Charleston?format=3\"}."
                    .to_string()
            }
            ToolParseError::MissingRecipients => {
                "Error: `to` must be a non-empty array of exact agent names. Call `directory` to \
                 see them."
                    .to_string()
            }
            ToolParseError::MissingText => {
                "Error: `text` must be a non-empty string containing the message body.".to_string()
            }
            ToolParseError::IncompleteAgent { needs } => format!(
                "Error: to create an agent you need {needs}. For example {{\"name\": \"Chief of \
                 Product\", \"instructions\": \"You own the product roadmap. Decide what gets \
                 built and in what order, and say why.\", \"skills\": [\"roadmap\", \
                 \"prioritisation\"]}}."
            ),
            ToolParseError::MissingContent => {
                "Error: `content` must be a string holding the complete new contents of your \
                 memory: everything you want to keep, not only the part you just learned. To \
                 clear it, pass an empty string."
                    .to_string()
            }
        }
    }
}

#[derive(Debug, Deserialize)]
struct SendArgs {
    #[serde(default)]
    to: Option<serde_json::Value>,
    #[serde(default)]
    text: Option<String>,
    /// Accepted because models reach for it by analogy with other APIs.
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    agent: Option<String>,
}

/// Reads one screen action, with the coordinates it needs.
///
/// A missing coordinate is reported as a missing coordinate rather than being
/// defaulted to zero: a click at the top-left corner is a real click on
/// something, and silently making one is worse than saying no.
fn parse_screen_action(value: &serde_json::Value) -> Result<ScreenAction, ToolParseError> {
    let action = value.get("action").and_then(|v| v.as_str()).unwrap_or_default();
    let coord = |name: &str| value.get(name).and_then(|v| v.as_i64()).map(|n| n as i32);
    let point = |action_name: &str| match (coord("x"), coord("y")) {
        (Some(x), Some(y)) => Ok((x, y)),
        _ => Err(ToolParseError::IncompleteScreenAction {
            action: action_name.to_string(),
            needs: "both `x` and `y`".to_string(),
        }),
    };

    match action {
        "look" | "screenshot" => Ok(ScreenAction::Look),
        "click" | "left_click" => {
            let (x, y) = point("click")?;
            Ok(ScreenAction::Click { x, y, button: 1, count: 1 })
        }
        "double_click" => {
            let (x, y) = point("double_click")?;
            Ok(ScreenAction::Click { x, y, button: 1, count: 2 })
        }
        "right_click" => {
            let (x, y) = point("right_click")?;
            Ok(ScreenAction::Click { x, y, button: 3, count: 1 })
        }
        "move" | "move_mouse" => {
            let (x, y) = point("move")?;
            Ok(ScreenAction::Move { x, y })
        }
        "type" | "write" => match value.get("text").and_then(|v| v.as_str()) {
            Some(text) if !text.is_empty() => Ok(ScreenAction::Type { text: text.to_string() }),
            _ => Err(ToolParseError::IncompleteScreenAction {
                action: "type".to_string(),
                needs: "a non-empty `text`".to_string(),
            }),
        },
        "key" | "press" => {
            match value.get("keys").or_else(|| value.get("key")).and_then(|v| v.as_str()) {
                Some(keys) if !keys.trim().is_empty() => {
                    Ok(ScreenAction::Key { keys: keys.to_string() })
                }
                _ => Err(ToolParseError::IncompleteScreenAction {
                    action: "key".to_string(),
                    needs: "a `keys` name such as `Return` or `ctrl+t`".to_string(),
                }),
            }
        }
        "scroll" => Ok(ScreenAction::Scroll {
            down: value.get("direction").and_then(|v| v.as_str()).unwrap_or("down") != "up",
            amount: value.get("amount").and_then(|v| v.as_i64()).unwrap_or(3).clamp(1, 15) as u8,
        }),
        _ => Err(ToolParseError::UnknownScreenAction),
    }
}

pub fn parse(call: &ToolCall) -> Result<ToolInvocation, ToolParseError> {
    match call.name.as_str() {
        DIRECTORY => Ok(ToolInvocation::Directory),
        SCHEDULE => {
            let value = call.parsed_arguments().map_err(|e| ToolParseError::BadJson {
                name: SCHEDULE.to_string(),
                detail: e.to_string(),
            })?;
            let secs = |name: &str| {
                value.get(name).and_then(|v| v.as_i64()).filter(|n| *n > 0).map(|n| n as u32)
            };
            match value.get("action").and_then(|v| v.as_str()).unwrap_or("list") {
                "list" => Ok(ToolInvocation::Schedule { action: ScheduleAction::List }),
                "add" | "create" => {
                    let what = value
                        .get("what")
                        .or_else(|| value.get("prompt"))
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string();
                    let every = secs("every_secs");
                    let delay = secs("in_secs");
                    if what.trim().is_empty() {
                        return Err(ToolParseError::IncompleteSchedule {
                            needs: "a `what` to do".to_string(),
                        });
                    }
                    if every.is_none() && delay.is_none() {
                        return Err(ToolParseError::IncompleteSchedule {
                            needs: "either `every_secs` to repeat or `in_secs` to happen once"
                                .to_string(),
                        });
                    }
                    Ok(ToolInvocation::Schedule {
                        action: ScheduleAction::Add { what, every_secs: every, in_secs: delay },
                    })
                }
                "cancel" | "remove" | "delete" => match value.get("id").and_then(|v| v.as_str()) {
                    Some(id) if !id.trim().is_empty() => Ok(ToolInvocation::Schedule {
                        action: ScheduleAction::Cancel { id: id.to_string() },
                    }),
                    _ => Err(ToolParseError::IncompleteSchedule {
                        needs: "the `id` of the routine, from `list`".to_string(),
                    }),
                },
                _ => Err(ToolParseError::UnknownScheduleAction),
            }
        }
        BROWSE => {
            let value = call.parsed_arguments().map_err(|e| ToolParseError::BadJson {
                name: BROWSE.to_string(),
                detail: e.to_string(),
            })?;
            let action = value.get("action").and_then(|v| v.as_str()).unwrap_or("read");
            if !["open", "read", "click", "type", "scroll", "back"].contains(&action) {
                return Err(ToolParseError::UnknownBrowseAction);
            }
            Ok(ToolInvocation::Browse { action: action.to_string(), args: value })
        }
        USE_SCREEN => {
            let value = call.parsed_arguments().map_err(|e| ToolParseError::BadJson {
                name: USE_SCREEN.to_string(),
                detail: e.to_string(),
            })?;
            parse_screen_action(&value).map(|action| ToolInvocation::UseScreen { action })
        }
        OPEN_ON_DESKTOP => {
            let value = call.parsed_arguments().map_err(|e| ToolParseError::BadJson {
                name: OPEN_ON_DESKTOP.to_string(),
                detail: e.to_string(),
            })?;
            match value.get("command").or_else(|| value.get("app")).or_else(|| value.get("url")) {
                Some(serde_json::Value::String(command)) if !command.trim().is_empty() => {
                    Ok(ToolInvocation::OpenOnDesktop { command: command.clone() })
                }
                _ => Err(ToolParseError::MissingDesktopCommand),
            }
        }
        RUN_COMMAND => {
            let value = call.parsed_arguments().map_err(|e| ToolParseError::BadJson {
                name: RUN_COMMAND.to_string(),
                detail: e.to_string(),
            })?;
            match value.get("command").or_else(|| value.get("cmd")) {
                Some(serde_json::Value::String(command)) if !command.trim().is_empty() => {
                    Ok(ToolInvocation::RunCommand { command: command.clone() })
                }
                _ => Err(ToolParseError::MissingCommand),
            }
        }
        // Memory is what this file is called everywhere a person reads about
        // it, and notes is what the tool is called. An agent told to update its
        // memory reaches for the word it was given, and the name it lands on is
        // the same file either way, so refusing one spends a turn on spelling.
        UPDATE_NOTES | "update_memory" | "save_memory" => {
            let value = call.parsed_arguments().map_err(|e| ToolParseError::BadJson {
                name: UPDATE_NOTES.to_string(),
                detail: e.to_string(),
            })?;
            // An empty string is a legitimate instruction: clear the memory.
            match value
                .get("content")
                .or_else(|| value.get("notes"))
                .or_else(|| value.get("memory"))
            {
                Some(serde_json::Value::String(content)) => {
                    Ok(ToolInvocation::UpdateNotes { content: content.clone() })
                }
                _ => Err(ToolParseError::MissingContent),
            }
        }
        CREATE_AGENT => {
            let value = call.parsed_arguments().map_err(|e| ToolParseError::BadJson {
                name: CREATE_AGENT.to_string(),
                detail: e.to_string(),
            })?;
            // Aliases for the same two ideas, because a model that has just been
            // told to write a colleague's brief reaches for whichever word its
            // training used. Rejecting a near miss costs a whole turn, and this
            // is the one tool where the retry also costs the operator a second
            // permission prompt for the same request.
            let field = |names: &[&str]| {
                names
                    .iter()
                    .find_map(|name| value.get(*name).and_then(|v| v.as_str()))
                    .map(str::trim)
                    .filter(|text| !text.is_empty())
                    .map(str::to_string)
            };

            let name =
                field(&["name", "agent_name", "agent"]).ok_or(ToolParseError::IncompleteAgent {
                    needs: "a `name` for the agent".to_string(),
                })?;
            let instructions = field(&["instructions", "system_prompt", "prompt", "role"]).ok_or(
                ToolParseError::IncompleteAgent {
                    needs: "`instructions` saying what the agent is for".to_string(),
                },
            )?;

            Ok(ToolInvocation::CreateAgent {
                draft: NewAgent {
                    name,
                    instructions,
                    skills: normalize_list(value.get("skills")),
                    notes: field(&["notes", "memory"]).unwrap_or_default(),
                },
            })
        }
        SEND_MESSAGE => {
            let value = call.parsed_arguments().map_err(|e| ToolParseError::BadJson {
                name: SEND_MESSAGE.to_string(),
                detail: e.to_string(),
            })?;
            let args: SendArgs = serde_json::from_value(value).map_err(|e| {
                ToolParseError::BadJson { name: SEND_MESSAGE.to_string(), detail: e.to_string() }
            })?;

            let mut to = normalize_list(args.to.as_ref());
            if to.is_empty() {
                // `agent: "Chef"` is a common near-miss worth accepting.
                if let Some(single) =
                    args.agent.as_ref().map(|s| s.trim()).filter(|s| !s.is_empty())
                {
                    to.push(single.to_string());
                }
            }
            if to.is_empty() {
                return Err(ToolParseError::MissingRecipients);
            }

            let text = args
                .text
                .or(args.message)
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())
                .ok_or(ToolParseError::MissingText)?;

            Ok(ToolInvocation::SendMessage { to, text })
        }
        other => Err(ToolParseError::UnknownTool { name: other.to_string() }),
    }
}

/// Coerces the several shapes models actually emit into a list of strings.
///
/// Specified as an array of strings. Observed in the wild: a bare string, a
/// comma-separated string, an array containing objects with a `name` field.
/// Each is unambiguous, so rejecting them buys nothing but a retry.
fn normalize_list(value: Option<&serde_json::Value>) -> Vec<String> {
    let mut out = Vec::new();
    match value {
        Some(serde_json::Value::String(one)) => {
            for piece in one.split(',') {
                let trimmed = piece.trim();
                if !trimmed.is_empty() {
                    out.push(trimmed.to_string());
                }
            }
        }
        Some(serde_json::Value::Array(items)) => {
            for item in items {
                match item {
                    serde_json::Value::String(name) => {
                        let trimmed = name.trim();
                        if !trimmed.is_empty() {
                            out.push(trimmed.to_string());
                        }
                    }
                    serde_json::Value::Object(map) => {
                        if let Some(serde_json::Value::String(name)) =
                            map.get("name").or_else(|| map.get("agent"))
                        {
                            let trimmed = name.trim();
                            if !trimmed.is_empty() {
                                out.push(trimmed.to_string());
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }

    // A model asked to message everyone sometimes lists a name twice. Sending
    // twice would waste a turn and trip the dedup guard for no reason.
    out.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    let mut seen = std::collections::HashSet::new();
    out.retain(|name| seen.insert(name.to_lowercase()));
    out
}

/// What `send_message` reports back per recipient.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "status")]
pub enum Delivery {
    Queued { to: String },
    Refused { to: String, reason: String },
}

/// Renders delivery results as the tool result string the model reads.
pub fn render_deliveries(results: &[Delivery]) -> String {
    let mut lines = Vec::new();
    let queued: Vec<&str> = results
        .iter()
        .filter_map(|d| match d {
            Delivery::Queued { to } => Some(to.as_str()),
            _ => None,
        })
        .collect();

    if !queued.is_empty() {
        lines.push(format!(
            "Queued for delivery to: {}. Replies will arrive later as new messages; do not wait.",
            queued.join(", ")
        ));
    }
    for result in results {
        if let Delivery::Refused { to, reason } = result {
            lines.push(format!("Not delivered to {to}: {reason}"));
        }
    }
    if lines.is_empty() {
        lines.push("No messages were sent.".to_string());
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(name: &str, arguments: &str) -> ToolCall {
        ToolCall { id: "call_1".into(), name: name.into(), arguments: arguments.into() }
    }

    #[test]
    fn a_command_is_parsed_from_either_spelling() {
        // Models reach for `cmd` about as often as `command`, and refusing one
        // of them wastes a whole turn on a rejection.
        for field in ["command", "cmd"] {
            let parsed = parse(&call(RUN_COMMAND, &format!("{{\"{field}\": \"echo hi\"}}")));
            assert_eq!(parsed, Ok(ToolInvocation::RunCommand { command: "echo hi".into() }));
        }
    }

    #[test]
    fn an_empty_command_is_refused_with_an_example() {
        let err = parse(&call(RUN_COMMAND, "{\"command\": \"   \"}")).unwrap_err();
        assert_eq!(err, ToolParseError::MissingCommand);
        assert!(err.guidance().contains("curl"), "the model needs to see a usable call");
    }

    #[test]
    fn a_desktop_program_is_parsed_from_any_of_the_obvious_spellings() {
        // Asked to visit a site, a model reaches for `url` as often as
        // `command`, and refusing one of them wastes a whole turn.
        for field in ["command", "app", "url"] {
            let parsed =
                parse(&call(OPEN_ON_DESKTOP, &format!("{{\"{field}\": \"google-chrome x\"}}")));
            assert_eq!(
                parsed,
                Ok(ToolInvocation::OpenOnDesktop { command: "google-chrome x".into() })
            );
        }
    }

    #[test]
    fn the_desktop_tool_names_a_browser_so_the_agent_knows_it_has_one() {
        // The failure this exists to stop: an agent with a working desktop
        // replying that it has no graphical browser.
        let spec = specs().into_iter().find(|s| s.name == OPEN_ON_DESKTOP).unwrap();
        assert!(spec.description.contains("google-chrome"), "{}", spec.description);
    }

    /// The one description under test, by name.
    fn description(name: &str) -> String {
        specs().into_iter().find(|s| s.name == name).unwrap().description
    }

    #[test]
    fn the_directory_reads_as_a_routing_decision() {
        // Described as a name lookup, it was used as one: a coordinator asked
        // for research called it, read three names back, and sent the task to
        // all three. The schema cannot express "pick the right one", so this
        // sentence is the only place the decision can live.
        let spec = description(DIRECTORY);
        assert!(spec.contains("decide who should do a piece of work"), "{spec}");
        assert!(
            !spec.contains("not certain of an agent's exact name"),
            "the spelling-check framing is what produced the broadcast: {spec}"
        );
    }

    #[test]
    fn send_message_tells_the_model_to_choose_its_recipients() {
        // `to` is an array with minItems 1 and no maximum, so one call to every
        // agent costs the model exactly what one call to the right agent costs.
        // Nothing in the schema can charge for breadth; the description has to.
        let spec = description(SEND_MESSAGE);
        assert!(spec.contains("Choose them by fit"), "{spec}");
        assert!(spec.contains("no others"), "{spec}");
        assert!(
            spec.contains("genuinely for all of them"),
            "an announcement is legitimate, so the rule has to leave room for one: {spec}"
        );

        let to = specs().into_iter().find(|s| s.name == SEND_MESSAGE).unwrap().parameters
            ["properties"]["to"]["description"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(
            to.contains("this particular message is for"),
            "the parameter is read closer to the call than the description is: {to}"
        );
    }

    #[test]
    fn schedule_forbids_polling_for_something_that_will_arrive_by_itself() {
        // A fired routine is a fresh run with a fresh step budget. Scheduling a
        // check for a reply is therefore the one use of this tool that spends
        // outside every limit the guard applies to the run that made it.
        let spec = description(SCHEDULE);
        assert!(spec.contains("Never schedule a check for a reply"), "{spec}");
        assert!(
            spec.contains("arrive as new messages on their own"),
            "a prohibition without the alternative gets reworded and retried: {spec}"
        );
    }

    #[test]
    fn browsing_defaults_to_reading_the_page() {
        // A model that calls `browse` with nothing useful should be shown the
        // page rather than told off.
        assert_eq!(
            parse(&call(BROWSE, "{}")),
            Ok(ToolInvocation::Browse { action: "read".into(), args: serde_json::json!({}) })
        );
    }

    #[test]
    fn an_invented_browse_action_is_refused_with_the_list() {
        let err = parse(&call(BROWSE, "{\"action\": \"teleport\"}")).unwrap_err();
        assert_eq!(err, ToolParseError::UnknownBrowseAction);
        assert!(err.guidance().contains("read"), "the model needs the way out");
    }

    #[test]
    fn a_routine_needs_a_time_as_well_as_a_task() {
        // Without either it would be a routine that never fires, which reads as
        // having worked.
        let err = parse(&call(SCHEDULE, "{\"action\":\"add\",\"what\":\"check\"}")).unwrap_err();
        assert!(matches!(err, ToolParseError::IncompleteSchedule { .. }));
        assert!(err.guidance().contains("every_secs"), "the way out has to be in the message");

        assert_eq!(
            parse(&call(SCHEDULE, "{\"action\":\"add\",\"what\":\"check\",\"every_secs\":18000}")),
            Ok(ToolInvocation::Schedule {
                action: ScheduleAction::Add {
                    what: "check".into(),
                    every_secs: Some(18000),
                    in_secs: None
                }
            })
        );
    }

    #[test]
    fn schedule_defaults_to_showing_what_is_already_set() {
        assert_eq!(
            parse(&call(SCHEDULE, "{}")),
            Ok(ToolInvocation::Schedule { action: ScheduleAction::List })
        );
    }

    #[test]
    fn every_tool_is_offered_with_a_strict_schema() {
        let specs = specs();
        assert_eq!(
            specs.len(),
            9,
            "directory, run_command, open_on_desktop, use_screen, browse, schedule, \
             create_agent, send_message, update_notes"
        );
        for spec in &specs {
            assert_eq!(
                spec.parameters["additionalProperties"], false,
                "{} must reject stray fields",
                spec.name
            );
            assert!(
                spec.description.len() > 60,
                "{} needs a description a model can act on",
                spec.name
            );
        }
    }

    #[test]
    fn send_message_description_tells_the_model_not_to_block() {
        let spec = specs().into_iter().find(|s| s.name == SEND_MESSAGE).unwrap();
        let text = spec.description.to_lowercase();
        assert!(text.contains("non-blocking") || text.contains("asynchronous"));
        assert!(text.contains("do not wait"), "blocking on a reply is the failure mode to prevent");
    }

    #[test]
    fn directory_takes_no_arguments() {
        assert_eq!(parse(&call(DIRECTORY, "")).unwrap(), ToolInvocation::Directory);
        assert_eq!(parse(&call(DIRECTORY, "{}")).unwrap(), ToolInvocation::Directory);
    }

    #[test]
    fn send_message_parses_the_specified_shape() {
        let parsed =
            parse(&call(SEND_MESSAGE, r#"{"to":["Chef","Barista"],"text":"hello"}"#)).unwrap();
        assert_eq!(
            parsed,
            ToolInvocation::SendMessage {
                to: vec!["Chef".into(), "Barista".into()],
                text: "hello".into()
            }
        );
    }

    #[test]
    fn a_bare_string_recipient_is_accepted() {
        let parsed = parse(&call(SEND_MESSAGE, r#"{"to":"Chef","text":"hi"}"#)).unwrap();
        assert_eq!(
            parsed,
            ToolInvocation::SendMessage { to: vec!["Chef".into()], text: "hi".into() }
        );
    }

    #[test]
    fn a_comma_separated_recipient_string_is_split() {
        let parsed =
            parse(&call(SEND_MESSAGE, r#"{"to":"Chef, Barista ,Host","text":"hi"}"#)).unwrap();
        assert_eq!(
            parsed,
            ToolInvocation::SendMessage {
                to: vec!["Chef".into(), "Barista".into(), "Host".into()],
                text: "hi".into()
            }
        );
    }

    #[test]
    fn recipient_objects_are_unwrapped() {
        let parsed =
            parse(&call(SEND_MESSAGE, r#"{"to":[{"name":"Chef"},{"agent":"Host"}],"text":"hi"}"#))
                .unwrap();
        assert_eq!(
            parsed,
            ToolInvocation::SendMessage {
                to: vec!["Chef".into(), "Host".into()],
                text: "hi".into()
            }
        );
    }

    #[test]
    fn duplicate_recipients_are_collapsed_case_insensitively() {
        let parsed =
            parse(&call(SEND_MESSAGE, r#"{"to":["Chef","chef","CHEF","Host"],"text":"hi"}"#))
                .unwrap();
        match parsed {
            ToolInvocation::SendMessage { to, .. } => assert_eq!(to, vec!["Chef", "Host"]),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn the_message_alias_is_accepted_for_text() {
        let parsed = parse(&call(SEND_MESSAGE, r#"{"to":["Chef"],"message":"hi"}"#)).unwrap();
        assert_eq!(
            parsed,
            ToolInvocation::SendMessage { to: vec!["Chef".into()], text: "hi".into() }
        );
    }

    #[test]
    fn the_agent_alias_is_accepted_for_a_single_recipient() {
        let parsed = parse(&call(SEND_MESSAGE, r#"{"agent":"Chef","text":"hi"}"#)).unwrap();
        assert_eq!(
            parsed,
            ToolInvocation::SendMessage { to: vec!["Chef".into()], text: "hi".into() }
        );
    }

    #[test]
    fn text_takes_precedence_over_the_message_alias() {
        let parsed =
            parse(&call(SEND_MESSAGE, r#"{"to":["Chef"],"text":"real","message":"alias"}"#))
                .unwrap();
        match parsed {
            ToolInvocation::SendMessage { text, .. } => assert_eq!(text, "real"),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn missing_recipients_are_rejected_with_guidance() {
        let err = parse(&call(SEND_MESSAGE, r#"{"text":"hi"}"#)).unwrap_err();
        assert_eq!(err, ToolParseError::MissingRecipients);
        assert!(err.guidance().contains("directory"), "tell the model how to recover");
    }

    #[test]
    fn empty_recipient_lists_are_rejected() {
        assert_eq!(
            parse(&call(SEND_MESSAGE, r#"{"to":[],"text":"hi"}"#)).unwrap_err(),
            ToolParseError::MissingRecipients
        );
        assert_eq!(
            parse(&call(SEND_MESSAGE, r#"{"to":["  ", ""],"text":"hi"}"#)).unwrap_err(),
            ToolParseError::MissingRecipients
        );
    }

    #[test]
    fn blank_text_is_rejected() {
        assert_eq!(
            parse(&call(SEND_MESSAGE, r#"{"to":["Chef"],"text":"   "}"#)).unwrap_err(),
            ToolParseError::MissingText
        );
    }

    #[test]
    fn malformed_json_is_reported_with_the_tool_name() {
        let err = parse(&call(SEND_MESSAGE, "{not json")).unwrap_err();
        assert!(matches!(err, ToolParseError::BadJson { ref name, .. } if name == SEND_MESSAGE));
        assert!(err.guidance().contains("well-formed JSON"));
    }

    #[test]
    fn update_notes_takes_the_complete_new_contents() {
        // Doubled hashes: a markdown heading inside the JSON would otherwise
        // close an `r#"..."#` literal early.
        let parsed = parse(&call(UPDATE_NOTES, r##"{"content":"# Style\nTerse."}"##)).unwrap();
        assert_eq!(parsed, ToolInvocation::UpdateNotes { content: "# Style\nTerse.".into() });
    }

    #[test]
    fn clearing_notes_is_allowed() {
        // An empty string is an instruction, not a mistake.
        assert_eq!(
            parse(&call(UPDATE_NOTES, r#"{"content":""}"#)).unwrap(),
            ToolInvocation::UpdateNotes { content: String::new() }
        );
    }

    #[test]
    fn the_memory_file_answers_to_both_of_its_names() {
        // The operator's word for this file is memory; the tool is called
        // `update_notes`. An agent told to update its memory writes the same
        // file whichever word it reaches for, so a rejection here would cost a
        // whole turn to say only that the two words mean one thing.
        for name in [UPDATE_NOTES, "update_memory", "save_memory"] {
            assert_eq!(
                parse(&call(name, r#"{"content":"kept"}"#)).unwrap(),
                ToolInvocation::UpdateNotes { content: "kept".into() },
                "{name} did not reach the memory file"
            );
        }
        for field in ["content", "notes", "memory"] {
            assert_eq!(
                parse(&call(UPDATE_NOTES, &format!("{{\"{field}\":\"kept\"}}"))).unwrap(),
                ToolInvocation::UpdateNotes { content: "kept".into() },
                "{field} was not read"
            );
        }
    }

    #[test]
    fn a_seeded_memory_is_accepted_under_either_word() {
        for field in ["notes", "memory"] {
            let parsed = parse(&call(
                CREATE_AGENT,
                &format!(
                    "{{\"name\":\"Scout\",\"instructions\":\"You look.\",\"{field}\":\"B2B.\"}}"
                ),
            ));
            match parsed {
                Ok(ToolInvocation::CreateAgent { draft }) => assert_eq!(draft.notes, "B2B."),
                other => panic!("{field} gave {other:?}"),
            }
        }
    }

    #[test]
    fn update_notes_without_content_is_rejected_with_guidance() {
        let err = parse(&call(UPDATE_NOTES, "{}")).unwrap_err();
        assert_eq!(err, ToolParseError::MissingContent);
        assert!(err.guidance().contains("empty string"));
    }

    #[test]
    fn the_memory_tool_asks_for_durable_things_and_forbids_a_transcript_dump() {
        // The description is the only control over what an agent writes, so the
        // selective-write instruction has to survive edits.
        let spec = specs().into_iter().find(|s| s.name == UPDATE_NOTES).unwrap();
        let text = spec.description.to_lowercase();
        // Both words, because the tool is named for one of them and asked for
        // in the other: an agent reading only "notes" here has to guess that
        // the operator's "update your memory" landed on this tool.
        assert!(text.contains("memory"), "{text}");
        assert!(text.contains("notes"), "{text}");
        assert!(text.contains("still matter in a week"));
        assert!(text.contains("do not record the conversation"));
        assert!(text.contains("replaces the file"), "consolidation must be explicit");
        assert!(text.contains("space is limited"));
    }

    #[test]
    fn creating_an_agent_takes_a_name_and_a_brief() {
        // Doubled hashes: the markdown heading in `notes` would otherwise close
        // an `r#"..."#` literal early.
        let parsed = parse(&call(
            CREATE_AGENT,
            r##"{"name":"  Chief of Product  ","instructions":"You own the roadmap.",
                 "skills":["roadmap","pricing"],"notes":"# Context\nB2B."}"##,
        ))
        .unwrap();
        assert_eq!(
            parsed,
            ToolInvocation::CreateAgent {
                draft: NewAgent {
                    name: "Chief of Product".into(),
                    instructions: "You own the roadmap.".into(),
                    skills: vec!["roadmap".into(), "pricing".into()],
                    notes: "# Context\nB2B.".into(),
                }
            }
        );
    }

    #[test]
    fn the_brief_is_accepted_under_the_names_a_model_reaches_for() {
        // A wrong guess here is not just a wasted turn: the retry asks the
        // operator to approve the same agent a second time.
        for field in ["instructions", "system_prompt", "prompt", "role"] {
            let parsed =
                parse(&call(CREATE_AGENT, &format!(r#"{{"name":"Scout","{field}":"You look."}}"#)));
            match parsed {
                Ok(ToolInvocation::CreateAgent { draft }) => {
                    assert_eq!(draft.instructions, "You look.", "{field} was not read")
                }
                other => panic!("{field} gave {other:?}"),
            }
        }
    }

    #[test]
    fn an_agent_with_no_brief_is_refused_with_a_usable_example() {
        // A nameless or briefless agent would reach the operator as a request
        // to approve nothing in particular.
        for arguments in [r#"{"instructions":"You look."}"#, r#"{"name":"Scout"}"#, "{}"] {
            let err = parse(&call(CREATE_AGENT, arguments)).unwrap_err();
            assert!(matches!(err, ToolParseError::IncompleteAgent { .. }), "{arguments}");
            assert!(
                err.guidance().contains("instructions"),
                "the way out has to be in the message"
            );
        }

        let blank = parse(&call(CREATE_AGENT, r#"{"name":"  ","instructions":"x"}"#)).unwrap_err();
        assert!(matches!(blank, ToolParseError::IncompleteAgent { .. }));
    }

    #[test]
    fn skills_survive_the_shapes_a_model_sends_them_in() {
        let parsed = parse(&call(
            CREATE_AGENT,
            r#"{"name":"Scout","instructions":"You look.","skills":"research, fact checking"}"#,
        ));
        match parsed {
            Ok(ToolInvocation::CreateAgent { draft }) => {
                assert_eq!(draft.skills, vec!["research".to_string(), "fact checking".to_string()])
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn creating_an_agent_says_it_needs_permission_and_starts_idle() {
        // Both were real failures in the conversation this tool came from: an
        // agent that reported it could not create anyone, and a crew created
        // and then left waiting for work that was never sent.
        let spec = description(CREATE_AGENT);
        assert!(spec.contains("operator has to approve"), "{spec}");
        assert!(spec.contains("does nothing at all until somebody messages it"), "{spec}");
        assert!(
            spec.contains("still need next week"),
            "without this it creates an agent per task: {spec}"
        );
    }

    #[test]
    fn creating_an_agent_offers_no_choice_of_model() {
        // What a new agent costs to run is the operator's call, not a field a
        // model can set on its own behalf.
        let spec = specs().into_iter().find(|s| s.name == CREATE_AGENT).unwrap();
        let properties = spec.parameters["properties"].as_object().unwrap();
        assert!(!properties.contains_key("model"), "{properties:?}");
        assert!(!properties.contains_key("group_id"), "an agent must not place one elsewhere");
    }

    #[test]
    fn an_unknown_tool_lists_the_real_ones() {
        let err = parse(&call("delete_everything", "{}")).unwrap_err();
        assert!(matches!(err, ToolParseError::UnknownTool { .. }));
        assert!(err.guidance().contains("directory"));
        assert!(err.guidance().contains("send_message"));
        assert!(err.guidance().contains("update_notes"));
        assert!(
            err.guidance().contains("memory"),
            "a model that invented a name for its memory has to recognise the real tool in the \
             list, and the tool is not named for the word it used"
        );
    }

    #[test]
    fn delivery_rendering_separates_success_from_refusal() {
        let rendered = render_deliveries(&[
            Delivery::Queued { to: "Chef".into() },
            Delivery::Queued { to: "Host".into() },
            Delivery::Refused { to: "Ghost".into(), reason: "no agent named Ghost".into() },
        ]);
        assert!(rendered.contains("Chef, Host"));
        assert!(rendered.contains("do not wait"), "reinforce non-blocking at the result too");
        assert!(rendered.contains("Not delivered to Ghost"));
    }

    #[test]
    fn delivery_rendering_handles_a_total_refusal() {
        let rendered = render_deliveries(&[Delivery::Refused {
            to: "Chef".into(),
            reason: "hop limit".into(),
        }]);
        assert!(!rendered.contains("Queued"));
        assert!(rendered.contains("hop limit"));
    }

    #[test]
    fn delivery_rendering_handles_an_empty_result() {
        assert_eq!(render_deliveries(&[]), "No messages were sent.");
    }
}
