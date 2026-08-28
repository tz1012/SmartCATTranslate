use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

#[cfg(windows)]
use std::ffi::OsString;

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
const MAX_AUTH_BYTES: u64 = 1024 * 1024;
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CredentialStoreMode {
    File,
    Keyring,
}

impl CredentialStoreMode {
    fn config_value(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Keyring => "keyring",
        }
    }
}

/// Complete app-owned configuration for the translation-only App Server.
pub struct CodexAppServerConfig {
    credential_store: CredentialStoreMode,
}

impl CodexAppServerConfig {
    pub fn tool_free(credential_store: CredentialStoreMode) -> Self {
        Self { credential_store }
    }

    pub fn to_toml(&self) -> Result<String, RuntimeError> {
        let rendered = format!(
            r#"allow_login_shell = false
approval_policy = "never"
check_for_update_on_startup = false
cli_auth_credentials_store = "{}"
file_opener = "none"
forced_login_method = "chatgpt"
model_provider = "openai"
notify = []
project_doc_fallback_filenames = []
project_doc_max_bytes = 0
project_root_markers = []
sandbox_mode = "read-only"
web_search = "disabled"

[agents]
enabled = false

[analytics]
enabled = false

[apps._default]
destructive_enabled = false
enabled = false
open_world_enabled = false

[computer_use.windows]
always_allowed_app_ids = []

[features]
apps = false
fast_mode = false
goals = false
hooks = false
memories = false
multi_agent = false
personality = false
remote_plugin = false
shell_snapshot = false
shell_tool = false
skill_mcp_dependency_install = false
unified_exec = false

[feedback]
enabled = false

[history]
persistence = "none"

[hooks]

[mcp_servers]

[otel]
exporter = "none"
log_user_prompt = false
metrics_exporter = "none"
trace_exporter = "none"

[shell_environment_policy]
ignore_default_excludes = false
inherit = "none"

[skills]
config = []

[tool_suggest]
disabled_tools = []
discoverables = []

[tools]
view_image = false
web_search = false
"#,
            self.credential_store.config_value()
        );
        toml::from_str::<toml::Value>(&rendered).map_err(|_| RuntimeError::FilesystemFailed)?;
        Ok(rendered)
    }
}

pub struct ProcessRuntimeLauncher {
    app_data_root: PathBuf,
    work_root: PathBuf,
    codex_home: PathBuf,
    credential_source: Option<PathBuf>,
}

impl ProcessRuntimeLauncher {
    pub fn new(app_data_root: PathBuf) -> Self {
        let credential_source = default_credential_source();
        Self::with_credential_source(app_data_root, credential_source)
    }

    pub fn with_credential_source(
        app_data_root: PathBuf,
        credential_source: Option<PathBuf>,
    ) -> Self {
        Self {
            work_root: app_data_root.join("runtime-work"),
            codex_home: app_data_root.join("codex-home"),
            app_data_root,
            credential_source,
        }
    }

    fn prepare_isolated_home(&self) -> Result<(), RuntimeError> {
        create_private_directory(&self.app_data_root)?;
        create_private_directory(&self.codex_home)?;
        create_private_directory(&self.work_root)?;

        let isolated_auth = self.codex_home.join("auth.json");
        let credential_store = if regular_private_file_exists(&isolated_auth)? {
            validate_auth_file(&isolated_auth)?;
            CredentialStoreMode::File
        } else if let Some(source_home) = &self.credential_source {
            let source_auth = source_home.join("auth.json");
            match std::fs::symlink_metadata(&source_auth) {
                Ok(metadata) => {
                    reject_link_or_reparse_ancestors(&source_auth)?;
                    if !metadata.is_file() || is_link_or_reparse(&metadata) {
                        return Err(RuntimeError::FilesystemFailed);
                    }
                    let bytes = read_bounded_auth(&source_auth, &metadata)?;
                    atomic_write_private(&isolated_auth, &bytes, false)?;
                    CredentialStoreMode::File
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    CredentialStoreMode::Keyring
                }
                Err(_) => return Err(RuntimeError::FilesystemFailed),
            }
        } else {
            CredentialStoreMode::Keyring
        };

        let config = CodexAppServerConfig::tool_free(credential_store).to_toml()?;
        atomic_write_private(
            &self.codex_home.join("config.toml"),
            config.as_bytes(),
            true,
        )
    }
}

#[async_trait]
impl RuntimeLauncher for ProcessRuntimeLauncher {
    async fn start(
        &self,
        candidate: &RuntimeCandidate,
    ) -> Result<Box<dyn LiveRuntimeSession>, RuntimeError> {
        self.prepare_isolated_home()?;
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
            .env_remove("CODEX_API_KEY")
            .env_remove("CODEX_ACCESS_TOKEN")
            .env_remove("CODEX_SQLITE_HOME")
            .env_remove("OPENAI_BASE_URL")
            .env_remove("OPENAI_ORG_ID")
            .env_remove("OPENAI_PROJECT_ID")
            .env("CODEX_HOME", &self.codex_home);
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

fn default_credential_source() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("CODEX_HOME") {
        return Some(PathBuf::from(path));
    }
    #[cfg(windows)]
    let home = std::env::var_os("USERPROFILE");
    #[cfg(not(windows))]
    let home = std::env::var_os("HOME");
    home.map(PathBuf::from).map(|path| path.join(".codex"))
}

fn create_private_directory(path: &Path) -> Result<(), RuntimeError> {
    reject_link_or_reparse_ancestors(path)?;
    std::fs::create_dir_all(path).map_err(|_| RuntimeError::FilesystemFailed)?;
    reject_link_or_reparse_ancestors(path)?;
    let metadata = std::fs::symlink_metadata(path).map_err(|_| RuntimeError::FilesystemFailed)?;
    if !metadata.is_dir() || is_link_or_reparse(&metadata) {
        return Err(RuntimeError::FilesystemFailed);
    }
    set_private_permissions(path, true)
}

fn regular_private_file_exists(path: &Path) -> Result<bool, RuntimeError> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => {
            reject_link_or_reparse_ancestors(path)?;
            if !metadata.is_file() || is_link_or_reparse(&metadata) {
                return Err(RuntimeError::FilesystemFailed);
            }
            set_private_permissions(path, false)?;
            Ok(true)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(_) => Err(RuntimeError::FilesystemFailed),
    }
}

fn validate_auth_file(path: &Path) -> Result<(), RuntimeError> {
    let metadata = std::fs::symlink_metadata(path).map_err(|_| RuntimeError::FilesystemFailed)?;
    let _ = read_bounded_auth(path, &metadata)?;
    Ok(())
}

fn read_bounded_auth(path: &Path, metadata: &std::fs::Metadata) -> Result<Vec<u8>, RuntimeError> {
    if metadata.len() == 0 || metadata.len() > MAX_AUTH_BYTES {
        return Err(RuntimeError::FilesystemFailed);
    }
    let bytes = std::fs::read(path).map_err(|_| RuntimeError::FilesystemFailed)?;
    if bytes.len() as u64 != metadata.len()
        || !serde_json::from_slice::<Value>(&bytes)
            .ok()
            .is_some_and(|value| value.is_object())
    {
        return Err(RuntimeError::FilesystemFailed);
    }
    Ok(bytes)
}

fn atomic_write_private(path: &Path, bytes: &[u8], replace: bool) -> Result<(), RuntimeError> {
    let parent = path.parent().ok_or(RuntimeError::FilesystemFailed)?;
    reject_link_or_reparse_ancestors(parent)?;
    if let Ok(metadata) = std::fs::symlink_metadata(path) {
        if !metadata.is_file() || is_link_or_reparse(&metadata) {
            return Err(RuntimeError::FilesystemFailed);
        }
        if !replace {
            return Err(RuntimeError::FilesystemFailed);
        }
    }
    let temporary = parent.join(format!(".smartcat-{}.tmp", uuid::Uuid::new_v4()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&temporary)
        .map_err(|_| RuntimeError::FilesystemFailed)?;
    let result = (|| {
        file.write_all(bytes)
            .map_err(|_| RuntimeError::FilesystemFailed)?;
        file.sync_all()
            .map_err(|_| RuntimeError::FilesystemFailed)?;
        drop(file);
        set_private_permissions(&temporary, false)?;
        atomic_move(&temporary, path, replace)?;
        set_private_permissions(path, false)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

#[cfg(windows)]
fn atomic_move(source: &Path, destination: &Path, replace: bool) -> Result<(), RuntimeError> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    let flags = MOVEFILE_WRITE_THROUGH
        | if replace {
            MOVEFILE_REPLACE_EXISTING
        } else {
            0
        };
    if unsafe { MoveFileExW(source.as_ptr(), destination.as_ptr(), flags) } == 0 {
        return Err(RuntimeError::FilesystemFailed);
    }
    Ok(())
}

#[cfg(unix)]
fn atomic_move(source: &Path, destination: &Path, replace: bool) -> Result<(), RuntimeError> {
    if !replace && destination.exists() {
        return Err(RuntimeError::FilesystemFailed);
    }
    std::fs::rename(source, destination).map_err(|_| RuntimeError::FilesystemFailed)
}

fn reject_link_or_reparse_ancestors(path: &Path) -> Result<(), RuntimeError> {
    for ancestor in path.ancestors() {
        match std::fs::symlink_metadata(ancestor) {
            Ok(metadata) if is_link_or_reparse(&metadata) => {
                return Err(RuntimeError::FilesystemFailed)
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(RuntimeError::FilesystemFailed),
        }
    }
    Ok(())
}

#[cfg(windows)]
fn is_link_or_reparse(metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    metadata.file_type().is_symlink() || metadata.file_attributes() & 0x400 != 0
}

#[cfg(unix)]
fn is_link_or_reparse(metadata: &std::fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

#[cfg(unix)]
fn set_private_permissions(path: &Path, directory: bool) -> Result<(), RuntimeError> {
    use std::os::unix::fs::PermissionsExt;

    let mode = if directory { 0o700 } else { 0o600 };
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
        .map_err(|_| RuntimeError::FilesystemFailed)
}

#[cfg(windows)]
fn set_private_permissions(path: &Path, directory: bool) -> Result<(), RuntimeError> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Authorization::{
        ConvertStringSecurityDescriptorToSecurityDescriptorW, SetNamedSecurityInfoW,
        SDDL_REVISION_1, SE_FILE_OBJECT,
    };
    use windows_sys::Win32::Security::{
        GetSecurityDescriptorDacl, DACL_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION,
        PSECURITY_DESCRIPTOR,
    };

    let descriptor_text = if directory {
        "D:P(A;OICI;FA;;;OW)(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)"
    } else {
        "D:P(A;;FA;;;OW)(A;;FA;;;SY)(A;;FA;;;BA)"
    };
    let descriptor_text: Vec<u16> = OsString::from(descriptor_text)
        .encode_wide()
        .chain(Some(0))
        .collect();
    let mut descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            descriptor_text.as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            std::ptr::null_mut(),
        )
    } == 0
    {
        return Err(RuntimeError::FilesystemFailed);
    }
    let mut present = 0;
    let mut defaulted = 0;
    let mut dacl = std::ptr::null_mut();
    let dacl_ok =
        unsafe { GetSecurityDescriptorDacl(descriptor, &mut present, &mut dacl, &mut defaulted) }
            != 0
            && present != 0;
    let mut path_wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let set_result = if dacl_ok {
        unsafe {
            SetNamedSecurityInfoW(
                path_wide.as_mut_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                dacl,
                std::ptr::null_mut(),
            )
        }
    } else {
        1
    };
    unsafe {
        LocalFree(descriptor);
    }
    if !dacl_ok || set_result != 0 {
        return Err(RuntimeError::FilesystemFailed);
    }
    Ok(())
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
