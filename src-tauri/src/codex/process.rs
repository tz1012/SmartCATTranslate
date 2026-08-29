use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

#[cfg(windows)]
use std::ffi::OsString;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, Command};
use tokio::time::timeout;

use crate::codex::runtime::{
    LiveRuntimeChannel, LiveRuntimeSession, RuntimeCandidate, RuntimeError, RuntimeLauncher,
};

pub const CODEX_APP_SERVER_PROTOCOL: &str = "codex-app-server-jsonl-v2";
pub const SMARTCAT_UPSTREAM_COMMIT: &str = "8c68d4c87dc54d38861f5114e920c3de2efa5876";
pub const SMARTCAT_PATCH_VERSION: &str = "smartcat-1";
const SMARTCAT_ATTESTATION_METHOD: &str = "smartcat/attestation";
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
    runtime_policy: Option<RuntimeSandboxPolicy>,
}

const TRANSLATION_PERMISSION_PROFILE: &str = "smartcat-translation";

#[derive(Clone, Debug)]
pub struct RuntimeSandboxPolicy {
    codex_home: PathBuf,
    work_root: PathBuf,
    executable: PathBuf,
}

impl RuntimeSandboxPolicy {
    pub fn new(
        codex_home: PathBuf,
        work_root: PathBuf,
        executable: PathBuf,
    ) -> Result<Self, RuntimeError> {
        if !codex_home.is_absolute() || !work_root.is_absolute() || !executable.is_absolute() {
            return Err(RuntimeError::FilesystemFailed);
        }
        Ok(Self {
            codex_home,
            work_root,
            executable,
        })
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ImportedChatGptAuth {
    auth_mode: ChatGptAuthMode,
    tokens: ImportedChatGptTokens,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_refresh: Option<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
enum ChatGptAuthMode {
    Chatgpt,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ImportedChatGptTokens {
    id_token: String,
    access_token: String,
    refresh_token: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    account_id: Option<String>,
}

fn canonicalize_chatgpt_auth(bytes: &[u8]) -> Result<Vec<u8>, RuntimeError> {
    let auth: ImportedChatGptAuth =
        serde_json::from_slice(bytes).map_err(|_| RuntimeError::FilesystemFailed)?;
    for token in [
        auth.tokens.id_token.as_str(),
        auth.tokens.access_token.as_str(),
        auth.tokens.refresh_token.as_str(),
    ] {
        if token.is_empty() || token.len() > 256 * 1024 || token.chars().any(char::is_control) {
            return Err(RuntimeError::FilesystemFailed);
        }
    }
    if auth.tokens.id_token.split('.').count() != 3 {
        return Err(RuntimeError::FilesystemFailed);
    }
    if auth.tokens.account_id.as_ref().is_some_and(|value| {
        value.is_empty() || value.len() > 512 || value.chars().any(char::is_control)
    }) {
        return Err(RuntimeError::FilesystemFailed);
    }
    if let Some(last_refresh) = &auth.last_refresh {
        if last_refresh.len() > 64 || chrono::DateTime::parse_from_rfc3339(last_refresh).is_err() {
            return Err(RuntimeError::FilesystemFailed);
        }
    }
    serde_json::to_vec(&auth).map_err(|_| RuntimeError::FilesystemFailed)
}

impl CodexAppServerConfig {
    pub fn tool_free(credential_store: CredentialStoreMode) -> Self {
        Self {
            credential_store,
            runtime_policy: None,
        }
    }

    pub fn with_runtime_policy(mut self, runtime_policy: RuntimeSandboxPolicy) -> Self {
        self.runtime_policy = Some(runtime_policy);
        self
    }

    pub fn to_toml(&self) -> Result<String, RuntimeError> {
        let rendered = format!(
            r#"allow_login_shell = false
approval_policy = "never"
check_for_update_on_startup = false
cli_auth_credentials_store = "{}"
developer_instructions = ""
file_opener = "none"
forced_login_method = "chatgpt"
include_apps_instructions = false
include_collaboration_mode_instructions = false
include_environment_context = false
include_permissions_instructions = false
instructions = ""
model_provider = "openai"
notify = []
project_doc_fallback_filenames = []
project_doc_max_bytes = 0
project_root_markers = []
sandbox_mode = "read-only"
web_search = "disabled"

[analytics]
enabled = false

[apps._default]
destructive_enabled = false
enabled = false
open_world_enabled = false

[features]
apply_patch_freeform = false
apps = false
auth_elicitation = false
browser_use = false
browser_use_external = false
browser_use_full_cdp_access = false
code_mode_host = false
codex_git_commit = false
codex_hooks = false
collab = false
collaboration_modes = false
computer_use = false
connectors = false
default_mode_request_user_input = false
deferred_executor = false
enable_fanout = false
enable_mcp_apps = false
exec_permission_approvals = false
experimental_use_unified_exec_tool = false
goals = false
guardian_approval = false
hooks = false
image_generation = false
imagegenext = false
in_app_browser = false
js_repl = false
js_repl_tools_only = false
memories = false
memory_tool = false
multi_agent = false
multi_agent_mode = false
plugin_hooks = false
plugin_sharing = false
plugins = false
remote_control = false
remote_plugin = false
request_permissions = false
request_permissions_tool = false
request_rule = false
search_tool = false
shell_snapshot = false
shell_tool = false
shell_zsh_fork = false
skill_env_var_dependency_prompt = false
skill_mcp_dependency_install = false
standalone_web_search = false
tool_call_mcp_elicitation = false
tool_search = false
tool_search_always_defer_mcp_tools = false
tool_suggest = false
unified_exec = false
unified_exec_zsh_fork = false
web_search = false
web_search_cached = false
web_search_request = false
workspace_dependencies = false

[feedback]
enabled = false

[history]
persistence = "none"

[hooks]

[marketplaces]

[mcp_servers]

[memories]
dedicated_tools = false
generate_memories = false
use_memories = false

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
include_instructions = false

[tool_suggest]
disabled_tools = []
discoverables = []
"#,
            self.credential_store.config_value()
        );
        let mut root =
            toml::from_str::<toml::Table>(&rendered).map_err(|_| RuntimeError::FilesystemFailed)?;
        root.insert("plugins".to_owned(), toml::Value::Table(toml::Table::new()));
        if let Some(policy) = &self.runtime_policy {
            let mut filesystem = toml::Table::new();
            filesystem.insert(
                ":minimal".to_owned(),
                toml::Value::String("read".to_owned()),
            );
            for (path, access) in [
                (&policy.codex_home, "write"),
                (&policy.work_root, "write"),
                (&policy.executable, "read"),
            ] {
                filesystem.insert(
                    path.to_string_lossy().into_owned(),
                    toml::Value::String(access.to_owned()),
                );
            }
            let mut network = toml::Table::new();
            network.insert("enabled".to_owned(), toml::Value::Boolean(true));
            let mut profile = toml::Table::new();
            profile.insert("filesystem".to_owned(), toml::Value::Table(filesystem));
            profile.insert("network".to_owned(), toml::Value::Table(network));
            let mut profiles = toml::Table::new();
            profiles.insert(
                TRANSLATION_PERMISSION_PROFILE.to_owned(),
                toml::Value::Table(profile),
            );
            root.insert(
                "default_permissions".to_owned(),
                toml::Value::String(TRANSLATION_PERMISSION_PROFILE.to_owned()),
            );
            root.insert("permissions".to_owned(), toml::Value::Table(profiles));
        }
        toml::to_string(&root).map_err(|_| RuntimeError::FilesystemFailed)
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

    fn prepare_isolated_home(&self, candidate: &RuntimeCandidate) -> Result<(), RuntimeError> {
        create_private_directory(&self.app_data_root)?;
        create_private_directory(&self.codex_home)?;
        create_private_directory(&self.work_root)?;

        let isolated_auth = self.codex_home.join("auth.json");
        if regular_private_file_exists(&isolated_auth)? {
            validate_auth_file(&isolated_auth)?;
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
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err(RuntimeError::FilesystemFailed),
            }
        }

        let credential_store = if regular_private_file_exists(&isolated_auth)? {
            CredentialStoreMode::File
        } else {
            CredentialStoreMode::Keyring
        };
        let sandbox_policy = RuntimeSandboxPolicy::new(
            self.codex_home.clone(),
            self.work_root.clone(),
            candidate.path().to_path_buf(),
        )?;
        let config = CodexAppServerConfig::tool_free(credential_store)
            .with_runtime_policy(sandbox_policy)
            .to_toml()?;
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
        let sandbox_root = sandboxed_app_data_root(&self.app_data_root)?;
        let isolated_launcher = Self {
            work_root: sandbox_root.join("runtime-work"),
            codex_home: sandbox_root.join("codex-home"),
            app_data_root: sandbox_root,
            credential_source: self.credential_source.clone(),
        };
        let launcher = &isolated_launcher;
        launcher.prepare_isolated_home(candidate)?;
        let process_temp = launcher.app_data_root.join("process-temp");
        create_private_directory(&process_temp)?;
        let workdir = tempfile::Builder::new()
            .prefix("session-")
            .tempdir_in(&launcher.work_root)
            .map_err(|_| RuntimeError::FilesystemFailed)?;

        #[cfg(any(windows, target_os = "macos"))]
        let (child, reader, writer) = {
            let mut command = Command::new(candidate.path());
            command
                .args(["app-server", "--listen", "stdio://"])
                .current_dir(workdir.path())
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .kill_on_drop(true)
                .env_clear()
                .envs(isolated_environment(&launcher.codex_home, &process_temp)?);
            let mut spawned = command.spawn().map_err(|_| RuntimeError::SpawnFailed)?;
            let writer = spawned.stdin.take().ok_or(RuntimeError::SpawnFailed)?;
            let reader = spawned.stdout.take().ok_or(RuntimeError::SpawnFailed)?;
            (
                ManagedProcess(spawned),
                Box::new(reader) as Box<dyn tokio::io::AsyncRead + Send + Unpin>,
                Box::new(writer) as Box<dyn tokio::io::AsyncWrite + Send + Unpin>,
            )
        };
        #[cfg(not(any(windows, target_os = "macos")))]
        return Err(RuntimeError::SandboxUnavailable);
        Ok(Box::new(ProcessRuntimeSession {
            child,
            reader: Some(reader),
            writer: Some(writer),
            _workdir: workdir,
            initialized: false,
        }))
    }
}

fn isolated_environment(
    codex_home: &Path,
    process_temp: &Path,
) -> Result<BTreeMap<std::ffi::OsString, std::ffi::OsString>, RuntimeError> {
    let mut environment = BTreeMap::new();
    environment.insert("CODEX_HOME".into(), codex_home.as_os_str().to_owned());
    #[cfg(windows)]
    {
        let windows = windows_directory()?;
        let system32 = windows.join("System32");
        environment.insert("SystemRoot".into(), windows.as_os_str().to_owned());
        environment.insert("WINDIR".into(), windows.as_os_str().to_owned());
        environment.insert("ComSpec".into(), system32.join("cmd.exe").into_os_string());
        environment.insert("PATH".into(), system32.into_os_string());
        environment.insert("USERPROFILE".into(), codex_home.as_os_str().to_owned());
        environment.insert("APPDATA".into(), codex_home.as_os_str().to_owned());
        environment.insert("LOCALAPPDATA".into(), codex_home.as_os_str().to_owned());
        environment.insert("TEMP".into(), process_temp.as_os_str().to_owned());
        environment.insert("TMP".into(), process_temp.as_os_str().to_owned());
    }
    #[cfg(target_os = "macos")]
    {
        environment.insert("HOME".into(), codex_home.as_os_str().to_owned());
        environment.insert("PATH".into(), "/usr/bin:/bin:/usr/sbin:/sbin".into());
        environment.insert("TMPDIR".into(), process_temp.as_os_str().to_owned());
        environment.insert("LANG".into(), "C.UTF-8".into());
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let _ = process_temp;
        return Err(RuntimeError::SandboxUnavailable);
    }
    Ok(environment)
}

#[cfg(windows)]
fn windows_directory() -> Result<PathBuf, RuntimeError> {
    use windows_sys::Win32::System::SystemInformation::GetWindowsDirectoryW;

    let mut buffer = vec![0_u16; 32_768];
    let length = unsafe { GetWindowsDirectoryW(buffer.as_mut_ptr(), buffer.len() as u32) };
    if length == 0 || length as usize >= buffer.len() {
        return Err(RuntimeError::SandboxUnavailable);
    }
    buffer.truncate(length as usize);
    let path =
        PathBuf::from(String::from_utf16(&buffer).map_err(|_| RuntimeError::SandboxUnavailable)?);
    reject_link_or_reparse_ancestors(&path)?;
    Ok(path)
}

pub fn sandboxed_app_data_root(requested_root: &Path) -> Result<PathBuf, RuntimeError> {
    #[cfg(windows)]
    {
        if !requested_root.is_absolute() {
            return Err(RuntimeError::FilesystemFailed);
        }
        reject_link_or_reparse_ancestors(requested_root)?;
        std::fs::create_dir_all(requested_root).map_err(|_| RuntimeError::FilesystemFailed)?;
        let canonical = requested_root
            .canonicalize()
            .map_err(|_| RuntimeError::FilesystemFailed)?;
        reject_link_or_reparse_ancestors(&canonical)?;
        Ok(canonical)
    }
    #[cfg(target_os = "macos")]
    {
        Ok(requested_root.to_path_buf())
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = requested_root;
        Err(RuntimeError::SandboxUnavailable)
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
    if bytes.len() as u64 != metadata.len() {
        return Err(RuntimeError::FilesystemFailed);
    }
    canonicalize_chatgpt_auth(&bytes)
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

struct ManagedProcess(Child);

impl ManagedProcess {
    async fn stop(&mut self) -> Result<(), RuntimeError> {
        if self
            .0
            .try_wait()
            .map_err(|_| RuntimeError::StopFailed)?
            .is_none()
        {
            self.0.start_kill().map_err(|_| RuntimeError::StopFailed)?;
        }
        self.0
            .wait()
            .await
            .map(|_| ())
            .map_err(|_| RuntimeError::StopFailed)
    }

    fn abort(&mut self) {
        let _ = self.0.start_kill();
    }
}

struct ProcessRuntimeSession {
    child: ManagedProcess,
    reader: Option<Box<dyn tokio::io::AsyncRead + Send + Unpin>>,
    writer: Option<Box<dyn tokio::io::AsyncWrite + Send + Unpin>>,
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
        self.child.stop().await
    }

    fn abort(&mut self) {
        self.writer.take();
        self.reader.take();
        self.child.abort();
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
            .map_err(|_| RuntimeError::HandshakeFailed)?;

        let attestation = serde_json::to_vec(&json!({
            "id": 1,
            "method": SMARTCAT_ATTESTATION_METHOD,
            "params": {}
        }))
        .map_err(|_| RuntimeError::HandshakeFailed)?;
        writer
            .write_all(&attestation)
            .await
            .map_err(|_| RuntimeError::HandshakeFailed)?;
        writer
            .write_all(b"\n")
            .await
            .map_err(|_| RuntimeError::HandshakeFailed)?;
        writer
            .flush()
            .await
            .map_err(|_| RuntimeError::HandshakeFailed)?;

        let response = read_smartcat_attestation_response(reader).await?;
        validate_smartcat_attestation(&response)
    }
}

async fn read_smartcat_attestation_response(
    reader: &mut (dyn tokio::io::AsyncRead + Send + Unpin),
) -> Result<Vec<u8>, RuntimeError> {
    const MAX_FRAMES: usize = 2;

    for _ in 0..MAX_FRAMES {
        let frame = read_handshake_line(reader).await?;
        if is_disabled_remote_control_status(&frame) {
            continue;
        }
        return Ok(frame);
    }
    Err(RuntimeError::HandshakeFailed)
}

fn is_disabled_remote_control_status(frame: &[u8]) -> bool {
    let Ok(Value::Object(message)) = serde_json::from_slice::<Value>(frame) else {
        return false;
    };
    if message.len() != 2
        || message.get("method").and_then(Value::as_str) != Some("remoteControl/status/changed")
    {
        return false;
    }
    let Some(Value::Object(params)) = message.get("params") else {
        return false;
    };
    params.len() == 4
        && params.get("status").and_then(Value::as_str) == Some("disabled")
        && params
            .get("serverName")
            .and_then(Value::as_str)
            .is_some_and(|value| !value.is_empty())
        && params
            .get("installationId")
            .and_then(Value::as_str)
            .is_some_and(|value| !value.is_empty())
        && params.get("environmentId").is_some_and(Value::is_null)
}

async fn read_handshake_line(
    reader: &mut (dyn tokio::io::AsyncRead + Send + Unpin),
) -> Result<Vec<u8>, RuntimeError> {
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

fn validate_smartcat_attestation(frame: &[u8]) -> Result<(), RuntimeError> {
    let value: Value = serde_json::from_slice(frame).map_err(|_| RuntimeError::HandshakeFailed)?;
    if value.get("id").and_then(Value::as_u64) != Some(1) || value.get("error").is_some() {
        return Err(RuntimeError::HandshakeFailed);
    }
    let result = value
        .get("result")
        .and_then(Value::as_object)
        .ok_or(RuntimeError::HandshakeFailed)?;
    if result.len() != 4
        || result.get("upstreamCommit").and_then(Value::as_str) != Some(SMARTCAT_UPSTREAM_COMMIT)
        || result.get("patchVersion").and_then(Value::as_str) != Some(SMARTCAT_PATCH_VERSION)
        || result.get("toolCount").and_then(Value::as_u64) != Some(0)
        || result.get("instructionDiscovery").and_then(Value::as_bool) != Some(false)
    {
        return Err(RuntimeError::HandshakeFailed);
    }
    Ok(())
}

#[cfg(test)]
mod security_tests {
    use super::{
        canonicalize_chatgpt_auth, CodexAppServerConfig, CredentialStoreMode, RuntimeSandboxPolicy,
    };
    use serde_json::json;
    use tempfile::tempdir;

    #[test]
    fn smartcat_attestation_rejects_stock_or_drifted_app_servers() {
        let expected = serde_json::json!({
            "id": 1,
            "result": {
                "upstreamCommit": "8c68d4c87dc54d38861f5114e920c3de2efa5876",
                "patchVersion": "smartcat-1",
                "toolCount": 0,
                "instructionDiscovery": false
            }
        });
        assert!(
            super::validate_smartcat_attestation(&serde_json::to_vec(&expected).unwrap()).is_ok()
        );

        for rejected in [
            serde_json::json!({"id": 1, "result": {}}),
            serde_json::json!({"id": 1, "result": {
                "upstreamCommit": "stock",
                "patchVersion": "smartcat-1",
                "toolCount": 0,
                "instructionDiscovery": false
            }}),
            serde_json::json!({"id": 1, "result": {
                "upstreamCommit": "8c68d4c87dc54d38861f5114e920c3de2efa5876",
                "patchVersion": "smartcat-1",
                "toolCount": 1,
                "instructionDiscovery": false
            }}),
        ] {
            assert!(
                super::validate_smartcat_attestation(&serde_json::to_vec(&rejected).unwrap())
                    .is_err()
            );
        }
    }

    #[test]
    fn only_the_exact_disabled_remote_control_lifecycle_notification_is_allowed() {
        let allowed = serde_json::json!({
            "method": "remoteControl/status/changed",
            "params": {
                "status": "disabled",
                "serverName": "test-host",
                "installationId": "test-installation",
                "environmentId": null
            }
        });
        assert!(super::is_disabled_remote_control_status(
            &serde_json::to_vec(&allowed).unwrap()
        ));

        for rejected in [
            serde_json::json!({
                "id": "server-request",
                "method": "remoteControl/status/changed",
                "params": allowed["params"].clone()
            }),
            serde_json::json!({
                "method": "remoteControl/status/changed",
                "params": {
                    "status": "enabled",
                    "serverName": "test-host",
                    "installationId": "test-installation",
                    "environmentId": null
                }
            }),
            serde_json::json!({
                "method": "another/notification",
                "params": allowed["params"].clone()
            }),
        ] {
            assert!(!super::is_disabled_remote_control_status(
                &serde_json::to_vec(&rejected).unwrap()
            ));
        }
    }

    #[cfg(windows)]
    #[test]
    fn pinned_binary_parses_the_generated_config_without_inheriting_host_environment() {
        use crate::codex::runtime::RuntimeCandidate;
        use semver::Version;

        let Some(binary) = std::env::var_os("SMARTCAT_PINNED_CODEX_0_144_4") else {
            return;
        };
        let binary = std::path::PathBuf::from(binary);
        let root = tempdir().unwrap();
        let launcher =
            super::ProcessRuntimeLauncher::with_credential_source(root.path().to_path_buf(), None);
        let candidate = RuntimeCandidate::system(&binary, Version::parse("0.144.4").unwrap());
        launcher.prepare_isolated_home(&candidate).unwrap();
        let process_temp = root.path().join("process-temp");
        std::fs::create_dir(&process_temp).unwrap();
        let environment = super::isolated_environment(&launcher.codex_home, &process_temp).unwrap();

        let status = std::process::Command::new(binary)
            .args(["features", "list"])
            .current_dir(&launcher.work_root)
            .env_clear()
            .envs(environment)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();

        assert!(status.success(), "pinned runtime rejected generated config");
    }

    #[test]
    fn generated_config_is_valid_against_the_pinned_0_144_4_schema() {
        let root = tempdir().unwrap();
        let policy = RuntimeSandboxPolicy::new(
            root.path().join("codex-home"),
            root.path().join("runtime-work"),
            root.path().join("codex.exe"),
        )
        .unwrap();
        let rendered = CodexAppServerConfig::tool_free(CredentialStoreMode::File)
            .with_runtime_policy(policy)
            .to_toml()
            .unwrap();
        let toml: toml::Value = toml::from_str(&rendered).unwrap();
        let instance = serde_json::to_value(toml).unwrap();
        let schema: serde_json::Value = serde_json::from_str(include_str!(
            "../../resources/codex-0.144.4-config.schema.json"
        ))
        .unwrap();
        let validator = jsonschema::validator_for(&schema).unwrap();

        if let Err(error) = validator.validate(&instance) {
            panic!("generated 0.144.4 configuration is invalid: {error}");
        }
        assert_eq!(instance["default_permissions"], "smartcat-translation");
        assert_eq!(
            instance["permissions"]["smartcat-translation"]["filesystem"][":minimal"],
            "read"
        );
        assert!(instance["mcp_servers"].as_object().unwrap().is_empty());
        assert!(instance.get("agents").is_none());
        assert!(instance.get("tools").is_none());
    }

    #[test]
    fn chatgpt_credential_import_accepts_only_documented_fields_and_canonicalizes() {
        let source = json!({
            "auth_mode": "chatgpt",
            "tokens": {
                "id_token": "header.payload.signature",
                "access_token": "access-token",
                "refresh_token": "refresh-token",
                "account_id": "account-1"
            },
            "last_refresh": "2026-08-28T12:00:00Z"
        });

        let canonical = canonicalize_chatgpt_auth(&serde_json::to_vec(&source).unwrap()).unwrap();

        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&canonical).unwrap(),
            source
        );
    }

    #[test]
    fn chatgpt_credential_import_rejects_extra_or_non_chatgpt_fields() {
        let invalid = [
            json!({
                "auth_mode": "chatgpt",
                "tokens": {
                    "id_token": "header.payload.signature",
                    "access_token": "access",
                    "refresh_token": "refresh",
                    "unexpected": "secret"
                }
            }),
            json!({
                "auth_mode": "apikey",
                "OPENAI_API_KEY": "secret",
                "tokens": {
                    "id_token": "header.payload.signature",
                    "access_token": "access",
                    "refresh_token": "refresh"
                }
            }),
            json!({
                "auth_mode": "chatgpt",
                "tokens": {
                    "id_token": "header.payload.signature",
                    "access_token": "access",
                    "refresh_token": "refresh"
                },
                "agent_identity": {"token": "secret"}
            }),
        ];

        for value in invalid {
            assert!(canonicalize_chatgpt_auth(&serde_json::to_vec(&value).unwrap()).is_err());
        }
    }

    #[test]
    fn isolated_home_selects_keyring_only_when_no_private_auth_file_exists() {
        use crate::codex::runtime::RuntimeCandidate;
        use semver::Version;

        let root = tempdir().unwrap();
        let executable = std::env::current_exe().unwrap();
        let launcher =
            super::ProcessRuntimeLauncher::with_credential_source(root.path().to_path_buf(), None);
        let candidate = RuntimeCandidate::system(executable, Version::parse("0.144.4").unwrap());

        launcher.prepare_isolated_home(&candidate).unwrap();

        let config = std::fs::read_to_string(launcher.codex_home.join("config.toml")).unwrap();
        let parsed: toml::Value = toml::from_str(&config).unwrap();
        assert_eq!(
            parsed["cli_auth_credentials_store"].as_str(),
            Some("keyring")
        );
        assert!(!launcher.codex_home.join("auth.json").exists());
    }
}
