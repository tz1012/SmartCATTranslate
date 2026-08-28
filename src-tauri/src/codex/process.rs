use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
#[cfg(target_os = "macos")]
use std::process::Stdio;
use std::time::Duration;

#[cfg(windows)]
use std::ffi::OsString;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
#[cfg(target_os = "macos")]
use tokio::process::{Child, Command};
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
        #[cfg(windows)]
        let sandbox_executable =
            stage_windows_sandbox_executable(candidate.path(), &launcher.app_data_root)?;
        let process_temp = launcher.app_data_root.join("process-temp");
        create_private_directory(&process_temp)?;
        let workdir = tempfile::Builder::new()
            .prefix("session-")
            .tempdir_in(&launcher.work_root)
            .map_err(|_| RuntimeError::FilesystemFailed)?;

        #[cfg(windows)]
        let (child, reader, writer) = {
            let arguments = vec![
                OsString::from("app-server"),
                OsString::from("--listen"),
                OsString::from("stdio://"),
            ];
            let environment = isolated_environment(&launcher.codex_home, &process_temp)?;
            let config_file = launcher.codex_home.join("config.toml");
            let mut grants = vec![
                (launcher.app_data_root.as_path(), WindowsGrant::Write),
                (launcher.codex_home.as_path(), WindowsGrant::Write),
                (launcher.work_root.as_path(), WindowsGrant::Write),
                (process_temp.as_path(), WindowsGrant::Write),
                (sandbox_executable.as_path(), WindowsGrant::ReadExecute),
                (config_file.as_path(), WindowsGrant::ReadExecute),
            ];
            let auth_file = launcher.codex_home.join("auth.json");
            if auth_file.exists() {
                grants.push((auth_file.as_path(), WindowsGrant::Write));
            }
            let mut spawned = spawn_windows_appcontainer_command(
                sandbox_executable.as_path(),
                &arguments,
                workdir.path(),
                &environment,
                grants.as_slice(),
            )?;
            let reader = spawned.stdout.take().ok_or(RuntimeError::SpawnFailed)?;
            let writer = spawned.stdin.take().ok_or(RuntimeError::SpawnFailed)?;
            (
                ManagedProcess::AppContainer(spawned),
                Box::new(reader) as Box<dyn tokio::io::AsyncRead + Send + Unpin>,
                Box::new(writer) as Box<dyn tokio::io::AsyncWrite + Send + Unpin>,
            )
        };
        #[cfg(target_os = "macos")]
        let (child, reader, writer) = {
            let mut command = Command::new(candidate.path());
            command
                .args(sandbox_arguments(candidate.path(), workdir.path())?)
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
                ManagedProcess::Tokio(spawned),
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

#[cfg(windows)]
fn stage_windows_sandbox_executable(
    source: &Path,
    app_data_root: &Path,
) -> Result<PathBuf, RuntimeError> {
    const MAX_RUNTIME_BYTES: u64 = 512 * 1024 * 1024;

    reject_link_or_reparse_ancestors(source)?;
    let metadata = std::fs::symlink_metadata(source).map_err(|_| RuntimeError::FilesystemFailed)?;
    if !metadata.is_file()
        || is_link_or_reparse(&metadata)
        || metadata.len() == 0
        || metadata.len() > MAX_RUNTIME_BYTES
    {
        return Err(RuntimeError::FilesystemFailed);
    }
    let runtime_dir = app_data_root.join("runtime-bin");
    create_private_directory(&runtime_dir)?;
    let destination = runtime_dir.join("codex-app-server.exe");
    let temporary = runtime_dir.join(format!(".smartcat-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let copied =
            std::fs::copy(source, &temporary).map_err(|_| RuntimeError::FilesystemFailed)?;
        if copied != metadata.len() {
            return Err(RuntimeError::FilesystemFailed);
        }
        let file = OpenOptions::new()
            .write(true)
            .open(&temporary)
            .map_err(|_| RuntimeError::FilesystemFailed)?;
        file.sync_all()
            .map_err(|_| RuntimeError::FilesystemFailed)?;
        drop(file);
        set_private_permissions(&temporary, false)?;
        atomic_move(&temporary, &destination, true)?;
        set_private_permissions(&destination, false)?;
        Ok(destination.clone())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

#[cfg(target_os = "macos")]
fn sandbox_arguments(
    executable: &Path,
    workdir: &Path,
) -> Result<Vec<std::ffi::OsString>, RuntimeError> {
    Ok(vec![
        "sandbox".into(),
        "--permission-profile".into(),
        TRANSLATION_PERMISSION_PROFILE.into(),
        "--cd".into(),
        workdir.as_os_str().to_owned(),
        "--".into(),
        executable.as_os_str().to_owned(),
        "app-server".into(),
        "--listen".into(),
        "stdio://".into(),
    ])
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

#[cfg(all(windows, test))]
fn windows_command_interpreter() -> Result<PathBuf, RuntimeError> {
    Ok(windows_directory()?.join("System32").join("cmd.exe"))
}

pub fn sandboxed_app_data_root(requested_root: &Path) -> Result<PathBuf, RuntimeError> {
    #[cfg(windows)]
    {
        use sha2::{Digest, Sha256};

        if !requested_root.is_absolute() {
            return Err(RuntimeError::FilesystemFailed);
        }
        reject_link_or_reparse_ancestors(requested_root)?;
        let canonical = requested_root
            .canonicalize()
            .map_err(|_| RuntimeError::FilesystemFailed)?;
        let digest = Sha256::digest(canonical.as_os_str().to_string_lossy().as_bytes());
        let instance = digest
            .iter()
            .take(16)
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        windows_appcontainer_storage_root()
            .map(|root| root.join("SmartCATTranslate").join(instance))
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

#[cfg(windows)]
fn windows_appcontainer_storage_root() -> Result<PathBuf, RuntimeError> {
    static STORAGE_ROOT: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    if let Some(root) = STORAGE_ROOT.get() {
        return Ok(root.clone());
    }
    let discovered = discover_windows_appcontainer_storage_root()?;
    let _ = STORAGE_ROOT.set(discovered);
    STORAGE_ROOT
        .get()
        .cloned()
        .ok_or(RuntimeError::SandboxUnavailable)
}

#[cfg(windows)]
fn discover_windows_appcontainer_storage_root() -> Result<PathBuf, RuntimeError> {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW;
    use windows_sys::Win32::Security::Isolation::{
        CreateAppContainerProfile, DeriveAppContainerSidFromAppContainerName,
        GetAppContainerFolderPath,
    };
    use windows_sys::Win32::Security::{FreeSid, PSID};
    use windows_sys::Win32::System::Com::CoTaskMemFree;

    let _profile_guard = lock_appcontainer_profile();
    let profile_name = wide_nul("SmartCATTranslate.Codex.0_144_4");
    let display_name = wide_nul("SmartCAT Translate Codex Runtime");
    let description = wide_nul("Isolated translation runtime");
    let mut app_sid: PSID = std::ptr::null_mut();
    let mut result =
        unsafe { DeriveAppContainerSidFromAppContainerName(profile_name.as_ptr(), &mut app_sid) };
    if result < 0 {
        result = unsafe {
            CreateAppContainerProfile(
                profile_name.as_ptr(),
                display_name.as_ptr(),
                description.as_ptr(),
                std::ptr::null(),
                0,
                &mut app_sid,
            )
        };
    }
    if result < 0 || app_sid.is_null() {
        return Err(RuntimeError::SandboxUnavailable);
    }
    let mut sid_string = std::ptr::null_mut();
    if unsafe { ConvertSidToStringSidW(app_sid, &mut sid_string) } == 0 || sid_string.is_null() {
        unsafe { FreeSid(app_sid) };
        return Err(RuntimeError::SandboxUnavailable);
    }
    let sid_length = unsafe {
        let mut length = 0;
        while *sid_string.add(length) != 0 {
            length += 1;
        }
        length
    };
    let sid = unsafe { std::slice::from_raw_parts(sid_string, sid_length) }.to_vec();
    unsafe {
        LocalFree(sid_string.cast());
        FreeSid(app_sid);
    }
    let mut folder = std::ptr::null_mut();
    let result = unsafe { GetAppContainerFolderPath(sid.as_ptr(), &mut folder) };
    if result < 0 || folder.is_null() {
        return Err(RuntimeError::SandboxUnavailable);
    }
    let length = unsafe {
        let mut length = 0;
        while *folder.add(length) != 0 {
            length += 1;
        }
        length
    };
    let value = String::from_utf16(unsafe { std::slice::from_raw_parts(folder, length) })
        .map(PathBuf::from)
        .map_err(|_| RuntimeError::SandboxUnavailable);
    unsafe { CoTaskMemFree(folder.cast()) };
    value
}

#[cfg(windows)]
fn lock_appcontainer_profile() -> std::sync::MutexGuard<'static, ()> {
    static PROFILE_LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    PROFILE_LOCK
        .get_or_init(|| std::sync::Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(windows)]
#[derive(Clone, Copy)]
enum WindowsGrant {
    Write,
    ReadExecute,
}

#[cfg(windows)]
struct WindowsAppContainerChild {
    process: std::os::windows::io::OwnedHandle,
    stdin: Option<tokio::fs::File>,
    stdout: Option<tokio::fs::File>,
    finished: bool,
}

#[cfg(windows)]
impl WindowsAppContainerChild {
    async fn wait(&mut self) -> Result<u32, RuntimeError> {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Foundation::{WAIT_FAILED, WAIT_OBJECT_0};
        use windows_sys::Win32::System::Threading::{
            GetExitCodeProcess, WaitForSingleObject, INFINITE,
        };

        let process = self.process.as_raw_handle() as usize;
        let exit_code = tokio::task::spawn_blocking(move || unsafe {
            let process = process as windows_sys::Win32::Foundation::HANDLE;
            let wait = WaitForSingleObject(process, INFINITE);
            if wait == WAIT_FAILED || wait != WAIT_OBJECT_0 {
                return Err(RuntimeError::StopFailed);
            }
            let mut exit_code = 0_u32;
            if GetExitCodeProcess(process, &mut exit_code) == 0 {
                return Err(RuntimeError::StopFailed);
            }
            Ok(exit_code)
        })
        .await
        .map_err(|_| RuntimeError::StopFailed)??;
        self.finished = true;
        Ok(exit_code)
    }

    async fn stop(&mut self) -> Result<(), RuntimeError> {
        self.stdin.take();
        self.stdout.take();
        self.abort();
        self.wait().await.map(|_| ())
    }

    fn abort(&mut self) {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::System::Threading::{GetExitCodeProcess, TerminateProcess};

        if self.finished {
            return;
        }
        let process = self.process.as_raw_handle() as windows_sys::Win32::Foundation::HANDLE;
        let mut exit_code = 0_u32;
        unsafe {
            if GetExitCodeProcess(process, &mut exit_code) != 0 && exit_code == 259 {
                let _ = TerminateProcess(process, 1);
            }
        }
    }
}

#[cfg(windows)]
impl Drop for WindowsAppContainerChild {
    fn drop(&mut self) {
        self.abort();
    }
}

#[cfg(windows)]
fn spawn_windows_appcontainer_command(
    executable: &Path,
    arguments: &[OsString],
    cwd: &Path,
    environment: &BTreeMap<OsString, OsString>,
    grants: &[(&Path, WindowsGrant)],
) -> Result<WindowsAppContainerChild, RuntimeError> {
    use std::mem::size_of;
    use std::os::windows::io::{FromRawHandle, OwnedHandle};
    use windows_sys::Win32::Foundation::{
        CloseHandle, SetHandleInformation, HANDLE, HANDLE_FLAG_INHERIT, INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::Security::Isolation::DeriveAppContainerSidFromAppContainerName;
    use windows_sys::Win32::Security::{
        CreateWellKnownSid, FreeSid, WinCapabilityInternetClientSid, PSID, SECURITY_ATTRIBUTES,
        SECURITY_CAPABILITIES, SID_AND_ATTRIBUTES,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };
    use windows_sys::Win32::System::Pipes::CreatePipe;
    use windows_sys::Win32::System::SystemServices::SE_GROUP_ENABLED;
    use windows_sys::Win32::System::Threading::{
        CreateProcessW, DeleteProcThreadAttributeList, InitializeProcThreadAttributeList,
        UpdateProcThreadAttribute, CREATE_NO_WINDOW, CREATE_UNICODE_ENVIRONMENT,
        EXTENDED_STARTUPINFO_PRESENT, PROCESS_INFORMATION,
        PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES, STARTF_USESTDHANDLES, STARTUPINFOEXW,
    };

    if !executable.is_absolute() || !cwd.is_absolute() {
        return Err(RuntimeError::SandboxUnavailable);
    }
    reject_link_or_reparse_ancestors(executable)?;
    reject_link_or_reparse_ancestors(cwd)?;

    let _profile_guard = lock_appcontainer_profile();
    let profile_name = wide_nul("SmartCATTranslate.Codex.0_144_4");
    let mut app_sid: PSID = std::ptr::null_mut();
    let result =
        unsafe { DeriveAppContainerSidFromAppContainerName(profile_name.as_ptr(), &mut app_sid) };
    if result < 0 || app_sid.is_null() {
        return Err(RuntimeError::SandboxUnavailable);
    }
    struct SidGuard(PSID);
    impl Drop for SidGuard {
        fn drop(&mut self) {
            unsafe {
                FreeSid(self.0);
            }
        }
    }
    let app_sid = SidGuard(app_sid);

    for (path, grant) in grants {
        grant_appcontainer_path(path, app_sid.0, *grant)?;
    }

    let mut internet_sid_bytes = 0_u32;
    unsafe {
        CreateWellKnownSid(
            WinCapabilityInternetClientSid,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut internet_sid_bytes,
        );
    }
    if internet_sid_bytes == 0 || internet_sid_bytes > 1024 {
        return Err(RuntimeError::SandboxUnavailable);
    }
    let mut internet_sid =
        vec![0_usize; (internet_sid_bytes as usize).div_ceil(size_of::<usize>())];
    if unsafe {
        CreateWellKnownSid(
            WinCapabilityInternetClientSid,
            std::ptr::null_mut(),
            internet_sid.as_mut_ptr().cast(),
            &mut internet_sid_bytes,
        )
    } == 0
    {
        return Err(RuntimeError::SandboxUnavailable);
    }
    let mut capability = SID_AND_ATTRIBUTES {
        Sid: internet_sid.as_mut_ptr().cast(),
        Attributes: SE_GROUP_ENABLED as u32,
    };
    let mut security_capabilities = SECURITY_CAPABILITIES {
        AppContainerSid: app_sid.0,
        Capabilities: &mut capability,
        CapabilityCount: 1,
        Reserved: 0,
    };

    let security_attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: std::ptr::null_mut(),
        bInheritHandle: 1,
    };
    let mut stdout_read: HANDLE = std::ptr::null_mut();
    let mut stdout_write: HANDLE = std::ptr::null_mut();
    let mut stdin_read: HANDLE = std::ptr::null_mut();
    let mut stdin_write: HANDLE = std::ptr::null_mut();
    if unsafe { CreatePipe(&mut stdout_read, &mut stdout_write, &security_attributes, 0) } == 0
        || unsafe { CreatePipe(&mut stdin_read, &mut stdin_write, &security_attributes, 0) } == 0
    {
        unsafe {
            close_if_valid(stdout_read);
            close_if_valid(stdout_write);
            close_if_valid(stdin_read);
            close_if_valid(stdin_write);
        }
        return Err(RuntimeError::SandboxUnavailable);
    }
    let stdout_read = unsafe { OwnedHandle::from_raw_handle(stdout_read.cast()) };
    let stdout_write = unsafe { OwnedHandle::from_raw_handle(stdout_write.cast()) };
    let stdin_read = unsafe { OwnedHandle::from_raw_handle(stdin_read.cast()) };
    let stdin_write = unsafe { OwnedHandle::from_raw_handle(stdin_write.cast()) };
    use std::os::windows::io::AsRawHandle;
    if unsafe { SetHandleInformation(stdout_read.as_raw_handle().cast(), HANDLE_FLAG_INHERIT, 0) }
        == 0
        || unsafe {
            SetHandleInformation(stdin_write.as_raw_handle().cast(), HANDLE_FLAG_INHERIT, 0)
        } == 0
    {
        return Err(RuntimeError::SandboxUnavailable);
    }

    let nul = wide_nul("NUL");
    let stderr = unsafe {
        CreateFileW(
            nul.as_ptr(),
            windows_sys::Win32::Foundation::GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            &security_attributes,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            std::ptr::null_mut(),
        )
    };
    if stderr == INVALID_HANDLE_VALUE {
        return Err(RuntimeError::SandboxUnavailable);
    }
    let stderr = unsafe { OwnedHandle::from_raw_handle(stderr.cast()) };

    let mut attribute_bytes = 0_usize;
    unsafe {
        InitializeProcThreadAttributeList(std::ptr::null_mut(), 1, 0, &mut attribute_bytes);
    }
    if attribute_bytes == 0 || attribute_bytes > 1024 * 1024 {
        return Err(RuntimeError::SandboxUnavailable);
    }
    let mut attribute_storage = vec![0_usize; attribute_bytes.div_ceil(size_of::<usize>())];
    let attribute_list = attribute_storage.as_mut_ptr().cast();
    if unsafe { InitializeProcThreadAttributeList(attribute_list, 1, 0, &mut attribute_bytes) } == 0
    {
        return Err(RuntimeError::SandboxUnavailable);
    }
    struct AttributeListGuard(*mut core::ffi::c_void);
    impl Drop for AttributeListGuard {
        fn drop(&mut self) {
            unsafe { DeleteProcThreadAttributeList(self.0) }
        }
    }
    let _attribute_guard = AttributeListGuard(attribute_list);
    if unsafe {
        UpdateProcThreadAttribute(
            attribute_list,
            0,
            PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES as usize,
            (&mut security_capabilities as *mut SECURITY_CAPABILITIES).cast(),
            size_of::<SECURITY_CAPABILITIES>(),
            std::ptr::null_mut(),
            std::ptr::null(),
        )
    } == 0
    {
        return Err(RuntimeError::SandboxUnavailable);
    }

    let mut startup = STARTUPINFOEXW::default();
    startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    startup.StartupInfo.hStdInput = stdin_read.as_raw_handle().cast();
    startup.StartupInfo.hStdOutput = stdout_write.as_raw_handle().cast();
    startup.StartupInfo.hStdError = stderr.as_raw_handle().cast();
    startup.lpAttributeList = attribute_list;

    let application = wide_nul(executable.as_os_str());
    let mut command_line = windows_command_line(executable.as_os_str(), arguments);
    let cwd = wide_nul(cwd.as_os_str());
    let environment = windows_environment_block(environment);
    let mut process_info = PROCESS_INFORMATION::default();
    let created = unsafe {
        CreateProcessW(
            application.as_ptr(),
            command_line.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            1,
            CREATE_NO_WINDOW | CREATE_UNICODE_ENVIRONMENT | EXTENDED_STARTUPINFO_PRESENT,
            environment.as_ptr().cast(),
            cwd.as_ptr(),
            (&startup as *const STARTUPINFOEXW).cast(),
            &mut process_info,
        )
    };
    if created == 0 {
        return Err(RuntimeError::SpawnFailed);
    }
    unsafe {
        CloseHandle(process_info.hThread);
    }
    let process = unsafe { OwnedHandle::from_raw_handle(process_info.hProcess.cast()) };
    drop(stdin_read);
    drop(stdout_write);
    drop(stderr);
    let stdin = tokio::fs::File::from_std(std::fs::File::from(stdin_write));
    let stdout = tokio::fs::File::from_std(std::fs::File::from(stdout_read));
    Ok(WindowsAppContainerChild {
        process,
        stdin: Some(stdin),
        stdout: Some(stdout),
        finished: false,
    })
}

#[cfg(windows)]
unsafe fn close_if_valid(handle: windows_sys::Win32::Foundation::HANDLE) {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    if !handle.is_null() && handle != INVALID_HANDLE_VALUE {
        unsafe { CloseHandle(handle) };
    }
}

#[cfg(windows)]
fn wide_nul(value: impl AsRef<std::ffi::OsStr>) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    value.as_ref().encode_wide().chain(Some(0)).collect()
}

#[cfg(windows)]
fn windows_environment_block(environment: &BTreeMap<OsString, OsString>) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    let mut block = Vec::new();
    for (key, value) in environment {
        block.extend(key.encode_wide());
        block.push('=' as u16);
        block.extend(value.encode_wide());
        block.push(0);
    }
    if block.is_empty() {
        block.push(0);
    }
    block.push(0);
    block
}

#[cfg(windows)]
fn windows_command_line(executable: &std::ffi::OsStr, arguments: &[OsString]) -> Vec<u16> {
    let mut rendered = quote_windows_argument(executable);
    for argument in arguments {
        rendered.push(' ');
        rendered.push_str(&quote_windows_argument(argument));
    }
    wide_nul(rendered)
}

#[cfg(windows)]
fn quote_windows_argument(argument: &std::ffi::OsStr) -> String {
    let value = argument.to_string_lossy();
    if !value.is_empty()
        && !value
            .chars()
            .any(|character| character.is_whitespace() || character == '"')
    {
        return value.into_owned();
    }
    let mut output = String::from("\"");
    let mut backslashes = 0;
    for character in value.chars() {
        if character == '\\' {
            backslashes += 1;
            continue;
        }
        if character == '"' {
            output.push_str(&"\\".repeat(backslashes * 2 + 1));
        } else {
            output.push_str(&"\\".repeat(backslashes));
        }
        backslashes = 0;
        output.push(character);
    }
    output.push_str(&"\\".repeat(backslashes * 2));
    output.push('"');
    output
}

#[cfg(windows)]
fn grant_appcontainer_path(
    path: &Path,
    app_sid: windows_sys::Win32::Security::PSID,
    grant: WindowsGrant,
) -> Result<(), RuntimeError> {
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use windows_sys::Win32::Foundation::{LocalFree, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Security::Authorization::{
        GetSecurityInfo, SetEntriesInAclW, SetSecurityInfo, EXPLICIT_ACCESS_W, SE_FILE_OBJECT,
        TRUSTEE_IS_SID, TRUSTEE_IS_UNKNOWN, TRUSTEE_W,
    };
    use windows_sys::Win32::Security::{
        DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, SUB_CONTAINERS_AND_OBJECTS_INHERIT,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, DELETE, FILE_FLAG_BACKUP_SEMANTICS, FILE_GENERIC_EXECUTE, FILE_GENERIC_READ,
        FILE_GENERIC_WRITE, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
        READ_CONTROL, WRITE_DAC,
    };

    let metadata = std::fs::symlink_metadata(path).map_err(|_| RuntimeError::SandboxUnavailable)?;
    if is_link_or_reparse(&metadata) {
        return Err(RuntimeError::SandboxUnavailable);
    }
    if matches!(grant, WindowsGrant::Write) {
        set_low_integrity_path(path, metadata.is_dir())?;
    }
    let path_wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let handle = unsafe {
        CreateFileW(
            path_wide.as_ptr(),
            READ_CONTROL | WRITE_DAC,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            if metadata.is_dir() {
                FILE_FLAG_BACKUP_SEMANTICS
            } else {
                0
            },
            std::ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(RuntimeError::SandboxUnavailable);
    }
    let handle = unsafe { OwnedHandle::from_raw_handle(handle.cast()) };
    let mut old_dacl = std::ptr::null_mut();
    let mut descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
    let get_result = unsafe {
        GetSecurityInfo(
            handle.as_raw_handle().cast(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut old_dacl,
            std::ptr::null_mut(),
            &mut descriptor,
        )
    };
    if get_result != 0 {
        return Err(RuntimeError::SandboxUnavailable);
    }
    let access = match grant {
        WindowsGrant::ReadExecute => FILE_GENERIC_READ | FILE_GENERIC_EXECUTE,
        WindowsGrant::Write => {
            FILE_GENERIC_READ | FILE_GENERIC_WRITE | FILE_GENERIC_EXECUTE | DELETE
        }
    };
    let entry = EXPLICIT_ACCESS_W {
        grfAccessPermissions: access,
        grfAccessMode: 1,
        grfInheritance: if metadata.is_dir() {
            SUB_CONTAINERS_AND_OBJECTS_INHERIT
        } else {
            0
        },
        Trustee: TRUSTEE_W {
            pMultipleTrustee: std::ptr::null_mut(),
            MultipleTrusteeOperation: 0,
            TrusteeForm: TRUSTEE_IS_SID,
            TrusteeType: TRUSTEE_IS_UNKNOWN,
            ptstrName: app_sid.cast(),
        },
    };
    let mut new_dacl = std::ptr::null_mut();
    let set_entries = unsafe { SetEntriesInAclW(1, &entry, old_dacl, &mut new_dacl) };
    let set_result = if set_entries == 0 {
        unsafe {
            SetSecurityInfo(
                handle.as_raw_handle().cast(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                new_dacl,
                std::ptr::null_mut(),
            )
        }
    } else {
        set_entries
    };
    unsafe {
        if !new_dacl.is_null() {
            LocalFree(new_dacl.cast());
        }
        if !descriptor.is_null() {
            LocalFree(descriptor);
        }
    }
    if set_result != 0 {
        return Err(RuntimeError::SandboxUnavailable);
    }
    Ok(())
}

#[cfg(windows)]
fn set_low_integrity_path(path: &Path, directory: bool) -> Result<(), RuntimeError> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Authorization::{
        ConvertStringSecurityDescriptorToSecurityDescriptorW, SetNamedSecurityInfoW,
        SDDL_REVISION_1, SE_FILE_OBJECT,
    };
    use windows_sys::Win32::Security::{
        GetSecurityDescriptorSacl, LABEL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR,
    };

    let descriptor_text = wide_nul(if directory {
        "S:(ML;OICI;NW;;;LW)"
    } else {
        "S:(ML;;NW;;;LW)"
    });
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
        return Err(RuntimeError::SandboxUnavailable);
    }
    let mut present = 0;
    let mut defaulted = 0;
    let mut sacl = std::ptr::null_mut();
    let valid =
        unsafe { GetSecurityDescriptorSacl(descriptor, &mut present, &mut sacl, &mut defaulted) }
            != 0
            && present != 0;
    let mut path_wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let result = if valid {
        unsafe {
            SetNamedSecurityInfoW(
                path_wide.as_mut_ptr(),
                SE_FILE_OBJECT,
                LABEL_SECURITY_INFORMATION,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                sacl,
            )
        }
    } else {
        1
    };
    unsafe {
        LocalFree(descriptor);
    }
    if result != 0 {
        return Err(RuntimeError::SandboxUnavailable);
    }
    Ok(())
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

enum ManagedProcess {
    #[cfg(windows)]
    AppContainer(WindowsAppContainerChild),
    #[cfg(target_os = "macos")]
    Tokio(Child),
}

impl ManagedProcess {
    async fn stop(&mut self) -> Result<(), RuntimeError> {
        match self {
            #[cfg(windows)]
            Self::AppContainer(child) => child.stop().await,
            #[cfg(target_os = "macos")]
            Self::Tokio(child) => {
                if child
                    .try_wait()
                    .map_err(|_| RuntimeError::StopFailed)?
                    .is_none()
                {
                    child.start_kill().map_err(|_| RuntimeError::StopFailed)?;
                }
                child
                    .wait()
                    .await
                    .map(|_| ())
                    .map_err(|_| RuntimeError::StopFailed)
            }
        }
    }

    fn abort(&mut self) {
        match self {
            #[cfg(windows)]
            Self::AppContainer(child) => child.abort(),
            #[cfg(target_os = "macos")]
            Self::Tokio(child) => {
                let _ = child.start_kill();
            }
        }
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
            .map_err(|_| RuntimeError::HandshakeFailed)
    }
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

#[cfg(test)]
mod security_tests {
    use super::{
        canonicalize_chatgpt_auth, CodexAppServerConfig, CredentialStoreMode, RuntimeSandboxPolicy,
    };
    use serde_json::json;
    use tempfile::tempdir;

    #[cfg(windows)]
    #[tokio::test]
    async fn windows_appcontainer_cannot_read_a_sibling_secret() {
        use std::ffi::OsString;
        use tokio::io::AsyncReadExt;

        let root = tempdir().unwrap();
        let allowed = root.path().join("allowed");
        std::fs::create_dir_all(&allowed).unwrap();
        let secret = root.path().join("outside-secret.txt");
        std::fs::write(&secret, b"SMARTCAT_OUTSIDE_SECRET_MUST_NOT_BE_READ").unwrap();
        let command = super::windows_command_interpreter().unwrap();
        let arguments = vec![
            OsString::from("/d"),
            OsString::from("/c"),
            OsString::from("type"),
            secret.as_os_str().to_owned(),
        ];
        let environment = super::isolated_environment(&allowed, &allowed).unwrap();
        let mut child = super::spawn_windows_appcontainer_command(
            &command,
            &arguments,
            &allowed,
            &environment,
            &[(&allowed, super::WindowsGrant::Write)],
        )
        .unwrap();
        drop(child.stdin.take());
        let mut output = Vec::new();
        child
            .stdout
            .take()
            .unwrap()
            .read_to_end(&mut output)
            .await
            .unwrap();
        let status = child.wait().await.unwrap();

        assert_ne!(status, 0);
        assert!(!String::from_utf8_lossy(&output).contains("SMARTCAT_OUTSIDE_SECRET"));
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn windows_appcontainer_can_write_only_inside_its_granted_root() {
        use std::ffi::OsString;

        let root = tempdir().unwrap();
        let allowed = root.path().join("allowed");
        std::fs::create_dir_all(&allowed).unwrap();
        let command = super::windows_command_interpreter().unwrap();
        let output = allowed.join("written.txt");
        let arguments = vec![
            OsString::from("/d"),
            OsString::from("/c"),
            OsString::from(format!("echo isolated>{}", output.display())),
        ];
        let environment = super::isolated_environment(&allowed, &allowed).unwrap();
        let mut child = super::spawn_windows_appcontainer_command(
            &command,
            &arguments,
            &allowed,
            &environment,
            &[(&allowed, super::WindowsGrant::Write)],
        )
        .unwrap();
        drop(child.stdin.take());
        assert_eq!(child.wait().await.unwrap(), 0);
        assert_eq!(std::fs::read_to_string(output).unwrap().trim(), "isolated");
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
