use std::path::{Path, PathBuf};

use anyhow::Result;
use chrono::Utc;

use crate::types::{
    DependencyHealthSnapshot, GhosttyOpenMode, NativeDesktopApp, NativeDesktopStatusResponse,
};

use super::host::host_is_loopback;
use super::script_path::{script_path_for_app, script_path_for_app_without_materializing};

const NATIVE_APP_ENV: &str = "SWIMMERS_NATIVE_APP";
const GHOSTTY_MODE_ENV: &str = "SWIMMERS_GHOSTTY_MODE";

pub fn default_native_app() -> NativeDesktopApp {
    std::env::var(NATIVE_APP_ENV)
        .ok()
        .as_deref()
        .map(NativeDesktopApp::from_env_value)
        .unwrap_or(NativeDesktopApp::Iterm)
}

pub fn default_ghostty_open_mode() -> GhosttyOpenMode {
    std::env::var(GHOSTTY_MODE_ENV)
        .ok()
        .as_deref()
        .map(GhosttyOpenMode::from_env_value)
        .unwrap_or(GhosttyOpenMode::Swap)
}

pub fn support_for_host(host: &str, app: NativeDesktopApp) -> NativeDesktopStatusResponse {
    let app_unavailable_reason = match app {
        NativeDesktopApp::Iterm => None,
        NativeDesktopApp::Ghostty => super::ghostty_unavailable_reason(),
    };
    support_for_host_with_script_resolver(
        host,
        app,
        cfg!(target_os = "macos"),
        || script_path_for_app(app),
        app_unavailable_reason,
    )
}

pub fn script_dependency_health(app: NativeDesktopApp) -> DependencyHealthSnapshot {
    let now = Utc::now();
    let path = script_path_for_app_without_materializing(app);
    match path {
        Ok(path) if path.is_file() => DependencyHealthSnapshot::healthy(now)
            .with_detail("app", app.display_name())
            .with_detail("script_path", path.to_string_lossy().into_owned()),
        Ok(path) => DependencyHealthSnapshot::unavailable(
            now,
            format!(
                "native {} script missing: {}",
                app.display_name(),
                path.display()
            ),
        )
        .with_detail("app", app.display_name())
        .with_detail("script_path", path.to_string_lossy().into_owned()),
        Err(err) => DependencyHealthSnapshot::unavailable(now, err.to_string())
            .with_detail("app", app.display_name()),
    }
}

fn support_for_host_with_script_resolver(
    host: &str,
    app: NativeDesktopApp,
    is_macos: bool,
    resolve_script_path: impl FnOnce() -> Result<PathBuf>,
    app_unavailable_reason: Option<String>,
) -> NativeDesktopStatusResponse {
    let reason = native_desktop_support_reason(
        host,
        app,
        is_macos,
        resolve_script_path,
        app_unavailable_reason,
    );
    native_desktop_status_response(app, reason)
}

fn native_desktop_status_response(
    app: NativeDesktopApp,
    reason: Option<String>,
) -> NativeDesktopStatusResponse {
    NativeDesktopStatusResponse {
        supported: reason.is_none(),
        platform: Some(std::env::consts::OS.to_string()),
        app_id: Some(app),
        ghostty_mode: None,
        app: Some(app.display_name().to_string()),
        reason,
    }
}

fn native_desktop_support_reason(
    host: &str,
    app: NativeDesktopApp,
    is_macos: bool,
    resolve_script_path: impl FnOnce() -> Result<PathBuf>,
    app_unavailable_reason: Option<String>,
) -> Option<String> {
    native_desktop_support_result(
        host,
        app,
        is_macos,
        resolve_script_path,
        app_unavailable_reason,
    )
    .err()
}

fn native_desktop_support_result(
    host: &str,
    app: NativeDesktopApp,
    is_macos: bool,
    resolve_script_path: impl FnOnce() -> Result<PathBuf>,
    app_unavailable_reason: Option<String>,
) -> std::result::Result<(), String> {
    native_desktop_platform_support(app, is_macos)?;
    native_desktop_host_support(host, app)?;
    native_desktop_script_support(app, resolve_script_path)?;
    app_unavailable_reason.map_or(Ok(()), Err)
}

fn native_desktop_platform_support(
    app: NativeDesktopApp,
    is_macos: bool,
) -> std::result::Result<(), String> {
    native_desktop_platform_reason(app, is_macos).map_or(Ok(()), Err)
}

fn native_desktop_platform_reason(app: NativeDesktopApp, is_macos: bool) -> Option<String> {
    (!is_macos).then_some(format!(
        "native {} control is only supported on macOS",
        app.display_name()
    ))
}

fn native_desktop_host_support(
    host: &str,
    app: NativeDesktopApp,
) -> std::result::Result<(), String> {
    native_desktop_host_reason(host, app).map_or(Ok(()), Err)
}

fn native_desktop_host_reason(host: &str, app: NativeDesktopApp) -> Option<String> {
    (!host_is_loopback(host)).then_some(format!(
        "native {} control is only available from localhost",
        app.display_name()
    ))
}

fn native_desktop_script_support(
    app: NativeDesktopApp,
    resolve_script_path: impl FnOnce() -> Result<PathBuf>,
) -> std::result::Result<(), String> {
    let script_path = resolve_script_path()
        .map_err(|err| format!("native {} script unavailable: {err}", app.display_name()))?;
    native_desktop_missing_script_reason(app, &script_path).map_or(Ok(()), Err)
}

fn native_desktop_missing_script_reason(
    app: NativeDesktopApp,
    script_path: &Path,
) -> Option<String> {
    (!script_path.exists()).then_some(format!(
        "native {} script missing: {}",
        app.display_name(),
        script_path.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn support_for_host_with(
        host: &str,
        app: NativeDesktopApp,
        is_macos: bool,
        script_path: &Path,
        app_unavailable_reason: Option<String>,
    ) -> NativeDesktopStatusResponse {
        support_for_host_with_script_resolver(
            host,
            app,
            is_macos,
            || Ok(script_path.to_path_buf()),
            app_unavailable_reason,
        )
    }

    #[test]
    fn support_for_host_with_reports_ghostty_app_and_unavailable_reason() {
        let temp = tempdir().unwrap();
        let script_path = temp.path().join("ghostty-open.scpt");
        std::fs::write(&script_path, "").unwrap();
        let response = support_for_host_with(
            "localhost:3210",
            NativeDesktopApp::Ghostty,
            true,
            &script_path,
            Some(
                "Ghostty 1.2.3 is installed, but native AppleScript control requires Ghostty 1.3.0+."
                    .to_string(),
            ),
        );
        assert!(!response.supported);
        assert_eq!(response.app.as_deref(), Some("Ghostty"));
        assert!(response
            .reason
            .as_deref()
            .unwrap_or_default()
            .contains("Ghostty 1.2.3 is installed"));
    }

    #[test]
    fn support_for_host_with_marks_loopback_host_supported_when_script_exists() {
        let temp = tempdir().unwrap();
        let script_path = temp.path().join("iterm-focus.scpt");
        std::fs::write(&script_path, "").unwrap();

        let response = support_for_host_with(
            "127.0.0.1:3210",
            NativeDesktopApp::Iterm,
            true,
            &script_path,
            None,
        );

        assert!(response.supported);
        assert_eq!(response.platform.as_deref(), Some(std::env::consts::OS));
        assert_eq!(response.app_id, Some(NativeDesktopApp::Iterm));
        assert_eq!(response.ghostty_mode, None);
        assert_eq!(response.app.as_deref(), Some("iTerm"));
        assert_eq!(response.reason, None);
    }

    #[test]
    fn support_for_host_with_reports_resolver_error() {
        let response = support_for_host_with_script_resolver(
            "localhost:3210",
            NativeDesktopApp::Iterm,
            true,
            || Err(anyhow::anyhow!("permission denied")),
            None,
        );

        assert!(!response.supported);
        assert_eq!(
            response.reason.as_deref(),
            Some("native iTerm script unavailable: permission denied")
        );
    }

    #[test]
    fn support_for_host_with_reports_non_macos_before_resolving_script() {
        let response = support_for_host_with_script_resolver(
            "example.com:3210",
            NativeDesktopApp::Ghostty,
            false,
            || -> Result<PathBuf> { panic!("script resolver should not run on unsupported OS") },
            Some("Ghostty unavailable".to_string()),
        );

        assert!(!response.supported);
        assert_eq!(
            response.reason.as_deref(),
            Some("native Ghostty control is only supported on macOS")
        );
    }

    #[test]
    fn support_for_host_with_uses_selected_app_in_loopback_errors() {
        let temp = tempdir().unwrap();
        let script_path = temp.path().join("ghostty-open.scpt");
        std::fs::write(&script_path, "").unwrap();
        let response = support_for_host_with(
            "example.com:3210",
            NativeDesktopApp::Ghostty,
            true,
            &script_path,
            None,
        );
        assert!(!response.supported);
        assert_eq!(response.app.as_deref(), Some("Ghostty"));
        assert_eq!(
            response.reason.as_deref(),
            Some("native Ghostty control is only available from localhost")
        );
    }

    #[test]
    fn support_for_host_with_reports_missing_script_path() {
        let temp = tempdir().unwrap();
        let script_path = temp.path().join("missing/iterm-focus.scpt");

        let response = support_for_host_with(
            "localhost:3210",
            NativeDesktopApp::Iterm,
            true,
            &script_path,
            None,
        );

        assert!(!response.supported);
        let reason = response.reason.as_deref().unwrap_or_default();
        assert!(reason.contains("native iTerm script missing:"));
        assert!(reason.contains("missing/iterm-focus.scpt"));
    }

    #[test]
    fn script_dependency_health_reports_missing_override_without_materializing() {
        let _env_guard = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let temp = tempdir().unwrap();
        let override_root = temp.path().join("missing-root");
        let original = std::env::var_os(super::super::NATIVE_SCRIPT_ROOT_ENV);
        std::env::set_var(super::super::NATIVE_SCRIPT_ROOT_ENV, &override_root);

        let health = script_dependency_health(NativeDesktopApp::Iterm);

        match original {
            Some(value) => std::env::set_var(super::super::NATIVE_SCRIPT_ROOT_ENV, value),
            None => std::env::remove_var(super::super::NATIVE_SCRIPT_ROOT_ENV),
        }

        assert_eq!(
            health.status,
            crate::types::DependencyHealthStatus::Unavailable
        );
        assert!(health
            .last_error
            .as_deref()
            .unwrap_or_default()
            .contains("script missing"));
        assert!(!override_root.exists());
    }
}
