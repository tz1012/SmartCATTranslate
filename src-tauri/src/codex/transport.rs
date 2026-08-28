use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::{broadcast, mpsc, oneshot, watch};
use tokio::task::JoinHandle;

use crate::codex::protocol::{AppServerNotification, JsonRpcRequest, JsonRpcResponse};
use crate::codex::runtime::{LiveRuntimeSession, ResolvedRuntime};

const WRITER_CAPACITY: usize = 64;
const EVENT_CAPACITY: usize = 64;
const MAX_INBOUND_LINE_BYTES: usize = 8 * 1024 * 1024;

type PendingResult = Result<Value, TransportError>;
type PendingSender = oneshot::Sender<PendingResult>;
type PendingMap = Arc<Mutex<HashMap<u64, PendingSender>>>;

#[async_trait]
pub trait AppServerTransport: Send + Sync {
    async fn request(&self, method: &str, params: Value) -> Result<Value, TransportError>;

    fn subscribe(&self) -> broadcast::Receiver<AppServerNotification>;
}

pub struct JsonlAppServerTransport {
    next_id: AtomicU64,
    writer: mpsc::Sender<JsonRpcRequest<Value>>,
    events: broadcast::Sender<AppServerNotification>,
    pending: PendingMap,
    exited: Arc<AtomicBool>,
    shutdown: watch::Sender<bool>,
    session: tokio::sync::Mutex<Option<Box<dyn LiveRuntimeSession>>>,
    tasks: tokio::sync::Mutex<Vec<JoinHandle<()>>>,
}

impl JsonlAppServerTransport {
    pub fn from_resolved_runtime(runtime: ResolvedRuntime) -> Result<Self, TransportError> {
        let handle = tokio::runtime::Handle::try_current()
            .map_err(|_| TransportError::AsyncRuntimeUnavailable)?;
        let (session, channel) = runtime
            .into_live_transport()
            .map_err(|_| TransportError::SessionUnavailable)?;
        let (reader, writer) = channel.into_parts();
        let pending = Arc::new(Mutex::new(HashMap::new()));
        let exited = Arc::new(AtomicBool::new(false));
        let (events, _) = broadcast::channel(EVENT_CAPACITY);
        let (writer_tx, writer_rx) = mpsc::channel(WRITER_CAPACITY);
        let (shutdown, shutdown_rx) = watch::channel(false);

        let writer_task = handle.spawn(writer_loop(
            writer,
            writer_rx,
            shutdown_rx.clone(),
            SharedExitState::new(pending.clone(), events.clone(), exited.clone()),
            shutdown.clone(),
        ));
        let reader_task = handle.spawn(reader_loop(
            reader,
            shutdown_rx,
            SharedExitState::new(pending.clone(), events.clone(), exited.clone()),
            shutdown.clone(),
        ));

        Ok(Self {
            next_id: AtomicU64::new(1),
            writer: writer_tx,
            events,
            pending,
            exited,
            shutdown,
            session: tokio::sync::Mutex::new(Some(session)),
            tasks: tokio::sync::Mutex::new(vec![writer_task, reader_task]),
        })
    }

    pub async fn shutdown(&self) -> Result<(), TransportError> {
        let _ = self.shutdown.send(true);
        let stop_result = {
            let mut session = self.session.lock().await;
            match session.take() {
                Some(mut session) => session.stop().await.map_err(|_| TransportError::StopFailed),
                None => Ok(()),
            }
        };
        SharedExitState::new(
            self.pending.clone(),
            self.events.clone(),
            self.exited.clone(),
        )
        .exit_once();

        let tasks = {
            let mut tasks = self.tasks.lock().await;
            std::mem::take(&mut *tasks)
        };
        for task in tasks {
            let _ = task.await;
        }
        stop_result
    }

    fn allocate_id(&self) -> Result<u64, TransportError> {
        self.next_id
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                current.checked_add(1)
            })
            .map_err(|_| TransportError::RequestIdExhausted)
    }
}

#[async_trait]
impl AppServerTransport for JsonlAppServerTransport {
    async fn request(&self, method: &str, params: Value) -> Result<Value, TransportError> {
        let id = self.allocate_id()?;
        let (response_tx, response_rx) = oneshot::channel();
        {
            let mut pending = lock_pending(&self.pending);
            if self.exited.load(Ordering::Acquire) {
                return Err(TransportError::ProcessExited);
            }
            pending.insert(id, response_tx);
        }
        let mut registration = PendingRegistration::new(id, self.pending.clone());

        if self
            .writer
            .send(JsonRpcRequest {
                id,
                method: method.to_owned(),
                params,
            })
            .await
            .is_err()
        {
            SharedExitState::new(
                self.pending.clone(),
                self.events.clone(),
                self.exited.clone(),
            )
            .exit_once();
            return Err(TransportError::ProcessExited);
        }

        let result = response_rx
            .await
            .unwrap_or(Err(TransportError::ProcessExited));
        registration.disarm();
        result
    }

    fn subscribe(&self) -> broadcast::Receiver<AppServerNotification> {
        self.events.subscribe()
    }
}

impl Drop for JsonlAppServerTransport {
    fn drop(&mut self) {
        let _ = self.shutdown.send(true);
        SharedExitState::new(
            self.pending.clone(),
            self.events.clone(),
            self.exited.clone(),
        )
        .exit_once();
    }
}

struct PendingRegistration {
    id: u64,
    pending: PendingMap,
    armed: bool,
}

impl PendingRegistration {
    fn new(id: u64, pending: PendingMap) -> Self {
        Self {
            id,
            pending,
            armed: true,
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for PendingRegistration {
    fn drop(&mut self) {
        if self.armed {
            lock_pending(&self.pending).remove(&self.id);
        }
    }
}

#[derive(Clone)]
struct SharedExitState {
    pending: PendingMap,
    events: broadcast::Sender<AppServerNotification>,
    exited: Arc<AtomicBool>,
}

impl SharedExitState {
    fn new(
        pending: PendingMap,
        events: broadcast::Sender<AppServerNotification>,
        exited: Arc<AtomicBool>,
    ) -> Self {
        Self {
            pending,
            events,
            exited,
        }
    }

    fn exit_once(&self) {
        if self.exited.swap(true, Ordering::AcqRel) {
            return;
        }
        let pending = {
            let mut requests = lock_pending(&self.pending);
            std::mem::take(&mut *requests)
        };
        for (_, sender) in pending {
            let _ = sender.send(Err(TransportError::ProcessExited));
        }
        let _ = self.events.send(AppServerNotification {
            method: "runtime/exited".to_owned(),
            params: json!({ "reason": "process_exited" }),
        });
    }
}

async fn writer_loop(
    mut writer: Box<dyn AsyncWrite + Send + Unpin>,
    mut requests: mpsc::Receiver<JsonRpcRequest<Value>>,
    mut shutdown: watch::Receiver<bool>,
    exit: SharedExitState,
    shutdown_signal: watch::Sender<bool>,
) {
    loop {
        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    break;
                }
            }
            request = requests.recv() => {
                let Some(request) = request else {
                    break;
                };
                let mut frame = match serde_json::to_vec(&request) {
                    Ok(frame) => frame,
                    Err(_) => {
                        exit.exit_once();
                        let _ = shutdown_signal.send(true);
                        break;
                    }
                };
                frame.push(b'\n');
                if writer.write_all(&frame).await.is_err() || writer.flush().await.is_err() {
                    exit.exit_once();
                    let _ = shutdown_signal.send(true);
                    break;
                }
            }
        }
    }
    let _ = writer.shutdown().await;
}

async fn reader_loop(
    reader: Box<dyn AsyncRead + Send + Unpin>,
    mut shutdown: watch::Receiver<bool>,
    exit: SharedExitState,
    shutdown_signal: watch::Sender<bool>,
) {
    let mut reader = BufReader::new(reader);
    loop {
        let frame = tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    return;
                }
                continue;
            }
            frame = read_bounded_line(&mut reader) => frame,
        };

        let Some(frame) = (match frame {
            Ok(frame) => frame,
            Err(()) => {
                exit.exit_once();
                let _ = shutdown_signal.send(true);
                return;
            }
        }) else {
            exit.exit_once();
            let _ = shutdown_signal.send(true);
            return;
        };
        route_inbound(&frame, &exit, &shutdown_signal);
        if exit.exited.load(Ordering::Acquire) {
            return;
        }
    }
}

fn route_inbound(frame: &[u8], exit: &SharedExitState, shutdown_signal: &watch::Sender<bool>) {
    let Ok(value) = serde_json::from_slice::<Value>(frame) else {
        return;
    };
    let Some(object) = value.as_object() else {
        return;
    };

    if object.contains_key("id") {
        let Some(id) = object.get("id").and_then(Value::as_u64) else {
            return;
        };
        let sender = lock_pending(&exit.pending).remove(&id);
        let Some(sender) = sender else {
            return;
        };
        let has_result = object.contains_key("result");
        let has_error = object.contains_key("error");
        let result = match serde_json::from_value::<JsonRpcResponse<Value>>(value) {
            Ok(response) if response.id == id => match (has_result, has_error) {
                (true, false) => Ok(response.result.unwrap_or(Value::Null)),
                (false, true) => response
                    .error
                    .map(|error| Err(TransportError::Remote { code: error.code }))
                    .unwrap_or(Err(TransportError::ProtocolViolation)),
                _ => Err(TransportError::ProtocolViolation),
            },
            _ => Err(TransportError::ProtocolViolation),
        };
        let _ = sender.send(result);
        return;
    }

    let Some(method) = object.get("method").and_then(Value::as_str) else {
        return;
    };
    if method == "runtime/exited" {
        exit.exit_once();
        let _ = shutdown_signal.send(true);
        return;
    }
    let _ = exit.events.send(AppServerNotification {
        method: method.to_owned(),
        params: object.get("params").cloned().unwrap_or(Value::Null),
    });
}

async fn read_bounded_line<R>(reader: &mut R) -> Result<Option<Vec<u8>>, ()>
where
    R: AsyncBufRead + Unpin,
{
    let mut line = Vec::new();
    loop {
        let available = reader.fill_buf().await.map_err(|_| ())?;
        if available.is_empty() {
            return if line.is_empty() { Ok(None) } else { Err(()) };
        }
        if let Some(newline) = available.iter().position(|byte| *byte == b'\n') {
            if line.len().saturating_add(newline) > MAX_INBOUND_LINE_BYTES {
                return Err(());
            }
            line.extend_from_slice(&available[..newline]);
            reader.consume(newline + 1);
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            return Ok(Some(line));
        }
        if line.len().saturating_add(available.len()) > MAX_INBOUND_LINE_BYTES {
            return Err(());
        }
        let consumed = available.len();
        line.extend_from_slice(available);
        reader.consume(consumed);
    }
}

fn lock_pending(pending: &PendingMap) -> std::sync::MutexGuard<'_, HashMap<u64, PendingSender>> {
    pending
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum TransportError {
    #[error("the Codex App Server process exited")]
    ProcessExited,
    #[error("the Codex App Server response violated the protocol")]
    ProtocolViolation,
    #[error("the Codex App Server returned error code {code}")]
    Remote { code: i64 },
    #[error("the initialized Codex session has no transport channel")]
    SessionUnavailable,
    #[error("the Codex session could not be stopped")]
    StopFailed,
    #[error("no Tokio runtime is active")]
    AsyncRuntimeUnavailable,
    #[error("the request identifier space is exhausted")]
    RequestIdExhausted,
}
