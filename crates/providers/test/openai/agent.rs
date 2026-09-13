use magenta_core::{AgentToolCall, EffortLevel};

use super::{AgentStreamState, StreamOutput, decode_agent_event};

const MODEL: &str = "gpt-5.6-luna";

#[test]
fn streamed_function_call_arguments_are_reassembled() {
    let effort = EffortLevel::XHigh;
    let mut state = AgentStreamState::default();

    let added = serde_json::json!({
        "type": "response.output_item.added",
        "output_index": 0,
        "item": {
            "type": "function_call",
            "id": "item_1",
            "call_id": "call_1",
            "name": "create_file",
            "arguments": ""
        }
    });
    assert!(
        decode_agent_event(&added.to_string(), MODEL, &effort, &mut state)
            .expect("output item should decode")
            .is_none()
    );

    for delta in [
        r#"{"path":"hello-gtk/"#,
        r#"main.c","content":"Hello, World"}"#,
    ] {
        let event = serde_json::json!({
            "type": "response.function_call_arguments.delta",
            "item_id": "item_1",
            "delta": delta
        });
        assert!(
            decode_agent_event(&event.to_string(), MODEL, &effort, &mut state)
                .expect("argument delta should decode")
                .is_none()
        );
    }

    let completed = serde_json::json!({
        "type": "response.completed",
        "response": { "output": [] }
    });
    let output = decode_agent_event(&completed.to_string(), MODEL, &effort, &mut state)
        .expect("completion should decode")
        .expect("completion should contain a tool call");

    let StreamOutput::AgentTools {
        calls,
        continuation,
    } = output
    else {
        panic!("expected a tool call output");
    };
    assert_eq!(
        calls,
        vec![AgentToolCall {
            id: "call_1".to_owned(),
            name: "create_file".to_owned(),
            arguments: r#"{"path":"hello-gtk/main.c","content":"Hello, World"}"#.to_owned(),
        }]
    );

    let payload: Vec<serde_json::Value> =
        serde_json::from_slice(&continuation.payload).expect("continuation should be JSON");
    assert_eq!(payload[0]["type"], "function_call");
    assert_eq!(payload[0]["call_id"], "call_1");
    assert_eq!(payload[0]["arguments"], calls[0].arguments);
}

#[test]
fn completed_response_function_call_is_forwarded() {
    let effort = EffortLevel::High;
    let user_input = serde_json::json!({
        "type": "message",
        "role": "user",
        "content": [{"type": "input_text", "text": "Inspect the project"}]
    });
    let mut state = AgentStreamState::with_input(vec![user_input.clone()]);
    let completed = serde_json::json!({
        "type": "response.completed",
        "response": {
            "output": [{
                "type": "function_call",
                "call_id": "call_2",
                "name": "list_files",
                "arguments": "{\"path\":\".\"}"
            }]
        }
    });

    let output = decode_agent_event(&completed.to_string(), MODEL, &effort, &mut state)
        .expect("completion should decode")
        .expect("completion should contain a tool call");
    let StreamOutput::AgentTools {
        calls,
        continuation,
    } = output
    else {
        panic!("expected a tool call output");
    };
    assert_eq!(calls[0].name, "list_files");
    assert_eq!(calls[0].arguments, r#"{"path":"."}"#);

    let payload: Vec<serde_json::Value> =
        serde_json::from_slice(&continuation.payload).expect("continuation should be JSON");
    assert_eq!(payload.len(), 2);
    assert_eq!(payload[0], user_input);
    assert_eq!(payload[1]["type"], "function_call");
}
