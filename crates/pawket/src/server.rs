use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
    path::{Path as FilePath, PathBuf},
    process::{Command, Stdio},
    sync::Arc,
    time::{Duration, Instant},
};

use crate::history::{History, HistoryEntry};
use crate::jobs::{JobStatus, JobStore, JobView};
use axum::{
    body::Body,
    extract::{
        rejection::{JsonRejection, QueryRejection},
        DefaultBodyLimit, Path, Query, Request, State,
    },
    http::{header, HeaderMap, HeaderValue, Method, StatusCode},
    middleware::{self, Next},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use pawket_core::{DownloadResult, Engine, MediaFormat, MediaInfo, PawketError, Progress};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::{
    fs,
    sync::{Mutex, Semaphore},
};
use tokio_util::io::ReaderStream;
use tower_http::services::ServeDir;
use uuid::Uuid;

const INDEX: &str = include_str!("../../../web/index.html");
const STYLE: &str = include_str!("../../../web/style.css");
const BOOTSTRAP: &str = include_str!("../../../web/bootstrap.js");
const BROWSER_JS: &str = include_str!("../../../web/pkg/pawket_browser.js");
const BROWSER_WASM: &[u8] = include_bytes!("../../../web/pkg/pawket_browser_bg.wasm");
const FILE_TTL: Duration = Duration::from_secs(30 * 60);
const PLAN_TTL: Duration = Duration::from_secs(10 * 60);
const MAX_FILES: usize = 20;
const MAX_PLANS: usize = 64;
/// Queued plus running background downloads allowed at once.
const MAX_ACTIVE_JOBS: usize = 12;

#[derive(Clone)]
struct AppState {
    folder: crate::folder::Folder,
    engine: Arc<Engine>,
    token: String,
    // Only this freshly-created session directory is ever recursively removed.
    // Dropping the last application handle cleans it after graceful shutdown.
    session: Arc<tempfile::TempDir>,
    files: Arc<Mutex<HashMap<String, SavedFile>>>,
    plans: Arc<Mutex<HashMap<String, InspectionPlan>>>,
    slots: Arc<Semaphore>,
    history: Arc<History>,
    jobs: Arc<JobStore>,
    port: u16,
}

impl AppState {
    async fn new(engine: Engine, port: u16, output: &FilePath) -> anyhow::Result<Self> {
        fs::create_dir_all(output).await?;
        let session = tempfile::Builder::new()
            .prefix("pawket-session-")
            .tempdir_in(output)?;
        Ok(Self {
            folder: crate::folder::Folder::new(output),
            engine: Arc::new(engine),
            token: Uuid::new_v4().simple().to_string() + &Uuid::new_v4().simple().to_string(),
            session: Arc::new(session),
            files: Default::default(),
            plans: Default::default(),
            slots: Arc::new(Semaphore::new(3)),
            history: Arc::new(History::load(output).await),
            jobs: Arc::new(JobStore::new(FILE_TTL)),
            port,
        })
    }
}

#[derive(Clone)]
struct SavedFile {
    path: PathBuf,
    filename: String,
    bytes: u64,
    ticket: String,
    created: Instant,
}

#[derive(Clone)]
struct InspectionPlan {
    original_url: String,
    media: MediaInfo,
    created: Instant,
}

pub fn capabilities(engine: &Engine) -> Value {
    let can_convert = engine.can_convert();
    let youtube_preview = engine.youtube_preview_enabled();
    let ytdlp = engine.ytdlp_version();
    let providers: Vec<&str> = pawket_core::PROVIDERS
        .iter()
        .copied()
        .chain(ytdlp.map(|_| "ytdlp"))
        .collect();
    let unsupported: &[&str] = match (ytdlp.is_some(), youtube_preview) {
        (true, _) => &[],
        (false, true) => &["tiktok"],
        (false, false) => &["youtube", "tiktok"],
    };
    let preview: &[&str] = if youtube_preview && ytdlp.is_none() {
        &["youtube"]
    } else {
        &[]
    };
    let mut notes = vec!["Original formats preserve source bytes. A configured FFmpeg enables explicit conversion of supported sources.", "HTML support covers video, audio, source, and direct Open Graph media.", "X posts: progressive MP4 variants, GIFs (labelled video-only, no audio) and original photos via X's public embed endpoint.", "The opt-in YouTube preview only handles usable direct URLs from the public player response. Encrypted media, HLS, DASH and live streams are outside this build’s download support."];
    if ytdlp.is_some() {
        notes.push("A configured yt-dlp handles YouTube, TikTok, Instagram, Vimeo, SoundCloud and its other supported sites, including HLS/DASH streams and playlists (first 50 entries). DRM and live streams are not supported.");
    }
    if let Some(source) = engine.cookies_kind() {
        notes.push(if source == "file" {
            "Signed-in media: yt-dlp uses the configured cookies file."
        } else {
            "Signed-in media: yt-dlp uses the logins from the configured browser."
        });
    }
    json!({
        "name": "pawket", "version": env!("CARGO_PKG_VERSION"),
        "providers": providers,
        "ytdlp_version": ytdlp,
        "playlists": ytdlp.is_some(),
        "accounts": engine.cookies_kind(),
        "streaming_formats": ytdlp.is_some(),
        "unsupported_providers": unsupported,
        "preview_providers": preview,
        "surfaces": ["web", "chrome_wasm", "cli", "mcp_stdio", "http_api"],
        "transcoding": can_convert,
        "conversion_outputs": if can_convert { &["mp3", "mp4"][..] } else { &[][..] },
        "max_media_bytes": null,
        "inspection_ttl_seconds": PLAN_TTL.as_secs(),
        "file_ttl_seconds": FILE_TTL.as_secs(),
        "max_retained_files": MAX_FILES,
        "max_retained_bytes": null,
        "notes": notes
    })
}

pub async fn run(
    engine: Engine,
    host: IpAddr,
    port: u16,
    output: PathBuf,
    assets: Option<PathBuf>,
    open: bool,
) -> anyhow::Result<()> {
    anyhow::ensure!(host.is_loopback(), "This release binds to loopback only. Public hosting requires a separate authenticated deployment.");
    let warm_engine = engine.clone();
    tokio::spawn(async move {
        warm_engine.warm_audio().await;
    });
    // Bind first so --port 0 uses the assigned port in Host and Origin checks.
    let listener = tokio::net::TcpListener::bind(SocketAddr::new(host, port))
        .await
        .map_err(|error| anyhow::anyhow!("Could not start Pawket on {host}:{port}: {error}"))?;
    let address = listener.local_addr()?;
    let state = AppState::new(engine, address.port(), &output).await?;
    let app = build_router(state, assets.as_deref());
    eprintln!("Pawket is ready at http://{address}");
    eprintln!("Open Connection in the website to pair the Chrome extension. Stop with Ctrl+C.");
    if open {
        open_browser(&format!("http://{address}/"));
    }
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}

fn open_browser(url: &str) {
    let url = url.to_owned();
    // Some launchers wait for the browser to close. Keep that wait independent
    // of the HTTP server and its shutdown.
    std::thread::spawn(move || {
        let (program, args): (&str, &[&str]) = if cfg!(target_os = "macos") {
            ("open", &[&url])
        } else if cfg!(windows) {
            ("rundll32.exe", &["url.dll,FileProtocolHandler", &url])
        } else {
            ("xdg-open", &[&url])
        };
        let mut command = Command::new(program);
        command
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if !command.status().is_ok_and(|status| status.success()) {
            eprintln!("Could not open your browser. Open the Pawket address above.");
        }
    });
}

fn build_router(state: AppState, assets: Option<&FilePath>) -> Router {
    let assets = match assets {
        Some(assets) => Router::new()
            .nest_service("/pkg", ServeDir::new(assets.join("pkg")))
            .route_service(
                "/bootstrap.js",
                tower_http::services::ServeFile::new(assets.join("bootstrap.js")),
            ),
        None => Router::new()
            .route(
                "/bootstrap.js",
                get(|| async {
                    (
                        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
                        BOOTSTRAP,
                    )
                }),
            )
            .route(
                "/pkg/pawket_browser.js",
                get(|| async {
                    (
                        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
                        BROWSER_JS,
                    )
                }),
            )
            .route(
                "/pkg/pawket_browser_bg.wasm",
                get(|| async { ([(header::CONTENT_TYPE, "application/wasm")], BROWSER_WASM) }),
            ),
    };
    assets
        .route("/", get(index))
        .route("/style.css", get(style))
        .route("/health", get(|| async { Json(json!({"status":"ok", "version":env!("CARGO_PKG_VERSION"), "providers":pawket_core::PROVIDERS})) }))
        .route("/v1/capabilities", get(|State(state): State<AppState>| async move {
                Json(capabilities(&state.engine))
        }))
        .route("/v1/folder/{action}", post(folder_action))
        .route("/v1/inspect", post(inspect))
        .route("/v1/download", post(download))
        .route("/v1/jobs", get(list_jobs).post(start_job))
        .route("/v1/jobs/clear", post(clear_jobs))
        .route("/v1/jobs/{id}/delivered", post(job_delivered))
        .route("/v1/history", get(history))
        .route("/v1/history/clear", post(clear_history))
        .route("/v1/files/{id}", get(file))
        .fallback(|| async { api_error(StatusCode::NOT_FOUND, "not_found", "This Pawket endpoint does not exist.") })
        .method_not_allowed_fallback(|| async { api_error(StatusCode::METHOD_NOT_ALLOWED, "method_not_allowed", "Use the documented HTTP method for this endpoint.") })
        .layer(DefaultBodyLimit::max(8192))
        .layer(middleware::from_fn_with_state(state.clone(), guard))
        .with_state(state)
}

async fn folder_action(State(state): State<AppState>, Path(action): Path<String>) -> Response {
    match tokio::task::spawn_blocking(move || -> std::io::Result<PathBuf> {
        match action.as_str() {
            "choose" => state.folder.choose()?,
            "open" => state.folder.open()?,
            "status" => (),
            _ => return Err(std::io::Error::other("Unknown folder action")),
        }
        state.folder.path()
    })
    .await
    {
        Ok(Ok(path)) => Json(json!({"path":path})).into_response(),
        Ok(Err(e)) => api_error(StatusCode::BAD_REQUEST, "folder_error", e),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, "folder_error", e),
    }
}

fn api_error(status: StatusCode, code: &str, message: impl ToString) -> Response {
    (
        status,
        Json(json!({"error":{"code":code,"message":message.to_string()}})),
    )
        .into_response()
}

fn json_error(rejection: JsonRejection) -> Response {
    let status = rejection.status();
    let message = if status == StatusCode::PAYLOAD_TOO_LARGE {
        "The request exceeds the 8 KiB limit."
    } else if status == StatusCode::UNSUPPORTED_MEDIA_TYPE {
        "Send the request as application/json."
    } else {
        "Send a JSON object with a URL string and, for downloads, an optional format_id string."
    };
    // Do not echo malformed request values: a media URL may contain a token.
    api_error(status, "invalid_request", message)
}

fn allowed_origin(origin: &str, port: u16) -> bool {
    if origin == format!("http://127.0.0.1:{port}")
        || origin == format!("http://localhost:{port}")
        || origin == format!("http://[::1]:{port}")
    {
        return true;
    }
    origin
        .strip_prefix("chrome-extension://")
        .is_some_and(|id| id.len() == 32 && id.bytes().all(|b| (b'a'..=b'p').contains(&b)))
}

fn response_headers(mut response: Response, allowed_origin: Option<&str>) -> Response {
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    headers.insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    headers.insert("x-frame-options", HeaderValue::from_static("DENY"));
    headers.insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static("default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self'; img-src 'self' data:; connect-src 'self'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'; form-action 'self'"));
    headers.append(header::VARY, HeaderValue::from_static("Origin"));
    if let Some(origin) = allowed_origin {
        if let Ok(value) = HeaderValue::from_str(origin) {
            headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, value);
        }
        headers.insert(
            header::ACCESS_CONTROL_ALLOW_METHODS,
            HeaderValue::from_static("GET, POST, OPTIONS"),
        );
        headers.insert(
            header::ACCESS_CONTROL_ALLOW_HEADERS,
            HeaderValue::from_static("content-type, x-pawket-token"),
        );
    }
    response
}

async fn guard(State(state): State<AppState>, request: Request, next: Next) -> Response {
    let raw_origin = request.headers().get(header::ORIGIN);
    let origin = raw_origin.and_then(|v| v.to_str().ok()).map(str::to_owned);
    let origin_is_valid = request.headers().get_all(header::ORIGIN).iter().count() <= 1
        && match raw_origin {
            Some(_) => origin
                .as_ref()
                .is_some_and(|value| allowed_origin(value, state.port)),
            None => true,
        };
    let cors_origin = if origin_is_valid {
        origin.as_deref()
    } else {
        None
    };
    let host = request
        .headers()
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    let valid_host = [
        format!("127.0.0.1:{}", state.port),
        format!("localhost:{}", state.port),
        format!("[::1]:{}", state.port),
    ];
    if !valid_host.iter().any(|h| h == host) {
        return response_headers(
            api_error(
                StatusCode::FORBIDDEN,
                "invalid_host",
                "Use the local Pawket address.",
            ),
            cors_origin,
        );
    }
    if !origin_is_valid {
        return response_headers(
            api_error(
                StatusCode::FORBIDDEN,
                "origin_denied",
                "This origin is not allowed.",
            ),
            None,
        );
    }
    if request.method() == Method::OPTIONS {
        let method = request
            .headers()
            .get(header::ACCESS_CONTROL_REQUEST_METHOD)
            .and_then(|value| value.to_str().ok());
        if origin.is_none() || !matches!(method, Some("GET" | "POST")) {
            return response_headers(
                api_error(
                    StatusCode::BAD_REQUEST,
                    "invalid_preflight",
                    "Use a permitted origin and GET or POST.",
                ),
                cors_origin,
            );
        }
        let requested_headers = request
            .headers()
            .get(header::ACCESS_CONTROL_REQUEST_HEADERS)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("");
        if requested_headers
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .any(|name| {
                !name.eq_ignore_ascii_case("content-type")
                    && !name.eq_ignore_ascii_case("x-pawket-token")
            })
        {
            return response_headers(
                api_error(
                    StatusCode::BAD_REQUEST,
                    "invalid_preflight",
                    "The requested headers are not allowed.",
                ),
                cors_origin,
            );
        }
        return response_headers(StatusCode::NO_CONTENT.into_response(), cors_origin);
    }
    // History and jobs list what the user saved, so reading them needs pairing too.
    let path = request.uri().path();
    let is_private_read = path.starts_with("/v1/history") || path.starts_with("/v1/jobs");
    let requires_token =
        path.starts_with("/v1/") && (request.method() == Method::POST || is_private_read);
    if requires_token
        && request
            .headers()
            .get("x-pawket-token")
            .and_then(|v| v.to_str().ok())
            != Some(state.token.as_str())
    {
        // Keep CORS headers on errors so extensions can display pairing help.
        return response_headers(
            api_error(
                StatusCode::UNAUTHORIZED,
                "pairing_required",
                "Pawket needs pairing. Open Connection and paste your pairing code.",
            ),
            cors_origin,
        );
    }
    if request.uri().path() == "/"
        && origin
            .as_ref()
            .is_some_and(|o| o.starts_with("chrome-extension://"))
    {
        return response_headers(
            api_error(
                StatusCode::FORBIDDEN,
                "pairing_required",
                "Copy the pairing code from the Pawket website.",
            ),
            cors_origin,
        );
    }
    response_headers(next.run(request).await, cors_origin)
}

async fn index(State(state): State<AppState>) -> Html<String> {
    Html(INDEX.replace("__SESSION_TOKEN__", &state.token))
}

async fn style() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "text/css; charset=utf-8")], STYLE)
}

#[derive(Deserialize)]
struct InspectInput {
    url: String,
}

async fn inspect(
    State(state): State<AppState>,
    payload: Result<Json<InspectInput>, JsonRejection>,
) -> Response {
    let input = match payload {
        Ok(Json(input)) => input,
        Err(error) => return json_error(error),
    };
    let Ok(_permit) = state.slots.clone().try_acquire_owned() else {
        return api_error(
            StatusCode::TOO_MANY_REQUESTS,
            "busy",
            "Pawket is busy with three things already. Try again in a moment.",
        );
    };
    prune_files(&state).await;
    match state.engine.inspect(&input.url).await {
        Ok(info) => Json(remember_inspection(&state, &input.url, info).await).into_response(),
        Err(error) => api_error(StatusCode::UNPROCESSABLE_ENTITY, error.code(), error),
    }
}

async fn remember_inspection(state: &AppState, original_url: &str, media: MediaInfo) -> MediaInfo {
    let id = Uuid::new_v4().simple().to_string();
    let mut response = media.clone();
    for (index, format) in response.formats.iter_mut().enumerate() {
        // The existing format_id field becomes an opaque, per-inspection
        // choice. No extra browser/API fields are needed to bind the selection.
        format.id = format!("plan_{id}_{index}");
    }
    let mut plans = state.plans.lock().await;
    plans.retain(|_, plan| plan.created.elapsed() < PLAN_TTL);
    if plans.len() >= MAX_PLANS {
        if let Some(oldest) = plans
            .iter()
            .min_by_key(|(_, plan)| plan.created)
            .map(|(id, _)| id.clone())
        {
            plans.remove(&oldest);
        }
    }
    plans.insert(
        id,
        InspectionPlan {
            original_url: original_url.trim().into(),
            media,
            created: Instant::now(),
        },
    );
    response
}

#[derive(Deserialize)]
struct DownloadInput {
    url: String,
    format_id: Option<String>,
    /// Which surface is asking; background jobs are grouped by it.
    client: Option<String>,
    #[serde(default)]
    save_to_folder: bool,
}

const MAX_CLIENT_CHARS: usize = 24;

/// A short, plain tag for the surface that started a job.
fn client_tag(input: &DownloadInput) -> String {
    let tag: String = input
        .client
        .as_deref()
        .unwrap_or("api")
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .take(MAX_CLIENT_CHARS)
        .collect();
    if tag.is_empty() {
        "api".to_owned()
    } else {
        tag
    }
}

// The error is the HTTP response sent straight back to the client, so boxing
// it would only add an allocation on the error path.
#[allow(clippy::result_large_err)]
async fn planned_selection(
    state: &AppState,
    input: &DownloadInput,
) -> Result<(MediaInfo, MediaFormat), Response> {
    if let Some(shortcut @ ("quick-mp3" | "quick-original")) = input.format_id.as_deref() {
        return state
            .engine
            .audio_selection(&input.url, shortcut == "quick-original")
            .await
            .map_err(|error| api_error(StatusCode::UNPROCESSABLE_ENTITY, error.code(), error));
    }
    let Some(format_id) = input.format_id.as_deref() else {
        // One-shot callers may omit format_id. Inspect and choose at this moment.
        let info =
            state.engine.inspect(&input.url).await.map_err(|error| {
                api_error(StatusCode::UNPROCESSABLE_ENTITY, error.code(), error)
            })?;
        let format = info.formats.first().cloned().ok_or_else(|| {
            api_error(
                StatusCode::UNPROCESSABLE_ENTITY,
                "format_unavailable",
                "No files found at that link.",
            )
        })?;
        return Ok((info, format));
    };
    let Some((plan_id, index)) = format_id
        .strip_prefix("plan_")
        .and_then(|value| value.rsplit_once('_'))
    else {
        return Err(api_error(
            StatusCode::CONFLICT,
            "inspection_required",
            "Find the link’s files first, then pick one.",
        ));
    };
    let index = index.parse::<usize>().map_err(|_| {
        api_error(
            StatusCode::CONFLICT,
            "inspection_required",
            "Find the link’s files again and pick one.",
        )
    })?;
    let plan = {
        let mut plans = state.plans.lock().await;
        plans.retain(|_, plan| plan.created.elapsed() < PLAN_TTL);
        plans.get(plan_id).cloned()
    }
    .ok_or_else(|| {
        api_error(
            StatusCode::CONFLICT,
            "inspection_expired",
            "That list of files is out of date. Press Find media again.",
        )
    })?;
    if input.url.trim() != plan.original_url && input.url.trim() != plan.media.source_url {
        return Err(api_error(
            StatusCode::CONFLICT,
            "inspection_mismatch",
            "That file belongs to a different link. Press Find media again.",
        ));
    }
    let format = plan.media.formats.get(index).cloned().ok_or_else(|| {
        api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "format_unavailable",
            "Pick one of the files in the list.",
        )
    })?;
    Ok((plan.media, format))
}

async fn download(
    State(state): State<AppState>,
    payload: Result<Json<DownloadInput>, JsonRejection>,
) -> Response {
    let input = match payload {
        Ok(Json(input)) => input,
        Err(error) => return json_error(error),
    };
    let Ok(_permit) = state.slots.clone().try_acquire_owned() else {
        return api_error(
            StatusCode::TOO_MANY_REQUESTS,
            "busy",
            "Pawket is busy with three things already. Try again in a moment.",
        );
    };
    prune_files(&state).await;
    let (info, format) = match planned_selection(&state, &input).await {
        Ok(selected) => selected,
        Err(response) => return response,
    };
    let id = Uuid::new_v4().simple().to_string();
    match perform_download(&state, &id, &info, &format, &Progress::default()).await {
        Ok((result, url)) => Json(json!({"id":id,"filename":result.filename,"bytes":result.bytes,"url":url,"source_url":result.source_url})).into_response(),
        Err(error) => api_error(StatusCode::UNPROCESSABLE_ENTITY, error.code(), error),
    }
}

/// Download one format into this session, register its ticketed file link
/// and record it in the history. Shared by the direct and background paths.
async fn perform_download(
    state: &AppState,
    id: &str,
    info: &MediaInfo,
    format: &MediaFormat,
    progress: &Progress,
) -> Result<(DownloadResult, String), PawketError> {
    let directory = state.session.path().join(id);
    let result = match state
        .engine
        .download_format_tracked(info, format, &directory, progress)
        .await
    {
        Ok(result) => result,
        Err(error) => {
            // This is a server-created UUID child of our own temporary session.
            let _ = fs::remove_dir_all(&directory).await;
            return Err(error);
        }
    };
    let ticket = Uuid::new_v4().simple().to_string();
    let saved = SavedFile {
        path: PathBuf::from(&result.path),
        filename: result.filename.clone(),
        bytes: result.bytes,
        ticket: ticket.clone(),
        created: Instant::now(),
    };
    state.files.lock().await.insert(id.to_owned(), saved);
    prune_files(state).await;
    let title = if matches!(
        format.id.as_str(),
        "ytdlp-quick-mp3" | "ytdlp-quick-original"
    ) {
        FilePath::new(&result.filename)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(&result.filename)
    } else {
        &info.title
    };
    let entry = HistoryEntry::now(
        title,
        &result.source_url,
        &result.filename,
        result.bytes,
        &info.provider,
    );
    if let Err(error) = state.history.record(entry).await {
        // The download itself succeeded; history is a convenience.
        eprintln!("Pawket could not update its download history: {error}");
    }
    Ok((result, format!("/v1/files/{id}?ticket={ticket}")))
}

/// Start a download that keeps running after this request returns.
async fn start_job(
    State(state): State<AppState>,
    payload: Result<Json<DownloadInput>, JsonRejection>,
) -> Response {
    let input = match payload {
        Ok(Json(input)) => input,
        Err(error) => return json_error(error),
    };
    if state
        .jobs
        .list()
        .iter()
        .filter(|job| is_active(job))
        .count()
        >= MAX_ACTIVE_JOBS
    {
        return api_error(
            StatusCode::TOO_MANY_REQUESTS,
            "busy",
            "Too many downloads are waiting. Let some finish first.",
        );
    }
    prune_files(&state).await;
    let (info, format) = match planned_selection(&state, &input).await {
        Ok(selected) => selected,
        Err(response) => return response,
    };
    let id = Uuid::new_v4().simple().to_string();
    let destination = if input.save_to_folder {
        match state.folder.destination() {
            Ok(path) => Some(path),
            Err(e) => return api_error(StatusCode::BAD_REQUEST, "folder_error", e),
        }
    } else {
        None
    };
    let progress = state.jobs.add(
        &id,
        &info.title,
        &info.source_url,
        format.label.as_deref(),
        &client_tag(&input),
    );
    tokio::spawn({
        let state = state.clone();
        let id = id.clone();
        async move {
            // Waiting here is what "queued" means: at most three transfers run at once.
            let Ok(_permit) = state.slots.clone().acquire_owned().await else {
                state.jobs.fail(&id, "busy", "Pawket is shutting down.");
                return;
            };
            state.jobs.start(&id);
            match perform_download(&state, &id, &info, &format, &progress).await {
                Ok((result, url)) => {
                    let Some(folder) = destination else {
                        state.jobs.finish(&id, &result.filename, &url);
                        return;
                    };
                    match tokio::task::spawn_blocking(move || {
                        crate::folder::keep(FilePath::new(&result.path), &folder, &result.filename)
                    })
                    .await
                    {
                        Ok(Ok(name)) => state.jobs.finish_local(&id, &name),
                        Ok(Err(e)) => state.jobs.fail(
                            &id,
                            "folder_error",
                            &format!("Could not save to Pawket: {e}"),
                        ),
                        Err(e) => state.jobs.fail(&id, "folder_error", &e.to_string()),
                    }
                }
                Err(error) => state.jobs.fail(&id, error.code(), &error.to_string()),
            }
        }
    });
    Json(json!({ "id": id, "jobs": state.jobs.list() })).into_response()
}

fn is_active(job: &JobView) -> bool {
    matches!(job.status, JobStatus::Queued | JobStatus::Running)
}

async fn list_jobs(State(state): State<AppState>) -> Response {
    Json(json!({ "jobs": state.jobs.list() })).into_response()
}

async fn job_delivered(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    match state.jobs.claim_delivery(&id) {
        Some(claimed) => Json(json!({ "claimed": claimed })).into_response(),
        None => api_error(
            StatusCode::NOT_FOUND,
            "job_not_found",
            "Pawket lost track of that download. Please start it again.",
        ),
    }
}

async fn clear_jobs(State(state): State<AppState>) -> Response {
    state.jobs.clear_finished();
    Json(json!({ "jobs": state.jobs.list() })).into_response()
}

async fn history(State(state): State<AppState>) -> Response {
    Json(json!({ "entries": state.history.list().await })).into_response()
}

async fn clear_history(State(state): State<AppState>) -> Response {
    match state.history.clear().await {
        Ok(()) => Json(json!({ "entries": [] })).into_response(),
        Err(error) => api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "storage_error",
            format!("Could not clear the download history: {error}"),
        ),
    }
}

async fn prune_files(state: &AppState) {
    let removed = {
        let mut files = state.files.lock().await;
        let mut removed: Vec<String> = files
            .iter()
            .filter(|(_, saved)| saved.created.elapsed() >= FILE_TTL)
            .map(|(id, _)| id.clone())
            .collect();
        for id in &removed {
            files.remove(id);
        }
        while files.len() > MAX_FILES {
            let Some(oldest) = files
                .iter()
                .min_by_key(|(_, saved)| saved.created)
                .map(|(id, _)| id.clone())
            else {
                break;
            };
            files.remove(&oldest);
            removed.push(oldest);
        }
        removed
    };
    // Do not hold the metadata mutex during filesystem operations. A transfer
    // already opened by a browser can finish even after its ticket expires.
    for id in removed {
        let _ = fs::remove_dir_all(state.session.path().join(id)).await;
    }
}

#[derive(Deserialize)]
struct FileQuery {
    ticket: String,
}

async fn file(
    State(state): State<AppState>,
    Path(id): Path<String>,
    query: Result<Query<FileQuery>, QueryRejection>,
) -> Response {
    let query = match query {
        Ok(Query(query)) => query,
        Err(_) => {
            return api_error(
                StatusCode::BAD_REQUEST,
                "invalid_ticket",
                "This download link is missing a valid ticket.",
            )
        }
    };
    prune_files(&state).await;
    let saved = state.files.lock().await.get(&id).cloned();
    let Some(saved) = saved else {
        return api_error(
            StatusCode::NOT_FOUND,
            "file_expired",
            "That file has expired. Save it again.",
        );
    };
    if saved.ticket != query.ticket {
        return api_error(
            StatusCode::UNAUTHORIZED,
            "invalid_ticket",
            "That download link isn’t valid.",
        );
    }
    match fs::File::open(&saved.path).await {
        Ok(file) => {
            let mut headers = HeaderMap::new();
            headers.insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/octet-stream"),
            );
            if let Ok(value) = HeaderValue::from_str(&saved.bytes.to_string()) {
                headers.insert(header::CONTENT_LENGTH, value);
            }
            let escaped_name = saved
                .filename
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || " ._-".contains(*c))
                .collect::<String>();
            if let Ok(value) =
                HeaderValue::from_str(&format!("attachment; filename=\"{escaped_name}\""))
            {
                headers.insert(header::CONTENT_DISPOSITION, value);
            }
            (headers, Body::from_stream(ReaderStream::new(file))).into_response()
        }
        Err(_) => api_error(
            StatusCode::NOT_FOUND,
            "file_expired",
            "That file is no longer here. Save it again.",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use pawket_core::MediaKind;
    use tower::ServiceExt;

    const PORT: u16 = 3987;
    const EXTENSION: &str = "chrome-extension://abcdefghijklmnopabcdefghijklmnop";

    async fn fixture() -> (tempfile::TempDir, AppState, Router) {
        let directory = tempfile::tempdir().unwrap();
        let state = AppState::new(Engine::new().unwrap(), PORT, directory.path())
            .await
            .unwrap();
        let router = build_router(state.clone(), None);
        (directory, state, router)
    }

    fn request(
        method: Method,
        path: &str,
        origin: Option<&str>,
        token: Option<&str>,
        body: &str,
    ) -> Request {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, format!("127.0.0.1:{PORT}"));
        if let Some(origin) = origin {
            request = request.header(header::ORIGIN, origin);
        }
        if let Some(token) = token {
            request = request.header("x-pawket-token", token);
        }
        if !body.is_empty() {
            request = request.header(header::CONTENT_TYPE, "application/json");
        }
        request.body(Body::from(body.to_owned())).unwrap()
    }

    async fn error_code(response: Response) -> String {
        assert_eq!(response.headers()[header::CONTENT_TYPE], "application/json");
        let bytes = to_bytes(response.into_body(), 16384).await.unwrap();
        serde_json::from_slice::<Value>(&bytes).unwrap()["error"]["code"]
            .as_str()
            .unwrap()
            .into()
    }

    async fn add_saved(state: &AppState, id: &str, bytes: u64, created: Instant) {
        let directory = state.session.path().join(id);
        fs::create_dir(&directory).await.unwrap();
        let path = directory.join("clip.mp4");
        fs::write(&path, b"media").await.unwrap();
        state.files.lock().await.insert(
            id.into(),
            SavedFile {
                path,
                filename: "clip.mp4".into(),
                bytes,
                ticket: "correct-ticket".into(),
                created,
            },
        );
    }

    fn media(url: &str, file: &str) -> MediaInfo {
        MediaInfo {
            source_url: url.into(),
            title: "Clip".into(),
            formats: vec![MediaFormat {
                id: "source-1".into(),
                url: file.into(),
                extension: "mp4".into(),
                mime_type: "video/mp4".into(),
                kind: MediaKind::Video,
                size_bytes: None,
                ..MediaFormat::default()
            }],
            ..MediaInfo::default()
        }
    }

    #[test]
    fn origins_are_exact_and_chrome_ids_are_validated() {
        assert!(allowed_origin("http://127.0.0.1:3987", PORT));
        assert!(allowed_origin(EXTENSION, PORT));
        for bad in [
            "https://evil.example",
            "http://127.0.0.1:3987.evil.example",
            "http://localhost:80",
            "null",
            "chrome-extension://bad",
            "chrome-extension://abcdefghijklmnopabcdefghijklmnop/evil",
        ] {
            assert!(!allowed_origin(bad, PORT), "{bad}");
        }
    }

    #[tokio::test]
    async fn wrong_host_and_untrusted_origin_are_rejected() {
        let (_directory, _state, router) = fixture().await;
        let mut wrong_host = request(Method::GET, "/health", None, None, "");
        wrong_host.headers_mut().insert(
            header::HOST,
            HeaderValue::from_static("rebound.example:3987"),
        );
        let response = router.clone().oneshot(wrong_host).await.unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(error_code(response).await, "invalid_host");
        let response = router
            .oneshot(request(
                Method::GET,
                "/health",
                Some("https://evil.example"),
                None,
                "",
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert!(response
            .headers()
            .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
            .is_none());
        assert_eq!(error_code(response).await, "origin_denied");
    }

    #[tokio::test]
    async fn pairing_errors_are_readable_by_allowed_extensions() {
        let (_directory, _state, router) = fixture().await;
        for token in [None, Some("wrong-token")] {
            let response = router
                .clone()
                .oneshot(request(
                    Method::POST,
                    "/v1/inspect",
                    Some(EXTENSION),
                    token,
                    r#"{"url":"https://example.com/file.mp4"}"#,
                ))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
            assert_eq!(
                response.headers()[header::ACCESS_CONTROL_ALLOW_ORIGIN],
                EXTENSION
            );
            assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
            assert_eq!(error_code(response).await, "pairing_required");
        }
    }

    #[tokio::test]
    async fn extension_cannot_read_the_website_pairing_token() {
        let (_directory, _state, router) = fixture().await;
        let response = router
            .oneshot(request(Method::GET, "/", Some(EXTENSION), None, ""))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(error_code(response).await, "pairing_required");
    }

    #[tokio::test]
    async fn preflight_allows_the_pairing_header_without_requiring_a_token() {
        let (_directory, _state, router) = fixture().await;
        let mut preflight = request(Method::OPTIONS, "/v1/inspect", Some(EXTENSION), None, "");
        preflight.headers_mut().insert(
            header::ACCESS_CONTROL_REQUEST_METHOD,
            HeaderValue::from_static("POST"),
        );
        preflight.headers_mut().insert(
            header::ACCESS_CONTROL_REQUEST_HEADERS,
            HeaderValue::from_static("content-type, x-pawket-token"),
        );
        let response = router.oneshot(preflight).await.unwrap();
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        assert_eq!(
            response.headers()[header::ACCESS_CONTROL_ALLOW_ORIGIN],
            EXTENSION
        );
    }

    #[tokio::test]
    async fn malformed_json_and_unsafe_urls_have_structured_errors() {
        let (_directory, state, router) = fixture().await;
        let response = router
            .clone()
            .oneshot(request(
                Method::POST,
                "/v1/inspect",
                Some(EXTENSION),
                Some(&state.token),
                "{",
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(error_code(response).await, "invalid_request");
        let response = router
            .oneshot(request(
                Method::POST,
                "/v1/inspect",
                Some(EXTENSION),
                Some(&state.token),
                r#"{"url":"file:///private/file.mp4"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(
            response.headers()[header::ACCESS_CONTROL_ALLOW_ORIGIN],
            EXTENSION
        );
        assert_eq!(error_code(response).await, "invalid_url");
    }

    #[tokio::test]
    async fn configured_proxy_rejection_is_an_actionable_api_error() {
        // Never change or clear inherited proxy policy. Without a configured
        // proxy this particular test makes no request to the public network.
        if ![
            "HTTPS_PROXY",
            "https_proxy",
            "ALL_PROXY",
            "all_proxy",
            "HTTP_PROXY",
            "http_proxy",
        ]
        .iter()
        .any(|key| std::env::var(key).is_ok_and(|value| !value.trim().is_empty()))
        {
            return;
        }
        let (_directory, state, router) = fixture().await;
        let response = router
            .oneshot(request(
                Method::POST,
                "/v1/inspect",
                Some(EXTENSION),
                Some(&state.token),
                r#"{"url":"https://example.com/file.mp4?secret=do-not-echo"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(
            response.headers()[header::ACCESS_CONTROL_ALLOW_ORIGIN],
            EXTENSION
        );
        let bytes = to_bytes(response.into_body(), 16384).await.unwrap();
        let text = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(!text.contains("do-not-echo"));
        assert_eq!(
            serde_json::from_str::<Value>(&text).unwrap()["error"]["code"],
            "proxy_unsupported"
        );
    }

    #[tokio::test]
    async fn file_tickets_are_required_and_stream_original_bytes() {
        let (_directory, state, router) = fixture().await;
        add_saved(&state, "one", 5, Instant::now()).await;
        let response = router
            .clone()
            .oneshot(request(Method::GET, "/v1/files/one", None, None, ""))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(error_code(response).await, "invalid_ticket");
        let response = router
            .clone()
            .oneshot(request(
                Method::GET,
                "/v1/files/one?ticket=wrong",
                None,
                None,
                "",
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(error_code(response).await, "invalid_ticket");
        let response = router
            .oneshot(request(
                Method::GET,
                "/v1/files/one?ticket=correct-ticket",
                None,
                None,
                "",
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers()[header::CONTENT_DISPOSITION],
            "attachment; filename=\"clip.mp4\""
        );
        assert_eq!(
            to_bytes(response.into_body(), 1024).await.unwrap().as_ref(),
            b"media"
        );
    }

    #[tokio::test]
    async fn expired_files_lose_their_tickets_and_are_removed() {
        let (_directory, state, router) = fixture().await;
        add_saved(
            &state,
            "old",
            5,
            Instant::now() - FILE_TTL - Duration::from_secs(1),
        )
        .await;
        let response = router
            .oneshot(request(
                Method::GET,
                "/v1/files/old?ticket=correct-ticket",
                None,
                None,
                "",
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(error_code(response).await, "file_expired");
        assert!(!state.session.path().join("old").exists());
    }

    #[tokio::test]
    async fn retention_evicts_oldest_and_keeps_large_files() {
        let (_directory, state, _router) = fixture().await;
        let now = Instant::now();
        for index in 0..=MAX_FILES {
            add_saved(
                &state,
                &format!("file-{index}"),
                1,
                now - Duration::from_secs((MAX_FILES - index) as u64),
            )
            .await;
        }
        prune_files(&state).await;
        assert_eq!(state.files.lock().await.len(), MAX_FILES);
        assert!(!state.files.lock().await.contains_key("file-0"));
        assert!(!state.session.path().join("file-0").exists());
        add_saved(&state, "large", 1024 * 1024 * 1024, Instant::now()).await;
        prune_files(&state).await;
        assert!(state.files.lock().await.contains_key("large"));
    }

    #[tokio::test]
    async fn session_cleanup_preserves_the_user_output_directory() {
        let directory = tempfile::tempdir().unwrap();
        let user_file = directory.path().join("keep.txt");
        fs::write(&user_file, b"existing file").await.unwrap();
        let state = AppState::new(Engine::new().unwrap(), PORT, directory.path())
            .await
            .unwrap();
        add_saved(&state, "one", 5, Instant::now()).await;
        let session = state.session.path().to_owned();
        drop(state);
        assert!(!session.exists());
        assert_eq!(fs::read(user_file).await.unwrap(), b"existing file");
    }

    #[tokio::test]
    async fn inspected_choices_do_not_drift_between_tabs_or_reinspections() {
        let (_directory, state, _router) = fixture().await;
        let url = "https://example.com/post";
        let first =
            remember_inspection(&state, url, media(url, "https://example.com/first.mp4")).await;
        let second =
            remember_inspection(&state, url, media(url, "https://example.com/second.mp4")).await;
        assert_ne!(first.formats[0].id, second.formats[0].id);
        let first_input = DownloadInput {
            url: url.into(),
            format_id: Some(first.formats[0].id.clone()),
            client: None,
            save_to_folder: false,
        };
        let (_, selected) = planned_selection(&state, &first_input).await.unwrap();
        assert_eq!(selected.url, "https://example.com/first.mp4");
        let wrong_url = DownloadInput {
            url: "https://example.com/other".into(),
            format_id: first_input.format_id,
            client: None,
            save_to_folder: false,
        };
        assert_eq!(
            error_code(planned_selection(&state, &wrong_url).await.unwrap_err()).await,
            "inspection_mismatch"
        );
    }

    #[tokio::test]
    async fn expired_inspection_requests_an_explicit_refresh() {
        let (_directory, state, _router) = fixture().await;
        let url = "https://example.com/post";
        let response =
            remember_inspection(&state, url, media(url, "https://example.com/clip.mp4")).await;
        for plan in state.plans.lock().await.values_mut() {
            plan.created = Instant::now() - PLAN_TTL - Duration::from_secs(1);
        }
        let input = DownloadInput {
            url: url.into(),
            format_id: Some(response.formats[0].id.clone()),
            client: None,
            save_to_folder: false,
        };
        let error = planned_selection(&state, &input).await.unwrap_err();
        assert_eq!(error.status(), StatusCode::CONFLICT);
        assert_eq!(error_code(error).await, "inspection_expired");
    }

    #[tokio::test]
    async fn history_needs_pairing_and_lists_then_clears_saved_files() {
        let (_directory, state, router) = fixture().await;
        let unpaired = router
            .clone()
            .oneshot(request(Method::GET, "/v1/history", None, None, ""))
            .await
            .unwrap();
        assert_eq!(unpaired.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(error_code(unpaired).await, "pairing_required");

        let entry = HistoryEntry::now("Clip", "https://example.com/a", "clip.mp4", 42, "x");
        state.history.record(entry).await.unwrap();
        let listed = router
            .clone()
            .oneshot(request(
                Method::GET,
                "/v1/history",
                None,
                Some(&state.token),
                "",
            ))
            .await
            .unwrap();
        assert_eq!(listed.status(), StatusCode::OK);
        let body: Value =
            serde_json::from_slice(&to_bytes(listed.into_body(), 65536).await.unwrap()).unwrap();
        assert_eq!(body["entries"][0]["filename"], "clip.mp4");
        assert_eq!(body["entries"][0]["bytes"], 42);

        let cleared = router
            .oneshot(request(
                Method::POST,
                "/v1/history/clear",
                None,
                Some(&state.token),
                "",
            ))
            .await
            .unwrap();
        assert_eq!(cleared.status(), StatusCode::OK);
        assert!(state.history.list().await.is_empty());
    }

    #[tokio::test]
    async fn jobs_need_pairing_report_progress_and_can_be_marked_delivered() {
        let (_directory, state, router) = fixture().await;
        let unpaired = router
            .clone()
            .oneshot(request(Method::GET, "/v1/jobs", None, None, ""))
            .await
            .unwrap();
        assert_eq!(unpaired.status(), StatusCode::UNAUTHORIZED);

        state
            .jobs
            .add("job1", "Clip", "https://example.com/a", Some("720p"), "web");
        state.jobs.start("job1");
        let listed = router
            .clone()
            .oneshot(request(
                Method::GET,
                "/v1/jobs",
                None,
                Some(&state.token),
                "",
            ))
            .await
            .unwrap();
        let body: Value =
            serde_json::from_slice(&to_bytes(listed.into_body(), 65536).await.unwrap()).unwrap();
        assert_eq!(body["jobs"][0]["status"], "running");
        assert_eq!(body["jobs"][0]["bytes_done"], 0);
        assert!(body["jobs"][0]["bytes_total"].is_null());
        assert_eq!(body["jobs"][0]["delivered"], false);

        state
            .jobs
            .finish("job1", "clip.mp4", "/v1/files/job1?ticket=t");
        let delivered = router
            .clone()
            .oneshot(request(
                Method::POST,
                "/v1/jobs/job1/delivered",
                None,
                Some(&state.token),
                "",
            ))
            .await
            .unwrap();
        assert_eq!(delivered.status(), StatusCode::OK);
        let claim: Value =
            serde_json::from_slice(&to_bytes(delivered.into_body(), 65536).await.unwrap()).unwrap();
        assert_eq!(claim["claimed"], true);
        assert!(state.jobs.list()[0].delivered);

        let missing = router
            .oneshot(request(
                Method::POST,
                "/v1/jobs/nope/delivered",
                None,
                Some(&state.token),
                "",
            ))
            .await
            .unwrap();
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);
        assert_eq!(error_code(missing).await, "job_not_found");
    }

    #[tokio::test]
    async fn starting_a_job_rejects_a_bad_link_before_queueing() {
        let (_directory, state, router) = fixture().await;
        let response = router
            .oneshot(request(
                Method::POST,
                "/v1/jobs",
                None,
                Some(&state.token),
                r#"{"url":"http://example.com/a.mp4"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(error_code(response).await, "invalid_url");
        assert!(state.jobs.list().is_empty());
    }
}
