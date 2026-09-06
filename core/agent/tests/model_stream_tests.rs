use agent_core::model_stream::OpenAiContentStream;

#[test]
fn every_byte_boundary_preserves_unicode_and_excludes_private_fields() {
    let input = concat!(
        ": heartbeat\r\n\r\n",
        "data: {\"choices\":[{\"delta\":{\"content\":\"你好🦀\",\"reasoning_content\":\"private\",\"tool_calls\":[{\"function\":{\"arguments\":\"secret\"}}]}}]}\r\n\r\n",
        "data: [DONE]\n\n",
        "data: {\"choices\":[{\"delta\":{\"content\":\"late\"}}]}\n\n"
    );
    for boundary in 0..=input.len() {
        let mut decoder = OpenAiContentStream::default();
        let mut output = String::new();
        decoder
            .push(&input.as_bytes()[..boundary], &mut |s| output.push_str(s))
            .unwrap();
        decoder
            .push(&input.as_bytes()[boundary..], &mut |s| output.push_str(s))
            .unwrap();
        assert_eq!(output, "你好🦀", "boundary {boundary}");
    }
    let mut decoder = OpenAiContentStream::default();
    let mut output = String::new();
    for byte in input.bytes() {
        decoder.push(&[byte], &mut |s| output.push_str(s)).unwrap();
    }
    assert_eq!(output, "你好🦀");
}

#[test]
fn malformed_event_fails_closed_without_exposing_raw_body() {
    let mut decoder = OpenAiContentStream::default();
    let mut output = String::new();
    assert!(decoder
        .push(b"data: secret invalid JSON\n\n", &mut |s| output
            .push_str(s))
        .is_err());
    decoder
        .push(b"data: {}\n\n", &mut |s| output.push_str(s))
        .unwrap();
    assert!(output.is_empty());
}

#[test]
fn multiline_data_and_cr_only_delimiters() {
    let mut decoder = OpenAiContentStream::default();
    let mut output = String::new();
    decoder
        .push(
            b"event: message\rdata: {\rdata: \"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\r\r",
            &mut |s| output.push_str(s),
        )
        .unwrap();
    assert_eq!(output, "ok");
}

#[test]
fn unterminated_and_oversized_events_never_emit() {
    let mut decoder = OpenAiContentStream::default();
    let mut output = String::new();
    decoder
        .push(
            b"data: {\"choices\":[{\"delta\":{\"content\":\"not committed\"}}]}\n",
            &mut |s| output.push_str(s),
        )
        .unwrap();
    assert!(output.is_empty());
    assert!(decoder
        .push(&vec![b'x'; 4 * 1024 * 1024 + 1], &mut |s| output
            .push_str(s))
        .is_err());
    assert!(output.is_empty());
}

#[test]
fn json_filter_streams_public_values_before_completion_and_hides_nested_tools() {
    use agent_core::model_stream::JsonPublicTextStream;
    let input = r#"{"working_still_action":{"tool":{"free_talk":"secret","final_answer":"private"}},"free_talk":"你好\n\"quoted\"\uD83E\uDD80","final_answer":"done"}"#;
    let mut decoder = JsonPublicTextStream::default();
    let mut output = String::new();
    for ch in input.chars() {
        decoder
            .push(&ch.to_string(), &mut |s| output.push_str(s))
            .unwrap();
    }
    assert_eq!(output, "你好\n\"quoted\"🦀done");
    let mut decoder = JsonPublicTextStream::default();
    let mut output = String::new();
    decoder
        .push(r#"{"free_talk":"already visible"#, &mut |s| {
            output.push_str(s)
        })
        .unwrap();
    assert_eq!(output, "already visible");
}

#[test]
fn json_filter_rejects_non_object_roots_and_trailing_responses() {
    use agent_core::model_stream::JsonPublicTextStream;
    for input in [
        r#"["free_talk":"private"]"#,
        r#"{} {"free_talk":"private"}"#,
        r#""free_talk":"private""#,
    ] {
        let mut decoder = JsonPublicTextStream::default();
        let mut output = String::new();
        assert!(decoder.push(input, &mut |s| output.push_str(s)).is_err());
        assert!(output.is_empty());
        decoder
            .push(r#"{"free_talk":"late"}"#, &mut |s| output.push_str(s))
            .unwrap();
        assert!(output.is_empty());
    }
}

#[test]
fn json_filter_bounds_total_input_not_only_pending_escape_buffer() {
    use agent_core::model_stream::JsonPublicTextStream;
    let mut decoder = JsonPublicTextStream::default();
    decoder.push(r#"{"free_talk":""#, &mut |_| {}).unwrap();
    let part = "a".repeat(8192);
    let mut failed = false;
    for _ in 0..129 {
        if decoder.push(&part, &mut |_| {}).is_err() {
            failed = true;
            break;
        }
    }
    assert!(failed);
}

#[test]
fn intermediate_preview_survives_next_request_until_first_visible_text() {
    use agent_core::model_stream::{PreviewStatus, ResponsePreviewState};
    let mut state = ResponsePreviewState::default();
    let first = state.begin();
    assert!(state.append(first, "first").unwrap());
    assert!(state.settle(first, false));
    assert_eq!(state.visible().unwrap().status, PreviewStatus::Intermediate);
    let second = state.begin();
    assert_eq!(state.visible().unwrap().text, "first");
    assert!(!state.append(first, "stale").unwrap());
    assert!(!state.append(second, "").unwrap());
    assert_eq!(state.visible().unwrap().text, "first");
    assert!(state.append(second, "second").unwrap());
    assert_eq!(state.visible().unwrap().text, "second");
    assert!(!state.retract(first));
    assert!(state.settle(second, true));
    assert_eq!(state.visible().unwrap().status, PreviewStatus::Final);
    assert!(!state.append(second, "late").unwrap());
}

#[test]
fn malformed_attempt_retracts_all_text_and_stop_blocks_late_chunks() {
    use agent_core::model_stream::ResponsePreviewState;
    let mut state = ResponsePreviewState::default();
    let first = state.begin();
    state.append(first, "unvalidated").unwrap();
    assert!(state.retract(first));
    assert!(state.visible().is_none());
    assert!(!state.append(first, "late").unwrap());
    let second = state.begin();
    state.append(second, "new").unwrap();
    state.cancel();
    assert!(state.visible().is_none());
    assert!(!state.append(second, "late").unwrap());
    let third = state.begin();
    state.append(third, "fresh").unwrap();
    assert!(!state.retract(second));
    assert_eq!(state.visible().unwrap().text, "fresh");
}

#[test]
fn interim_chat_tool_arguments_are_not_public_preview_text() {
    use agent_core::model_stream::JsonPublicTextStream;
    let input = r#"{"free_talk":"正在核实","actions":[{"name":"sub_answer","arguments":{"task":"阶段结果","answer":"必须执行后才进入 Chat","final_answer":"不可冒充最终回答","free_talk":"不可冒充公开文字"}}],"continue_work":true}"#;
    let mut decoder = JsonPublicTextStream::default();
    let mut output = String::new();
    for ch in input.chars() {
        decoder
            .push(&ch.to_string(), &mut |text| output.push_str(text))
            .unwrap();
    }
    assert_eq!(output, "正在核实");
}

#[test]
fn native_interim_chat_arguments_never_enter_content_callback() {
    let input = concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"正在核实\",\"tool_calls\":[{\"function\":{\"name\":\"sub_answer\",\"arguments\":\"{\\\"answer\\\":\\\"private pending chat\\\"}\"}}]}}]}\n\n",
        "data: [DONE]\n\n"
    );
    let mut decoder = OpenAiContentStream::default();
    let mut output = String::new();
    for byte in input.bytes() {
        decoder
            .push(&[byte], &mut |text| output.push_str(text))
            .unwrap();
    }
    assert_eq!(output, "正在核实");
}

#[test]
fn inline_chat_streams_before_response_completion_and_excludes_other_tools() {
    use agent_core::model_stream::{JsonChatTextStream, PublicTextTarget};
    let input = r#"{"status":"working","working_still_action":[{"sub_answer":{"task":"阶段结果","answer":"逐步回答"}},{"run_bash":{"cmd":"secret","sub_answer":{"answer":"private nested"}}},{"sub_answer":{"task":"第二项","answer":"另一条"}}]}"#;
    let mut decoder = JsonChatTextStream::default();
    let mut tasks = [String::new(), String::new()];
    let mut answers = [String::new(), String::new()];
    for ch in input.chars() {
        decoder
            .push(&ch.to_string(), &mut |target, text| match target {
                PublicTextTarget::ChatTask { index } => tasks[index].push_str(text),
                PublicTextTarget::ChatAnswer { index } => answers[index].push_str(text),
                PublicTextTarget::Response => panic!("chat filter exposed response"),
            })
            .unwrap();
    }
    assert_eq!(tasks, ["阶段结果", "第二项"]);
    assert_eq!(answers, ["逐步回答", "另一条"]);
    let mut decoder = JsonChatTextStream::default();
    let mut early = String::new();
    decoder
        .push(
            r#"{"working_still_action":{"sub_answer":{"answer":"尚未结束"#,
            &mut |target, text| {
                if matches!(target, PublicTextTarget::ChatAnswer { .. }) {
                    early.push_str(text);
                }
            },
        )
        .unwrap();
    assert_eq!(early, "尚未结束");
}

#[test]
fn xml_chat_and_response_stream_to_separate_destinations_at_every_boundary() {
    use agent_core::model_stream::{PublicTextTarget, XmlPublicTextStream};
    let input = "<ASSISTANT><free_talk>公开</free_talk><actions><parallel><sub_answer><task>阶段</task><answer><![CDATA[你好 <x> ]] 内容]]></answer></sub_answer><run_bash><answer>private</answer></run_bash></parallel><sub_answer><task>第二</task><answer>甲 &amp; 乙</answer></sub_answer></actions><finish_confirm>private</finish_confirm></ASSISTANT>";
    for boundary in input.char_indices().map(|(i, _)| i).chain([input.len()]) {
        let mut decoder = XmlPublicTextStream::default();
        let mut response = String::new();
        let mut tasks = [String::new(), String::new()];
        let mut answers = [String::new(), String::new()];
        let mut emit = |target, text: &str| match target {
            PublicTextTarget::Response => response.push_str(text),
            PublicTextTarget::ChatTask { index } => tasks[index].push_str(text),
            PublicTextTarget::ChatAnswer { index } => answers[index].push_str(text),
        };
        decoder.push_typed(&input[..boundary], &mut emit).unwrap();
        decoder.push_typed(&input[boundary..], &mut emit).unwrap();
        assert_eq!(response, "公开");
        assert_eq!(tasks, ["阶段", "第二"]);
        assert_eq!(
            answers,
            ["你好 <x> ]] 内容", "甲 & 乙"],
            "boundary {boundary}"
        );
    }
}

#[test]
fn native_chat_arguments_stream_by_call_index_without_other_tool_disclosure() {
    use agent_core::model_stream::{NativeChatTextStream, PublicTextTarget};
    let mut decoder = NativeChatTextStream::default();
    let mut task = String::new();
    let mut answer = String::new();
    let mut emit = |target, text: &str| match target {
        PublicTextTarget::ChatTask { index: 2 } => task.push_str(text),
        PublicTextTarget::ChatAnswer { index: 2 } => answer.push_str(text),
        _ => panic!("unexpected destination"),
    };
    for (index, name, args) in [
        (0, "run_bash", r#"{"answer":"secret"}"#),
        (2, "sub_", ""),
        (2, "answer", r#"{"task":"阶段","answer":"早"#),
        (2, "", r#"到正文"}"#),
    ] {
        decoder.push(&serde_json::json!({"choices":[{"delta":{"tool_calls":[{"index":index,"function":{"name":name,"arguments":args}}]}}]}), &mut emit).unwrap();
    }
    assert_eq!(task, "阶段");
    assert_eq!(answer, "早到正文");
}

#[test]
fn native_changed_tool_name_fails_closed_and_blocks_late_chat() {
    use agent_core::model_stream::NativeChatTextStream;
    let mut decoder = NativeChatTextStream::default();
    let mut output = String::new();
    let event = |name: &str, args: &str| serde_json::json!({"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"name":name,"arguments":args}}]}}]});
    decoder
        .push(&event("sub_answer", r#"{"answer":"early"#), &mut |_, s| {
            output.push_str(s)
        })
        .unwrap();
    assert_eq!(output, "early");
    assert!(decoder
        .push(&event("_other", "private"), &mut |_, s| output.push_str(s))
        .is_err());
    decoder
        .push(&event("", "late"), &mut |_, s| output.push_str(s))
        .unwrap();
    assert_eq!(output, "early");
}

#[test]
fn xml_mismatched_close_fails_and_late_public_text_is_blocked() {
    use agent_core::model_stream::XmlPublicTextStream;
    let mut decoder = XmlPublicTextStream::default();
    let mut output = String::new();
    assert!(decoder
        .push("<ASSISTANT><free_talk>early</answer>", &mut |s| output
            .push_str(s))
        .is_err());
    decoder
        .push("<free_talk>late</free_talk>", &mut |s| output.push_str(s))
        .unwrap();
    assert_eq!(output, "early");
}

#[test]
fn native_chat_indices_and_total_arguments_are_bounded() {
    use agent_core::model_stream::NativeChatTextStream;
    let event = |index: usize, args: String| serde_json::json!({"choices":[{"delta":{"tool_calls":[{"index":index,"function":{"name":"sub_answer","arguments":args}}]}}]});
    let mut decoder = NativeChatTextStream::default();
    assert!(decoder
        .push(&event(128, String::new()), &mut |_, _| panic!(
            "must not emit"
        ))
        .is_err());
    let mut decoder = NativeChatTextStream::default();
    assert!(decoder
        .push(&event(0, "x".repeat(1024 * 1024)), &mut |_, _| panic!(
            "must not emit"
        ))
        .is_err());
}

#[test]
fn invalid_attempt_retracts_every_chat_window_and_blocks_late_updates() {
    use agent_core::model_stream::{ChatPreviewState, PublicTextTarget};
    let mut state = ChatPreviewState::default();
    state.begin(1);
    for index in 0..3 {
        state
            .append(1, PublicTextTarget::ChatAnswer { index }, "temporary")
            .unwrap();
    }
    assert_eq!(state.windows().count(), 3);
    state.validated(1, false);
    assert_eq!(state.windows().count(), 0);
    assert!(!state
        .append(1, PublicTextTarget::ChatAnswer { index: 0 }, "late")
        .unwrap());
    state.begin(2);
    state
        .append(2, PublicTextTarget::ChatAnswer { index: 0 }, "new")
        .unwrap();
    assert!(!state.retract(1));
    assert_eq!(state.windows().next().unwrap().answer, "new");
}

#[test]
fn validated_chat_waits_for_actual_delivery_and_execution_failure_can_retract() {
    use agent_core::model_stream::{ChatPreviewState, PublicTextTarget};
    let mut state = ChatPreviewState::default();
    state.begin(1);
    for index in 0..2 {
        state
            .append(1, PublicTextTarget::ChatAnswer { index }, "temporary")
            .unwrap();
    }
    state.validated(1, true);
    assert_eq!(state.windows().count(), 2);
    assert!(state.delivered(1, 0));
    assert_eq!(state.windows().count(), 1);
    assert!(state.retract(1));
    assert_eq!(state.windows().count(), 0);
}

#[derive(Default)]
struct PreviewCapture(Vec<serde_json::Value>);
impl agent_core::TurnUi for PreviewCapture {
    fn on_core_topic_events(&mut self, events: &[agent_core::CoreTopicEvent]) {
        self.0
            .extend(events.iter().map(|event| event.payload.clone()));
    }
}

#[test]
fn interrupted_projection_survives_retry_until_first_visible_text() {
    use agent_core::model_stream::{PublicTextTarget, TurnResponsePreview};
    let mut state = TurnResponsePreview::default();
    let mut ui = PreviewCapture::default();
    state.begin();
    state
        .append(PublicTextTarget::Response, "partial response")
        .unwrap();
    state
        .append(PublicTextTarget::ChatAnswer { index: 0 }, "partial chat")
        .unwrap();
    state.interrupt("network_error");
    state.begin();
    state.publish(&mut ui, "session", "turn");
    let snapshot = ui.0.last().unwrap();
    assert_eq!(snapshot["response"]["text"], "partial response");
    assert_eq!(snapshot["chat"][0]["answer"], "partial chat");
    assert_eq!(snapshot["interruption"], "network_error");
    state
        .append(PublicTextTarget::Response, "new response")
        .unwrap();
    state.publish(&mut ui, "session", "turn");
    let snapshot = ui.0.last().unwrap();
    assert_eq!(snapshot["response"]["text"], "new response");
    assert_eq!(snapshot["chat"].as_array().unwrap().len(), 0);
    assert!(snapshot["interruption"].is_null());
}

#[test]
fn chat_first_text_replaces_prior_response_without_waiting_for_plain_content() {
    use agent_core::model_stream::{PublicTextTarget, TurnResponsePreview};
    let mut state = TurnResponsePreview::default();
    let mut ui = PreviewCapture::default();
    state.begin();
    state
        .append(PublicTextTarget::Response, "old response")
        .unwrap();
    state.validated(true, false);
    state.begin();
    state.publish(&mut ui, "session", "turn");
    assert_eq!(ui.0.last().unwrap()["response"]["text"], "old response");
    state
        .append(PublicTextTarget::ChatAnswer { index: 0 }, "new chat")
        .unwrap();
    state.publish(&mut ui, "session", "turn");
    assert!(ui.0.last().unwrap()["response"].is_null());
    assert_eq!(ui.0.last().unwrap()["chat"][0]["answer"], "new chat");
}

#[test]
fn sse_event_between_one_and_four_mib_is_accepted() {
    let text = "x".repeat(2 * 1024 * 1024);
    let wire = format!(
        "data: {}\n\n",
        serde_json::json!({"choices":[{"delta":{"content":text}}]})
    );
    let mut decoder = OpenAiContentStream::default();
    let mut output = String::new();
    for chunk in wire.as_bytes().chunks(8192) {
        decoder.push(chunk, &mut |s| output.push_str(s)).unwrap();
    }
    assert_eq!(output, text);
}
