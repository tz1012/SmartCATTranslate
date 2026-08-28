mod fake_codex_server;

use std::sync::atomic::Ordering;

use fake_codex_server::{read_request, spawn_fake_transport, write_json_line, write_raw_line};
use serde_json::json;
use smartcat_translate::codex::transport::{AppServerTransport, TransportError};
use tokio::sync::broadcast::error::TryRecvError;

#[tokio::test]
async fn routes_response_by_id_and_forwards_notifications() {
    let harness = spawn_fake_transport(|mut reader, mut writer| async move {
        let request = read_request(&mut reader).await;
        assert_eq!(request["method"], "account/read");
        assert_eq!(request["params"], json!({ "refreshToken": false }));
        write_json_line(
            &mut writer,
            &json!({
                "method": "account/updated",
                "params": { "accountType": "chatgpt" }
            }),
        )
        .await;
        write_json_line(
            &mut writer,
            &json!({
                "id": request["id"],
                "result": { "account": { "type": "chatgpt" } }
            }),
        )
        .await;
    })
    .await;
    let mut events = harness.transport.subscribe();

    let response = harness
        .transport
        .request("account/read", json!({ "refreshToken": false }))
        .await
        .unwrap();

    assert_eq!(response["account"]["type"], "chatgpt");
    let event = events.recv().await.unwrap();
    assert_eq!(event.method, "account/updated");
    assert_eq!(harness.starts.load(Ordering::SeqCst), 1);
    harness.server_task.await.unwrap();
}

#[tokio::test]
async fn concurrent_requests_have_unique_ids_and_route_out_of_order_responses() {
    let harness = spawn_fake_transport(|mut reader, mut writer| async move {
        let first = read_request(&mut reader).await;
        let second = read_request(&mut reader).await;
        assert_ne!(first["id"], second["id"]);
        write_json_line(
            &mut writer,
            &json!({ "id": second["id"], "result": { "method": second["method"] } }),
        )
        .await;
        write_json_line(
            &mut writer,
            &json!({ "id": first["id"], "result": { "method": first["method"] } }),
        )
        .await;
    })
    .await;

    let (alpha, beta) = tokio::join!(
        harness.transport.request("alpha", json!({ "value": 1 })),
        harness.transport.request("beta", json!({ "value": 2 }))
    );

    assert_eq!(alpha.unwrap()["method"], "alpha");
    assert_eq!(beta.unwrap()["method"], "beta");
    harness.server_task.await.unwrap();
}

#[tokio::test]
async fn malformed_and_unknown_messages_do_not_break_the_next_response() {
    let harness = spawn_fake_transport(|mut reader, mut writer| async move {
        let request = read_request(&mut reader).await;
        write_raw_line(&mut writer, b"not-json").await;
        write_json_line(&mut writer, &json!({ "unrecognized": true })).await;
        write_json_line(
            &mut writer,
            &json!({ "id": 999_999_u64, "result": { "ignored": true } }),
        )
        .await;
        write_json_line(
            &mut writer,
            &json!({ "id": request["id"], "result": { "ok": true } }),
        )
        .await;
    })
    .await;

    let response = harness
        .transport
        .request("translation/test", json!({}))
        .await
        .unwrap();

    assert_eq!(response, json!({ "ok": true }));
    harness.server_task.await.unwrap();
}

#[tokio::test]
async fn an_object_with_an_invalid_id_is_not_forwarded_as_a_notification() {
    let harness = spawn_fake_transport(|mut reader, mut writer| async move {
        let request = read_request(&mut reader).await;
        write_json_line(
            &mut writer,
            &json!({
                "id": "not-a-number",
                "method": "spoofed/notification",
                "params": { "text": "PRIVATE SPOOF" }
            }),
        )
        .await;
        write_json_line(
            &mut writer,
            &json!({ "id": request["id"], "result": { "ok": true } }),
        )
        .await;
        std::future::pending::<()>().await;
    })
    .await;
    let mut events = harness.transport.subscribe();

    let response = harness
        .transport
        .request("translation/test", json!({}))
        .await
        .unwrap();
    tokio::task::yield_now().await;

    assert_eq!(response, json!({ "ok": true }));
    assert!(matches!(events.try_recv(), Err(TryRecvError::Empty)));
    harness.server_task.abort();
}

#[tokio::test]
async fn accepts_an_explicit_null_json_rpc_result() {
    let harness = spawn_fake_transport(|mut reader, mut writer| async move {
        let request = read_request(&mut reader).await;
        write_json_line(&mut writer, &json!({ "id": request["id"], "result": null })).await;
    })
    .await;

    let response = harness
        .transport
        .request("thread/no-content", json!({}))
        .await
        .unwrap();

    assert_eq!(response, serde_json::Value::Null);
    harness.server_task.await.unwrap();
}

#[tokio::test]
async fn malformed_known_response_resolves_with_a_sanitized_protocol_error() {
    let harness = spawn_fake_transport(|mut reader, mut writer| async move {
        let request = read_request(&mut reader).await;
        write_json_line(&mut writer, &json!({ "id": request["id"] })).await;
    })
    .await;

    let error = harness
        .transport
        .request("translation/test", json!({ "text": "PRIVATE SAMPLE" }))
        .await
        .unwrap_err();

    assert_eq!(error, TransportError::ProtocolViolation);
    assert!(!format!("{error:?} {error}").contains("PRIVATE SAMPLE"));
    harness.server_task.await.unwrap();
}

#[tokio::test]
async fn process_exit_drains_pending_requests_and_emits_runtime_exited_once() {
    let harness = spawn_fake_transport(|mut reader, writer| async move {
        let _first = read_request(&mut reader).await;
        let _second = read_request(&mut reader).await;
        drop(writer);
    })
    .await;
    let mut events = harness.transport.subscribe();

    let (first, second) = tokio::join!(
        harness
            .transport
            .request("translation/one", json!({ "text": "SECRET ONE" })),
        harness
            .transport
            .request("translation/two", json!({ "text": "SECRET TWO" }))
    );

    assert_eq!(first.unwrap_err(), TransportError::ProcessExited);
    assert_eq!(second.unwrap_err(), TransportError::ProcessExited);
    let event = events.recv().await.unwrap();
    assert_eq!(event.method, "runtime/exited");
    tokio::task::yield_now().await;
    assert!(matches!(events.try_recv(), Err(TryRecvError::Empty)));
    assert_eq!(
        harness
            .transport
            .request("after/exit", json!({}))
            .await
            .unwrap_err(),
        TransportError::ProcessExited
    );
    harness.server_task.await.unwrap();
}

#[tokio::test]
async fn remote_errors_never_expose_server_message_data_or_request_payload() {
    let harness = spawn_fake_transport(|mut reader, mut writer| async move {
        let request = read_request(&mut reader).await;
        write_json_line(
            &mut writer,
            &json!({
                "id": request["id"],
                "error": {
                    "code": -32001,
                    "message": "Bearer TOP-SECRET from C:\\Users\\private",
                    "data": { "prompt": "DO NOT LEAK THIS" }
                }
            }),
        )
        .await;
    })
    .await;

    let error = harness
        .transport
        .request(
            "translation/private",
            json!({ "text": "PRIVATE REQUEST PAYLOAD" }),
        )
        .await
        .unwrap_err();
    let rendered = format!("{error:?} {error}");

    assert_eq!(error, TransportError::Remote { code: -32001 });
    for secret in [
        "TOP-SECRET",
        "C:\\Users\\private",
        "DO NOT LEAK THIS",
        "PRIVATE REQUEST PAYLOAD",
    ] {
        assert!(!rendered.contains(secret));
    }
    harness.server_task.await.unwrap();
}

#[tokio::test]
async fn shutdown_stops_the_consumed_live_session_once() {
    let harness = spawn_fake_transport(|_reader, _writer| async move {
        std::future::pending::<()>().await;
    })
    .await;
    let mut events = harness.transport.subscribe();

    harness.transport.shutdown().await.unwrap();
    harness.transport.shutdown().await.unwrap();

    assert_eq!(harness.stops.load(Ordering::SeqCst), 1);
    assert_eq!(events.recv().await.unwrap().method, "runtime/exited");
    tokio::task::yield_now().await;
    assert!(matches!(events.try_recv(), Err(TryRecvError::Empty)));
    harness.server_task.abort();
}
