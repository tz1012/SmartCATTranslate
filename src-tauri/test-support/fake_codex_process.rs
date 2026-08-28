use std::io::{self, BufRead, Write};

fn main() {
    let arguments: Vec<_> = std::env::args().skip(1).collect();
    if arguments != ["-c", "mcp_servers={}", "app-server", "--listen", "stdio://"] {
        std::process::exit(10);
    }
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
        "\"title\":\"SmartCAT Translate\"",
        "\"version\":\"0.1.0\"",
    ] {
        if !initialize.contains(required) {
            std::process::exit(12);
        }
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

    for line in lines.map_while(Result::ok) {
        let Some(id) = numeric_id(&line) else {
            continue;
        };
        if line.contains("\"method\":\"account/read\"") {
            write_line(&format!(
                r#"{{"id":{id},"result":{{"account":{{"type":"chatgpt","email":"person@example.com","planType":"plus"}},"requiresOpenaiAuth":true}}}}"#,
            ));
        } else if line.contains("\"method\":\"thread/start\"") {
            write_line(&format!(
                r#"{{"id":{id},"result":{{"thread":{{"id":"smartcat-base"}}}}}}"#,
            ));
        } else {
            write_line(&format!(r#"{{"id":{id},"result":{{}}}}"#));
        }
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
