#![allow(dead_code)]

use std::future::Future;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use semver::Version;
use serde_json::Value;
use smartcat_translate::codex::runtime::{
    LiveRuntimeChannel, LiveRuntimeSession, ResolvedRuntime, RuntimeCandidate, RuntimeError,
    RuntimeFailureRecord, RuntimeFailureRecorder, RuntimeLauncher, RuntimeResolver,
};
use smartcat_translate::codex::transport::JsonlAppServerTransport;
use tokio::io::{
    split, AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream, ReadHalf, WriteHalf,
};
use tokio::sync::Notify;
use tokio::task::JoinHandle;

pub type ServerReader = BufReader<ReadHalf<DuplexStream>>;
pub type ServerWriter = WriteHalf<DuplexStream>;

pub struct FakeTransportHarness {
    pub transport: JsonlAppServerTransport,
    pub server_task: JoinHandle<()>,
    pub starts: Arc<AtomicUsize>,
    pub stops: Arc<AtomicUsize>,
    pub aborts: Arc<AtomicUsize>,
}

pub struct UnavailableRuntimeHarness {
    pub runtime: ResolvedRuntime,
    pub stops: Arc<AtomicUsize>,
    pub aborts: Arc<AtomicUsize>,
}

#[derive(Clone)]
pub struct StopGate {
    pub started: Arc<Notify>,
    pub release: Arc<Notify>,
}

impl StopGate {
    fn new() -> Self {
        Self {
            started: Arc::new(Notify::new()),
            release: Arc::new(Notify::new()),
        }
    }
}

pub async fn spawn_fake_transport<F, Fut>(handler: F) -> FakeTransportHarness
where
    F: FnOnce(ServerReader, ServerWriter) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    spawn_fake_transport_inner(handler, None).await
}

pub async fn spawn_fake_transport_with_stop_gate<F, Fut>(
    handler: F,
) -> (FakeTransportHarness, StopGate)
where
    F: FnOnce(ServerReader, ServerWriter) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let gate = StopGate::new();
    let harness = spawn_fake_transport_inner(handler, Some(gate.clone())).await;
    (harness, gate)
}

async fn spawn_fake_transport_inner<F, Fut>(
    handler: F,
    stop_gate: Option<StopGate>,
) -> FakeTransportHarness
where
    F: FnOnce(ServerReader, ServerWriter) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let (client, server) = tokio::io::duplex(64 * 1024);
    let (client_reader, client_writer) = split(client);
    let channel = LiveRuntimeChannel::new(client_reader, client_writer);
    let starts = Arc::new(AtomicUsize::new(0));
    let stops = Arc::new(AtomicUsize::new(0));
    let aborts = Arc::new(AtomicUsize::new(0));
    let launcher = Arc::new(FakeLauncher {
        channel: Mutex::new(Some(channel)),
        starts: starts.clone(),
        stops: stops.clone(),
        aborts: aborts.clone(),
        stop_gate,
    });
    let resolver = RuntimeResolver::system_only(
        vec![RuntimeCandidate::system(
            "must-not-be-relaunched",
            Version::parse("0.144.4").unwrap(),
        )],
        "0.144.4",
        "smartcat-pinned-protocol",
        launcher,
        Arc::new(NoopFailureRecorder),
    );

    let server_task = tokio::spawn(async move {
        let (reader, writer) = split(server);
        handler(BufReader::new(reader), writer).await;
    });
    let resolved = resolver.resolve().await.unwrap();
    let transport = JsonlAppServerTransport::from_resolved_runtime(resolved).unwrap();

    FakeTransportHarness {
        transport,
        server_task,
        starts,
        stops,
        aborts,
    }
}

pub async fn resolve_fake_without_transport_channel() -> UnavailableRuntimeHarness {
    let starts = Arc::new(AtomicUsize::new(0));
    let stops = Arc::new(AtomicUsize::new(0));
    let aborts = Arc::new(AtomicUsize::new(0));
    let resolver = RuntimeResolver::system_only(
        vec![RuntimeCandidate::system(
            "must-not-be-relaunched",
            Version::parse("0.144.4").unwrap(),
        )],
        "0.144.4",
        "smartcat-pinned-protocol",
        Arc::new(FakeLauncher {
            channel: Mutex::new(None),
            starts,
            stops: stops.clone(),
            aborts: aborts.clone(),
            stop_gate: None,
        }),
        Arc::new(NoopFailureRecorder),
    );

    UnavailableRuntimeHarness {
        runtime: resolver.resolve().await.unwrap(),
        stops,
        aborts,
    }
}

pub async fn read_request(reader: &mut ServerReader) -> Value {
    let mut line = String::new();
    reader.read_line(&mut line).await.unwrap();
    serde_json::from_str(&line).unwrap()
}

pub async fn write_json_line(writer: &mut ServerWriter, value: &Value) {
    let mut bytes = serde_json::to_vec(value).unwrap();
    bytes.push(b'\n');
    writer.write_all(&bytes).await.unwrap();
    writer.flush().await.unwrap();
}

pub async fn write_raw_line(writer: &mut ServerWriter, bytes: &[u8]) {
    writer.write_all(bytes).await.unwrap();
    writer.write_all(b"\n").await.unwrap();
    writer.flush().await.unwrap();
}

struct FakeLauncher {
    channel: Mutex<Option<LiveRuntimeChannel>>,
    starts: Arc<AtomicUsize>,
    stops: Arc<AtomicUsize>,
    aborts: Arc<AtomicUsize>,
    stop_gate: Option<StopGate>,
}

#[async_trait]
impl RuntimeLauncher for FakeLauncher {
    async fn start(
        &self,
        _candidate: &RuntimeCandidate,
    ) -> Result<Box<dyn LiveRuntimeSession>, RuntimeError> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(FakeSession {
            channel: self.channel.lock().unwrap().take(),
            stops: self.stops.clone(),
            aborts: self.aborts.clone(),
            stop_gate: self.stop_gate.clone(),
        }))
    }
}

struct FakeSession {
    channel: Option<LiveRuntimeChannel>,
    stops: Arc<AtomicUsize>,
    aborts: Arc<AtomicUsize>,
    stop_gate: Option<StopGate>,
}

#[async_trait]
impl LiveRuntimeSession for FakeSession {
    fn session_id(&self) -> &str {
        "initialized-live-session"
    }

    async fn initialize(&mut self) -> Result<String, RuntimeError> {
        Ok("smartcat-pinned-protocol".to_owned())
    }

    async fn stop(&mut self) -> Result<(), RuntimeError> {
        self.stops.fetch_add(1, Ordering::SeqCst);
        if let Some(gate) = &self.stop_gate {
            gate.started.notify_one();
            gate.release.notified().await;
        }
        Ok(())
    }

    fn take_transport_channel(&mut self) -> Result<LiveRuntimeChannel, RuntimeError> {
        self.channel
            .take()
            .ok_or(RuntimeError::TransportUnavailable)
    }

    fn abort(&mut self) {
        self.aborts.fetch_add(1, Ordering::SeqCst);
    }
}

struct NoopFailureRecorder;

impl RuntimeFailureRecorder for NoopFailureRecorder {
    fn record(&self, _record: RuntimeFailureRecord) {}
}
