//! Named session server. A running editor listens on a Unix socket so a
//! second `hx` invocation can hand it files to open.

use std::io::ErrorKind;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use anyhow::Context as _;
use futures_util::Future;
use helix_core::{pos_at_coords, Position, Selection};
use helix_view::{align_view, editor::Action, tree::Layout, Align, Editor};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

/// Cap on one request so a buggy client cannot force unbounded growth.
const MAX_REQUEST_BYTES: u64 = 1 << 20;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileArg {
    pub path: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub col: Option<usize>,
}

/// How a remote open places each file. Maps to the editor's split actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Split {
    Vertical,
    Horizontal,
}

impl Split {
    fn action(self) -> Action {
        match self {
            Split::Vertical => Action::VerticalSplit,
            Split::Horizontal => Action::HorizontalSplit,
        }
    }

    /// Translate the client's `--vsplit`/`--hsplit` flag into a wire value.
    pub fn from_layout(layout: Layout) -> Self {
        match layout {
            Layout::Vertical => Split::Vertical,
            Layout::Horizontal => Split::Horizontal,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenRequest {
    pub files: Vec<FileArg>,
    /// Absent means load each file into the focused view as the current
    /// buffer. A value opens each file in that kind of split instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub split: Option<Split>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenResponse {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// A session name has to be usable as a single filesystem path component.
/// Reject anything that could escape the socket directory or confuse a shell.
pub fn validate_name(name: &str) -> anyhow::Result<()> {
    if name.is_empty() {
        anyhow::bail!("session name must not be empty");
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    {
        anyhow::bail!("session name may only contain letters, digits, '.', '_' and '-'");
    }
    Ok(())
}

pub fn socket_path(name: &str) -> anyhow::Result<PathBuf> {
    validate_name(name)?;
    Ok(helix_loader::session_socket(name))
}

/// Work out where the session socket lives: a command-line override, then the
/// config value, then the name-derived default. An override must be absolute
/// so the server and client resolve the same path from any directory.
pub fn resolve_socket_path(
    name: &str,
    cli_override: Option<&Path>,
    config_override: Option<&Path>,
) -> anyhow::Result<PathBuf> {
    match cli_override.or(config_override) {
        Some(path) if path.is_absolute() => Ok(path.to_path_buf()),
        Some(path) => anyhow::bail!("session socket path must be absolute: {}", path.display()),
        None => socket_path(name),
    }
}

/// Bind the session socket, reclaiming a stale one left by a crashed editor.
///
/// If the address is already in use we try to connect. A refused connection
/// means the old socket is dead, so we remove it and bind again. A successful
/// connection means a live editor already owns this name, so we refuse.
pub fn bind(path: &Path) -> anyhow::Result<UnixListener> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("could not create session dir {}", parent.display()))?;
    }
    match bind_private(path) {
        Ok(listener) => Ok(listener),
        Err(err) if err.kind() == ErrorKind::AddrInUse => {
            match std::os::unix::net::UnixStream::connect(path) {
                Ok(_) => {
                    anyhow::bail!("a helix session is already running at {}", path.display())
                }
                Err(_) => {
                    std::fs::remove_file(path)?;
                    bind_private(path).with_context(|| format!("could not bind {}", path.display()))
                }
            }
        }
        Err(err) => Err(err).with_context(|| format!("could not bind {}", path.display())),
    }
}

/// Bind with a private umask so the socket is owner-only from the moment it is
/// created. The explicit chmod is a backstop for platforms that do not apply
/// the umask to socket files.
fn bind_private(path: &Path) -> std::io::Result<UnixListener> {
    let prev = unsafe { libc::umask(0o177) };
    let result = UnixListener::bind(path);
    unsafe { libc::umask(prev) };
    let listener = result?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(listener)
}

/// Removes the socket file when the running editor shuts down cleanly.
pub struct SocketGuard {
    pub path: PathBuf,
}

impl Drop for SocketGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Accept connections forever, handing each decoded request to `handler`.
pub async fn serve<H, F>(listener: UnixListener, handler: H)
where
    H: Fn(OpenRequest) -> F + Clone + Send + 'static,
    F: Future<Output = OpenResponse> + Send + 'static,
{
    loop {
        match listener.accept().await {
            Ok((stream, _addr)) => {
                let handler = handler.clone();
                tokio::spawn(async move {
                    if let Err(err) = handle_connection(stream, handler).await {
                        log::warn!("session: dropping bad connection: {err}");
                    }
                });
            }
            Err(err) => {
                // A transient error like hitting the fd limit must not kill
                // the listener. Back off briefly so a persistent error cannot
                // spin the CPU, then keep accepting.
                log::warn!("session: accept failed: {err}");
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        }
    }
}

/// Read one newline-terminated JSON request, run the handler, write one
/// newline-terminated JSON response, then close.
async fn handle_connection<H, F>(stream: UnixStream, handler: H) -> anyhow::Result<()>
where
    H: Fn(OpenRequest) -> F,
    F: Future<Output = OpenResponse>,
{
    let (read_half, mut write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half.take(MAX_REQUEST_BYTES));
    let mut line = String::new();
    reader.read_line(&mut line).await?;
    let request: OpenRequest =
        serde_json::from_str(line.trim_end()).context("malformed session request")?;
    let response = handler(request).await;
    let mut encoded = serde_json::to_string(&response)?;
    encoded.push('\n');
    write_half.write_all(encoded.as_bytes()).await?;
    write_half.flush().await?;
    Ok(())
}

/// Connect to a session socket, send one request, read one response.
pub async fn send_request(path: &Path, req: &OpenRequest) -> anyhow::Result<OpenResponse> {
    let stream = UnixStream::connect(path)
        .await
        .with_context(|| format!("no helix session listening at {}", path.display()))?;
    let (read_half, mut write_half) = stream.into_split();
    let mut encoded = serde_json::to_string(req)?;
    encoded.push('\n');
    write_half.write_all(encoded.as_bytes()).await?;
    write_half.flush().await?;

    let mut reader = BufReader::new(read_half);
    let mut line = String::new();
    reader.read_line(&mut line).await?;
    serde_json::from_str(line.trim_end()).context("malformed session response")
}

/// Client mode. Send the parsed files to a running session and print the
/// outcome. Returns the process exit code.
pub async fn run_client(
    socket: &Path,
    files: &IndexMap<PathBuf, Vec<Position>>,
    split: Option<Layout>,
) -> anyhow::Result<i32> {
    if files.is_empty() {
        anyhow::bail!("--connect requires at least one file to open");
    }
    let req = OpenRequest {
        files: files
            .iter()
            .map(|(path, positions)| {
                let pos = positions.first();
                FileArg {
                    path: path.clone(),
                    line: pos.map(|p| p.row),
                    col: pos.map(|p| p.col),
                }
            })
            .collect(),
        split: split.map(Split::from_layout),
    };
    let resp = send_request(socket, &req).await?;
    if resp.ok {
        Ok(0)
    } else {
        eprintln!(
            "session at {}: {}",
            socket.display(),
            resp.error.as_deref().unwrap_or("open failed")
        );
        Ok(1)
    }
}

/// Open the requested files on the editor. Runs on the main task.
///
/// Best-effort. Every file that can be opened is opened. The returned error,
/// if any, lists the files that could not be, so a partial success reports
/// exactly what failed rather than pretending the whole request was rejected.
pub fn apply_open(editor: &mut Editor, req: &OpenRequest) -> Result<(), String> {
    let action = req.split.map_or(Action::Replace, Split::action);
    let mut errors = Vec::new();

    for file in &req.files {
        let doc_id = match editor.open(&file.path, action) {
            Ok(doc_id) => doc_id,
            Err(err) => {
                errors.push(format!("{}: {err}", file.path.display()));
                continue;
            }
        };

        if let Some(line) = file.line {
            let view_id = editor.tree.focus;
            let coords = Position::new(line, file.col.unwrap_or(0));
            {
                let doc = doc_mut!(editor, &doc_id);
                let anchor = pos_at_coords(doc.text().slice(..), coords, true);
                doc.set_selection(view_id, Selection::point(anchor));
            }
            // Center the view so this file's cursor is not stuck at the top.
            let (view, doc) = current!(editor);
            align_view(doc, view, Align::Center);
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

/// Server-side request handler. Marshals the open onto the editor's main task
/// through the job queue and waits for the result.
pub async fn open_via_jobs(req: OpenRequest) -> OpenResponse {
    let (tx, rx) = tokio::sync::oneshot::channel();
    crate::job::dispatch(move |editor, _compositor| {
        let _ = tx.send(apply_open(editor, &req));
    })
    .await;
    match rx.await {
        Ok(Ok(())) => OpenResponse {
            ok: true,
            error: None,
        },
        Ok(Err(message)) => OpenResponse {
            ok: false,
            error: Some(message),
        },
        Err(_) => OpenResponse {
            ok: false,
            error: Some("editor did not answer".into()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_round_trips_through_json() {
        let req = OpenRequest {
            files: vec![
                FileArg {
                    path: "/abs/a.rs".into(),
                    line: Some(9),
                    col: Some(2),
                },
                FileArg {
                    path: "/abs/b.rs".into(),
                    line: None,
                    col: None,
                },
            ],
            split: None,
        };
        let line = serde_json::to_string(&req).unwrap();
        assert_eq!(req, serde_json::from_str(&line).unwrap());
        // an omitted position or split must not serialize null fields
        assert!(!line.contains("null"));
        assert!(!line.contains("split"));

        // an explicit split round-trips as a snake_case tag
        let req = OpenRequest {
            files: vec![],
            split: Some(Split::Horizontal),
        };
        let line = serde_json::to_string(&req).unwrap();
        assert!(line.contains(r#""split":"horizontal""#));
        assert_eq!(req, serde_json::from_str(&line).unwrap());
    }

    #[test]
    fn response_ok_omits_error() {
        let ok = OpenResponse {
            ok: true,
            error: None,
        };
        assert_eq!(serde_json::to_string(&ok).unwrap(), r#"{"ok":true}"#);
    }

    #[test]
    fn resolve_socket_path_precedence_and_rejection() {
        let abs = Path::new("/tmp/x.sock");
        let cli = Path::new("/tmp/cli.sock");
        // cli override wins, then config, both taken verbatim when absolute
        assert_eq!(resolve_socket_path("n", Some(cli), Some(abs)).unwrap(), cli);
        assert_eq!(resolve_socket_path("n", None, Some(abs)).unwrap(), abs);
        // a relative override is rejected rather than silently cwd-resolved
        assert!(resolve_socket_path("n", Some(Path::new("rel.sock")), None).is_err());
        // no override falls back to the name-derived default
        assert!(resolve_socket_path("proj", None, None).is_ok());
    }

    #[test]
    fn name_validation_rejects_bad_names() {
        assert!(validate_name("proj").is_ok());
        assert!(validate_name("my-repo_2.0").is_ok());
        assert!(validate_name("").is_err());
        assert!(validate_name("../etc").is_err());
        assert!(validate_name("a/b").is_err());
    }

    #[tokio::test]
    async fn round_trip_open_request() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.sock");
        let listener = bind(&path).unwrap();

        let handler = |req: OpenRequest| async move {
            OpenResponse {
                ok: true,
                error: req.files.first().map(|f| f.path.display().to_string()),
            }
        };
        let server = tokio::spawn(serve(listener, handler));

        let req = OpenRequest {
            files: vec![FileArg {
                path: "/abs/x.rs".into(),
                line: Some(1),
                col: None,
            }],
            split: None,
        };
        let resp = send_request(&path, &req).await.unwrap();
        assert!(resp.ok);
        assert_eq!(resp.error.as_deref(), Some("/abs/x.rs"));

        server.abort();
    }

    #[tokio::test]
    async fn bound_socket_is_owner_only() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("perm.sock");
        let _listener = bind(&path).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[tokio::test]
    async fn bind_removes_stale_socket() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.sock");
        let first = bind(&path).unwrap();
        drop(first);
        // socket file still exists but nothing listens, so bind must reclaim it
        let _second = bind(&path).unwrap();
    }

    #[tokio::test]
    async fn send_request_errors_when_no_server() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("absent.sock");
        let req = OpenRequest {
            files: vec![],
            split: None,
        };
        assert!(send_request(&path, &req).await.is_err());
    }

    #[tokio::test]
    async fn client_reports_open_failure() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("c.sock");
        let listener = bind(&path).unwrap();
        let handler = |_req: OpenRequest| async move {
            OpenResponse {
                ok: false,
                error: Some("nope".into()),
            }
        };
        let server = tokio::spawn(serve(listener, handler));

        let req = OpenRequest {
            files: vec![FileArg {
                path: "/x".into(),
                line: None,
                col: None,
            }],
            split: None,
        };
        let resp = send_request(&path, &req).await.unwrap();
        assert!(!resp.ok);
        assert_eq!(resp.error.as_deref(), Some("nope"));

        server.abort();
    }
}
