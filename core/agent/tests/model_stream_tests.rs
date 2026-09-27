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

#[derive(Default)]
struct PreviewCapture(Vec<serde_json::Value>);
impl agent_core::TurnUi for PreviewCapture {
    fn on_core_topic_events(&mut self, events: &[agent_core::CoreTopicEvent]) {
        for event in events {
            if event.topic.name == "core.model.preview" {
                self.0.push(event.payload.clone());
            }
        }
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
    state.interrupt("network_error");
    state.begin();
    state.publish(&mut ui, "session", "turn");
    let snapshot = ui.0.last().unwrap();
    assert_eq!(snapshot["response"]["text"], "partial response");
    assert_eq!(snapshot["interruption"], "network_error");
    state
        .append(PublicTextTarget::Response, "new response")
        .unwrap();
    state.publish(&mut ui, "session", "turn");
    let snapshot = ui.0.last().unwrap();
    assert_eq!(snapshot["response"]["text"], "new response");
    assert!(snapshot["interruption"].is_null());
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
