use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::time::timeout;

use crate::codex::runtime::{
    LiveRuntimeChannel, LiveRuntimeSession, RuntimeCandidate, RuntimeError, RuntimeLauncher,
};

pub const CODEX_APP_SERVER_PROTOCOL: &str = "codex-app-server-jsonl-v2";
const MAX_HANDSHAKE_BYTES: usize = 64 * 1024;
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

pub struct ProcessRuntimeLauncher {
    work_root: PathBuf,
}

impl ProcessRuntimeLauncher {
    pub fn new(work_root: PathBuf) -> Self {
        Self { work_root }
    }
}

#[async_trait]
impl RuntimeLauncher for ProcessRuntimeLauncher {
    async fn start(
        &self,
        candidate: &RuntimeCandidate,
    ) -> Result<Box<dyn LiveRuntimeSession>, RuntimeError> {
        std::fs::create_dir_all(&self.work_root).map_err(|_| RuntimeError::FilesystemFailed)?;
        let workdir = tempfile::Builder::new()
            .prefix("session-")
            .tempdir_in(&self.work_root)
            .map_err(|_| RuntimeError::FilesystemFailed)?;

        let mut command = Command::new(candidate.path());
        command
            .arg("app-server")
            .arg("--listen")
            .arg("stdio://")
            .current_dir(workdir.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .env_remove("OPENAI_API_KEY")
            .env_remove("CODEX_API_KEY");
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.as_std_mut().creation_flags(0x0800_0000);
        }

        let mut child = command.spawn().map_err(|_| RuntimeError::SpawnFailed)?;
        let writer = child.stdin.take().ok_or(RuntimeError::SpawnFailed)?;
        let reader = child.stdout.take().ok_or(RuntimeError::SpawnFailed)?;
        Ok(Box::new(ProcessRuntimeSession {
            child,
            reader: Some(reader),
            writer: Some(writer),
            _workdir: workdir,
            initialized: false,
        }))
    }
}

struct ProcessRuntimeSession {
    child: Child,
    reader: Option<ChildStdout>,
    writer: Option<ChildStdin>,
    _workdir: TempDir,
    initialized: bool,
}

#[async_trait]
impl LiveRuntimeSession for ProcessRuntimeSession {
    fn session_id(&self) -> &str {
        "codex-app-server-stdio"
    }

    async fn initialize(&mut self) -> Result<String, RuntimeError> {
        timeout(HANDSHAKE_TIMEOUT, self.initialize_inner())
            .await
            .map_err(|_| RuntimeError::HandshakeFailed)??;
        self.initialized = true;
        Ok(CODEX_APP_SERVER_PROTOCOL.to_owned())
    }

    async fn stop(&mut self) -> Result<(), RuntimeError> {
        self.writer.take();
        self.reader.take();
        if self
            .child
            .try_wait()
            .map_err(|_| RuntimeError::StopFailed)?
            .is_none()
        {
            self.child
                .start_kill()
                .map_err(|_| RuntimeError::StopFailed)?;
        }
        self.child
            .wait()
            .await
            .map(|_| ())
            .map_err(|_| RuntimeError::StopFailed)
    }

    fn abort(&mut self) {
        self.writer.take();
        self.reader.take();
        let _ = self.child.start_kill();
    }

    fn take_transport_channel(&mut self) -> Result<LiveRuntimeChannel, RuntimeError> {
        if !self.initialized {
            return Err(RuntimeError::TransportUnavailable);
        }
        let reader = self
            .reader
            .take()
            .ok_or(RuntimeError::TransportUnavailable)?;
        let writer = self
            .writer
            .take()
            .ok_or(RuntimeError::TransportUnavailable)?;
        Ok(LiveRuntimeChannel::new(reader, writer))
    }
}

impl ProcessRuntimeSession {
    async fn initialize_inner(&mut self) -> Result<(), RuntimeError> {
        let request = json!({
            "id": 0,
            "method": "initialize",
            "params": {
                "clientInfo": {
                    "name": "smartcat_translate",
                    "title": "SmartCAT Translate",
                    "version": env!("CARGO_PKG_VERSION")
                }
            }
        });
        let mut frame = serde_json::to_vec(&request).map_err(|_| RuntimeError::HandshakeFailed)?;
        frame.push(b'\n');
        let writer = self.writer.as_mut().ok_or(RuntimeError::HandshakeFailed)?;
        writer
            .write_all(&frame)
            .await
            .map_err(|_| RuntimeError::HandshakeFailed)?;
        writer
            .flush()
            .await
            .map_err(|_| RuntimeError::HandshakeFailed)?;

        let reader = self.reader.as_mut().ok_or(RuntimeError::HandshakeFailed)?;
        let response = read_handshake_line(reader).await?;
        validate_initialize_response(&response)?;

        let initialized = serde_json::to_vec(&json!({ "method": "initialized", "params": {} }))
            .map_err(|_| RuntimeError::HandshakeFailed)?;
        writer
            .write_all(&initialized)
            .await
            .map_err(|_| RuntimeError::HandshakeFailed)?;
        writer
            .write_all(b"\n")
            .await
            .map_err(|_| RuntimeError::HandshakeFailed)?;
        writer
            .flush()
            .await
            .map_err(|_| RuntimeError::HandshakeFailed)
    }
}

async fn read_handshake_line(reader: &mut ChildStdout) -> Result<Vec<u8>, RuntimeError> {
    let mut line = Vec::new();
    let mut byte = [0_u8; 1];
    loop {
        let read = reader
            .read(&mut byte)
            .await
            .map_err(|_| RuntimeError::HandshakeFailed)?;
        if read == 0 {
            return Err(RuntimeError::HandshakeFailed);
        }
        if byte[0] == b'\n' {
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            return Ok(line);
        }
        if line.len() >= MAX_HANDSHAKE_BYTES {
            return Err(RuntimeError::HandshakeFailed);
        }
        line.push(byte[0]);
    }
}

fn validate_initialize_response(frame: &[u8]) -> Result<(), RuntimeError> {
    let value: Value = serde_json::from_slice(frame).map_err(|_| RuntimeError::HandshakeFailed)?;
    if value.get("id").and_then(Value::as_u64) != Some(0) || value.get("error").is_some() {
        return Err(RuntimeError::HandshakeFailed);
    }
    let result = value
        .get("result")
        .and_then(Value::as_object)
        .ok_or(RuntimeError::HandshakeFailed)?;
    for field in ["userAgent", "platformFamily", "platformOs"] {
        if !result
            .get(field)
            .and_then(Value::as_str)
            .is_some_and(|value| !value.is_empty())
        {
            return Err(RuntimeError::HandshakeFailed);
        }
    }
    Ok(())
}
