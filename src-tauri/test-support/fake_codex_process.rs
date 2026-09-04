use std::io::{self, BufRead, Write};

fn main() {
    let mut arguments: Vec<_> = std::env::args().skip(1).collect();
    if arguments.first().map(String::as_str) == Some("sandbox") {
        let delimiter = arguments
            .iter()
            .position(|value| value == "--")
            .unwrap_or(usize::MAX);
        if delimiter < 5
            || arguments.get(1).map(String::as_str) != Some("--permission-profile")
            || arguments.get(2).map(String::as_str) != Some("smartcat-translation")
            || arguments.get(3).map(String::as_str) != Some("--cd")
            || delimiter + 1 >= arguments.len()
        {
            std::process::exit(9);
        }
        arguments = arguments[(delimiter + 2)..].to_vec();
    }
    if arguments != ["app-server", "--listen", "stdio://"] {
        std::process::exit(10);
    }
    validate_isolated_home();
    if std::fs::read_dir(std::env::current_dir().expect("current directory"))
        .expect("read current directory")
        .next()
        .is_some()
    {
        std::process::exit(11);
    }

    let stdin = io::stdin();
    let mut lines = stdin.lock().lines();
    let initialize = lines.next().and_then(Result::ok).unwrap_or_default();
    for required in [
        "\"method\":\"initialize\"",
        "\"id\":0",
        "\"name\":\"smartcat_translate\"",
        "\"title\":\"BYOK Translator\"",
    ] {
        if !initialize.contains(required) {
            std::process::exit(12);
        }
    }
    let initialize_json: serde_json::Value = serde_json::from_str(&initialize).unwrap();
    if initialize_json["params"]["clientInfo"]["version"].as_str()
        != Some(env!("CARGO_PKG_VERSION"))
    {
        std::process::exit(12);
    }
    if initialize_json["params"].get("capabilities").is_some() {
        std::process::exit(14);
    }

    write_line(
        r#"{"id":0,"result":{"userAgent":"fake-codex","platformFamily":"test","platformOs":"test"}}"#,
    );
    let initialized = lines.next().and_then(Result::ok).unwrap_or_default();
    if !initialized.contains("\"method\":\"initialized\"")
        || !initialized.contains("\"params\":{}")
        || initialized.contains("\"id\"")
    {
        std::process::exit(13);
    }

    let attestation = lines.next().and_then(Result::ok).unwrap_or_default();
    if !attestation.contains("\"method\":\"smartcat/attestation\"")
        || !attestation.contains("\"id\":1")
    {
        std::process::exit(15);
    }
    let is_stock_fixture = std::env::current_exe()
        .ok()
        .and_then(|path| {
            path.file_stem()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .is_some_and(|name| name.contains("stock-codex"));
    if is_stock_fixture {
        write_line(r#"{"id":1,"result":{}}"#);
        std::process::exit(0);
    }
    write_line(
        r#"{"id":1,"result":{"upstreamCommit":"8c68d4c87dc54d38861f5114e920c3de2efa5876","patchVersion":"smartcat-1","toolCount":0,"instructionDiscovery":false}}"#,
    );

    for line in lines.map_while(Result::ok) {
        let Some(id) = numeric_id(&line) else {
            continue;
        };
        if line.contains("\"method\":\"account/read\"") {
            write_line(&format!(
                r#"{{"id":{id},"result":{{"account":{{"type":"chatgpt","email":"person@example.com","planType":"plus"}},"requiresOpenaiAuth":true}}}}"#,
            ));
        } else if line.contains("\"method\":\"account/login/start\"") {
            write_line(&format!(
                r#"{{"id":{id},"result":{{"type":"chatgpt","loginId":"runtime-login","authUrl":"https://chatgpt.com/auth/login"}}}}"#,
            ));
        } else if line.contains("\"method\":\"account/login/cancel\"") {
            write_line(&format!(r#"{{"id":{id},"result":{{}}}}"#));
        } else if line.contains("\"method\":\"thread/start\"") {
            write_line(&format!(
                r#"{{"id":{id},"result":{{"thread":{{"id":"smartcat-base"}},"instructionSources":[]}}}}"#,
            ));
        } else {
            write_line(&format!(r#"{{"id":{id},"result":{{}}}}"#));
        }
    }
}

fn validate_isolated_home() {
    let home = std::env::var_os("CODEX_HOME").unwrap_or_default();
    if home.is_empty()
        || std::env::var_os("CODEX_SQLITE_HOME").is_some()
        || std::env::var_os("CODEX_ACCESS_TOKEN").is_some()
        || std::env::var_os("CODEX_API_KEY").is_some()
        || std::env::var_os("OPENAI_API_KEY").is_some()
    {
        std::process::exit(20);
    }
    if std::env::vars_os().any(|(name, _)| {
        let name = name.to_string_lossy().to_ascii_uppercase();
        (name.starts_with("CODEX_") && name != "CODEX_HOME")
            || name.starts_with("OPENAI_")
            || name.starts_with("CHATGPT_")
            || matches!(
                name.as_str(),
                "HTTP_PROXY" | "HTTPS_PROXY" | "ALL_PROXY" | "NO_PROXY"
            )
    }) {
        std::process::exit(22);
    }
    let config = std::fs::read_to_string(std::path::PathBuf::from(home).join("config.toml"))
        .unwrap_or_default();
    let Ok(value) = toml::from_str::<toml::Value>(&config) else {
        std::process::exit(21);
    };
    if value["approval_policy"].as_str() != Some("never")
        || value["sandbox_mode"].as_str() != Some("read-only")
        || value["web_search"].as_str() != Some("disabled")
        || value["features"]["apps"].as_bool() != Some(false)
        || value["features"]["shell_tool"].as_bool() != Some(false)
        || value["features"]["multi_agent"].as_bool() != Some(false)
        || value["default_permissions"].as_str() != Some("smartcat-translation")
        || value["permissions"]["smartcat-translation"]["filesystem"][":minimal"].as_str()
            != Some("read")
        || value.get("agents").is_some()
        || value.get("tools").is_some()
        || !value["mcp_servers"]
            .as_table()
            .is_some_and(toml::Table::is_empty)
        || config.contains("HOSTILE_USER_CONFIG")
    {
        std::process::exit(21);
    }
}

fn numeric_id(line: &str) -> Option<u64> {
    let value = line.split_once("\"id\":")?.1;
    let digits: String = value.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

fn write_line(value: &str) {
    let mut stdout = io::stdout().lock();
    writeln!(stdout, "{value}").expect("write response");
    stdout.flush().expect("flush response");
}
