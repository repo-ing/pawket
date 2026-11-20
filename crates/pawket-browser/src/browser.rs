mod picker_view;

use std::{cell::RefCell, rc::Rc};

use crate::picker;

use js_sys::{Function, Object, Reflect};
use serde::{Deserialize, Serialize};
use wasm_bindgen::{closure::Closure, prelude::*, JsCast};
use wasm_bindgen_futures::{spawn_local, JsFuture};
use web_sys::{
    AbortController, Document, Element, Event, HtmlAnchorElement, HtmlButtonElement,
    HtmlInputElement, Request, RequestInit, RequestMode, Response, Url,
};

const CONNECTION_KEY: &str = "pawket.connection.v1";
const DEFAULT_ENGINE: &str = "http://127.0.0.1:3987";
const MIN_ITEMS_FOR_SAVE_ALL: usize = 2;
const MAX_HISTORY_SHOWN: usize = 8;
const JOB_POLL_MS: i32 = 600;
const MAX_JOBS_SHOWN: usize = 6;

type Shared = Rc<RefCell<State>>;

#[derive(Clone, Copy, PartialEq)]
enum Surface {
    Web,
    Extension,
}

impl Surface {
    /// Tag sent with background jobs so each surface collects only its own.
    fn client_tag(self) -> &'static str {
        match self {
            Surface::Web => "web",
            Surface::Extension => "extension",
        }
    }
}

struct State {
    surface: Surface,
    origin: String,
    connection: Connection,
    connection_revision: u64,
    request_revision: u64,
    busy: bool,
    media: Option<MediaInfo>,
    selected: Option<String>,
    /// The person's picks; kept across links so preferences stick.
    selection: picker::Selection,
    /// Checked items of a multi-item source (playlist entries, post media).
    checked: Vec<u32>,
    format_listeners: Vec<Closure<dyn FnMut(Event)>>,
    history_listeners: Vec<Closure<dyn FnMut(Event)>>,
    /// True while the job list is being refreshed on a timer.
    is_polling: bool,
}

#[derive(Clone, Serialize, Deserialize)]
struct Connection {
    base: String,
    token: String,
}

#[derive(Clone, Deserialize)]
struct MediaInfo {
    source_url: String,
    title: String,
    formats: Vec<MediaFormat>,
}

#[derive(Clone, Deserialize)]
struct MediaFormat {
    id: String,
    extension: String,
    kind: String,
    size_bytes: Option<u64>,
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    has_audio: Option<bool>,
    #[serde(default)]
    item: Option<u32>,
    #[serde(default)]
    choice: Option<picker::Choice>,
    #[serde(default)]
    conversion: Option<String>,
}

#[derive(Deserialize)]
struct JobList {
    jobs: Vec<Job>,
}

#[derive(Clone, Deserialize)]
struct Job {
    id: String,
    title: String,
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    client: String,
    status: String,
    bytes_done: u64,
    bytes_total: Option<u64>,
    #[serde(default)]
    filename: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    error: Option<JobFailure>,
    delivered: bool,
}

#[derive(Clone, Deserialize)]
struct JobFailure {
    message: String,
}

#[derive(Deserialize)]
struct Claim {
    claimed: bool,
}

impl Job {
    fn is_active(&self) -> bool {
        self.status == "queued" || self.status == "running"
    }
}

#[derive(Deserialize)]
struct HistoryList {
    entries: Vec<HistoryEntry>,
}

#[derive(Deserialize)]
struct HistoryEntry {
    title: String,
    source_url: String,
    filename: String,
    bytes: u64,
    saved_at: u64,
}

#[derive(Debug)]
struct ApiError {
    message: String,
    pairing: bool,
}

pub fn start(surface: &str, current_url: &str, auto_inspect: bool) -> Result<(), JsValue> {
    let surface = if surface == "extension" {
        Surface::Extension
    } else {
        Surface::Web
    };
    let document = document()?;
    let origin = window()?.location().origin()?;
    let default_base = if surface == Surface::Extension {
        DEFAULT_ENGINE.to_owned()
    } else {
        origin.clone()
    };
    let saved = read_connection().and_then(|mut connection| {
        connection.base = normalize_base(&connection.base, surface, &origin).ok()?;
        valid_token(&connection.token).then_some(connection)
    });
    let mut connection = saved.unwrap_or(Connection {
        base: default_base,
        token: String::new(),
    });

    // A newly started engine has a new session token. Its own page must not
    // silently keep the previous engine session's saved credential.
    if surface == Surface::Web {
        if let Some(meta) = document.query_selector("meta[name='pawket-token']")? {
            if let Some(token) = meta.get_attribute("content").filter(|v| !v.is_empty()) {
                connection = Connection {
                    base: origin.clone(),
                    token,
                };
            }
        }
    }

    let state = Rc::new(RefCell::new(State {
        surface,
        origin,
        connection,
        connection_revision: 0,
        request_revision: 0,
        busy: false,
        media: None,
        selected: None,
        selection: picker::Selection::default(),
        checked: Vec::new(),
        format_listeners: Vec::new(),
        history_listeners: Vec::new(),
        is_polling: false,
    }));

    input("api-base")?.set_value(&state.borrow().connection.base);
    input("api-token")?.set_value(&state.borrow().connection.token);
    if let Ok(token_field) = input("pairing-token") {
        // Reveal the token only after the user opens the connection panel.
        token_field.set_value("");
    }
    if surface == Surface::Extension && valid_source(current_url).is_ok() {
        input("url-input")?.set_value(current_url);
    }

    bind("inspect-form", "submit", {
        let state = state.clone();
        move |event| {
            event.prevent_default();
            let state = state.clone();
            spawn_local(async move { inspect(state).await });
        }
    })?;

    bind("url-input", "input", {
        let state = state.clone();
        move |_| {
            clear_result(&state);
            status("", "idle");
        }
    })?;

    bind("paste-button", "click", {
        let state = state.clone();
        move |_| {
            let state = state.clone();
            spawn_local(async move {
                let Ok(window) = window() else { return };
                match JsFuture::from(window.navigator().clipboard().read_text()).await {
                    Ok(value) => {
                        if let Ok(field) = input("url-input") {
                            field.set_value(value.as_string().unwrap_or_default().trim());
                            clear_result(&state);
                            status("Link pasted. Press Find media.", "idle");
                            let _ = field.focus();
                        }
                    }
                    Err(_) => status(
                        "Your browser blocked the paste. Paste the link into the box yourself.",
                        "error",
                    ),
                }
            });
        }
    })?;

    bind("formats", "change", {
        let state = state.clone();
        move |event| {
            let Some(control) = event
                .target()
                .and_then(|target| target.dyn_into::<HtmlInputElement>().ok())
            else {
                return;
            };
            let Some(change) =
                picker::parse_change(&control.name(), &control.value(), control.checked())
            else {
                return;
            };
            let next = {
                let state = state.borrow();
                state.media.as_ref().map(|media| {
                    let entries = picker_view::entries(media);
                    let current = picker::normalize(&state.selection, &entries);
                    let all = picker::items(&entries);
                    picker::apply(&current, &state.checked, &all, change)
                })
            };
            let Some((selection, checked)) = next else {
                return;
            };
            {
                let mut state = state.borrow_mut();
                state.selection = selection;
                state.checked = checked;
            }
            if show_media(&state).is_err() {
                status("Couldn’t update the choices. Reload the page.", "error");
            }
        }
    })?;

    bind("save-button", "click", {
        let state = state.clone();
        move |_| {
            let state = state.clone();
            spawn_local(async move { save_file(state).await });
        }
    })?;

    // Optional controls: present on surfaces that offer them.
    if element("save-all-button").is_ok() {
        bind("save-all-button", "click", {
            let state = state.clone();
            move |_| {
                let state = state.clone();
                spawn_local(async move { save_all(state).await });
            }
        })?;
    }
    if element("clear-jobs").is_ok() {
        bind("clear-jobs", "click", {
            let state = state.clone();
            move |_| {
                let state = state.clone();
                spawn_local(async move {
                    let connection = state.borrow().connection.clone();
                    let body = serde_json::json!({});
                    let _ = request_json(&connection, "/v1/jobs/clear", Some(body), 8_000).await;
                    refresh_jobs(&state).await;
                });
            }
        })?;
    }
    if element("clear-history").is_ok() {
        bind("clear-history", "click", {
            let state = state.clone();
            move |_| {
                let state = state.clone();
                spawn_local(async move { clear_history(state).await });
            }
        })?;
    }

    bind("connection-toggle", "click", {
        let state = state.clone();
        move |_| {
            let is_hidden = element("connection-panel")
                .map(|panel| panel.has_attribute("hidden"))
                .unwrap_or(true);
            show_connection(&state, is_hidden);
            if is_hidden {
                if let Ok(field) = input("pairing-token") {
                    field.set_value(&state.borrow().connection.token);
                }
            }
        }
    })?;

    bind("save-connection", "click", {
        let state = state.clone();
        move |_| {
            let (surface, origin, busy) = {
                let state = state.borrow();
                (state.surface, state.origin.clone(), state.busy)
            };
            if busy {
                status("Hang on, Pawket is busy. Try again in a moment.", "idle");
                return;
            }
            let base = input("api-base")
                .map(|field| field.value())
                .unwrap_or_default();
            let token = input("api-token")
                .map(|field| field.value().trim().to_owned())
                .unwrap_or_default();
            let base = match normalize_base(&base, surface, &origin) {
                Ok(base) => base,
                Err(message) => {
                    status(&message, "error");
                    return;
                }
            };
            if !valid_token(&token) || token.is_empty() {
                status("Paste the pairing code from the Pawket website.", "error");
                return;
            }
            let connection = Connection { base, token };
            let saved = write_connection(&connection);
            {
                let mut state = state.borrow_mut();
                state.connection = connection;
                state.connection_revision = state.connection_revision.wrapping_add(1);
            }
            clear_result(&state);
            show_connection(&state, false);
            status(
                if saved {
                    "Saved. Checking the connection…"
                } else {
                    "Connected for now. Your browser couldn’t save it, so pair again next time."
                },
                "idle",
            );
            let state = state.clone();
            spawn_local(async move { check_health(state).await });
        }
    })?;

    if element("copy-token").is_ok() {
        bind("copy-token", "click", {
            let state = state.clone();
            move |_| {
                let token = state.borrow().connection.token.clone();
                spawn_local(async move {
                    if token.is_empty() {
                        status("There’s no pairing code to copy yet.", "error");
                        return;
                    }
                    let Ok(window) = window() else { return };
                    match JsFuture::from(window.navigator().clipboard().write_text(&token)).await {
                        Ok(_) => status(
                            "Code copied. Paste it into the extension, under Connection.",
                            "success",
                        ),
                        Err(_) => status(
                            "Your browser blocked the copy. Select the code and copy it yourself.",
                            "error",
                        ),
                    }
                });
            }
        })?;
    }

    // Browsers that restore a back/forward page do not need another mount.
    if let Some(body) = document.body() {
        body.set_attribute("data-pawket-ready", "true")?;
    }
    if state.borrow().connection.token.is_empty() {
        status(
            "Almost there. Open Connection below and paste your pairing code.",
            "idle",
        );
    }
    // A right-click "Save with Pawket" opens the popup with a link to look up.
    let should_inspect = auto_inspect
        && valid_source(current_url).is_ok()
        && !state.borrow().connection.token.is_empty();
    spawn_local(async move {
        check_health(state.clone()).await;
        load_history(state.clone()).await;
        // Pick up downloads that kept running while this window was closed.
        watch_jobs(state.clone());
        if should_inspect {
            inspect(state).await;
        }
    });
    Ok(())
}

async fn inspect(state: Shared) {
    if state.borrow().busy {
        return;
    }
    let source = input("url-input")
        .map(|field| field.value().trim().to_owned())
        .unwrap_or_default();
    if let Err(message) = valid_source(&source) {
        status(&message, "error");
        let _ = input("url-input").and_then(|field| field.focus());
        return;
    }
    if state.borrow().connection.token.is_empty() {
        show_connection(&state, true);
        status(
            "Pair with Pawket first. Open Connection and paste your pairing code.",
            "error",
        );
        return;
    }
    clear_result(&state);
    let (connection, revision) = begin_request(&state);
    busy(true, "Inspecting…");
    status("Looking for files…", "busy");
    let body = serde_json::json!({ "url": source });
    let result = request_json(&connection, "/v1/inspect", Some(body), 45_000).await;
    if !finish_request(&state, revision) {
        return;
    }
    match result.and_then(|value| {
        serde_json::from_value::<MediaInfo>(value).map_err(|_| ApiError {
            message: "Something went wrong reading that link. Please try again.".to_owned(),
            pairing: false,
        })
    }) {
        Ok(media) if media.formats.is_empty() => {
            status("No files found at that link.", "error");
        }
        Ok(media) => {
            let formats = media.formats.len();
            let uses_picker = picker_view::supports(&media);
            {
                let mut state = state.borrow_mut();
                state.selected = media.formats.first().map(|format| format.id.clone());
                state.checked = picker::items(&picker_view::entries(&media));
                state.media = Some(media);
            }
            if show_media(&state).is_err() {
                status(
                    "Couldn’t show the files. Reload the page and try again.",
                    "error",
                );
                return;
            }
            set_badge("online", "Ready");
            let message = match (uses_picker, formats) {
                (true, _) => "Pick what you want, then press Save.".to_owned(),
                (false, 1) => "Found 1 file.".to_owned(),
                (false, n) => format!("Found {n} files. Pick one to save."),
            };
            status(&message, "success");
        }
        Err(error) => show_error(&state, error),
    }
}

async fn save_file(state: Shared) {
    if state.borrow().busy {
        return;
    }
    let selection = {
        let state = state.borrow();
        state.media.as_ref().and_then(|media| {
            if picker_view::supports(media) {
                let ids = picker_view::chosen_ids(media, &state.selection, &state.checked);
                return (!ids.is_empty()).then(|| (media.source_url.clone(), ids));
            }
            state.selected.as_ref().and_then(|id| {
                media
                    .formats
                    .iter()
                    .any(|format| &format.id == id)
                    .then(|| (media.source_url.clone(), vec![id.clone()]))
            })
        })
    };
    let Some((source, format_ids)) = selection else {
        status(
            "Find a link’s files first, then pick what to save.",
            "error",
        );
        return;
    };
    start_jobs(state, source, format_ids).await;
}

/// Ask the engine to download each format in the background, then follow
/// their progress. The engine keeps going if this window closes.
async fn start_jobs(state: Shared, source: String, format_ids: Vec<String>) {
    let surface = state.borrow().surface;
    let (connection, revision) = begin_request(&state);
    busy(true, "Inspect");
    status("Starting…", "busy");
    let mut started = 0usize;
    let mut failure: Option<ApiError> = None;
    for format_id in &format_ids {
        let body = serde_json::json!({
            "url": source,
            "format_id": format_id,
            "client": surface.client_tag(),
        });
        match request_json(&connection, "/v1/jobs", Some(body), 60_000).await {
            Ok(_) => started += 1,
            Err(error) => {
                failure = Some(error);
                break;
            }
        }
    }
    if !finish_request(&state, revision) {
        return;
    }
    match failure {
        Some(error) if started == 0 => show_error(&state, error),
        Some(error) => status(
            &format!(
                "Started {started} of {} downloads. Then this happened: {}",
                format_ids.len(),
                error.message
            ),
            "error",
        ),
        None if started == 1 => status("Downloading. Feel free to close this window.", "success"),
        None => status(
            &format!("Downloading {started} files. Feel free to close this window."),
            "success",
        ),
    }
    // Restore the Save button's label and any switches that stay locked.
    let _ = show_media(&state);
    watch_jobs(state);
}

/// Draw the picker (or the plain list for engines without choices) and
/// match the Save buttons to it.
fn show_media(state: &Shared) -> Result<(), JsValue> {
    let (media, selection, checked, is_busy) = {
        let state = state.borrow();
        (
            state.media.clone(),
            state.selection.clone(),
            state.checked.clone(),
            state.busy,
        )
    };
    let Some(media) = media else {
        return Ok(());
    };
    if !picker_view::supports(&media) {
        hidden("save-summary", true);
        return render_formats(state, &media);
    }
    element("media-title")?.set_text_content(Some(&media.title));
    let save = picker_view::render(&element("formats")?, &media, &selection, &checked)?;
    hidden("results", false);
    hidden("save-button", false);
    hidden("save-all-button", true);
    set_button_text("save-button", &save.label);
    if let Ok(summary) = element("save-summary") {
        summary.set_text_content(Some(&save.summary));
    }
    hidden("save-summary", save.summary.is_empty());
    if let Ok(button) = element("save-button")
        .and_then(|element| element.dyn_into::<HtmlButtonElement>().map_err(Into::into))
    {
        button.set_disabled(!save.enabled || is_busy);
    }
    Ok(())
}

fn render_formats(state: &Shared, media: &MediaInfo) -> Result<(), JsValue> {
    let document = document()?;
    element("media-title")?.set_text_content(Some(&media.title));
    let container = element("formats")?;
    container.set_text_content(None);
    container.set_class_name("formats");
    container.set_attribute("role", "radiogroup")?;
    state.borrow_mut().format_listeners.clear();
    for (index, format) in media.formats.iter().enumerate() {
        let label = document.create_element("label")?;
        label.set_class_name("format-option");
        let radio: HtmlInputElement = document.create_element("input")?.dyn_into()?;
        radio.set_type("radio");
        radio.set_name("pawket-format");
        radio.set_value(&format.id);
        radio.set_checked(index == 0);
        radio.set_class_name("format-radio");

        let details = document.create_element("span")?;
        details.set_class_name("format-info");
        let name = document.create_element("strong")?;
        name.set_class_name("format-name");
        let kind = match format.kind.as_str() {
            "audio" => "audio",
            "image" => "image",
            "video" => "video",
            _ => "file",
        };
        let heading = format
            .label
            .clone()
            .unwrap_or_else(|| format!("{} {kind}", format.extension.to_uppercase()));
        name.set_text_content(Some(&heading));
        let meta = document.create_element("span")?;
        meta.set_class_name("format-meta");
        let size = format
            .size_bytes
            .map(human_bytes)
            .unwrap_or_else(|| "Original quality".to_owned());
        let file_type = format!("{} {kind}", format.extension.to_uppercase());
        let mut description = match (format.label.is_some(), format.size_bytes) {
            // A labelled file still says what kind of file it is.
            (true, Some(bytes)) => format!("{file_type} · {}", human_bytes(bytes)),
            (true, None) => file_type,
            (false, _) => size,
        };
        if format.has_audio == Some(false) && !heading.contains("no sound") {
            description.push_str(" · No sound");
        }
        meta.set_text_content(Some(&description));
        details.append_child(&name)?;
        details.append_child(&meta)?;

        let listener = Closure::<dyn FnMut(Event)>::new({
            let state = state.clone();
            let id = format.id.clone();
            move |_| {
                if state.borrow().busy {
                    return;
                }
                state.borrow_mut().selected = Some(id.clone());
                hidden("save-button", false);
                status("Press Save file when you’re ready.", "idle");
            }
        });
        radio.add_event_listener_with_callback("change", listener.as_ref().unchecked_ref())?;
        state.borrow_mut().format_listeners.push(listener);
        label.append_child(&radio)?;
        label.append_child(&details)?;
        container.append_child(&label)?;
    }
    hidden("results", false);
    hidden("save-button", false);
    set_button_text("save-button", "Save file");
    let items = first_format_per_item(media).len();
    if items >= MIN_ITEMS_FOR_SAVE_ALL {
        set_button_text("save-all-button", &format!("Save all {items} files"));
    }
    hidden("save-all-button", items < MIN_ITEMS_FOR_SAVE_ALL);
    Ok(())
}

/// The best (first-listed) format of each media item in a multi-item post.
/// Formats without an item number are alternatives of one file, not a set.
fn first_format_per_item(media: &MediaInfo) -> Vec<String> {
    let mut seen: Vec<u32> = Vec::new();
    let mut ids = Vec::new();
    for format in &media.formats {
        let Some(item) = format.item else { continue };
        if !seen.contains(&item) {
            seen.push(item);
            ids.push(format.id.clone());
        }
    }
    ids
}

/// Download the best quality of every media item in the inspected post.
async fn save_all(state: Shared) {
    if state.borrow().busy {
        return;
    }
    let plan = {
        let state = state.borrow();
        state
            .media
            .as_ref()
            .map(|media| (media.source_url.clone(), first_format_per_item(media)))
    };
    let Some((source, format_ids)) = plan.filter(|(_, ids)| ids.len() >= MIN_ITEMS_FOR_SAVE_ALL)
    else {
        status("Find a post with more than one file first.", "error");
        return;
    };
    start_jobs(state, source, format_ids).await;
}

/// Refresh the job list on a timer until nothing is downloading. Safe to
/// call repeatedly: only one watcher runs at a time.
fn watch_jobs(state: Shared) {
    if element("jobs-list").is_err() || state.borrow().is_polling {
        return;
    }
    if state.borrow().connection.token.is_empty() {
        return;
    }
    state.borrow_mut().is_polling = true;
    spawn_local(async move {
        loop {
            let has_active = refresh_jobs(&state).await;
            if !has_active {
                break;
            }
            sleep_ms(JOB_POLL_MS).await;
        }
        state.borrow_mut().is_polling = false;
    });
}

/// Fetch this surface's jobs, draw them, and collect any finished file.
/// Returns true while something is still queued or downloading.
async fn refresh_jobs(state: &Shared) -> bool {
    let (connection, surface) = {
        let state = state.borrow();
        (state.connection.clone(), state.surface)
    };
    let Some(jobs) = request_json(&connection, "/v1/jobs", None, 8_000)
        .await
        .ok()
        .and_then(|value| serde_json::from_value::<JobList>(value).ok())
        .map(|list| list.jobs)
    else {
        // The engine is unreachable; stop watching rather than spin.
        return false;
    };
    let mine: Vec<Job> = jobs
        .into_iter()
        .filter(|job| job.client == surface.client_tag())
        .collect();
    let mut collected = false;
    for job in mine
        .iter()
        .filter(|job| job.status == "done" && !job.delivered)
    {
        collected |= collect_file(&connection, surface, job).await;
    }
    if render_jobs(&connection, &mine).is_err() {
        hidden("jobs", true);
    }
    if collected {
        load_history(state.clone()).await;
    }
    mine.iter().any(Job::is_active)
}

/// Hand a finished file to the browser, once. The engine lets only the
/// first caller claim it, so two open windows do not both download it.
async fn collect_file(connection: &Connection, surface: Surface, job: &Job) -> bool {
    let (Some(url), Some(filename)) = (job.url.as_deref(), job.filename.as_deref()) else {
        return false;
    };
    let Ok(url) = file_url(url, &connection.base) else {
        return false;
    };
    let claim_path = format!("/v1/jobs/{}/delivered", job.id);
    let body = serde_json::json!({});
    let is_ours = request_json(connection, &claim_path, Some(body), 8_000)
        .await
        .ok()
        .and_then(|value| serde_json::from_value::<Claim>(value).ok())
        .is_some_and(|claim| claim.claimed);
    if !is_ours {
        return false;
    }
    if surface == Surface::Extension {
        chrome_download(&url, filename, false).await.is_ok()
    } else {
        page_download(&url, filename).is_ok()
    }
}

fn render_jobs(connection: &Connection, jobs: &[Job]) -> Result<(), JsValue> {
    let document = document()?;
    let list = element("jobs-list")?;
    list.set_text_content(None);
    for job in jobs.iter().take(MAX_JOBS_SHOWN) {
        let row = document.create_element("li")?;
        row.set_class_name("job");
        row.set_attribute("data-status", &job.status)?;
        let title = document.create_element("strong")?;
        let heading = match job.label.as_deref() {
            Some(label) => format!("{} · {label}", job.title),
            None => job.title.clone(),
        };
        title.set_text_content(Some(&heading));
        row.append_child(&title)?;
        if job.is_active() {
            let bar = document.create_element("progress")?;
            // Without a value the bar shows as "working, size unknown".
            if let Some(total) = job.bytes_total.filter(|_| job.status == "running") {
                bar.set_attribute("max", &total.to_string())?;
                bar.set_attribute("value", &job.bytes_done.min(total).to_string())?;
            }
            row.append_child(&bar)?;
        }
        let detail = document.create_element("span")?;
        detail.set_text_content(Some(&job_detail(job)));
        row.append_child(&detail)?;
        if let (Some(url), Some(filename)) = (job.url.as_deref(), job.filename.as_deref()) {
            if let Ok(href) = file_url(url, &connection.base) {
                let again: HtmlAnchorElement = document.create_element("a")?.dyn_into()?;
                again.set_href(&href);
                again.set_download(filename);
                again.set_text_content(Some("Download again"));
                again.set_attribute("rel", "noopener noreferrer")?;
                row.append_child(&again)?;
            }
        }
        list.append_child(&row)?;
    }
    hidden("jobs", jobs.is_empty());
    Ok(())
}

/// One line describing where a job stands.
fn job_detail(job: &Job) -> String {
    match job.status.as_str() {
        "queued" => "Waiting its turn".to_owned(),
        // Sites like YouTube take a few seconds before the first byte arrives.
        "running" if job.bytes_done == 0 => "Getting ready…".to_owned(),
        "running" => match job.bytes_total.filter(|total| *total > 0) {
            Some(total) => format!(
                "{}% · {} of {}",
                job.bytes_done.min(total) * 100 / total,
                human_bytes(job.bytes_done),
                human_bytes(total)
            ),
            None => format!("{} so far", human_bytes(job.bytes_done)),
        },
        "done" => format!("Saved · {}", human_bytes(job.bytes_done)),
        _ => job
            .error
            .as_ref()
            .map(|error| error.message.clone())
            .unwrap_or_else(|| "That download didn’t work. Please try again.".to_owned()),
    }
}

async fn sleep_ms(milliseconds: i32) {
    let promise = js_sys::Promise::new(&mut |resolve, _reject| {
        if let Ok(window) = window() {
            let _ = window
                .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, milliseconds);
        }
    });
    let _ = JsFuture::from(promise).await;
}

/// Start a normal browser download of a prepared file from the web page.
fn page_download(url: &str, filename: &str) -> Result<(), JsValue> {
    let document = document()?;
    let anchor: HtmlAnchorElement = document.create_element("a")?.dyn_into()?;
    anchor.set_href(url);
    anchor.set_download(filename);
    anchor.set_attribute("rel", "noopener noreferrer")?;
    let body = document
        .body()
        .ok_or_else(|| JsValue::from_str("Pawket needs a page body"))?;
    body.append_child(&anchor)?;
    anchor.click();
    anchor.remove();
    Ok(())
}

/// Fetch and show recent downloads. Surfaces without a history list skip it.
async fn load_history(state: Shared) {
    if element("history-list").is_err() {
        return;
    }
    let connection = state.borrow().connection.clone();
    if connection.token.is_empty() {
        return;
    }
    let entries = request_json(&connection, "/v1/history", None, 8_000)
        .await
        .ok()
        .and_then(|value| serde_json::from_value::<HistoryList>(value).ok())
        .map(|list| list.entries)
        .unwrap_or_default();
    // History is a convenience: a failed render leaves the section hidden.
    if render_history(&state, &entries).is_err() {
        hidden("history", true);
    }
}

fn render_history(state: &Shared, entries: &[HistoryEntry]) -> Result<(), JsValue> {
    let document = document()?;
    let list = element("history-list")?;
    list.set_text_content(None);
    state.borrow_mut().history_listeners.clear();
    for entry in entries.iter().take(MAX_HISTORY_SHOWN) {
        let row = document.create_element("li")?;
        let button: HtmlButtonElement = document.create_element("button")?.dyn_into()?;
        button.set_type("button");
        button.set_class_name("history-item");
        let title = document.create_element("strong")?;
        title.set_text_content(Some(&entry.title));
        let meta = document.create_element("span")?;
        meta.set_text_content(Some(&format!(
            "{} · {}",
            human_bytes(entry.bytes),
            saved_date(entry.saved_at)
        )));
        button.set_title(&entry.filename);
        button.append_child(&title)?;
        button.append_child(&meta)?;
        let listener = Closure::<dyn FnMut(Event)>::new({
            let state = state.clone();
            let source = entry.source_url.clone();
            move |_| {
                if state.borrow().busy {
                    return;
                }
                let Ok(field) = input("url-input") else {
                    return;
                };
                field.set_value(&source);
                clear_result(&state);
                field.scroll_into_view();
                let state = state.clone();
                spawn_local(async move { inspect(state).await });
            }
        });
        button.add_event_listener_with_callback("click", listener.as_ref().unchecked_ref())?;
        state.borrow_mut().history_listeners.push(listener);
        row.append_child(&button)?;
        list.append_child(&row)?;
    }
    hidden("history", entries.is_empty());
    Ok(())
}

async fn clear_history(state: Shared) {
    let connection = state.borrow().connection.clone();
    let body = serde_json::json!({});
    match request_json(&connection, "/v1/history/clear", Some(body), 8_000).await {
        Ok(_) => load_history(state).await,
        Err(error) => status(&error.message, "error"),
    }
}

/// A short local date such as "Nov 10" for a Unix timestamp in seconds.
fn saved_date(seconds: u64) -> String {
    let date = js_sys::Date::new(&JsValue::from_f64(seconds as f64 * 1000.0));
    let options = Object::new();
    let _ = Reflect::set(&options, &"month".into(), &"short".into());
    let _ = Reflect::set(&options, &"day".into(), &"numeric".into());
    date.to_locale_date_string("en-US", &options).into()
}

async fn check_health(state: Shared) {
    let (connection, revision) = {
        let state = state.borrow();
        (state.connection.clone(), state.connection_revision)
    };
    set_badge("checking", "Checking…");
    let result = request_json(&connection, "/health", None, 8_000).await;
    if state.borrow().connection_revision != revision {
        return;
    }
    match result {
        Ok(value) if value.get("status").and_then(|value| value.as_str()) == Some("ok") => {
            if connection.token.is_empty() {
                set_badge("pairing", "Not paired");
            } else {
                // Health proves reachability; only authenticated API calls prove pairing.
                set_badge("online", "Ready");
            }
        }
        _ => {
            set_badge("offline", "Not running");
            if !state.borrow().busy && state.borrow().media.is_none() {
                status("Pawket isn’t running. Start it, then try again.", "error");
            }
        }
    }
}

async fn request_json(
    connection: &Connection,
    path: &str,
    body: Option<serde_json::Value>,
    timeout_ms: i32,
) -> Result<serde_json::Value, ApiError> {
    let controller = AbortController::new().map_err(|_| network_error())?;
    let options = RequestInit::new();
    options.set_method(if body.is_some() { "POST" } else { "GET" });
    options.set_mode(RequestMode::Cors);
    options.set_signal(Some(&controller.signal()));
    if let Some(body) = body {
        options.set_body(&JsValue::from_str(&body.to_string()));
    }
    let request = Request::new_with_str_and_init(&format!("{}{path}", connection.base), &options)
        .map_err(|_| network_error())?;
    request
        .headers()
        .set("Accept", "application/json")
        .map_err(|_| network_error())?;
    let is_post = options.get_method().as_deref() == Some("POST");
    if is_post {
        request
            .headers()
            .set("Content-Type", "application/json")
            .map_err(|_| network_error())?;
    }
    // Plain GETs such as /health stay header-free; history needs pairing.
    if is_post || path.starts_with("/v1/history") || path.starts_with("/v1/jobs") {
        request
            .headers()
            .set("X-Pawket-Token", &connection.token)
            .map_err(|_| ApiError {
                message:
                    "That pairing code doesn’t look right. Copy it again from the Pawket website."
                        .to_owned(),
                pairing: true,
            })?;
    }
    let timeout = Closure::<dyn FnMut()>::new({
        let controller = controller.clone();
        move || controller.abort()
    });
    let window = window().map_err(|_| network_error())?;
    let timer = window
        .set_timeout_with_callback_and_timeout_and_arguments_0(
            timeout.as_ref().unchecked_ref(),
            timeout_ms,
        )
        .map_err(|_| network_error())?;
    let result = async {
        let response: Response = JsFuture::from(window.fetch_with_request(&request))
            .await
            .map_err(|_| network_error())?
            .dyn_into()
            .map_err(|_| network_error())?;
        let status_code = response.status();
        let ok = response.ok();
        let text = JsFuture::from(response.text().map_err(|_| network_error())?)
            .await
            .map_err(|_| network_error())?
            .as_string()
            .unwrap_or_default();
        let value: serde_json::Value = serde_json::from_str(&text).map_err(|_| ApiError {
            message: "Pawket sent back something unexpected. Check the address under Connection."
                .to_owned(),
            pairing: false,
        })?;
        if !ok {
            let pairing = status_code == 401 || status_code == 403;
            let message = value
                .pointer("/error/message")
                .and_then(|value| value.as_str())
                .map(str::to_owned)
                .unwrap_or_else(|| {
                    if pairing {
                        "That pairing code is out of date. Copy the new one from the Pawket website."
                            .to_owned()
                    } else {
                        format!("Something went wrong (error {status_code}). Please try again.")
                    }
                });
            return Err(ApiError { message, pairing });
        }
        Ok(value)
    }
    .await;
    window.clear_timeout_with_handle(timer);
    if controller.signal().aborted() {
        return Err(ApiError {
            message: "Pawket took too long to answer. Check it’s running, then try again."
                .to_owned(),
            pairing: false,
        });
    }
    result
}

/// Platform binding only: Chrome handles the actual browser download lifecycle.
async fn chrome_download(url: &str, filename: &str, ask_where: bool) -> Result<(), JsValue> {
    let chrome = Reflect::get(&js_sys::global(), &JsValue::from_str("chrome"))?;
    let downloads = Reflect::get(&chrome, &JsValue::from_str("downloads"))?;
    let download: Function =
        Reflect::get(&downloads, &JsValue::from_str("download"))?.dyn_into()?;
    let options = Object::new();
    Reflect::set(&options, &JsValue::from_str("url"), &JsValue::from_str(url))?;
    // The engine sanitizes filenames; avoid path components in the Chrome API too.
    let filename = filename
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("pawket-download");
    Reflect::set(
        &options,
        &JsValue::from_str("filename"),
        &JsValue::from_str(filename),
    )?;
    Reflect::set(
        &options,
        &JsValue::from_str("saveAs"),
        &JsValue::from_bool(ask_where),
    )?;
    let promise = js_sys::Promise::resolve(&download.call1(&downloads, &options)?);
    JsFuture::from(promise).await?;
    Ok(())
}

fn normalize_base(value: &str, surface: Surface, origin: &str) -> Result<String, String> {
    let url = Url::new(value.trim())
        .map_err(|_| "Enter the full Pawket address, like http://127.0.0.1:3987.".to_owned())?;
    if !url.username().is_empty()
        || !url.password().is_empty()
        || !url.search().is_empty()
        || !url.hash().is_empty()
        || url.pathname() != "/"
    {
        return Err("Use just the address, with nothing after the port number.".to_owned());
    }
    let base = url.origin();
    let local = base == "http://127.0.0.1:3987" || base == "http://localhost:3987";
    if local || (surface == Surface::Web && base == origin) {
        Ok(base)
    } else {
        Err("Pawket only connects to this computer, on port 3987.".to_owned())
    }
}

fn valid_source(value: &str) -> Result<(), String> {
    if value.is_empty() {
        return Err("Paste a link first.".to_owned());
    }
    let url = Url::new(value)
        .map_err(|_| "That doesn’t look like a link. It should start with https://".to_owned())?;
    if !matches!(url.protocol().as_str(), "http:" | "https:")
        || url.hostname().is_empty()
        || !url.username().is_empty()
        || !url.password().is_empty()
    {
        return Err("Use a normal web link, without a username or password in it.".to_owned());
    }
    Ok(())
}

fn valid_token(token: &str) -> bool {
    token.len() <= 512 && token.bytes().all(|byte| (0x21..=0x7e).contains(&byte))
}

fn file_url(value: &str, base: &str) -> Result<String, String> {
    let url = Url::new_with_base(value, &format!("{base}/"))
        .map_err(|_| "Something went wrong getting your file. Please try again.".to_owned())?;
    if url.origin() != base || !url.pathname().starts_with("/v1/files/") {
        return Err("Something went wrong getting your file. Please try again.".to_owned());
    }
    Ok(url.href())
}

fn read_connection() -> Option<Connection> {
    let storage = window().ok()?.local_storage().ok()??;
    serde_json::from_str(&storage.get_item(CONNECTION_KEY).ok()??).ok()
}

fn write_connection(connection: &Connection) -> bool {
    window()
        .ok()
        .and_then(|window| window.local_storage().ok().flatten())
        .and_then(|storage| {
            serde_json::to_string(connection)
                .ok()
                .and_then(|value| storage.set_item(CONNECTION_KEY, &value).ok())
        })
        .is_some()
}

fn show_connection(state: &Shared, show: bool) {
    hidden("connection-panel", !show);
    if let Ok(button) = element("connection-toggle") {
        let _ = button.set_attribute("aria-expanded", if show { "true" } else { "false" });
    }
    if let Ok(field) = input("pairing-token") {
        // The explicit toggle handler is the only place that reveals this value.
        field.set_value("");
    }
    if show {
        if let Ok(field) = input("api-base") {
            field.set_value(&state.borrow().connection.base);
        }
        if let Ok(field) = input("api-token") {
            field.set_value(&state.borrow().connection.token);
        }
    }
}

fn begin_request(state: &Shared) -> (Connection, u64) {
    let mut state = state.borrow_mut();
    state.request_revision = state.request_revision.wrapping_add(1);
    state.busy = true;
    (state.connection.clone(), state.request_revision)
}

fn finish_request(state: &Shared, revision: u64) -> bool {
    let mut state = state.borrow_mut();
    state.busy = false;
    busy(false, "Find media");
    set_button_text("save-button", "Save file");
    state.request_revision == revision
}

fn clear_result(state: &Shared) {
    if let Ok(container) = element("formats") {
        container.set_text_content(None);
    }
    {
        let mut state = state.borrow_mut();
        state.request_revision = state.request_revision.wrapping_add(1);
        state.media = None;
        state.selected = None;
        state.checked.clear();
        state.format_listeners.clear();
    }
    hidden("results", true);
    hidden("save-button", true);
    hidden("save-all-button", true);
    hidden("save-summary", true);
}

fn busy(value: bool, inspect_label: &str) {
    if let Ok(field) = input("url-input") {
        field.set_disabled(value);
    }
    for id in [
        "inspect-button",
        "paste-button",
        "save-button",
        "save-all-button",
        "save-connection",
    ] {
        if let Ok(button) = element(id)
            .and_then(|element| element.dyn_into::<HtmlButtonElement>().map_err(Into::into))
        {
            button.set_disabled(value);
        }
    }
    if let Ok(container) = element("formats") {
        if let Ok(radios) = container.query_selector_all("input") {
            for index in 0..radios.length() {
                if let Some(node) = radios.item(index) {
                    if let Ok(radio) = node.dyn_into::<HtmlInputElement>() {
                        // Switches with no alternative stay disabled.
                        if value || !radio.has_attribute(picker_view::LOCKED) {
                            radio.set_disabled(value);
                        }
                    }
                }
            }
        }
    }
    if let Ok(form) = element("inspect-form") {
        let _ = form.set_attribute("aria-busy", if value { "true" } else { "false" });
    }
    set_button_text("inspect-button", inspect_label);
}

fn show_error(state: &Shared, error: ApiError) {
    if error.pairing {
        show_connection(state, true);
        set_badge("pairing", "Not paired");
    }
    status(&error.message, "error");
}

fn network_error() -> ApiError {
    ApiError {
        message: "Can’t reach Pawket. Make sure it’s running, then try again.".to_owned(),
        pairing: false,
    }
}

fn status(message: &str, kind: &str) {
    if let Ok(element) = element("status") {
        element.set_text_content(Some(message));
        let _ = element.set_attribute("data-state", kind);
    }
    // Repo Cat reacts to the same states: idle, busy, success, error.
    if let Ok(cat) = element("repo-cat") {
        let _ = cat.set_attribute("data-mood", kind);
    }
}

fn set_badge(kind: &str, label: &str) {
    if let Ok(element) = element("engine-badge") {
        element.set_text_content(Some(label));
        let _ = element.set_attribute("data-state", kind);
    }
}

fn set_button_text(id: &str, label: &str) {
    if let Ok(element) = element(id) {
        // Preserve an optional SVG icon supplied by the HTML shell.
        if let Ok(Some(text)) = element.query_selector("[data-button-label]") {
            text.set_text_content(Some(label));
        } else {
            element.set_text_content(Some(label));
        }
    }
}

fn hidden(id: &str, hidden: bool) {
    if let Ok(element) = element(id) {
        if hidden {
            let _ = element.set_attribute("hidden", "");
        } else {
            let _ = element.remove_attribute("hidden");
        }
    }
}

fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1_000.0 && unit + 1 < UNITS.len() {
        value /= 1_000.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

fn window() -> Result<web_sys::Window, JsValue> {
    web_sys::window().ok_or_else(|| JsValue::from_str("Pawket needs a browser window"))
}

fn document() -> Result<Document, JsValue> {
    window()?
        .document()
        .ok_or_else(|| JsValue::from_str("Pawket needs an HTML document"))
}

fn element(id: &str) -> Result<Element, JsValue> {
    document()?
        .get_element_by_id(id)
        .ok_or_else(|| JsValue::from_str(&format!("Missing #{id}")))
}

fn input(id: &str) -> Result<HtmlInputElement, JsValue> {
    element(id)?.dyn_into().map_err(Into::into)
}

fn bind(id: &str, event: &str, handler: impl FnMut(Event) + 'static) -> Result<(), JsValue> {
    let listener = Closure::<dyn FnMut(Event)>::new(handler);
    element(id)?.add_event_listener_with_callback(event, listener.as_ref().unchecked_ref())?;
    listener.forget();
    Ok(())
}
