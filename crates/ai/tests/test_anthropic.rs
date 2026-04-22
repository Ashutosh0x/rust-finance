use ai::anthropic_client::MessageResponse;
use ai::signal::AISignal;
use serde_json::json;

#[test]
fn test_anthropic_message_response_parsing() {
    let mock_json = json!({
        "id": "msg_01XFDWW...",
        "type": "message",
        "role": "assistant",
        "model": "claude-3-opus-20240229",
        "content": [
            {
                "type": "text",
                "text": "The latest FOMC minutes point to a hawkish pause."
            }
        ],
        "stop_reason": "end_turn",
        "stop_sequence": null,
        "usage": {
            "input_tokens": 12,
            "output_tokens": 64
        }
    });

    let json_str = mock_json.to_string();

    // Simulate typical reqwest::Response::json() behavior by running serde directly
    let parsed: MessageResponse =
        serde_json::from_str(&json_str).expect("Failed to parse anthropic response");

    assert_eq!(parsed.content.len(), 1);
    assert_eq!(
        parsed.content[0].text.as_deref(),
        Some("The latest FOMC minutes point to a hawkish pause.")
    );
}

#[test]
fn test_ai_signal_serialization_and_deserialization() {
    let raw_payload = r#"{
        "symbol": "BTC",
        "action": "SELL",
        "confidence": 0.95,
        "reason": "Whale wallet movement to exchange"
    }"#;

    let signal: AISignal = serde_json::from_str(raw_payload).expect("AISignal parsing failed");

    assert_eq!(signal.symbol, "BTC");
    assert_eq!(signal.action, "SELL");
    assert!((signal.confidence - 0.95).abs() < f64::EPSILON);
    assert_eq!(signal.reason, "Whale wallet movement to exchange");

    // Test round trip
    let round_trip = serde_json::to_string(&signal).unwrap();
    assert!(round_trip.contains("\"symbol\":\"BTC\""));
    assert!(round_trip.contains("\"confidence\":0.95"));
}

#[test]
fn test_anthropic_empty_content_block() {
    let mock_json = json!({
        "content": [
            {
                "type": "image"
            }
        ]
    });

    let json_str = mock_json.to_string();
    let parsed: MessageResponse = serde_json::from_str(&json_str).unwrap();

    // Anthropic returns other block types, 'text' could be null
    assert_eq!(parsed.content.len(), 1);
    assert!(parsed.content[0].text.is_none());
}
