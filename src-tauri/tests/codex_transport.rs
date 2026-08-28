mod fake_codex_server;

use std::sync::atomic::Ordering;
use std::sync::Arc;

use fake_codex_server::{
    read_request, resolve_fake_without_transport_channel, spawn_fake_transport,
    spawn_fake_transport_with_stop_gate, write_json_line, write_raw_line,
};
use serde_json::json;
use smartcat_translate::codex::protocol::AppServerNotification;
use smartcat_translate::codex::transport::{
    AppServerTransport, JsonlAppServerTransport, TransportError,
};
use tokio::sync::broadcast::error::TryRecvError;
use tokio::sync::Notify;

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
async fn an_unknown_object_terminates_the_protocol_instead_of_accepting_a_later_response() {
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

    let error = harness
        .transport
        .request("translation/test", json!({}))
        .await
        .unwrap_err();

    assert_eq!(error, TransportError::ProcessExited);
    harness.server_task.await.unwrap();
}

#[tokio::test]
async fn any_server_request_shape_terminates_transport_without_a_subscriber() {
    let harness = spawn_fake_transport(|mut reader, mut writer| async move {
        let request = read_request(&mut reader).await;
        write_json_line(
            &mut writer,
            &json!({
                "id": "not-a-number",
                "method": "item/commandExecution/requestApproval",
                "params": { "threadId": "ephemeral", "turnId": "turn-1" }
            }),
        )
        .await;
        let _ = request;
        std::future::pending::<()>().await;
    })
    .await;

    let error = tokio::time::timeout(
        std::time::Duration::from_millis(100),
        harness.transport.request("translation/test", json!({})),
    )
    .await
    .expect("a server request must terminate transport immediately")
    .unwrap_err();

    assert_eq!(error, TransportError::ProcessExited);
    harness.server_task.abort();
}

#[tokio::test]
async fn numeric_and_null_server_request_ids_are_also_fatal() {
    for server_id in [json!(91), json!(null)] {
        let harness = spawn_fake_transport(move |mut reader, mut writer| async move {
            let _request = read_request(&mut reader).await;
            write_json_line(
                &mut writer,
                &json!({
                    "id": server_id,
                    "method": "future/serverRequest",
                    "params": {}
                }),
            )
            .await;
            std::future::pending::<()>().await;
        })
        .await;

        let error = tokio::time::timeout(
            std::time::Duration::from_millis(100),
            harness.transport.request("translation/test", json!({})),
        )
        .await
        .expect("every request ID shape must terminate transport")
        .unwrap_err();

        assert_eq!(error, TransportError::ProcessExited);
        harness.server_task.abort();
    }
}

#[tokio::test]
async fn a_well_formed_server_request_is_fatal_before_its_id_can_match_a_response() {
    let harness = spawn_fake_transport(|mut reader, mut writer| async move {
        let request = read_request(&mut reader).await;
        write_json_line(
            &mut writer,
            &json!({
                "id": request["id"],
                "method": "item/commandExecution/requestApproval",
                "params": {
                    "threadId": "ephemeral",
                    "turnId": "turn-1",
                    "itemId": "tool-1"
                }
            }),
        )
        .await;
        std::future::pending::<()>().await;
    })
    .await;

    let error = tokio::time::timeout(
        std::time::Duration::from_millis(100),
        harness.transport.request("translation/test", json!({})),
    )
    .await
    .expect("a numeric request ID must still be recognized as a server request")
    .unwrap_err();

    assert_eq!(error, TransportError::ProcessExited);
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
async fn a_subscriber_created_after_immediate_process_exit_receives_current_exit_state() {
    let harness = spawn_fake_transport(|_reader, writer| async move {
        drop(writer);
    })
    .await;
    harness.server_task.await.unwrap();
    assert_eq!(
        harness
            .transport
            .request("after/immediate-exit", json!({}))
            .await
            .unwrap_err(),
        TransportError::ProcessExited
    );

    let mut events = harness.transport.subscribe();
    let event = tokio::select! {
        event = events.recv() => Some(event.unwrap()),
        _ = async {
            for _ in 0..32 {
                tokio::task::yield_now().await;
            }
        } => None,
    };

    assert_eq!(
        event.map(|event| event.method).as_deref(),
        Some("runtime/exited")
    );
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

#[test]
fn notification_debug_redacts_untrusted_method_params_tokens_and_paths() {
    let notification = AppServerNotification {
        method: "Bearer TOP-SECRET C:\\Users\\private".to_owned(),
        params: json!({ "text": "PRIVATE PARAMS" }),
        server_request: false,
    };

    let rendered = format!("{notification:?}");

    for secret in ["TOP-SECRET", "C:\\Users\\private", "PRIVATE PARAMS"] {
        assert!(!rendered.contains(secret));
    }
    assert!(rendered.contains("<redacted>"));
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

#[tokio::test]
async fn shutdown_drains_pending_requests_before_a_delayed_session_stop_finishes() {
    let request_seen = Arc::new(Notify::new());
    let server_signal = request_seen.clone();
    let (harness, stop_gate) =
        spawn_fake_transport_with_stop_gate(move |mut reader, _writer| async move {
            let _request = read_request(&mut reader).await;
            server_signal.notify_one();
            std::future::pending::<()>().await;
        })
        .await;
    let mut request = Box::pin(
        harness
            .transport
            .request("translation/delayed-stop", json!({})),
    );
    tokio::select! {
        _ = request_seen.notified() => {}
        result = &mut request => panic!("request ended before reaching the server: {result:?}"),
    }
    let mut shutdown = Box::pin(harness.transport.shutdown());
    tokio::select! {
        _ = stop_gate.started.notified() => {}
        result = &mut shutdown => panic!("shutdown skipped the delayed stop: {result:?}"),
    }

    let pending_result = tokio::select! {
        result = &mut request => Some(result),
        _ = yield_many() => None,
    };

    assert_eq!(pending_result, Some(Err(TransportError::ProcessExited)));
    stop_gate.release.notify_one();
    shutdown.await.unwrap();
    harness.server_task.abort();
}

#[tokio::test]
async fn shutdown_cancels_an_in_flight_write_blocked_by_a_full_pipe() {
    let writer_started = Arc::new(Notify::new());
    let server_signal = writer_started.clone();
    let harness = spawn_fake_transport(move |mut reader, _writer| async move {
        let available = tokio::io::AsyncBufReadExt::fill_buf(&mut reader)
            .await
            .unwrap();
        assert!(!available.is_empty());
        server_signal.notify_one();
        std::future::pending::<()>().await;
    })
    .await;
    let large_payload = "X".repeat(1024 * 1024);
    let mut request = Box::pin(harness.transport.request(
        "translation/blocked-write",
        json!({ "text": large_payload }),
    ));
    tokio::select! {
        _ = writer_started.notified() => {}
        result = &mut request => panic!("request ended before the writer blocked: {result:?}"),
    }
    let mut shutdown = Box::pin(harness.transport.shutdown());

    let shutdown_result = tokio::select! {
        result = &mut shutdown => Some(result),
        _ = yield_many() => None,
    };

    assert_eq!(shutdown_result, Some(Ok(())));
    assert_eq!(request.await, Err(TransportError::ProcessExited));
    harness.server_task.abort();
}

#[tokio::test]
async fn unavailable_channel_aborts_the_resolved_live_session() {
    let harness = resolve_fake_without_transport_channel().await;

    let result = JsonlAppServerTransport::from_resolved_runtime(harness.runtime);

    assert!(matches!(result, Err(TransportError::SessionUnavailable)));
    assert_eq!(harness.stops.load(Ordering::SeqCst), 0);
    assert_eq!(harness.aborts.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn dropping_transport_aborts_the_owned_live_session() {
    let harness = spawn_fake_transport(|_reader, _writer| async move {
        std::future::pending::<()>().await;
    })
    .await;
    let aborts = harness.aborts.clone();
    let server_task = harness.server_task;

    drop(harness.transport);

    assert_eq!(aborts.load(Ordering::SeqCst), 1);
    server_task.abort();
}

async fn yield_many() {
    for _ in 0..128 {
        tokio::task::yield_now().await;
    }
}
