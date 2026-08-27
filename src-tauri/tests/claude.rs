//! Whether a turn set to Claude is answered by the `claude` program.
//!
//! Everything about the translation is covered beside it: the schema, the
//! conversation, the argument vector and the fold each have a unit test in
//! `llm/claude.rs`, and none of them spawns anything. What none of them can see
//! is the seam this suite exists for, which is that a config naming this
//! provider reaches that transport at all. Drop the branch in `stream_chat` and
//! every one of those unit tests still passes, while every turn in every
//! workspace goes to an endpoint the operator did not choose.
//!
//! ## Why there is a program on `PATH` here and not a mock
//!
//! The same reason `coding.rs` has one: the thing being tested is a process.
//! `claude::stream` spawns a binary by name, writes a conversation to its stdin
//! and reads its stdout as one JSON object per line. A fake in front of that
//! would be a test of the fold. So the stand-in below is a real executable,
//! found on `PATH` the way the real one is, and it answers on the basis of what
//! it was actually handed on stdin, which is the half a fold test cannot reach:
//! a conversation that never arrives is a program that waits, and a program
//! that waits looks exactly like a model that is thinking.
//!
//! The one thing this cannot check is whether the real program still accepts
//! that command line and still answers in that shape. That is what the
//! `#[ignore]`d test at the bottom is for, and it is the same live half
//! `coding.rs`, `subscription.rs` and `plugins.rs` each keep for the reason.

use std::io::Write;
use std::path::Path;
use std::time::Duration;

use guac_lib::config::{InferenceConfig, Provider};
use guac_lib::llm::openrouter::{ChatMessage, ChatRequest, LlmClient, LlmError, Token, ToolSpec};

/// A phrase the test puts in the conversation and the stand-in looks for, so
/// that "the program was started" and "the program was told what to answer"
/// cannot pass for each other.
const MARKER: &str = "guaca-conversation-arrived";

/// Puts a stand-in `claude` on `PATH`, exactly once.
///
/// Once, because `PATH` is process-wide and these tests run concurrently:
/// writing it per test is a read racing a write on another thread.
fn stand_in() {
    static ONCE: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
    ONCE.get_or_init(|| {
        let dir = tempfile::tempdir().unwrap();
        write_stand_in(dir.path());
        let path = std::env::var("PATH").unwrap_or_default();
        std::env::set_var("PATH", format!("{}:{path}", dir.path().display()));
        dir
    });
}

/// The stand-in: reads the conversation, then answers on the basis of it.
///
/// It branches on stdin rather than printing a constant, because the failure
/// this suite is here to catch is the conversation not arriving. A stand-in
/// that answered the same either way would pass with the write removed.
fn write_stand_in(dir: &Path) {
    let script = format!(
        "#!/bin/sh\n\
         if [ \"$1\" = '--version' ]; then echo 'stand-in'; exit 0; fi\n\
         input=$(cat)\n\
         case \"$input\" in\n\
           *'{MARKER}'*) ;;\n\
           *) printf '%s\\n' '{{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"structured_output\":{{\"say\":\"the conversation never arrived\",\"calls\":[]}}}}'; exit 0 ;;\n\
         esac\n\
         case \"$input\" in\n\
           *'no-answer'*) echo 'refusing' >&2; exit 3 ;;\n\
         esac\n\
         printf '%s\\n' '{{\"type\":\"stream_event\",\"event\":{{\"type\":\"content_block_delta\",\"index\":0,\"delta\":{{\"type\":\"thinking_delta\",\"thinking\":\"weighing it up\"}}}}}}'\n\
         printf '%s\\n' '{{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"stop_reason\":\"end_turn\",\"structured_output\":{{\"say\":\"told Pat\",\"calls\":[{{\"name\":\"send_message\",\"arguments\":{{\"to\":\"Pat\"}}}}]}},\"usage\":{{\"input_tokens\":7,\"cache_read_input_tokens\":3,\"output_tokens\":5}},\"total_cost_usd\":0.42}}'\n"
    );
    let at = dir.join("claude");
    let mut file = std::fs::File::create(&at).unwrap();
    file.write_all(script.as_bytes()).unwrap();
    drop(file);
    std::fs::set_permissions(&at, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
}

fn on_claude() -> InferenceConfig {
    InferenceConfig {
        provider: Provider::Claude,
        // Deliberately filled in, and deliberately nonsense. A turn on this
        // provider must not reach an endpoint, so a config that would fail
        // loudly if it did is worth more here than an empty one.
        base_url: "http://127.0.0.1:1/not-this".into(),
        api_key: "not-this-either".into(),
        request_timeout_secs: 30,
        ..InferenceConfig::default()
    }
}

fn asking() -> ChatRequest {
    ChatRequest {
        model: "ignored".into(),
        messages: vec![
            ChatMessage::system("you are Dana"),
            ChatMessage::user(format!("the build is green ({MARKER})")),
        ],
        tools: vec![ToolSpec {
            name: "send_message".into(),
            description: "say something to a peer".into(),
            parameters: serde_json::json!({ "type": "object", "properties": {} }),
        }],
        temperature: None,
    }
}

#[tokio::test]
async fn a_turn_set_to_claude_is_answered_by_the_program() {
    stand_in();
    let mut thinking = Vec::new();

    let completion = LlmClient::new()
        .unwrap()
        .stream_chat(&on_claude(), &asking(), |token| {
            if let Token::Reasoning(text) = token {
                thinking.push(text.to_string());
            }
        })
        .await
        .expect("the program should have answered");

    assert_eq!(completion.content, "told Pat");
    assert_eq!(completion.tool_calls.len(), 1);
    assert_eq!(completion.tool_calls[0].name, "send_message");
    assert_eq!(completion.finish_reason.as_deref(), Some("end_turn"));

    // Shown while the call ran, and it is the only thing that was: the reply
    // lands whole on this provider. `fold` in `llm/claude.rs` is the argument.
    assert_eq!(thinking, ["weighing it up"]);

    let usage = completion.usage.unwrap();
    assert_eq!(usage.prompt_tokens, 10);
    assert_eq!(usage.completion_tokens, 5);
    // It quoted 0.42 and no money moved. `ProgramUsage::tally`.
    assert_eq!(usage.cost, None);
}

#[tokio::test]
async fn the_conversation_reaches_the_programs_stdin() {
    // The failure this guards is silent in both directions: a write that never
    // happens is a program waiting on input, and a program waiting on input is
    // indistinguishable from a model that is thinking until the timeout.
    stand_in();
    let completion =
        LlmClient::new().unwrap().stream_chat(&on_claude(), &asking(), |_| {}).await.unwrap();
    assert_ne!(completion.content, "the conversation never arrived");
}

#[tokio::test]
async fn a_program_that_ends_without_answering_says_what_it_said() {
    // Exit code and stderr are the only account of this failure there is, and
    // the usual cause is a sign-in the operator has to go and fix.
    stand_in();
    let mut request = asking();
    request.messages.push(ChatMessage::user("no-answer"));

    let err = LlmClient::new()
        .unwrap()
        .stream_chat(&on_claude(), &request, |_| {})
        .await
        .expect_err("a program that answered nothing is not a turn with nothing to say");

    assert!(matches!(err, LlmError::ProgramFailed { .. }), "{err}");
    let said = err.to_string();
    assert!(said.contains("exit 3"), "{said}");
    assert!(said.contains("refusing"), "{said}");
    // Retrying an identical request cannot fix a sign-in.
    assert!(!err.is_transient());
}

#[tokio::test]
async fn a_program_that_never_finishes_is_given_up_on() {
    stand_in();
    let mut config = on_claude();
    // Clamped to five by `stream`, which is the floor. The stand-in answers at
    // once, so what is being timed here is a call that cannot: a `cat` with no
    // end of input.
    config.request_timeout_secs = 1;

    let mut request = asking();
    request.messages.clear();
    request.messages.push(ChatMessage::system("you are Dana"));

    let started = std::time::Instant::now();
    let result = tokio::time::timeout(
        Duration::from_secs(20),
        LlmClient::new().unwrap().stream_chat(&config, &request, |_| {}),
    )
    .await
    .expect("it must give up on its own rather than be given up on here");

    // Either the program answered that nothing arrived, or the call timed out.
    // Both are bounded, and being bounded is the whole assertion: nothing here
    // may wait on a process forever.
    assert!(started.elapsed() < Duration::from_secs(20), "{result:?}");
}

/// The live half: whether the real program still accepts that command line and
/// still answers in that shape.
///
/// No offline test can see a flag the vendor renamed or a field they moved,
/// which is the failure that takes a working provider off the air on an update
/// the operator did not read the notes for.
#[tokio::test]
#[ignore = "live: spends the operator's own Claude plan"]
async fn the_real_program_still_answers_in_the_shape_this_build_reads() {
    let mut said = Vec::new();
    let completion = LlmClient::new()
        .unwrap()
        .stream_chat(
            &InferenceConfig {
                provider: Provider::Claude,
                request_timeout_secs: 120,
                ..InferenceConfig::default()
            },
            &ChatRequest {
                model: "ignored".into(),
                messages: vec![
                    ChatMessage::system(
                        "You are Dana, an agent in a workspace. Peers: Pat. Do not narrate.",
                    ),
                    ChatMessage::user("Tell Pat the build is green."),
                ],
                tools: vec![ToolSpec {
                    name: "send_message".into(),
                    description: "Send a message to a peer agent.".into(),
                    parameters: serde_json::json!({
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["to", "body"],
                        "properties": {
                            "to": { "type": "string" },
                            "body": { "type": "string" },
                        },
                    }),
                }],
                temperature: None,
            },
            |token| {
                if let Token::Reasoning(text) = token {
                    said.push(text.to_string());
                }
            },
        )
        .await
        .expect("the real program should answer");

    // The schema is the assertion. If the program still honors it, the call
    // named the one tool it was offered and carried that tool's arguments.
    assert_eq!(completion.tool_calls.len(), 1, "{completion:?}");
    assert_eq!(completion.tool_calls[0].name, "send_message");
    let args: serde_json::Value =
        serde_json::from_str(&completion.tool_calls[0].arguments).unwrap();
    assert!(args.get("to").is_some(), "{args}");
    assert!(args.get("body").is_some(), "{args}");

    // Tokens are real and are counted; the price it quotes is not money that
    // moved and is not recorded.
    let usage = completion.usage.expect("the program reports what it read");
    assert!(usage.prompt_tokens > 0);
    assert_eq!(usage.cost, None);
}
