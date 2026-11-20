//! The format picker's rules, independent of the DOM so they are unit tested
//! natively: which options exist, how a selection stays valid as the person
//! changes one control, and which format a selection means for each item.

use serde::Deserialize;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[default]
    Video,
    Audio,
    Image,
}

impl Mode {
    pub const ALL: [Mode; 3] = [Mode::Video, Mode::Audio, Mode::Image];

    pub fn key(self) -> &'static str {
        match self {
            Mode::Video => "video",
            Mode::Audio => "audio",
            Mode::Image => "image",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Mode::Video => "Video",
            Mode::Audio => "Audio",
            Mode::Image => "Photo",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|mode| mode.key() == key)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AudioKind {
    Original,
    Mp3,
    Wav,
    Flac,
    Opus,
}

impl AudioKind {
    /// Display order: the most wanted first.
    pub const ALL: [AudioKind; 5] = [
        AudioKind::Mp3,
        AudioKind::Original,
        AudioKind::Wav,
        AudioKind::Flac,
        AudioKind::Opus,
    ];

    pub fn key(self) -> &'static str {
        match self {
            AudioKind::Original => "original",
            AudioKind::Mp3 => "mp3",
            AudioKind::Wav => "wav",
            AudioKind::Flac => "flac",
            AudioKind::Opus => "opus",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            AudioKind::Original => "Original",
            AudioKind::Mp3 => "MP3",
            AudioKind::Wav => "WAV",
            AudioKind::Flac => "FLAC",
            AudioKind::Opus => "Opus",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.key() == key)
    }
}

/// The engine's description of one format.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct Choice {
    #[serde(default)]
    pub mode: Mode,
    #[serde(default)]
    pub height: Option<u32>,
    #[serde(default)]
    pub sound: bool,
    #[serde(default)]
    pub compatible: bool,
    #[serde(default)]
    pub audio: Option<AudioKind>,
    #[serde(default)]
    pub kbps: Option<u32>,
}

/// One format as the picker sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub choice: Choice,
    pub item: Option<u32>,
}

/// What the person has picked. Kept across links, so someone who wants
/// MP3s keeps getting MP3s.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selection {
    pub mode: Mode,
    pub height: Option<u32>,
    pub sound: bool,
    pub compatible: bool,
    pub audio: AudioKind,
    pub kbps: Option<u32>,
}

pub const DEFAULT_KBPS: u32 = 192;

impl Default for Selection {
    fn default() -> Self {
        Self {
            mode: Mode::Video,
            height: None,
            sound: true,
            compatible: false,
            audio: AudioKind::Mp3,
            kbps: Some(DEFAULT_KBPS),
        }
    }
}

fn of_mode(entries: &[Entry], mode: Mode) -> impl Iterator<Item = &Choice> {
    entries
        .iter()
        .map(|entry| &entry.choice)
        .filter(move |choice| choice.mode == mode)
}

pub fn modes(entries: &[Entry]) -> Vec<Mode> {
    Mode::ALL
        .into_iter()
        .filter(|mode| of_mode(entries, *mode).next().is_some())
        .collect()
}

/// Video heights, best first; `None` ("Best") last.
pub fn heights(entries: &[Entry]) -> Vec<Option<u32>> {
    let mut heights: Vec<Option<u32>> = of_mode(entries, Mode::Video).map(|c| c.height).collect();
    heights.sort_unstable_by(|a, b| match (a, b) {
        (Some(a), Some(b)) => b.cmp(a),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    });
    heights.dedup();
    heights
}

pub fn audio_kinds(entries: &[Entry]) -> Vec<AudioKind> {
    AudioKind::ALL
        .into_iter()
        .filter(|kind| of_mode(entries, Mode::Audio).any(|c| c.audio == Some(*kind)))
        .collect()
}

/// MP3 bitrates, highest first.
pub fn bitrates(entries: &[Entry]) -> Vec<u32> {
    let mut rates: Vec<u32> = of_mode(entries, Mode::Audio)
        .filter(|c| c.audio == Some(AudioKind::Mp3))
        .filter_map(|c| c.kbps)
        .collect();
    rates.sort_unstable_by(|a, b| b.cmp(a));
    rates.dedup();
    rates
}

/// Distinct item numbers in listing order.
pub fn items(entries: &[Entry]) -> Vec<u32> {
    let mut items = Vec::new();
    for item in entries.iter().filter_map(|entry| entry.item) {
        if !items.contains(&item) {
            items.push(item);
        }
    }
    items
}

fn video_at(entries: &[Entry], height: Option<u32>) -> impl Iterator<Item = &Choice> {
    of_mode(entries, Mode::Video).filter(move |c| c.height == height)
}

/// The nearest valid selection to `wanted`: each control keeps its value
/// when some format still has it, and otherwise takes the first available.
pub fn normalize(wanted: &Selection, entries: &[Entry]) -> Selection {
    let available = modes(entries);
    let mode = if available.contains(&wanted.mode) {
        wanted.mode
    } else {
        available.first().copied().unwrap_or_default()
    };
    let mut selection = Selection {
        mode,
        ..wanted.clone()
    };
    let heights = heights(entries);
    if !heights.contains(&selection.height) {
        selection.height = heights.first().copied().flatten();
    }
    let at_height: Vec<&Choice> = video_at(entries, selection.height).collect();
    if !at_height.iter().any(|c| c.sound == selection.sound) {
        selection.sound = !selection.sound;
    }
    if !at_height
        .iter()
        .any(|c| c.sound == selection.sound && c.compatible == selection.compatible)
    {
        selection.compatible = !selection.compatible;
    }
    let kinds = audio_kinds(entries);
    if !kinds.contains(&selection.audio) {
        selection.audio = kinds.first().copied().unwrap_or(AudioKind::Mp3);
    }
    let rates = bitrates(entries);
    if selection.audio == AudioKind::Mp3 && !selection.kbps.is_some_and(|k| rates.contains(&k)) {
        selection.kbps = rates
            .iter()
            .copied()
            .find(|k| *k == DEFAULT_KBPS)
            .or_else(|| rates.first().copied());
    }
    selection
}

/// True when another format at the selected height has the other sound
/// setting, so the Sound switch can change.
pub fn can_change_sound(selection: &Selection, entries: &[Entry]) -> bool {
    video_at(entries, selection.height).any(|c| c.sound != selection.sound)
}

/// True when the "Plays everywhere" switch can change at this height and
/// sound setting.
pub fn can_change_compatible(selection: &Selection, entries: &[Entry]) -> bool {
    video_at(entries, selection.height)
        .any(|c| c.sound == selection.sound && c.compatible != selection.compatible)
}

fn matches(selection: &Selection, choice: &Choice) -> bool {
    if choice.mode != selection.mode {
        return false;
    }
    match choice.mode {
        Mode::Video => {
            choice.height == selection.height
                && choice.sound == selection.sound
                && choice.compatible == selection.compatible
        }
        Mode::Audio => {
            choice.audio == Some(selection.audio)
                && (selection.audio != AudioKind::Mp3 || choice.kbps == selection.kbps)
        }
        Mode::Image => true,
    }
}

/// The index of the format `selection` means for `item` (`None` for a
/// single-item source). Items without that exact choice fall back to their
/// best format of the same kind, then to their best format of any kind.
pub fn pick(selection: &Selection, entries: &[Entry], item: Option<u32>) -> Option<usize> {
    let in_item = || {
        entries
            .iter()
            .enumerate()
            .filter(move |(_, e)| e.item == item)
    };
    in_item()
        .find(|(_, e)| matches(selection, &e.choice))
        .or_else(|| in_item().find(|(_, e)| e.choice.mode == selection.mode))
        .or_else(|| in_item().next())
        .map(|(index, _)| index)
}

/// One control the person changed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    Mode(Mode),
    Height(Option<u32>),
    Sound(bool),
    Compatible(bool),
    Audio(AudioKind),
    Kbps(u32),
    Item { item: u32, on: bool },
    AllItems(bool),
}

/// Control names used in the DOM. Values come from `Mode::key`,
/// `AudioKind::key`, heights (or "best") and bitrates.
pub const MODE: &str = "pk-mode";
pub const HEIGHT: &str = "pk-height";
pub const SOUND: &str = "pk-sound";
pub const COMPATIBLE: &str = "pk-compatible";
pub const AUDIO: &str = "pk-audio";
pub const KBPS: &str = "pk-kbps";
pub const ITEM: &str = "pk-item";
pub const ALL_ITEMS: &str = "pk-all";

pub fn height_value(height: Option<u32>) -> String {
    height.map_or_else(|| "best".into(), |h| h.to_string())
}

/// Read a change event from a control's name, value and checked state.
pub fn parse_change(name: &str, value: &str, checked: bool) -> Option<Change> {
    match name {
        MODE => Mode::from_key(value).map(Change::Mode),
        HEIGHT if value == "best" => Some(Change::Height(None)),
        HEIGHT => value.parse().ok().map(|h| Change::Height(Some(h))),
        SOUND => Some(Change::Sound(checked)),
        COMPATIBLE => Some(Change::Compatible(checked)),
        AUDIO => AudioKind::from_key(value).map(Change::Audio),
        KBPS => value.parse().ok().map(Change::Kbps),
        ITEM => value
            .parse()
            .ok()
            .map(|item| Change::Item { item, on: checked }),
        ALL_ITEMS => Some(Change::AllItems(checked)),
        _ => None,
    }
}

/// Apply a change to the current selection and checked items.
pub fn apply(
    selection: &Selection,
    checked: &[u32],
    all_items: &[u32],
    change: Change,
) -> (Selection, Vec<u32>) {
    let next = selection.clone();
    let selection = match change {
        Change::Mode(mode) => Selection { mode, ..next },
        Change::Height(height) => Selection { height, ..next },
        Change::Sound(sound) => Selection { sound, ..next },
        Change::Compatible(compatible) => Selection { compatible, ..next },
        Change::Audio(audio) => Selection { audio, ..next },
        Change::Kbps(kbps) => Selection {
            kbps: Some(kbps),
            ..next
        },
        Change::Item { .. } | Change::AllItems(_) => next,
    };
    let checked = match change {
        Change::Item { item, on: true } => all_items
            .iter()
            .copied()
            .filter(|i| *i == item || checked.contains(i))
            .collect(),
        Change::Item { item, on: false } => {
            checked.iter().copied().filter(|i| *i != item).collect()
        }
        Change::AllItems(true) => all_items.to_vec(),
        Change::AllItems(false) => Vec::new(),
        _ => checked.to_vec(),
    };
    (selection, checked)
}

/// "4K", "1440p", or "Best" when the height isn't known.
pub fn height_label(height: Option<u32>) -> String {
    match height {
        Some(h) if h >= 4320 => "8K".into(),
        Some(h) if h >= 2160 => "4K".into(),
        Some(h) => format!("{h}p"),
        None => "Best".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn video(height: Option<u32>, sound: bool, compatible: bool) -> Entry {
        Entry {
            choice: Choice {
                mode: Mode::Video,
                height,
                sound,
                compatible,
                ..Choice::default()
            },
            item: None,
        }
    }

    fn audio(kind: AudioKind, kbps: Option<u32>) -> Entry {
        Entry {
            choice: Choice {
                mode: Mode::Audio,
                sound: true,
                audio: Some(kind),
                kbps,
                ..Choice::default()
            },
            item: None,
        }
    }

    /// A 4K YouTube video: no H.264 above 1080p.
    fn youtube() -> Vec<Entry> {
        vec![
            video(Some(2160), true, false),
            video(Some(2160), false, false),
            video(Some(2160), true, true),
            video(Some(1080), true, true),
            video(Some(1080), false, true),
            audio(AudioKind::Original, None),
            audio(AudioKind::Mp3, Some(320)),
            audio(AudioKind::Mp3, Some(192)),
            audio(AudioKind::Mp3, Some(128)),
            audio(AudioKind::Wav, None),
            audio(AudioKind::Flac, None),
        ]
    }

    #[test]
    fn lists_what_exists_in_display_order() {
        let entries = youtube();
        assert_eq!(modes(&entries), [Mode::Video, Mode::Audio]);
        assert_eq!(heights(&entries), [Some(2160), Some(1080)]);
        assert_eq!(
            audio_kinds(&entries),
            [
                AudioKind::Mp3,
                AudioKind::Original,
                AudioKind::Wav,
                AudioKind::Flac
            ]
        );
        assert_eq!(bitrates(&entries), [320, 192, 128]);
        assert_eq!(height_label(Some(2160)), "4K");
        assert_eq!(height_label(Some(720)), "720p");
        assert_eq!(height_label(None), "Best");
    }

    #[test]
    fn defaults_to_the_best_video_with_sound_without_re_encoding() {
        let entries = youtube();
        let selection = normalize(&Selection::default(), &entries);
        assert_eq!(
            (
                selection.mode,
                selection.height,
                selection.sound,
                selection.compatible
            ),
            (Mode::Video, Some(2160), true, false)
        );
        assert_eq!(pick(&selection, &entries, None), Some(0));
        assert!(can_change_sound(&selection, &entries));
        assert!(can_change_compatible(&selection, &entries));
    }

    #[test]
    fn switching_quality_keeps_choices_that_still_exist() {
        let entries = youtube();
        let at_4k = normalize(&Selection::default(), &entries);
        let at_1080 = normalize(
            &Selection {
                height: Some(1080),
                ..at_4k
            },
            &entries,
        );
        // 1080p is already H.264, so "plays everywhere" turns on by itself
        // and can't be turned off.
        assert!(at_1080.compatible);
        assert!(!can_change_compatible(&at_1080, &entries));
        assert_eq!(pick(&at_1080, &entries, None), Some(3));
        let muted = normalize(
            &Selection {
                sound: false,
                ..at_1080
            },
            &entries,
        );
        assert_eq!(pick(&muted, &entries, None), Some(4));
        // Muted 4K has no H.264 version: the switch falls back.
        let muted_4k = normalize(
            &Selection {
                height: Some(2160),
                compatible: true,
                ..muted
            },
            &entries,
        );
        assert!(!muted_4k.compatible);
        assert_eq!(pick(&muted_4k, &entries, None), Some(1));
    }

    #[test]
    fn audio_picks_format_and_mp3_bitrate() {
        let entries = youtube();
        let mp3 = normalize(
            &Selection {
                mode: Mode::Audio,
                ..Selection::default()
            },
            &entries,
        );
        assert_eq!((mp3.audio, mp3.kbps), (AudioKind::Mp3, Some(192)));
        assert_eq!(pick(&mp3, &entries, None), Some(7));
        let high = Selection {
            kbps: Some(320),
            ..mp3.clone()
        };
        assert_eq!(pick(&high, &entries, None), Some(6));
        let flac = Selection {
            audio: AudioKind::Flac,
            ..mp3
        };
        assert_eq!(pick(&flac, &entries, None), Some(10));
        // A choice the source lacks falls back to what it has.
        let opus = normalize(
            &Selection {
                audio: AudioKind::Opus,
                ..flac
            },
            &entries,
        );
        assert_eq!(opus.audio, AudioKind::Mp3);
    }

    #[test]
    fn missing_modes_fall_back_and_photos_need_no_options() {
        let photo = vec![Entry {
            choice: Choice {
                mode: Mode::Image,
                ..Choice::default()
            },
            item: None,
        }];
        let selection = normalize(&Selection::default(), &photo);
        assert_eq!(selection.mode, Mode::Image);
        assert_eq!(pick(&selection, &photo, None), Some(0));
    }

    #[test]
    fn reads_and_applies_control_changes() {
        assert_eq!(
            parse_change(MODE, "audio", true),
            Some(Change::Mode(Mode::Audio))
        );
        assert_eq!(
            parse_change(HEIGHT, "best", true),
            Some(Change::Height(None))
        );
        assert_eq!(
            parse_change(HEIGHT, "1080", true),
            Some(Change::Height(Some(1080)))
        );
        assert_eq!(parse_change(SOUND, "on", false), Some(Change::Sound(false)));
        assert_eq!(parse_change(KBPS, "320", true), Some(Change::Kbps(320)));
        assert_eq!(parse_change(HEIGHT, "big", true), None);
        assert_eq!(parse_change("other", "x", true), None);
        let all = [1, 2, 3];
        let (_, checked) = apply(
            &Selection::default(),
            &[1, 2, 3],
            &all,
            Change::Item { item: 2, on: false },
        );
        assert_eq!(checked, [1, 3]);
        let (_, checked) = apply(
            &Selection::default(),
            &[3],
            &all,
            Change::Item { item: 1, on: true },
        );
        assert_eq!(checked, [1, 3]);
        let (_, checked) = apply(&Selection::default(), &[1], &all, Change::AllItems(true));
        assert_eq!(checked, all);
        let (selection, _) = apply(
            &Selection::default(),
            &[],
            &all,
            Change::Audio(AudioKind::Wav),
        );
        assert_eq!(selection.audio, AudioKind::Wav);
    }

    #[test]
    fn each_item_gets_the_same_choice_or_its_best_of_that_kind() {
        let with_item = |mut entry: Entry, item| {
            entry.item = Some(item);
            entry
        };
        let entries = vec![
            with_item(video(None, true, false), 1),
            with_item(audio(AudioKind::Mp3, Some(320)), 1),
            with_item(video(None, true, false), 2),
            with_item(audio(AudioKind::Original, None), 2),
            with_item(
                Entry {
                    choice: Choice {
                        mode: Mode::Image,
                        ..Choice::default()
                    },
                    item: None,
                },
                3,
            ),
        ];
        assert_eq!(items(&entries), [1, 2, 3]);
        let selection = Selection {
            mode: Mode::Audio,
            audio: AudioKind::Mp3,
            kbps: Some(320),
            ..Selection::default()
        };
        assert_eq!(pick(&selection, &entries, Some(1)), Some(1));
        assert_eq!(pick(&selection, &entries, Some(2)), Some(3));
        assert_eq!(pick(&selection, &entries, Some(3)), Some(4));
        assert_eq!(pick(&selection, &entries, Some(9)), None);
    }
}
