//! Native action commands for desktop convenience operations.
#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tauri::command;
use tauri_plugin_opener::OpenerExt;

use crate::types::{CommandResult, DesktopError};

#[derive(Debug, Deserialize)]
pub struct OpenFileRequest {
    pub title: Option<String>,
    pub accepts: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
pub struct OpenFileResponse {
    pub path: Option<String>,
}

#[command]
pub async fn open_file(_req: OpenFileRequest) -> CommandResult<OpenFileResponse> {
    Ok(OpenFileResponse { path: None })
}

#[derive(Debug, Serialize)]
pub struct OpenFolderResponse {
    pub path: Option<String>,
}

#[command]
pub async fn open_folder(_title: Option<String>) -> CommandResult<OpenFolderResponse> {
    Ok(OpenFolderResponse { path: None })
}

#[derive(Debug, Deserialize)]
pub struct OpenRepositoryFolderRequest {
    pub path: String,
}

#[derive(Debug, Serialize)]
pub struct OpenRepositoryFolderResponse {
    pub opened: bool,
}

#[command]
pub async fn open_repository_folder(
    app: tauri::AppHandle,
    req: OpenRepositoryFolderRequest,
) -> CommandResult<OpenRepositoryFolderResponse> {
    let p = Path::new(&req.path);
    let canon = canonicalize_action_path(p)?;
    if !is_safe_workspace_path(&canon) {
        return Err(DesktopError::PermissionDenied);
    }
    let url = url::Url::from_file_path(&canon).map_err(|_| DesktopError::Internal {
        message: "invalid file path".into(),
    })?;
    app.opener()
        .open_url(url.as_str(), None::<&str>)
        .map_err(|e| DesktopError::Internal {
            message: format!("failed to open folder: {e}"),
        })?;
    Ok(OpenRepositoryFolderResponse { opened: true })
}

#[derive(Debug, Deserialize)]
pub struct RevealWorkspaceRequest {
    pub path: String,
    pub safety_token: String,
}

#[derive(Debug, Serialize)]
pub struct RevealWorkspaceResponse {
    pub revealed: bool,
}

#[command]
pub async fn reveal_workspace(
    app: tauri::AppHandle,
    req: RevealWorkspaceRequest,
) -> CommandResult<RevealWorkspaceResponse> {
    if req.safety_token != "opensymphony-workspace" {
        return Err(DesktopError::PermissionDenied);
    }
    let p = Path::new(&req.path);

    let canon = canonicalize_action_path(p)?;
    let canon_base = canonical_workspace_base()?;
    if !path_is_under_existing_base(&canon, &canon_base) {
        return Err(DesktopError::PermissionDenied);
    }

    let url = url::Url::from_file_path(&canon).map_err(|_| DesktopError::Internal {
        message: "invalid workspace path".into(),
    })?;
    app.opener()
        .open_url(url.as_str(), None::<&str>)
        .map_err(|e| DesktopError::Internal {
            message: format!("failed to reveal workspace: {e}"),
        })?;
    Ok(RevealWorkspaceResponse { revealed: true })
}

fn canonicalize_action_path(path: &Path) -> Result<PathBuf, DesktopError> {
    path.canonicalize().map_err(desktop_error_from_canonicalize)
}

fn desktop_error_from_canonicalize(error: std::io::Error) -> DesktopError {
    match error.kind() {
        std::io::ErrorKind::NotFound => DesktopError::NotFound,
        std::io::ErrorKind::PermissionDenied => DesktopError::PermissionDenied,
        _ => DesktopError::Internal {
            message: format!("failed to canonicalize: {error}"),
        },
    }
}

fn canonical_workspace_base() -> Result<PathBuf, DesktopError> {
    let home = dirs::home_dir().ok_or_else(|| DesktopError::Internal {
        message: "could not determine home directory".into(),
    })?;
    let canon_home = home.canonicalize().map_err(|e| DesktopError::Internal {
        message: format!("failed to canonicalize home directory: {e}"),
    })?;
    canonicalize_action_path(&canon_home.join(".opensymphony").join("workspaces"))
}

fn is_safe_workspace_path(path: &Path) -> bool {
    let Ok(canon_base) = canonical_workspace_base() else {
        return false;
    };
    path_is_under_existing_base(path, &canon_base)
}

fn path_is_under_existing_base(path: &Path, canon_base: &Path) -> bool {
    let Ok(canon_path) = path.canonicalize() else {
        return false;
    };
    canon_path.starts_with(canon_base)
}

#[derive(Debug, Deserialize)]
pub struct CopyToClipboardRequest {
    pub text: String,
}

#[derive(Debug, Serialize)]
pub struct CopyToClipboardResponse {
    pub copied: bool,
}

#[command]
pub async fn copy_to_clipboard(
    app: tauri::AppHandle,
    req: CopyToClipboardRequest,
) -> CommandResult<CopyToClipboardResponse> {
    use tauri_plugin_clipboard_manager::ClipboardExt;
    app.clipboard()
        .write_text(&req.text)
        .map_err(|e| DesktopError::Internal {
            message: format!("failed to copy: {e}"),
        })?;
    Ok(CopyToClipboardResponse { copied: true })
}

#[derive(Debug, Deserialize)]
pub struct OpenLinearLinkRequest {
    pub issue_id: String,
}

// Centralized Linear workspace base URL for deployment-specific builds.
const LINEAR_WORKSPACE_BASE: &str = "https://linear.app/trilogy-ai-coe";

fn linear_issue_url(issue_id: &str) -> String {
    let encoded_id = urlencoding::encode(issue_id);
    format!("{}/issue/{}", LINEAR_WORKSPACE_BASE, encoded_id)
}

#[command]
pub async fn open_linear_link(
    app: tauri::AppHandle,
    req: OpenLinearLinkRequest,
) -> CommandResult<()> {
    let url = linear_issue_url(&req.issue_id);
    app.opener()
        .open_url(&url, None::<&str>)
        .map_err(|e| DesktopError::Internal {
            message: format!("failed to open link: {e}"),
        })
}

#[derive(Debug, Deserialize)]
pub struct NotifyRequest {
    pub title: String,
    pub body: String,
}

#[derive(Debug, Serialize)]
pub struct NotifyResponse {
    pub acknowledged: bool,
}

#[command]
pub async fn notify(app: tauri::AppHandle, req: NotifyRequest) -> CommandResult<NotifyResponse> {
    use tauri_plugin_notification::NotificationExt;
    app.notification()
        .builder()
        .title(&req.title)
        .body(&req.body)
        .show()
        .map_err(|e| DesktopError::Internal {
            message: format!("failed to show notification: {e}"),
        })?;
    Ok(NotifyResponse { acknowledged: true })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_linear_url_constant() {
        assert!(LINEAR_WORKSPACE_BASE.starts_with("https://linear.app/"));
        let url = linear_issue_url("COE-123");
        assert_eq!(url, format!("{}/issue/COE-123", LINEAR_WORKSPACE_BASE));
    }

    #[test]
    fn test_workspace_path_allows_nested_existing_path() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().join("workspaces");
        let nested = base.join("issue").join("nested");
        std::fs::create_dir_all(&nested).unwrap();
        let canon_base = base.canonicalize().unwrap();

        assert!(
            path_is_under_existing_base(&nested, &canon_base),
            "existing paths under the workspace base should be allowed"
        );
    }

    #[test]
    fn test_is_safe_workspace_path_blocks_paths_outside_whitelist() {
        let path = std::path::Path::new("/System/Volumes/Data");
        assert!(!is_safe_workspace_path(path));
        let path = std::path::Path::new("/usr/bin/something");
        assert!(!is_safe_workspace_path(path));
        let path = std::path::Path::new("/etc/passwd");
        assert!(!is_safe_workspace_path(path));
        let path = std::path::Path::new("/private/var/folders");
        assert!(!is_safe_workspace_path(path));
    }

    #[test]
    fn test_is_safe_workspace_path_blocks_workspace_like_paths_outside_whitelist() {
        let blocked = vec![
            "/System/Volumes/Data/.opensymphony/workspaces/escape",
            "/usr/local/.opensymphony/workspaces/escape",
            "/etc/opensymphony/workspaces/escape",
            "/private/var/folders/.opensymphony/test",
        ];
        for path_str in blocked {
            assert!(
                !is_safe_workspace_path(std::path::Path::new(path_str)),
                "Path {path_str} should be blocked"
            );
        }
    }

    #[test]
    fn test_canonicalize_not_found_maps_to_desktop_error() {
        let path = std::path::Path::new("/definitely/does/not/exist/12345");
        let result = canonicalize_action_path(path);
        assert!(matches!(result, Err(DesktopError::NotFound)));
    }

    #[test]
    fn test_canonicalize_permission_denied_maps_to_desktop_error() {
        let error = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        assert!(matches!(
            desktop_error_from_canonicalize(error),
            DesktopError::PermissionDenied
        ));
    }

    #[test]
    fn test_path_under_existing_base_allows_opensymphony_subdirs() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().join("workspaces");
        let valid = vec![
            base.join("test-subdir-1"),
            base.join("test-subdir-2").join("nested"),
            base.join("test-subdir-3"),
        ];

        for path in &valid {
            std::fs::create_dir_all(path).unwrap();
        }
        let canon_base = base.canonicalize().unwrap();

        for path in &valid {
            assert!(
                path_is_under_existing_base(path, &canon_base),
                "Path {:?} should be allowed",
                path
            );
        }
    }

    #[test]
    fn test_path_under_existing_base_blocks_path_traversal_attempts() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().join("workspaces");
        let outside = tmp.path().join("outside");
        std::fs::create_dir_all(&base).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        let canon_base = base.canonicalize().unwrap();
        let blocked = vec![base.join("..").join("outside")];

        for path in blocked {
            assert!(
                !path_is_under_existing_base(&path, &canon_base),
                "Path {:?} should be blocked",
                path
            );
        }
    }

    #[test]
    fn test_notify_request_deserialize() {
        let json = r#"{"title":"T","body":"B"}"#;
        let req: NotifyRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.title, "T");
        assert_eq!(req.body, "B");
    }

    #[cfg(unix)]
    #[test]
    fn test_symlink_inside_workspace_is_rejected() {
        use std::os::unix::fs::symlink;

        let tmp = tempfile::tempdir().unwrap();
        let workspace_base = tmp.path().join("workspaces");
        let outside = tmp.path().join("outside");
        std::fs::create_dir_all(&workspace_base).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        let canon_base = workspace_base.canonicalize().unwrap();

        let symlink_path = workspace_base.join("escape");
        symlink(&outside, &symlink_path).unwrap();

        assert!(
            !path_is_under_existing_base(&symlink_path, &canon_base),
            "symlinks inside the workspace base pointing outside should be rejected after canonicalization"
        );
    }

    #[test]
    fn test_open_linear_link_request_url_encoding() {
        let url = linear_issue_url("COE-409");
        assert_eq!(url, format!("{}/issue/COE-409", LINEAR_WORKSPACE_BASE));

        let special_url = linear_issue_url("COE-409/test");
        assert_eq!(
            special_url,
            format!("{}/issue/COE-409%2Ftest", LINEAR_WORKSPACE_BASE)
        );
    }

    #[test]
    fn test_open_repository_folder_rejects_etc_path() {
        // /etc should be rejected by is_safe_workspace_path
        let path = std::path::Path::new("/etc");
        if let Ok(canon) = path.canonicalize() {
            assert!(
                !is_safe_workspace_path(&canon),
                "/etc should be rejected by workspace safety check"
            );
        }
    }

    #[test]
    fn test_reveal_workspace_path_resembles_opensymphony_elsewhere() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp
            .path()
            .join(".opensymphony")
            .join("workspaces")
            .join("fake");
        std::fs::create_dir_all(&path).unwrap();

        assert!(
            !is_safe_workspace_path(&path),
            "workspace-like paths outside the real workspace root should be rejected"
        );
    }
}
