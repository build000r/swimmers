use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Extension, Json, Router};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::UNIX_EPOCH;

use crate::api::AppState;
use crate::auth::{AuthInfo, AuthScope};
use crate::types::{DirEntry, DirListResponse, ErrorResponse};

#[derive(serde::Deserialize)]
struct DirQuery {
    path: Option<String>,
}

/// Base path for directory browsing. Falls back to the server's cwd.
fn dirs_base_path() -> PathBuf {
    std::env::var("DIRS_BASE_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")))
}

// GET /v1/dirs?path=...
async fn list_dirs(
    Extension(auth): Extension<AuthInfo>,
    State(_state): State<Arc<AppState>>,
    Query(query): Query<DirQuery>,
) -> impl IntoResponse {
    if let Err(resp) = auth.require_scope(AuthScope::SessionsRead) {
        return resp;
    }

    let base = dirs_base_path();
    let target = match &query.path {
        Some(p) if !p.is_empty() => PathBuf::from(p),
        _ => base.clone(),
    };

    // Canonicalize to prevent traversal attacks.
    let canonical = match target.canonicalize() {
        Ok(p) => p,
        Err(_) => {
            return (
                StatusCode::NOT_FOUND,
                Json(
                    serde_json::to_value(ErrorResponse {
                        code: "DIR_NOT_FOUND".to_string(),
                        message: Some(format!("directory not found: {}", target.display())),
                    })
                    .unwrap(),
                ),
            )
                .into_response();
        }
    };

    let canonical_base = base.canonicalize().unwrap_or(base);
    if !canonical.starts_with(&canonical_base) {
        return (
            StatusCode::FORBIDDEN,
            Json(
                serde_json::to_value(ErrorResponse {
                    code: "DIR_OUTSIDE_BASE".to_string(),
                    message: Some("path is outside the allowed base directory".to_string()),
                })
                .unwrap(),
            ),
        )
            .into_response();
    }

    // Read directory entries.
    let read_dir = match std::fs::read_dir(&canonical) {
        Ok(rd) => rd,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(
                    serde_json::to_value(ErrorResponse {
                        code: "DIR_READ_ERROR".to_string(),
                        message: Some(e.to_string()),
                    })
                    .unwrap(),
                ),
            )
                .into_response();
        }
    };

    let mut entries: Vec<(DirEntry, u64)> = Vec::new();
    for entry in read_dir.flatten() {
        let Ok(ft) = entry.file_type() else {
            continue;
        };
        if !ft.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        // Skip hidden directories.
        if name.starts_with('.') {
            continue;
        }
        // Check if this dir has any subdirectories.
        let has_children = std::fs::read_dir(entry.path())
            .map(|rd| {
                rd.flatten().any(|e| {
                    e.file_type().map(|ft| ft.is_dir()).unwrap_or(false)
                        && !e.file_name().to_string_lossy().starts_with('.')
                })
            })
            .unwrap_or(false);

        let modified_at = entry
            .metadata()
            .ok()
            .and_then(|meta| meta.modified().ok())
            .and_then(|mtime| mtime.duration_since(UNIX_EPOCH).ok())
            .map(|dur| dur.as_secs())
            .unwrap_or(0);

        entries.push((DirEntry { name, has_children }, modified_at));
    }

    entries.sort_by(|(a, a_modified), (b, b_modified)| {
        b_modified
            .cmp(a_modified)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });

    let entries: Vec<DirEntry> = entries.into_iter().map(|(entry, _)| entry).collect();

    (
        StatusCode::OK,
        Json(
            serde_json::to_value(DirListResponse {
                path: canonical.to_string_lossy().into_owned(),
                entries,
            })
            .unwrap(),
        ),
    )
        .into_response()
}

pub fn routes() -> Router<Arc<AppState>> {
    Router::new().route("/v1/dirs", get(list_dirs))
}
