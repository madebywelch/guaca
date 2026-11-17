use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use guac_lib::config::{InferenceConfig, Provider};
use guac_lib::llm::openrouter::{ChatMessage, ChatRequest, LlmClient, LlmError, Token, ToolSpec};

const MARKER: &str = "guaca-conversation-arrived";

fn stand_in() -> PathBuf {
    static ONCE: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
    ONCE.get_or_init(|| {
        let dir = tempfile::tempdir().unwrap();
        write_stand_in(dir.path());
        let path = std::env::var("PATH").unwrap_or_default();
        std::env::set_var("PATH", format!("{}:{path}", dir.path().display()));
        dir
    })
    .path()
    .to_path_buf()
}

fn asking_into(record: &Path) -> ChatRequest {
    let mut request = asking();
    request.messages[0] =
        ChatMessage::system(format!("you are Dana guaca-record={}", record.display()));
    request
}

fn write_stand_in(dir: &Path) {
    let script = format!(
        "#!/bin/sh\n\
         record=''\n\
         for a in \"$@\"; do case \"$a\" in *guaca-record=*) record=\"${{a##*guaca-record=}}\" ;; esac; done\n\
         [ -n \"$record\" ] && pwd -P > \"$record\"\n\
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

    assert_eq!(thinking, ["weighing it up"]);

    let usage = completion.usage.unwrap();
    assert_eq!(usage.prompt_tokens, 10);
    assert_eq!(usage.completion_tokens, 5);
    assert_eq!(usage.cost, None);
}

#[tokio::test]
async fn the_conversation_reaches_the_programs_stdin() {
    stand_in();
    let completion =
        LlmClient::new().unwrap().stream_chat(&on_claude(), &asking(), |_| {}).await.unwrap();
    assert_ne!(completion.content, "the conversation never arrived");
}

#[tokio::test]
async fn a_program_that_ends_without_answering_says_what_it_said() {
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
    assert!(!err.is_transient());
}

#[tokio::test]
async fn a_program_that_never_finishes_is_given_up_on() {
    stand_in();
    let mut config = on_claude();
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

    assert!(started.elapsed() < Duration::from_secs(20), "{result:?}");
}

#[tokio::test]
async fn the_program_is_started_somewhere_this_app_owns() {
    stand_in();
    let dir = tempfile::tempdir().unwrap();
    let record = dir.path().join("cwd");
    LlmClient::new()
        .unwrap()
        .stream_chat(&on_claude(), &asking_into(&record), |_| {})
        .await
        .unwrap();

    let recorded =
        std::fs::read_to_string(&record).expect("the stand-in records where it was started");
    let started_in = Path::new(recorded.trim());

    assert_ne!(started_in, Path::new("/"), "started in the root of the disk");
    if let Some(home) = std::env::var_os("HOME") {
        let home = std::fs::canonicalize(home).unwrap();
        assert_ne!(started_in, home, "started in the operator's home directory");
    }

    let scratch = std::fs::canonicalize(std::env::temp_dir()).unwrap();
    assert!(
        started_in.starts_with(&scratch),
        "started outside this app's own scratch space: {}",
        started_in.display()
    );
}

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

    assert_eq!(completion.tool_calls.len(), 1, "{completion:?}");
    assert_eq!(completion.tool_calls[0].name, "send_message");
    let args: serde_json::Value =
        serde_json::from_str(&completion.tool_calls[0].arguments).unwrap();
    assert!(args.get("to").is_some(), "{args}");
    assert!(args.get("body").is_some(), "{args}");

    let usage = completion.usage.expect("the program reports what it read");
    assert!(usage.prompt_tokens > 0);
    assert_eq!(usage.cost, None);
}
