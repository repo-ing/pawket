//! Draws the format picker: Video / Audio / Photo, quality chips, Sound and
//! "Plays everywhere" switches, audio format and bitrate chips, playlist
//! items, and a one-line summary of what Save will produce.
//!
//! Every control is a native radio or checkbox input, so keyboard and
//! screen-reader behaviour come from the browser. Changes reach the page
//! through one delegated `change` listener on the container.

use super::{document, human_bytes, MediaFormat, MediaInfo};
use crate::picker::{self, AudioKind, Entry, Mode, Selection};
use wasm_bindgen::{prelude::*, JsCast};
use web_sys::{Document, Element, HtmlInputElement};

/// Attribute that keeps an input disabled even when the page stops being
/// busy, for switches with no alternative.
pub(super) const LOCKED: &str = "data-locked";

/// True when every format describes itself, so the picker can be used.
pub(super) fn supports(media: &MediaInfo) -> bool {
    !media.formats.is_empty() && media.formats.iter().all(|f| f.choice.is_some())
}

pub(super) fn entries(media: &MediaInfo) -> Vec<Entry> {
    media
        .formats
        .iter()
        .map(|format| Entry {
            choice: format.choice.clone().unwrap_or_default(),
            item: format.item,
        })
        .collect()
}

/// The format IDs Save should start, in item order.
pub(super) fn chosen_ids(media: &MediaInfo, selection: &Selection, checked: &[u32]) -> Vec<String> {
    let entries = entries(media);
    let selection = picker::normalize(selection, &entries);
    let items = picker::items(&entries);
    let wanted: Vec<Option<u32>> = if items.is_empty() {
        vec![None]
    } else {
        items
            .into_iter()
            .filter(|item| checked.contains(item))
            .map(Some)
            .collect()
    };
    wanted
        .into_iter()
        .filter_map(|item| picker::pick(&selection, &entries, item))
        .map(|index| media.formats[index].id.clone())
        .collect()
}

/// What the Save button should say, and whether it can be pressed.
pub(super) struct SaveButton {
    pub(super) label: String,
    pub(super) enabled: bool,
    /// One line describing what Save will produce.
    pub(super) summary: String,
}

pub(super) fn render(
    container: &Element,
    media: &MediaInfo,
    wanted: &Selection,
    checked: &[u32],
) -> Result<SaveButton, JsValue> {
    let document = document()?;
    let entries = entries(media);
    let selection = picker::normalize(wanted, &entries);
    let focused = focused_control(&document, container);
    container.set_text_content(None);
    container.set_class_name("formats pk");
    container.set_attribute("role", "group")?;
    container.set_attribute("aria-label", "What to save")?;

    let modes = picker::modes(&entries);
    if modes.len() > 1 {
        container.append_child(&*mode_tabs(&document, &modes, selection.mode)?)?;
    }
    let body = div(&document, "pk-body")?;
    match selection.mode {
        Mode::Video => video_controls(&document, &body, &entries, &selection)?,
        Mode::Audio => audio_controls(&document, &body, &entries, &selection)?,
        Mode::Image => {}
    }
    let items = picker::items(&entries);
    let single = picker::pick(&selection, &entries, None).map(|i| &media.formats[i]);
    if let Some(text) = hint(&selection, &entries, single) {
        let hint = document.create_element("p")?;
        hint.set_class_name("pk-hint");
        hint.set_text_content(Some(&text));
        body.append_child(&hint)?;
    }
    if body.child_element_count() > 0 {
        container.append_child(&body)?;
    }
    if items.len() > 1 {
        container.append_child(&*item_list(&document, media, &items, checked)?)?;
    }
    let save = if items.len() > 1 {
        let count = items.iter().filter(|i| checked.contains(i)).count();
        SaveButton {
            label: match count {
                0 => "Select something to save".into(),
                1 => "Save 1 file".into(),
                n => format!("Save {n} files"),
            },
            enabled: count > 0,
            summary: format!("{count} of {} · {}", items.len(), describe(&selection)),
        }
    } else {
        let format = single.or_else(|| media.formats.first());
        SaveButton {
            label: "Save".into(),
            enabled: format.is_some(),
            summary: format.map(format_summary).unwrap_or_default(),
        }
    };
    restore_focus(container, focused);
    Ok(save)
}

fn div(document: &Document, class: &str) -> Result<Element, JsValue> {
    let element = document.create_element("div")?;
    element.set_class_name(class);
    Ok(element)
}

/// A labelled native radio or checkbox with visible text.
fn control(
    document: &Document,
    class: &str,
    kind: &str,
    name: &str,
    value: &str,
    checked: bool,
) -> Result<(Element, HtmlInputElement), JsValue> {
    let label = document.create_element("label")?;
    label.set_class_name(class);
    let input: HtmlInputElement = document.create_element("input")?.dyn_into()?;
    input.set_type(kind);
    input.set_name(name);
    input.set_value(value);
    input.set_checked(checked);
    label.append_child(&input)?;
    Ok((label, input))
}

fn text(document: &Document, class: &str, value: &str) -> Result<Element, JsValue> {
    let span = document.create_element("span")?;
    span.set_class_name(class);
    span.set_text_content(Some(value));
    Ok(span)
}

fn mode_tabs(document: &Document, modes: &[Mode], current: Mode) -> Result<Element, JsValue> {
    let group = div(document, "pk-modes")?;
    group.set_attribute("role", "radiogroup")?;
    group.set_attribute("aria-label", "Save as")?;
    for mode in modes {
        let (label, _) = control(
            document,
            "pk-mode",
            "radio",
            picker::MODE,
            mode.key(),
            *mode == current,
        )?;
        label.append_child(&*text(document, "pk-mode-text", mode.label())?)?;
        group.append_child(&label)?;
    }
    Ok(group)
}

/// A labelled row of chips (radio buttons) for one setting.
fn chip_row(
    document: &Document,
    title: &str,
    name: &str,
    chips: &[(String, String, bool)],
) -> Result<Element, JsValue> {
    let row = div(document, "pk-row")?;
    let id = format!("{name}-label");
    let heading = text(document, "pk-label", title)?;
    heading.set_id(&id);
    row.append_child(&heading)?;
    let group = div(document, "pk-chips")?;
    group.set_attribute("role", "radiogroup")?;
    group.set_attribute("aria-labelledby", &id)?;
    for (value, caption, checked) in chips {
        let (label, _) = control(document, "pk-chip", "radio", name, value, *checked)?;
        label.append_child(&*text(document, "pk-chip-text", caption)?)?;
        group.append_child(&label)?;
    }
    row.append_child(&group)?;
    Ok(row)
}

fn switch(
    document: &Document,
    name: &str,
    caption: &str,
    on: bool,
    can_change: bool,
) -> Result<Element, JsValue> {
    let (label, input) = control(document, "pk-switch", "checkbox", name, "on", on)?;
    input.set_attribute("role", "switch")?;
    if !can_change {
        input.set_disabled(true);
        input.set_attribute(LOCKED, "")?;
        let reason = match (name, on) {
            (picker::COMPATIBLE, true) => "This quality already plays everywhere.",
            (picker::SOUND, false) => "This quality has no sound.",
            _ => "No other version at this quality.",
        };
        label.set_attribute("title", reason)?;
    }
    label.append_child(&*text(document, "pk-track", "")?)?;
    label.append_child(&*text(document, "pk-switch-text", caption)?)?;
    Ok(label)
}

fn video_controls(
    document: &Document,
    body: &Element,
    entries: &[Entry],
    selection: &Selection,
) -> Result<(), JsValue> {
    let heights = picker::heights(entries);
    if heights.len() > 1 || heights.first().is_some_and(Option::is_some) {
        let chips: Vec<(String, String, bool)> = heights
            .iter()
            .map(|h| {
                (
                    picker::height_value(*h),
                    picker::height_label(*h),
                    *h == selection.height,
                )
            })
            .collect();
        body.append_child(&*chip_row(document, "Quality", picker::HEIGHT, &chips)?)?;
    }
    let can_sound = picker::can_change_sound(selection, entries);
    let can_compatible = picker::can_change_compatible(selection, entries);
    if !can_sound && !can_compatible && !selection.compatible {
        return Ok(());
    }
    let switches = div(document, "pk-switches")?;
    switches.append_child(&*switch(
        document,
        picker::SOUND,
        "Sound",
        selection.sound,
        can_sound,
    )?)?;
    if can_compatible || selection.compatible {
        switches.append_child(&*switch(
            document,
            picker::COMPATIBLE,
            "Plays everywhere",
            selection.compatible,
            can_compatible,
        )?)?;
    }
    body.append_child(&switches)?;
    Ok(())
}

fn audio_controls(
    document: &Document,
    body: &Element,
    entries: &[Entry],
    selection: &Selection,
) -> Result<(), JsValue> {
    let kinds = picker::audio_kinds(entries);
    if kinds.len() > 1 {
        let chips: Vec<(String, String, bool)> = kinds
            .iter()
            .map(|kind| {
                (
                    kind.key().into(),
                    kind.label().into(),
                    *kind == selection.audio,
                )
            })
            .collect();
        body.append_child(&*chip_row(document, "Format", picker::AUDIO, &chips)?)?;
    }
    let rates = picker::bitrates(entries);
    if selection.audio == AudioKind::Mp3 && rates.len() > 1 {
        let chips: Vec<(String, String, bool)> = rates
            .iter()
            .map(|kbps| {
                (
                    kbps.to_string(),
                    format!("{kbps} kbps"),
                    Some(*kbps) == selection.kbps,
                )
            })
            .collect();
        body.append_child(&*chip_row(document, "Quality", picker::KBPS, &chips)?)?;
    }
    Ok(())
}

/// A short note on what the current choice means, when it helps.
fn hint(selection: &Selection, entries: &[Entry], picked: Option<&MediaFormat>) -> Option<String> {
    let re_encodes = picked.is_some_and(|f| f.conversion.as_deref() == Some("mp4"));
    match selection.mode {
        Mode::Video if selection.compatible && re_encodes => {
            Some("Converted to H.264 so it opens anywhere. This takes a little longer.".into())
        }
        Mode::Video
            if !selection.compatible && picker::can_change_compatible(selection, entries) =>
        {
            Some("Best quality. Some players, like QuickTime, may not open it.".into())
        }
        Mode::Audio => match selection.audio {
            AudioKind::Original => Some("Exactly as the site serves it. No conversion.".into()),
            AudioKind::Wav | AudioKind::Flac => {
                Some("Lossless: no further quality loss, but a much bigger file.".into())
            }
            AudioKind::Mp3 | AudioKind::Opus => None,
        },
        _ => None,
    }
}

/// "4K · with sound" or "MP3 · 320 kbps": the selection in a few words.
fn describe(selection: &Selection) -> String {
    match selection.mode {
        Mode::Video => {
            let quality = picker::height_label(selection.height);
            let sound = if selection.sound {
                "with sound"
            } else {
                "no sound"
            };
            format!("{quality} video · {sound}")
        }
        Mode::Audio => match (selection.audio, selection.kbps) {
            (AudioKind::Mp3, Some(kbps)) => format!("MP3 · {kbps} kbps"),
            (AudioKind::Original, _) => "Original audio".into(),
            (kind, _) => kind.label().into(),
        },
        Mode::Image => "Photos".into(),
    }
}

/// "MP4 · 1080p · with sound · 23.4 MB".
fn format_summary(format: &MediaFormat) -> String {
    let extension = format.extension.to_uppercase();
    let mut parts = Vec::new();
    match format.label.as_deref() {
        Some(label) if label.to_uppercase().starts_with(&extension) => {
            parts.push(label.to_string())
        }
        Some(label) => {
            parts.push(extension);
            parts.push(label.to_string());
        }
        None => parts.push(extension),
    }
    if let Some(bytes) = format.size_bytes {
        parts.push(human_bytes(bytes));
    }
    parts.join(" · ")
}

/// The item's name: a playlist entry's title, or "Video 2" / "Photo 3".
fn item_title(media: &MediaInfo, item: u32) -> String {
    let first = media.formats.iter().find(|f| f.item == Some(item));
    let prefix = format!("{item}. ");
    if let Some(title) = first
        .and_then(|f| f.label.as_deref())
        .and_then(|label| label.strip_prefix(&prefix))
    {
        return title.to_string();
    }
    let kind = match first.map(|f| f.kind.as_str()) {
        Some("image") => "Photo",
        Some("audio") => "Audio",
        _ => "Video",
    };
    format!("{kind} {item}")
}

fn item_list(
    document: &Document,
    media: &MediaInfo,
    items: &[u32],
    checked: &[u32],
) -> Result<Element, JsValue> {
    let list = div(document, "pk-items")?;
    let all = items.iter().all(|item| checked.contains(item));
    let (head, toggle) = control(
        document,
        "pk-items-head",
        "checkbox",
        picker::ALL_ITEMS,
        "on",
        all,
    )?;
    toggle.set_indeterminate(!all && items.iter().any(|item| checked.contains(item)));
    head.append_child(&*text(document, "pk-item-title", "Select all")?)?;
    head.append_child(&*text(
        document,
        "pk-item-count",
        &format!("{} items", items.len()),
    )?)?;
    list.append_child(&head)?;
    let rows = document.create_element("ul")?;
    rows.set_class_name("pk-item-rows");
    for item in items {
        let row = document.create_element("li")?;
        let (label, _) = control(
            document,
            "pk-item",
            "checkbox",
            picker::ITEM,
            &item.to_string(),
            checked.contains(item),
        )?;
        label.append_child(&*text(document, "pk-item-number", &item.to_string())?)?;
        label.append_child(&*text(
            document,
            "pk-item-title",
            &item_title(media, *item),
        )?)?;
        row.append_child(&label)?;
        rows.append_child(&row)?;
    }
    list.append_child(&rows)?;
    Ok(list)
}

/// The name and value of the focused picker control, so focus survives a
/// re-render (arrow keys inside a chip group keep working).
fn focused_control(document: &Document, container: &Element) -> Option<(String, String)> {
    let active = document.active_element()?;
    if !container.contains(Some(&active)) {
        return None;
    }
    let input: HtmlInputElement = active.dyn_into().ok()?;
    Some((input.name(), input.value()))
}

fn restore_focus(container: &Element, focused: Option<(String, String)>) {
    let Some((name, value)) = focused else { return };
    let inputs = container.query_selector_all("input").ok();
    let Some(inputs) = inputs else { return };
    for index in 0..inputs.length() {
        let Some(input) = inputs
            .item(index)
            .and_then(|node| node.dyn_into::<HtmlInputElement>().ok())
        else {
            continue;
        };
        if input.name() == name && input.value() == value {
            let _ = input.focus();
            return;
        }
    }
}
