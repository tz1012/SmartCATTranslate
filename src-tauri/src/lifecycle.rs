use std::sync::atomic::{AtomicBool, Ordering};

use tauri::{
    menu::{CheckMenuItem, CheckMenuItemBuilder, MenuBuilder, MenuItem, MenuItemBuilder},
    tray::TrayIconBuilder,
    App, AppHandle, Emitter, Manager, Runtime, Wry,
};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogResult};

use crate::{
    app_state::AppState,
    commands::settings::open_store,
    settings::types::{AppLocale, CloseBehavior, QuickAccessPosition},
};

const TRAY_ID: &str = "smartcat-residency";
const QUICK_ID: &str = "tray-quick";
const MAIN_ID: &str = "tray-main";
const PAUSE_ID: &str = "tray-pause";
const SETTINGS_ID: &str = "tray-settings";
const QUIT_ID: &str = "tray-quit";

pub struct LifecycleState {
    quitting: AtomicBool,
    restart_requested: AtomicBool,
    close_dialog_open: AtomicBool,
    locale_en: AtomicBool,
    account_ready: AtomicBool,
    quick_item: MenuItem<Wry>,
    main_item: MenuItem<Wry>,
    pause_item: CheckMenuItem<Wry>,
    status_item: MenuItem<Wry>,
    account_item: MenuItem<Wry>,
    settings_item: MenuItem<Wry>,
    quit_item: MenuItem<Wry>,
}

pub(crate) fn should_intercept_window_close(label: &str) -> bool {
    matches!(label, "main" | "quick-popup")
}

impl LifecycleState {
    fn new(
        quick_item: MenuItem<Wry>,
        main_item: MenuItem<Wry>,
        pause_item: CheckMenuItem<Wry>,
        status_item: MenuItem<Wry>,
        account_item: MenuItem<Wry>,
        settings_item: MenuItem<Wry>,
        quit_item: MenuItem<Wry>,
    ) -> Self {
        Self {
            quitting: AtomicBool::new(false),
            restart_requested: AtomicBool::new(false),
            close_dialog_open: AtomicBool::new(false),
            locale_en: AtomicBool::new(false),
            account_ready: AtomicBool::new(false),
            quick_item,
            main_item,
            pause_item,
            status_item,
            account_item,
            settings_item,
            quit_item,
        }
    }

    pub fn should_intercept_close(&self) -> bool {
        !self.quitting.load(Ordering::Acquire) && !self.restart_requested.load(Ordering::Acquire)
    }

    pub fn set_paused(&self, paused: bool) {
        let _ = self.pause_item.set_checked(paused);
        let english = self.locale_en.load(Ordering::Acquire);
        let _ = self.status_item.set_text(if english && paused {
            "Status: hotkeys paused"
        } else if english {
            "Status: ready"
        } else if paused {
            "상태: 단축키 일시 중지"
        } else {
            "상태: 준비됨"
        });
    }

    pub fn set_account_ready(&self, ready: bool) {
        self.account_ready.store(ready, Ordering::Release);
        let english = self.locale_en.load(Ordering::Acquire);
        let _ = self.account_item.set_text(if english && ready {
            "ChatGPT account: connected"
        } else if english {
            "ChatGPT account: sign in required"
        } else if ready {
            "ChatGPT 계정: 연결됨"
        } else {
            "ChatGPT 계정: 연결 필요"
        });
    }

    pub fn set_locale(&self, locale: AppLocale, paused: bool) {
        let english = locale == AppLocale::En;
        self.locale_en.store(english, Ordering::Release);
        let _ = self.quick_item.set_text(if english {
            "Quick translate"
        } else {
            "빠른 번역"
        });
        let _ = self.main_item.set_text(if english {
            "Open main window"
        } else {
            "전체 창 열기"
        });
        let _ = self.pause_item.set_text(if english {
            "Pause hotkeys"
        } else {
            "단축키 일시 중지"
        });
        let _ = self
            .settings_item
            .set_text(if english { "Settings" } else { "설정" });
        let _ = self
            .quit_item
            .set_text(if english { "Quit" } else { "종료" });
        self.set_paused(paused);
        self.set_account_ready(self.account_ready.load(Ordering::Acquire));
    }
}

pub fn setup(app: &mut App<Wry>) -> tauri::Result<()> {
    // setup runs once for the process; the stable id prevents reload-created duplicates.
    if app.tray_by_id(TRAY_ID).is_some() {
        return Ok(());
    }
    let quick = MenuItemBuilder::with_id(QUICK_ID, "빠른 번역").build(app)?;
    let main = MenuItemBuilder::with_id(MAIN_ID, "전체 창 열기").build(app)?;
    let pause = CheckMenuItemBuilder::with_id(PAUSE_ID, "단축키 일시 중지")
        .checked(false)
        .build(app)?;
    let status = MenuItemBuilder::with_id("tray-status", "상태: 준비됨")
        .enabled(false)
        .build(app)?;
    let account = MenuItemBuilder::with_id("tray-account", "ChatGPT 계정: 확인 중")
        .enabled(false)
        .build(app)?;
    let settings = MenuItemBuilder::with_id(SETTINGS_ID, "설정").build(app)?;
    let quit = MenuItemBuilder::with_id(QUIT_ID, "종료").build(app)?;
    let menu = MenuBuilder::new(app)
        .item(&quick)
        .item(&main)
        .separator()
        .item(&pause)
        .item(&status)
        .item(&account)
        .separator()
        .item(&settings)
        .item(&quit)
        .build()?;

    app.manage(LifecycleState::new(
        quick.clone(),
        main.clone(),
        pause.clone(),
        status.clone(),
        account.clone(),
        settings.clone(),
        quit.clone(),
    ));
    let icon = app.default_window_icon().cloned();
    let mut tray = TrayIconBuilder::with_id(TRAY_ID)
        .menu(&menu)
        .tooltip("SmartCAT Translate")
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| match event.id().as_ref() {
            QUICK_ID => {
                let app = app.clone();
                tauri::async_runtime::spawn(async move { open_quick_access(app).await });
            }
            MAIN_ID => {
                let _ = crate::commands::windows::open_main_window(app.clone());
            }
            SETTINGS_ID => {
                let _ = crate::commands::windows::open_main_window(app.clone());
                let _ = app.emit("open-settings", ());
            }
            PAUSE_ID => {
                let state = app.state::<AppState>();
                let paused = !state.hotkeys_suspended();
                state.set_hotkeys_suspended(paused);
                app.state::<LifecycleState>().set_paused(paused);
                let _ = app.emit("hotkeys-paused", paused);
            }
            QUIT_ID => request_quit(app.clone()),
            _ => {}
        });
    if let Some(icon) = icon {
        tray = tray.icon(icon);
    }
    #[cfg(target_os = "macos")]
    {
        tray = tray.icon_as_template(true);
    }
    tray.build(app)?;
    Ok(())
}

pub fn handle_window_close<R: Runtime>(app: &AppHandle<R>, label: &str) {
    if label == "quick-popup" {
        if let Some(popup) = app.get_webview_window("quick-popup") {
            let _ = popup.hide();
            let _ = popup.set_always_on_top(false);
        }
        return;
    }
    if label != "main" {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let behavior = match open_store(&app) {
            Ok(store) => store
                .load()
                .await
                .map(|settings| (settings.close_behavior, settings.locale))
                .unwrap_or((CloseBehavior::KeepInTray, AppLocale::Ko)),
            Err(_) => (CloseBehavior::KeepInTray, AppLocale::Ko),
        };
        match behavior.0 {
            CloseBehavior::KeepInTray => hide_main(&app),
            CloseBehavior::Quit => request_quit(app),
            CloseBehavior::AskEveryTime => ask_close_action(app, behavior.1),
        }
    });
}

fn ask_close_action<R: Runtime>(app: AppHandle<R>, locale: AppLocale) {
    let state = app.state::<LifecycleState>();
    if state.close_dialog_open.swap(true, Ordering::AcqRel) {
        return;
    }
    let (title, message, keep, quit, cancel) = match locale {
        AppLocale::Ko => (
            "SmartCAT Translate 닫기",
            "앱을 트레이에 유지할까요?",
            "트레이에 유지",
            "앱 종료",
            "취소",
        ),
        AppLocale::En => (
            "Close SmartCAT Translate",
            "Keep the app available in the tray?",
            "Keep in tray",
            "Quit app",
            "Cancel",
        ),
    };
    let dialog_app = app.clone();
    app.dialog()
        .message(message)
        .title(title)
        .buttons(MessageDialogButtons::YesNoCancelCustom(
            keep.to_owned(),
            quit.to_owned(),
            cancel.to_owned(),
        ))
        .show_with_result(move |result| {
            dialog_app
                .state::<LifecycleState>()
                .close_dialog_open
                .store(false, Ordering::Release);
            match result {
                MessageDialogResult::Yes => hide_main(&dialog_app),
                MessageDialogResult::No => request_quit(dialog_app),
                MessageDialogResult::Custom(value) if value == keep => hide_main(&dialog_app),
                MessageDialogResult::Custom(value) if value == quit => request_quit(dialog_app),
                _ => {}
            }
        });
}

fn hide_main<R: Runtime>(app: &AppHandle<R>) {
    if let Some(main) = app.get_webview_window("main") {
        let _ = main.hide();
    }
    #[cfg(target_os = "macos")]
    let _ = app.set_activation_policy(tauri::ActivationPolicy::Accessory);
}

pub async fn open_quick_access<R: Runtime>(app: AppHandle<R>) {
    let position = match open_store(&app) {
        Ok(store) => store
            .load()
            .await
            .map(|settings| settings.quick_access_position)
            .unwrap_or_default(),
        Err(_) => QuickAccessPosition::default(),
    };
    match position {
        QuickAccessPosition::MainWindow => {
            let _ = crate::commands::windows::open_main_window(app);
        }
        QuickAccessPosition::Popup => {
            if let Some(popup) = app.get_webview_window("quick-popup") {
                let _ = popup.set_always_on_top(true);
                let _ = popup.show();
                let _ = popup.set_focus();
            }
        }
    }
}

pub fn request_quit<R: Runtime>(app: AppHandle<R>) {
    // Atomic transition makes repeated menu clicks and close events idempotent.
    let state = app.state::<LifecycleState>();
    if state.quitting.swap(true, Ordering::AcqRel) {
        return;
    }
    if let Some(popup) = app.get_webview_window("quick-popup") {
        let _ = popup.hide();
    }
    let shutdown_app = app.clone();
    tauri::async_runtime::spawn(async move {
        let app_state = shutdown_app.state::<AppState>();
        let _ = app_state.shutdown().await;
        shutdown_app.exit(0);
    });
}

#[cfg(test)]
mod tests {
    use super::should_intercept_window_close;

    #[test]
    fn close_interception_excludes_capture_overlay_windows() {
        assert!(should_intercept_window_close("main"));
        assert!(should_intercept_window_close("quick-popup"));
        assert!(!should_intercept_window_close("capture-overlay-primary"));
    }
}
