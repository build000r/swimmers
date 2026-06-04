use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::extract::Path as AxumPath;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use serde::Serialize;

use crate::api::AppState;

pub(super) const APP_JS_ROUTE: &str = "/app.js";
pub(super) const API_CLIENT_JS_ROUTE: &str = "/api_client.js";
pub(super) const SESSION_PERSISTENCE_JS_ROUTE: &str = "/session_persistence.js";
pub(super) const APP_EVENT_HANDLERS_JS_ROUTE: &str = "/app_event_handlers.js";
pub(super) const APP_EVENT_BINDINGS_JS_ROUTE: &str = "/app_event_bindings.js";
pub(super) const TROGDOR_EVENT_BINDINGS_JS_ROUTE: &str = "/trogdor_event_bindings.js";
pub(super) const RENDERED_SURFACE_JS_ROUTE: &str = "/rendered_surface.js";
pub(super) const RENDERED_SURFACE_DRAW_JS_ROUTE: &str = "/rendered_surface_draw.js";
pub(super) const INPUT_SUPPORT_JS_ROUTE: &str = "/input_support.js";
pub(super) const SURFACE_ACTION_PLANS_JS_ROUTE: &str = "/surface_action_plans.js";
pub(super) const SEND_SHEET_JS_ROUTE: &str = "/send_sheet.js";
pub(super) const SEND_CONTROLLER_JS_ROUTE: &str = "/send_controller.js";
pub(super) const THOUGHT_CONFIG_SHEET_JS_ROUTE: &str = "/thought_config_sheet.js";
pub(super) const NATIVE_DESKTOP_SHEET_JS_ROUTE: &str = "/native_desktop_sheet.js";
pub(super) const TERMINAL_SURFACE_SETUP_JS_ROUTE: &str = "/terminal_surface_setup.js";
pub(super) const TERMINAL_FOCUS_JS_ROUTE: &str = "/terminal_focus.js";
pub(super) const TERMINAL_ZOOM_INPUT_JS_ROUTE: &str = "/terminal_zoom_input.js";
pub(super) const TERMINAL_RESIZE_JS_ROUTE: &str = "/terminal_resize.js";
pub(super) const GLOBAL_SHORTCUT_DISPATCH_JS_ROUTE: &str = "/global_shortcut_dispatch.js";
pub(super) const SESSION_REFRESH_JS_ROUTE: &str = "/session_refresh.js";
pub(super) const AGENT_CONTEXT_REFRESH_JS_ROUTE: &str = "/agent_context_refresh.js";
pub(super) const MERMAID_ARTIFACT_JS_ROUTE: &str = "/mermaid_artifact.js";
pub(super) const MERMAID_ARTIFACT_CONTROLLER_JS_ROUTE: &str = "/mermaid_artifact_controller.js";
pub(super) const TERMINAL_SAFETY_JS_ROUTE: &str = "/terminal_safety.js";
pub(super) const TERMINAL_SEARCH_LINKS_JS_ROUTE: &str = "/terminal_search_links.js";
pub(super) const TERMINAL_STATUS_JS_ROUTE: &str = "/terminal_status.js";
pub(super) const TERMINAL_PROTOCOL_JS_ROUTE: &str = "/terminal_protocol.js";
pub(super) const TERMINAL_INPUT_JS_ROUTE: &str = "/terminal_input.js";
pub(super) const SESSION_SOCKET_CONTROLLER_JS_ROUTE: &str = "/session_socket_controller.js";
pub(super) const DIR_BROWSER_JS_ROUTE: &str = "/dir_browser.js";
pub(super) const DIR_BROWSER_CONTROLLER_JS_ROUTE: &str = "/dir_browser_controller.js";
pub(super) const COMMAND_PALETTE_JS_ROUTE: &str = "/command_palette.js";
pub(super) const COMMAND_PALETTE_CONTROLLER_JS_ROUTE: &str = "/command_palette_controller.js";
pub(super) const TROGDOR_LOGIC_JS_ROUTE: &str = "/trogdor_logic.js";
pub(super) const TROGDOR_DOM_LOGIC_JS_ROUTE: &str = "/trogdor_dom_logic.js";
pub(super) const TROGDOR_RENDER_JS_ROUTE: &str = "/trogdor_render.js";
pub(super) const WORKBENCH_DOM_JS_ROUTE: &str = "/workbench_dom.js";
pub(super) const WORKBENCH_RENDER_JS_ROUTE: &str = "/workbench_render.js";
pub(super) const WORKBENCH_LOG_LENS_JS_ROUTE: &str = "/workbench_log_lens.js";
pub(super) const WORKBENCH_REFRESH_JS_ROUTE: &str = "/workbench_refresh.js";
pub(super) const WORKBENCH_RECORDS_JS_ROUTE: &str = "/workbench_records.js";
pub(super) const TERMINAL_WORKBENCH_CONTROLLER_JS_ROUTE: &str = "/terminal_workbench_controller.js";
pub(super) const APP_CSS_ROUTE: &str = "/app.css";
pub(super) const FRANKENTERM_JS_ROUTE: &str = "/assets/frankenterm/FrankenTerm.js";
pub(super) const FRANKENTERM_WASM_ROUTE: &str = "/assets/frankenterm/FrankenTerm_bg.wasm";
pub(super) const FRANKENTERM_FONT_ROUTE: &str = "/assets/frankenterm/pragmasevka-nf-subset.woff2";
const TROGDOR_DRAGON_ASSET_ROUTE: &str = "/assets/dragon/{pose}/{frame}";
const DEFAULT_FRANKENTUI_PKG_CANDIDATES: &[&str] = &[];

pub(super) fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route(APP_JS_ROUTE, get(app_js))
        .route(API_CLIENT_JS_ROUTE, get(api_client_js))
        .route(SESSION_PERSISTENCE_JS_ROUTE, get(session_persistence_js))
        .route(APP_EVENT_HANDLERS_JS_ROUTE, get(app_event_handlers_js))
        .route(APP_EVENT_BINDINGS_JS_ROUTE, get(app_event_bindings_js))
        .route(
            TROGDOR_EVENT_BINDINGS_JS_ROUTE,
            get(trogdor_event_bindings_js),
        )
        .route(RENDERED_SURFACE_JS_ROUTE, get(rendered_surface_js))
        .route(
            RENDERED_SURFACE_DRAW_JS_ROUTE,
            get(rendered_surface_draw_js),
        )
        .route(INPUT_SUPPORT_JS_ROUTE, get(input_support_js))
        .route(SURFACE_ACTION_PLANS_JS_ROUTE, get(surface_action_plans_js))
        .route(SEND_SHEET_JS_ROUTE, get(send_sheet_js))
        .route(SEND_CONTROLLER_JS_ROUTE, get(send_controller_js))
        .route(THOUGHT_CONFIG_SHEET_JS_ROUTE, get(thought_config_sheet_js))
        .route(NATIVE_DESKTOP_SHEET_JS_ROUTE, get(native_desktop_sheet_js))
        .route(
            TERMINAL_SURFACE_SETUP_JS_ROUTE,
            get(terminal_surface_setup_js),
        )
        .route(TERMINAL_FOCUS_JS_ROUTE, get(terminal_focus_js))
        .route(TERMINAL_ZOOM_INPUT_JS_ROUTE, get(terminal_zoom_input_js))
        .route(TERMINAL_RESIZE_JS_ROUTE, get(terminal_resize_js))
        .route(
            GLOBAL_SHORTCUT_DISPATCH_JS_ROUTE,
            get(global_shortcut_dispatch_js),
        )
        .route(SESSION_REFRESH_JS_ROUTE, get(session_refresh_js))
        .route(
            AGENT_CONTEXT_REFRESH_JS_ROUTE,
            get(agent_context_refresh_js),
        )
        .route(MERMAID_ARTIFACT_JS_ROUTE, get(mermaid_artifact_js))
        .route(
            MERMAID_ARTIFACT_CONTROLLER_JS_ROUTE,
            get(mermaid_artifact_controller_js),
        )
        .route(TERMINAL_SAFETY_JS_ROUTE, get(terminal_safety_js))
        .route(
            TERMINAL_SEARCH_LINKS_JS_ROUTE,
            get(terminal_search_links_js),
        )
        .route(TERMINAL_STATUS_JS_ROUTE, get(terminal_status_js))
        .route(TERMINAL_PROTOCOL_JS_ROUTE, get(terminal_protocol_js))
        .route(TERMINAL_INPUT_JS_ROUTE, get(terminal_input_js))
        .route(
            SESSION_SOCKET_CONTROLLER_JS_ROUTE,
            get(session_socket_controller_js),
        )
        .route(DIR_BROWSER_JS_ROUTE, get(dir_browser_js))
        .route(
            DIR_BROWSER_CONTROLLER_JS_ROUTE,
            get(dir_browser_controller_js),
        )
        .route(COMMAND_PALETTE_JS_ROUTE, get(command_palette_js))
        .route(
            COMMAND_PALETTE_CONTROLLER_JS_ROUTE,
            get(command_palette_controller_js),
        )
        .route(TROGDOR_LOGIC_JS_ROUTE, get(trogdor_logic_js))
        .route(TROGDOR_DOM_LOGIC_JS_ROUTE, get(trogdor_dom_logic_js))
        .route(TROGDOR_RENDER_JS_ROUTE, get(trogdor_render_js))
        .route(WORKBENCH_DOM_JS_ROUTE, get(workbench_dom_js))
        .route(WORKBENCH_RENDER_JS_ROUTE, get(workbench_render_js))
        .route(WORKBENCH_LOG_LENS_JS_ROUTE, get(workbench_log_lens_js))
        .route(WORKBENCH_REFRESH_JS_ROUTE, get(workbench_refresh_js))
        .route(WORKBENCH_RECORDS_JS_ROUTE, get(workbench_records_js))
        .route(
            TERMINAL_WORKBENCH_CONTROLLER_JS_ROUTE,
            get(terminal_workbench_controller_js),
        )
        .route(APP_CSS_ROUTE, get(app_css))
        .route(FRANKENTERM_JS_ROUTE, get(franken_term_js))
        .route(FRANKENTERM_WASM_ROUTE, get(franken_term_wasm))
        .route(FRANKENTERM_FONT_ROUTE, get(franken_term_font))
        .route(TROGDOR_DRAGON_ASSET_ROUTE, get(trogdor_dragon_asset))
}

#[derive(Debug, Serialize)]
pub(super) struct FrankenTermAssetInfo {
    pub(super) js: FrankenTermAssetFileInfo,
    pub(super) wasm: FrankenTermAssetFileInfo,
    pub(super) font: Option<FrankenTermAssetFileInfo>,
}

#[derive(Debug, Serialize)]
pub(super) struct FrankenTermAssetFileInfo {
    pub(super) route: &'static str,
    pub(super) size_bytes: u64,
    pub(super) checksum: String,
}

/// In debug builds, serve a web asset from its on-disk source so CSS/JS edits
/// show up on a plain browser refresh (no rebuild). Falls back to the baked-in
/// copy if the file can't be read. Release builds always use the embedded copy,
/// at zero cost. `relative` is the path from the crate root (where Cargo.toml
/// lives); `baked` is the matching `include_str!` constant. Note: the page HTML
/// is templated in Rust, so markup changes in this file still need a rebuild.
#[cfg(debug_assertions)]
fn dev_asset(relative: &str, baked: &'static str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(relative);
    std::fs::read_to_string(&path).unwrap_or_else(|_| baked.to_string())
}

#[cfg(not(debug_assertions))]
fn dev_asset(_relative: &str, baked: &'static str) -> &'static str {
    baked
}

struct CssAssetPart {
    relative: &'static str,
    baked: &'static str,
}

const APP_CSS_PARTS: &[CssAssetPart] = &[
    CssAssetPart {
        relative: "src/web/app.css",
        baked: include_str!("app.css"),
    },
    CssAssetPart {
        relative: "src/web/app_trogdor.css",
        baked: include_str!("app_trogdor.css"),
    },
    CssAssetPart {
        relative: "src/web/app_sheets.css",
        baked: include_str!("app_sheets.css"),
    },
    CssAssetPart {
        relative: "src/web/app_create_console.css",
        baked: include_str!("app_create_console.css"),
    },
    CssAssetPart {
        relative: "src/web/app_sheet_results.css",
        baked: include_str!("app_sheet_results.css"),
    },
    CssAssetPart {
        relative: "src/web/app_mobile.css",
        baked: include_str!("app_mobile.css"),
    },
    CssAssetPart {
        relative: "src/web/app_reduced_motion.css",
        baked: include_str!("app_reduced_motion.css"),
    },
    CssAssetPart {
        relative: "src/web/app_scrollbar.css",
        baked: include_str!("app_scrollbar.css"),
    },
];

pub(super) fn app_css_body() -> String {
    let mut body = String::new();
    for part in APP_CSS_PARTS {
        let chunk = dev_asset(part.relative, part.baked);
        body.push_str(chunk.as_ref());
    }
    body
}

fn javascript_asset(relative: &str, baked: &'static str) -> Response {
    (
        [
            (
                header::CONTENT_TYPE,
                "application/javascript; charset=utf-8",
            ),
            (header::CACHE_CONTROL, "no-store"),
        ],
        dev_asset(relative, baked),
    )
        .into_response()
}

macro_rules! javascript_route {
    ($name:ident, $relative:literal, $baked:literal) => {
        pub(super) async fn $name() -> Response {
            javascript_asset($relative, include_str!($baked))
        }
    };
}

javascript_route!(app_js, "src/web/app.js", "app.js");
javascript_route!(api_client_js, "src/web/api_client.js", "api_client.js");
javascript_route!(
    session_persistence_js,
    "src/web/session_persistence.js",
    "session_persistence.js"
);
javascript_route!(
    app_event_handlers_js,
    "src/web/app_event_handlers.js",
    "app_event_handlers.js"
);
javascript_route!(
    app_event_bindings_js,
    "src/web/app_event_bindings.js",
    "app_event_bindings.js"
);
javascript_route!(
    trogdor_event_bindings_js,
    "src/web/trogdor_event_bindings.js",
    "trogdor_event_bindings.js"
);
javascript_route!(
    rendered_surface_js,
    "src/web/rendered_surface.js",
    "rendered_surface.js"
);
javascript_route!(
    rendered_surface_draw_js,
    "src/web/rendered_surface_draw.js",
    "rendered_surface_draw.js"
);
javascript_route!(
    input_support_js,
    "src/web/input_support.js",
    "input_support.js"
);
javascript_route!(
    surface_action_plans_js,
    "src/web/surface_action_plans.js",
    "surface_action_plans.js"
);
javascript_route!(send_sheet_js, "src/web/send_sheet.js", "send_sheet.js");
javascript_route!(
    send_controller_js,
    "src/web/send_controller.js",
    "send_controller.js"
);
javascript_route!(
    thought_config_sheet_js,
    "src/web/thought_config_sheet.js",
    "thought_config_sheet.js"
);
javascript_route!(
    native_desktop_sheet_js,
    "src/web/native_desktop_sheet.js",
    "native_desktop_sheet.js"
);
javascript_route!(
    terminal_surface_setup_js,
    "src/web/terminal_surface_setup.js",
    "terminal_surface_setup.js"
);
javascript_route!(
    terminal_focus_js,
    "src/web/terminal_focus.js",
    "terminal_focus.js"
);
javascript_route!(
    terminal_zoom_input_js,
    "src/web/terminal_zoom_input.js",
    "terminal_zoom_input.js"
);
javascript_route!(
    terminal_resize_js,
    "src/web/terminal_resize.js",
    "terminal_resize.js"
);
javascript_route!(
    global_shortcut_dispatch_js,
    "src/web/global_shortcut_dispatch.js",
    "global_shortcut_dispatch.js"
);
javascript_route!(
    session_refresh_js,
    "src/web/session_refresh.js",
    "session_refresh.js"
);
javascript_route!(
    agent_context_refresh_js,
    "src/web/agent_context_refresh.js",
    "agent_context_refresh.js"
);
javascript_route!(
    mermaid_artifact_js,
    "src/web/mermaid_artifact.js",
    "mermaid_artifact.js"
);
javascript_route!(
    mermaid_artifact_controller_js,
    "src/web/mermaid_artifact_controller.js",
    "mermaid_artifact_controller.js"
);
javascript_route!(
    terminal_safety_js,
    "src/web/terminal_safety.js",
    "terminal_safety.js"
);
javascript_route!(
    terminal_search_links_js,
    "src/web/terminal_search_links.js",
    "terminal_search_links.js"
);
javascript_route!(
    terminal_status_js,
    "src/web/terminal_status.js",
    "terminal_status.js"
);
javascript_route!(
    terminal_protocol_js,
    "src/web/terminal_protocol.js",
    "terminal_protocol.js"
);
javascript_route!(
    terminal_input_js,
    "src/web/terminal_input.js",
    "terminal_input.js"
);
javascript_route!(
    session_socket_controller_js,
    "src/web/session_socket_controller.js",
    "session_socket_controller.js"
);
javascript_route!(dir_browser_js, "src/web/dir_browser.js", "dir_browser.js");
javascript_route!(
    dir_browser_controller_js,
    "src/web/dir_browser_controller.js",
    "dir_browser_controller.js"
);
javascript_route!(
    command_palette_js,
    "src/web/command_palette.js",
    "command_palette.js"
);
javascript_route!(
    command_palette_controller_js,
    "src/web/command_palette_controller.js",
    "command_palette_controller.js"
);
javascript_route!(
    trogdor_logic_js,
    "src/web/trogdor_logic.js",
    "trogdor_logic.js"
);
javascript_route!(
    trogdor_dom_logic_js,
    "src/web/trogdor_dom_logic.js",
    "trogdor_dom_logic.js"
);
javascript_route!(
    trogdor_render_js,
    "src/web/trogdor_render.js",
    "trogdor_render.js"
);
javascript_route!(
    workbench_dom_js,
    "src/web/workbench_dom.js",
    "workbench_dom.js"
);
javascript_route!(
    workbench_render_js,
    "src/web/workbench_render.js",
    "workbench_render.js"
);
javascript_route!(
    workbench_log_lens_js,
    "src/web/workbench_log_lens.js",
    "workbench_log_lens.js"
);
javascript_route!(
    workbench_refresh_js,
    "src/web/workbench_refresh.js",
    "workbench_refresh.js"
);
javascript_route!(
    workbench_records_js,
    "src/web/workbench_records.js",
    "workbench_records.js"
);
javascript_route!(
    terminal_workbench_controller_js,
    "src/web/terminal_workbench_controller.js",
    "terminal_workbench_controller.js"
);

pub(super) async fn app_css() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "text/css; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        app_css_body(),
    )
}

pub(super) async fn trogdor_dragon_asset(
    AxumPath((pose, frame)): AxumPath<(String, String)>,
) -> Response {
    let Some(bytes) = trogdor_dragon_asset_bytes(&pose, &frame) else {
        return super::json_error(
            StatusCode::NOT_FOUND,
            "TROGDOR_DRAGON_ASSET_NOT_FOUND",
            "The requested Trogdor dragon sprite frame is not available",
        );
    };

    (
        [
            (header::CONTENT_TYPE, "image/png"),
            (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
        ],
        bytes,
    )
        .into_response()
}

fn trogdor_dragon_asset_bytes(pose: &str, frame: &str) -> Option<&'static [u8]> {
    // All eight body frames (8-way directional) shipped for every pose. Names
    // match the on-disk filenames: cardinal directions plus 3/4 views.
    macro_rules! frames_for {
        ($pose:literal) => {
            match frame {
                "left.png" => {
                    Some(&include_bytes!(concat!("../../assets/dragon/", $pose, "/left.png"))[..])
                }
                "right.png" => {
                    Some(&include_bytes!(concat!("../../assets/dragon/", $pose, "/right.png"))[..])
                }
                "front.png" => {
                    Some(&include_bytes!(concat!("../../assets/dragon/", $pose, "/front.png"))[..])
                }
                "back.png" => {
                    Some(&include_bytes!(concat!("../../assets/dragon/", $pose, "/back.png"))[..])
                }
                "3q-left.png" => Some(
                    &include_bytes!(concat!("../../assets/dragon/", $pose, "/3q-left.png"))[..],
                ),
                "3q-right.png" => Some(
                    &include_bytes!(concat!("../../assets/dragon/", $pose, "/3q-right.png"))[..],
                ),
                "back-left.png" => Some(
                    &include_bytes!(concat!("../../assets/dragon/", $pose, "/back-left.png"))[..],
                ),
                "back-right.png" => Some(
                    &include_bytes!(concat!("../../assets/dragon/", $pose, "/back-right.png"))[..],
                ),
                _ => None,
            }
        };
    }
    match pose {
        "mouth-closed" => frames_for!("mouth-closed"),
        "mouth-open" => frames_for!("mouth-open"),
        "fire-left-short" => frames_for!("fire-left-short"),
        "fire-left-mid" => frames_for!("fire-left-mid"),
        "fire-left-full" => frames_for!("fire-left-full"),
        "fire-right-short" => frames_for!("fire-right-short"),
        "fire-right-mid" => frames_for!("fire-right-mid"),
        "fire-right-full" => frames_for!("fire-right-full"),
        _ => None,
    }
}

async fn franken_term_js() -> Response {
    serve_frankentui_asset("FrankenTerm.js", "application/javascript; charset=utf-8").await
}

async fn franken_term_wasm() -> Response {
    serve_frankentui_asset("FrankenTerm_bg.wasm", "application/wasm").await
}

async fn franken_term_font() -> Response {
    serve_franken_term_font(franken_term_font_path(resolve_frankentui_pkg_dir())).await
}

#[derive(Debug)]
pub(super) enum FrankenTermFontPath {
    Available(PathBuf),
    AssetsUnavailable,
    RootUnavailable,
}

pub(super) fn franken_term_font_path(pkg_dir: Option<PathBuf>) -> FrankenTermFontPath {
    let Some(pkg_dir) = pkg_dir else {
        return FrankenTermFontPath::AssetsUnavailable;
    };

    let Some(root_dir) = pkg_dir.parent() else {
        return FrankenTermFontPath::RootUnavailable;
    };

    FrankenTermFontPath::Available(root_dir.join("fonts").join("pragmasevka-nf-subset.woff2"))
}

pub(super) async fn serve_franken_term_font(font_path: FrankenTermFontPath) -> Response {
    let FrankenTermFontPath::Available(path) = font_path else {
        return franken_term_font_path_error(font_path);
    };

    match tokio::fs::read(&path).await {
        Ok(bytes) => (
            [
                (header::CONTENT_TYPE, "font/woff2"),
                (header::CACHE_CONTROL, "no-store"),
            ],
            bytes,
        )
            .into_response(),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => super::json_error(
            StatusCode::NOT_FOUND,
            "FRANKENTERM_FONT_UNAVAILABLE",
            &format!("font asset was not found in {}", path.display()),
        ),
        Err(err) => super::json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "FRANKENTERM_FONT_READ_FAILED",
            &format!("failed to read font asset: {err}"),
        ),
    }
}

fn franken_term_font_path_error(font_path: FrankenTermFontPath) -> Response {
    match font_path {
        FrankenTermFontPath::AssetsUnavailable => super::json_error(
            StatusCode::NOT_FOUND,
            "FRANKENTERM_ASSET_UNAVAILABLE",
            "FrankenTerm package assets are not available on this host",
        ),
        FrankenTermFontPath::RootUnavailable => super::json_error(
            StatusCode::NOT_FOUND,
            "FRANKENTERM_FONT_UNAVAILABLE",
            "FrankenTerm root directory could not be resolved",
        ),
        FrankenTermFontPath::Available(_) => unreachable!("available font paths are served first"),
    }
}

async fn serve_frankentui_asset(file_name: &str, content_type: &'static str) -> Response {
    let Some(pkg_dir) = resolve_frankentui_pkg_dir() else {
        return frankentui_asset_unavailable_response();
    };

    read_frankentui_asset_response(file_name, content_type, &pkg_dir).await
}

async fn read_frankentui_asset_response(
    file_name: &str,
    content_type: &'static str,
    pkg_dir: &Path,
) -> Response {
    let path = pkg_dir.join(file_name);
    match tokio::fs::read(&path).await {
        Ok(bytes) => frankentui_asset_response(content_type, bytes),
        Err(err) => frankentui_asset_read_error_response(file_name, pkg_dir, err),
    }
}

pub(super) fn frankentui_asset_unavailable_response() -> Response {
    super::json_error(
        StatusCode::NOT_FOUND,
        "FRANKENTERM_ASSET_UNAVAILABLE",
        "FrankenTerm package assets are not available on this host",
    )
}

pub(super) fn frankentui_asset_response(content_type: &'static str, bytes: Vec<u8>) -> Response {
    (
        [
            (header::CONTENT_TYPE, content_type),
            (header::CACHE_CONTROL, "no-store"),
        ],
        bytes,
    )
        .into_response()
}

pub(super) fn frankentui_asset_read_error_response(
    file_name: &str,
    pkg_dir: &Path,
    err: std::io::Error,
) -> Response {
    match err.kind() {
        std::io::ErrorKind::NotFound => super::json_error(
            StatusCode::NOT_FOUND,
            "FRANKENTERM_ASSET_MISSING",
            &format!("{file_name} was not found in {}", pkg_dir.display()),
        ),
        _ => super::json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "FRANKENTERM_ASSET_READ_FAILED",
            &format!("failed to read {file_name}: {err}"),
        ),
    }
}

pub(super) async fn franken_term_asset_info() -> Option<FrankenTermAssetInfo> {
    let pkg_dir = resolve_frankentui_pkg_dir()?;
    let js_path = pkg_dir.join("FrankenTerm.js");
    let wasm_path = pkg_dir.join("FrankenTerm_bg.wasm");
    let js = franken_term_asset_file_info(&js_path, FRANKENTERM_JS_ROUTE).await?;
    let wasm = franken_term_asset_file_info(&wasm_path, FRANKENTERM_WASM_ROUTE).await?;
    let font = pkg_dir
        .parent()
        .map(|root| root.join("fonts").join("pragmasevka-nf-subset.woff2"))
        .and_then(|path| {
            std::fs::metadata(&path)
                .ok()
                .filter(|meta| meta.is_file())
                .map(|_| path)
        });
    let font = match font {
        Some(path) => franken_term_asset_file_info(&path, FRANKENTERM_FONT_ROUTE).await,
        None => None,
    };
    Some(FrankenTermAssetInfo { js, wasm, font })
}

pub(super) async fn franken_term_asset_file_info(
    path: &Path,
    route: &'static str,
) -> Option<FrankenTermAssetFileInfo> {
    let bytes = tokio::fs::read(path).await.ok()?;
    Some(FrankenTermAssetFileInfo {
        route,
        size_bytes: bytes.len() as u64,
        checksum: format!("crc32:{:08x}", crc32fast::hash(&bytes)),
    })
}

pub(super) fn resolve_frankentui_pkg_dir() -> Option<PathBuf> {
    for key in ["SWIMMERS_FRANKENTUI_PKG_DIR", "FRANKENTUI_PKG_DIR"] {
        if let Some(path) = std::env::var(key)
            .ok()
            .filter(|value| !value.trim().is_empty())
        {
            let candidate = PathBuf::from(path);
            if valid_frankentui_pkg_dir(&candidate) {
                return Some(candidate);
            }
        }
    }

    DEFAULT_FRANKENTUI_PKG_CANDIDATES
        .iter()
        .map(PathBuf::from)
        .find(|candidate| valid_frankentui_pkg_dir(candidate))
}

pub(super) fn valid_frankentui_pkg_dir(path: &Path) -> bool {
    path.join("FrankenTerm.js").is_file() && path.join("FrankenTerm_bg.wasm").is_file()
}
