//! Short-lived, read-only file preview links for the WebUI.
//!
//! Model answers can contain local file paths (`file:///...` or absolute
//! workspace paths). A remote browser cannot open those directly. The WebUI
//! exchanges the path for an opaque, short-lived ticket and opens a separate
//! preview page. Tickets are capabilities: they never contain the long-lived
//! WebUI access token and are bounded by TTL and count.

use super::{authorized_api_request, html_escape, AppState, AuthQuery};
use axum::{
    extract::{ConnectInfo, Path as AxumPath, Query, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    collections::HashMap,
    io::Read,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, Instant},
};

pub const FILE_PREVIEW_TTL: Duration = Duration::from_secs(10 * 60);
const MAX_FILE_PREVIEW_TICKETS: usize = 128;
const MAX_TEXT_PREVIEW_BYTES: u64 = 1024 * 1024;
const MAX_IMAGE_PREVIEW_BYTES: u64 = 16 * 1024 * 1024;
const MAX_FILE_DOWNLOAD_BYTES: u64 = 64 * 1024 * 1024;
const MAX_PREVIEW_PATH_CHARS: usize = 4096;

#[derive(Debug, Clone)]
pub struct FilePreviewTicket {
    pub path: PathBuf,
    pub roots: Vec<PathBuf>,
    created_at: Instant,
}

/// In-memory ticket store. A preview link is deliberately not durable: a
/// runtime restart invalidates every outstanding link.
#[derive(Default)]
pub struct FilePreviewLinks {
    tickets: Mutex<HashMap<String, FilePreviewTicket>>,
}

impl FilePreviewLinks {
    pub fn issue(&self, path: PathBuf, roots: Vec<PathBuf>) -> Result<String, String> {
        self.issue_at(Instant::now(), path, roots)
    }

    pub(crate) fn issue_at(
        &self,
        now: Instant,
        path: PathBuf,
        roots: Vec<PathBuf>,
    ) -> Result<String, String> {
        if roots.is_empty() {
            return Err("no_preview_root".to_string());
        }
        let mut tickets = self
            .tickets
            .lock()
            .map_err(|_| "file_preview_store_poisoned".to_string())?;
        purge_expired(&mut tickets, now);
        if tickets.len() >= MAX_FILE_PREVIEW_TICKETS {
            let oldest = tickets
                .iter()
                .min_by_key(|(_, ticket)| ticket.created_at)
                .map(|(token, _)| token.clone());
            if let Some(token) = oldest {
                tickets.remove(&token);
            }
        }
        let token = new_ticket_token()?;
        tickets.insert(
            token.clone(),
            FilePreviewTicket {
                path,
                roots,
                created_at: now,
            },
        );
        Ok(token)
    }

    pub fn get(&self, token: &str) -> Option<FilePreviewTicket> {
        self.get_at(Instant::now(), token)
    }

    pub(crate) fn get_at(&self, now: Instant, token: &str) -> Option<FilePreviewTicket> {
        if token.is_empty() || token.len() > 128 {
            return None;
        }
        let mut tickets = self.tickets.lock().ok()?;
        purge_expired(&mut tickets, now);
        let ticket = tickets.get(token).cloned()?;
        if now.duration_since(ticket.created_at) >= FILE_PREVIEW_TTL {
            tickets.remove(token);
            return None;
        }
        Some(ticket)
    }
}

fn purge_expired(tickets: &mut HashMap<String, FilePreviewTicket>, now: Instant) {
    tickets.retain(|_, ticket| now.duration_since(ticket.created_at) < FILE_PREVIEW_TTL);
}

fn new_ticket_token() -> Result<String, String> {
    let mut bytes = [0_u8; 16];
    agent_core::os::fill_secure_random(&mut bytes)
        .map_err(|error| format!("file_preview_ticket_generation_failed:{error}"))?;
    let mut token = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(&mut token, "{byte:02x}");
    }
    Ok(token)
}

/// Canonical directories a preview may read from. The session CWD is always
/// first; configured workspace roots are added when they exist. Nonexistent
/// roots are ignored rather than widening the path boundary.
pub fn preview_roots(session_dir: &Path, workspace_dirs: &[PathBuf]) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    for path in std::iter::once(session_dir).chain(workspace_dirs.iter().map(PathBuf::as_path)) {
        let Ok(canonical) = path.canonicalize() else {
            continue;
        };
        if canonical.is_dir() && !roots.contains(&canonical) {
            roots.push(canonical);
        }
    }
    roots
}

/// Resolves a requested path (absolute or relative to the first root) and
/// verifies the canonical target is a regular file inside one of the roots.
/// Symlinks are resolved before the boundary check, so `..` and symlink
/// escapes cannot cross the workspace boundary.
pub fn resolve_preview_target(roots: &[PathBuf], requested: &str) -> Result<PathBuf, String> {
    let requested = requested.trim();
    if requested.is_empty() {
        return Err("empty_file_preview_path".to_string());
    }
    if requested.chars().count() > MAX_PREVIEW_PATH_CHARS {
        return Err("file_preview_path_too_long".to_string());
    }
    let requested_path = PathBuf::from(requested);
    let target = if requested_path.is_absolute() {
        requested_path
    } else {
        roots
            .first()
            .ok_or_else(|| "no_preview_root".to_string())?
            .join(requested_path)
    };
    let canonical = target
        .canonicalize()
        .map_err(|_| "file_preview_not_found".to_string())?;
    if !canonical.is_file() {
        return Err("file_preview_not_a_file".to_string());
    }
    if !roots.iter().any(|root| canonical.starts_with(root)) {
        return Err("file_preview_outside_workspace".to_string());
    }
    Ok(canonical)
}

#[derive(Debug, Deserialize)]
pub struct FilePreviewLinkRequest {
    pub session_id: String,
    pub path: String,
}

#[derive(Debug, Serialize)]
struct FilePreviewLinkResponse {
    url: String,
    name: String,
    expires_in_seconds: u64,
}

#[derive(Debug, Default, Deserialize)]
pub struct FilePreviewQuery {
    #[serde(default)]
    download: bool,
    #[serde(default)]
    raw: bool,
}

pub async fn create_file_preview_link(
    State((state, _)): State<(AppState, u16)>,
    Query(auth): Query<AuthQuery>,
    headers: HeaderMap,
    ConnectInfo(connect_info): ConnectInfo<SocketAddr>,
    Json(request): Json<FilePreviewLinkRequest>,
) -> Response {
    if !authorized_api_request(&state, auth.token.as_deref(), &headers, connect_info) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let session_dir = {
        let Ok(sessions) = state.sessions.lock() else {
            return json_error(StatusCode::INTERNAL_SERVER_ERROR, "session_state_poisoned");
        };
        let Some(session) = sessions.get(request.session_id.trim()) else {
            return json_error(StatusCode::NOT_FOUND, "session_not_found");
        };
        PathBuf::from(&session.current_dir)
    };
    let roots = preview_roots(&session_dir, &state.template.workspace_dirs);
    let target = match resolve_preview_target(&roots, &request.path) {
        Ok(target) => target,
        Err(error) => return json_error(StatusCode::BAD_REQUEST, &error),
    };
    let name = display_name(&target);
    let token = match state.file_previews.issue(target, roots) {
        Ok(token) => token,
        Err(error) => return json_error(StatusCode::INTERNAL_SERVER_ERROR, &error),
    };
    (
        StatusCode::OK,
        Json(FilePreviewLinkResponse {
            url: format!("/api/file-preview/{token}"),
            name,
            expires_in_seconds: FILE_PREVIEW_TTL.as_secs(),
        }),
    )
        .into_response()
}

pub async fn serve_file_preview(
    State((state, _)): State<(AppState, u16)>,
    AxumPath(token): AxumPath<String>,
    Query(query): Query<FilePreviewQuery>,
) -> Response {
    let Some(ticket) = state.file_previews.get(&token) else {
        return preview_error_page(
            StatusCode::NOT_FOUND,
            "This preview link is invalid or has expired.",
        );
    };
    let target = match resolve_preview_target(&ticket.roots, &ticket.path.to_string_lossy()) {
        Ok(target) => target,
        Err(error) => return preview_error_page(StatusCode::GONE, &error),
    };
    let metadata = match std::fs::metadata(&target) {
        Ok(metadata) if metadata.is_file() => metadata,
        _ => return preview_error_page(StatusCode::GONE, "The file is no longer available."),
    };
    if query.download {
        return download_response(&target, metadata.len()).await;
    }
    if let Some(content_type) = image_content_type(&target) {
        if query.raw {
            if metadata.len() > MAX_IMAGE_PREVIEW_BYTES {
                return preview_error_page(StatusCode::PAYLOAD_TOO_LARGE, "Image is too large.");
            }
            return match tokio::fs::read(&target).await {
                Ok(bytes) if bytes.len() as u64 <= MAX_IMAGE_PREVIEW_BYTES => (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, HeaderValue::from_static(content_type))],
                    bytes,
                )
                    .into_response(),
                Ok(_) => preview_error_page(StatusCode::PAYLOAD_TOO_LARGE, "Image is too large."),
                Err(_) => preview_error_page(StatusCode::GONE, "The file is no longer readable."),
            };
        }
        return preview_image_page(&target, metadata.len());
    }
    if query.raw {
        return preview_error_page(StatusCode::BAD_REQUEST, "This file is not an image.");
    }
    preview_text_page(&target, metadata.len())
}

async fn download_response(target: &Path, size: u64) -> Response {
    if size > MAX_FILE_DOWNLOAD_BYTES {
        return preview_error_page(
            StatusCode::PAYLOAD_TOO_LARGE,
            "This file is too large to download from the preview page.",
        );
    }
    let bytes = match tokio::fs::read(target).await {
        Ok(bytes) if bytes.len() as u64 <= MAX_FILE_DOWNLOAD_BYTES => bytes,
        Ok(_) => {
            return preview_error_page(
                StatusCode::PAYLOAD_TOO_LARGE,
                "This file is too large to download from the preview page.",
            )
        }
        Err(_) => return preview_error_page(StatusCode::GONE, "The file is no longer readable."),
    };
    let filename = safe_header_filename(&display_name(target));
    let disposition = HeaderValue::from_str(&format!("attachment; filename=\"{filename}\""))
        .unwrap_or_else(|_| HeaderValue::from_static("attachment"));
    (
        StatusCode::OK,
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/octet-stream"),
            ),
            (header::CONTENT_DISPOSITION, disposition),
        ],
        bytes,
    )
        .into_response()
}

fn preview_text_page(target: &Path, total_bytes: u64) -> Response {
    let truncated = total_bytes > MAX_TEXT_PREVIEW_BYTES;
    let mut head = Vec::new();
    let read_ok = std::fs::File::open(target)
        .and_then(|file| {
            let mut limited = file.take(MAX_TEXT_PREVIEW_BYTES);
            limited.read_to_end(&mut head)
        })
        .is_ok();
    if !read_ok {
        return preview_error_page(StatusCode::GONE, "The file is no longer readable.");
    }
    let mut text = String::from_utf8_lossy(&head).into_owned();
    if truncated && text.ends_with('\u{FFFD}') {
        text.pop();
    }
    if text.contains('\0') {
        return preview_binary_page(target, total_bytes);
    }
    let can_download = total_bytes <= MAX_FILE_DOWNLOAD_BYTES;
    let notice = if truncated {
        if can_download {
            format!(
                "<p class=\"notice\">Showing the first {} bytes of {} bytes. <a href=\"?download=1\">Download the full file</a>.</p>",
                head.len(),
                total_bytes
            )
        } else {
            format!(
                "<p class=\"notice\">Showing the first {} bytes of {} bytes. This file is too large to download from the preview page.</p>",
                head.len(),
                total_bytes
            )
        }
    } else {
        String::new()
    };
    let download = if !truncated && can_download {
        "<p><a href=\"?download=1\">Download</a></p>".to_string()
    } else {
        String::new()
    };
    let body = format!("{notice}{download}<pre>{}</pre>", html_escape(&text));
    html_response(page_shell(
        &display_name(target),
        &path_label(target, total_bytes),
        &body,
    ))
}

fn preview_image_page(target: &Path, size: u64) -> Response {
    let body = if size > MAX_IMAGE_PREVIEW_BYTES {
        let download = if size <= MAX_FILE_DOWNLOAD_BYTES {
            " <a href=\"?download=1\">Download the full image</a>."
        } else {
            ""
        };
        format!(
            "<p class=\"notice\">Image is {} bytes; preview is limited to {} bytes.{} </p>",
            size, MAX_IMAGE_PREVIEW_BYTES, download
        )
    } else {
        format!(
            "<p><img class=\"preview-image\" src=\"?raw=1\" alt=\"{}\"></p><p><a href=\"?download=1\">Download</a></p>",
            html_escape(&display_name(target))
        )
    };
    html_response(page_shell(
        &display_name(target),
        &path_label(target, size),
        &body,
    ))
}

fn preview_binary_page(target: &Path, size: u64) -> Response {
    let download = if size <= MAX_FILE_DOWNLOAD_BYTES {
        "<p><a href=\"?download=1\">Download file</a></p>"
    } else {
        "<p class=\"notice\">This file is too large to download from the preview page.</p>"
    };
    let body = format!(
        "<p class=\"notice\">Binary file ({} bytes). It is not rendered as text.</p>{}",
        size, download
    );
    html_response(page_shell(
        &display_name(target),
        &path_label(target, size),
        &body,
    ))
}

fn preview_error_page(status: StatusCode, message: &str) -> Response {
    let body = format!("<p class=\"notice\">{}</p>", html_escape(message));
    let mut response = (
        status,
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/html; charset=utf-8"),
        )],
        page_shell("Timem file preview", "", &body),
    )
        .into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

fn html_response(body: String) -> Response {
    (
        StatusCode::OK,
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/html; charset=utf-8"),
        )],
        body,
    )
        .into_response()
}

fn page_shell(title: &str, subtitle: &str, body: &str) -> String {
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>{title}</title><style>body{{margin:0;background:#111;color:#d8e0e5;font:14px/1.55 ui-sans-serif,system-ui,sans-serif}}main{{max-width:1100px;margin:0 auto;padding:24px}}h1{{font-size:18px;margin:0 0 4px}}p{{color:#9aabb3}}a{{color:#7ab7ff}}pre{{background:#1b1b1b;border-radius:6px;padding:14px;overflow:auto;white-space:pre-wrap;overflow-wrap:anywhere}}img.preview-image{{display:block;max-width:100%;height:auto;margin:18px 0;background:#1b1b1b}}p.notice{{background:#2a2118;border:1px solid #7a5a2a;border-radius:6px;padding:10px 12px;color:#f1c56f}}.muted{{color:#7f929e}}</style></head><body><main><h1>{title}</h1>{subtitle}{body}</main></body></html>",
        title = html_escape(title),
        subtitle = if subtitle.is_empty() {
            String::new()
        } else {
            format!("<p class=\"muted\">{}</p>", html_escape(subtitle))
        },
        body = body,
    )
}

fn path_label(target: &Path, size: u64) -> String {
    format!("{} · {} bytes", target.display(), size)
}

fn display_name(target: &Path) -> String {
    target
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".to_string())
}

fn safe_header_filename(name: &str) -> String {
    let sanitized: String = name
        .chars()
        .map(|ch| {
            if ch.is_ascii() && !ch.is_ascii_control() && ch != '"' && ch != '\\' {
                ch
            } else {
                '_'
            }
        })
        .collect();
    if sanitized.is_empty() {
        "download".to_string()
    } else {
        sanitized
    }
}

fn image_content_type(path: &Path) -> Option<&'static str> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    match extension.as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        "bmp" => Some("image/bmp"),
        "ico" => Some("image/x-icon"),
        "avif" => Some("image/avif"),
        _ => None,
    }
}

fn json_error(status: StatusCode, error: &str) -> Response {
    (status, Json(json!({ "error": error }))).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("timem_file_preview_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn resolves_only_regular_files_inside_canonical_roots() {
        let root = temp_dir("resolve_root");
        let outside = temp_dir("resolve_outside");
        std::fs::write(root.join("inside.txt"), "ok").unwrap();
        std::fs::write(root.join("nested.txt"), "ok").unwrap();
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("sub/deep.txt"), "ok").unwrap();
        std::fs::write(outside.join("outside.txt"), "no").unwrap();
        let roots = preview_roots(&root, &[]);
        assert_eq!(
            resolve_preview_target(&roots, "inside.txt").unwrap(),
            root.join("inside.txt").canonicalize().unwrap()
        );
        assert!(resolve_preview_target(&roots, "sub/deep.txt").is_ok());
        // 绝对路径必须真实存在才能走到 roots 检查（canonicalize 失败先报
        // not_found）；按平台取一个 workspace 之外必然存在的系统文件。
        let absolute_escape = if cfg!(windows) {
            std::env::var("SystemRoot")
                .map(|root| format!(r"{root}\System32\drivers\etc\hosts"))
                .unwrap_or_else(|_| r"C:\Windows\System32\drivers\etc\hosts".to_string())
        } else {
            "/etc/passwd".to_string()
        };
        assert_eq!(
            resolve_preview_target(&roots, &absolute_escape).unwrap_err(),
            "file_preview_outside_workspace"
        );
        assert_eq!(
            resolve_preview_target(&roots, &outside.join("outside.txt").to_string_lossy())
                .unwrap_err(),
            "file_preview_outside_workspace"
        );
        assert_eq!(
            resolve_preview_target(&roots, "sub").unwrap_err(),
            "file_preview_not_a_file"
        );
        assert_eq!(
            resolve_preview_target(&roots, "missing.txt").unwrap_err(),
            "file_preview_not_found"
        );
        assert_eq!(
            resolve_preview_target(&roots, "").unwrap_err(),
            "empty_file_preview_path"
        );
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(outside.join("outside.txt"), root.join("outside-link.txt"))
                .unwrap();
            assert_eq!(
                resolve_preview_target(&roots, "outside-link.txt").unwrap_err(),
                "file_preview_outside_workspace"
            );
        }
        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(outside);
    }

    #[test]
    fn tickets_are_bounded_and_expire() {
        let root = temp_dir("tickets");
        let file = root.join("note.txt");
        std::fs::write(&file, "hello").unwrap();
        let roots = preview_roots(&root, &[]);
        let store = FilePreviewLinks::default();
        let start = Instant::now();
        let token = store.issue_at(start, file.clone(), roots.clone()).unwrap();
        assert!(store
            .get_at(start + Duration::from_secs(1), &token)
            .is_some());
        assert!(store.get_at(start + FILE_PREVIEW_TTL, &token).is_none());
        for index in 0..=MAX_FILE_PREVIEW_TICKETS {
            store
                .issue_at(
                    start + Duration::from_millis(index as u64),
                    file.clone(),
                    roots.clone(),
                )
                .unwrap();
        }
        assert!(store
            .get_at(start + Duration::from_secs(1), &token)
            .is_none());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn image_types_are_explicit_and_svg_stays_text() {
        assert_eq!(image_content_type(Path::new("a.PNG")), Some("image/png"));
        assert_eq!(image_content_type(Path::new("a.jpeg")), Some("image/jpeg"));
        assert_eq!(image_content_type(Path::new("a.svg")), None);
        assert_eq!(image_content_type(Path::new("a.txt")), None);
    }

    #[test]
    fn download_filename_header_is_ascii_safe() {
        assert_eq!(safe_header_filename("a\"b\\c.txt"), "a_b_c.txt");
        assert_eq!(safe_header_filename("报告.txt"), "__.txt");
        assert_eq!(safe_header_filename(""), "download");
    }
}
