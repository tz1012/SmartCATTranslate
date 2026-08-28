pub mod app_state;
pub mod codex;
pub mod commands;
pub mod core;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(app_state::AppState::default())
        .invoke_handler(tauri::generate_handler![
            commands::account::get_account,
            commands::account::get_rate_limits,
            commands::account::start_chatgpt_login,
            commands::account::cancel_chatgpt_login,
        ])
        .run(tauri::generate_context!())
        .expect("failed to run SmartCAT Translate");
}

#[cfg(test)]
mod tests {
    #[test]
    fn package_name_is_stable() {
        assert_eq!(env!("CARGO_PKG_NAME"), "smartcat-translate");
    }
}
