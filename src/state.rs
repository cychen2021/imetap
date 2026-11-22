//! What the HUD and the tray icon show: which input method is active and, if known, whether it is
//! in Chinese or English mode.

use std::fmt;
use std::num::NonZeroU32;
use std::sync::Mutex;

use serde::Deserialize;

/// The input methods the tool knows about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ime {
    /// The plain US keyboard layout.
    Us,
    /// Microsoft Pinyin (Windows).
    Pinyin,
    /// Doubao IME (both platforms).
    Doubao,
    /// Apple Shuangpin (macOS).
    Shuangpin,
    /// Mingkuai (Windows). Known only when `mingkuai-integrate.enable` is set.
    Mingkuai,
    /// Anything else (another layout or IME).
    Other,
}

impl Ime {
    /// The English name, used in messages.
    pub fn name(self) -> &'static str {
        match self {
            Ime::Us => "US keyboard",
            Ime::Pinyin => "Microsoft Pinyin",
            Ime::Doubao => "Doubao",
            Ime::Shuangpin => "Shuangpin",
            Ime::Mingkuai => "Mingkuai",
            Ime::Other => "Other",
        }
    }

    /// The name shown on the HUD: the Chinese input methods under their Chinese names.
    pub fn display_name(self) -> &'static str {
        match self {
            Ime::Pinyin => "微软拼音",
            Ime::Doubao => "豆包",
            Ime::Shuangpin => "双拼",
            Ime::Mingkuai => "明快",
            Ime::Us | Ime::Other => self.name(),
        }
    }

    /// Parses a command-line name (`us`, `pinyin`, `doubao`, `shuangpin`).
    pub fn parse(name: &str) -> Option<Ime> {
        match name.to_ascii_lowercase().as_str() {
            "us" => Some(Ime::Us),
            "pinyin" => Some(Ime::Pinyin),
            "doubao" => Some(Ime::Doubao),
            "shuangpin" => Some(Ime::Shuangpin),
            "mingkuai" => Some(Ime::Mingkuai),
            _ => None,
        }
    }
}

/// An sRGB colour.
pub type Rgb = [u8; 3];

/// A colour the HUD can be configured with, by name. Each has a deep shade for backgrounds, with
/// white text on it, and a bright shade for the input method's stripe, with dark text on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Color {
    Red,
    Orange,
    Green,
    Teal,
    Blue,
    Purple,
    Pink,
    Brown,
    Grey,
    Black,
}

impl Color {
    /// The deep shade, for backgrounds.
    pub fn fill(self) -> Rgb {
        match self {
            Color::Red => [0xA8, 0x32, 0x2B],
            Color::Orange => [0xB8, 0x52, 0x0A],
            Color::Green => [0x2E, 0x7D, 0x32],
            Color::Teal => [0x14, 0x74, 0x6F],
            Color::Blue => [0x1F, 0x5A, 0xA6],
            Color::Purple => [0x6A, 0x3F, 0xA0],
            Color::Pink => [0xB0, 0x3A, 0x74],
            Color::Brown => [0x6D, 0x4C, 0x41],
            Color::Grey => [0x5F, 0x63, 0x68],
            Color::Black => [0x20, 0x21, 0x24],
        }
    }

    /// The bright shade, for the stripe.
    pub fn accent(self) -> Rgb {
        match self {
            Color::Red => [0xF2, 0x7B, 0x72],
            Color::Orange => [0xFF, 0xA7, 0x4A],
            Color::Green => [0x6C, 0xD4, 0x72],
            Color::Teal => [0x45, 0xD3, 0xC9],
            Color::Blue => [0x6B, 0xAA, 0xF2],
            Color::Purple => [0xB8, 0x92, 0xF0],
            Color::Pink => [0xF2, 0x84, 0xB6],
            Color::Brown => [0xC9, 0xA4, 0x82],
            Color::Grey => [0xC4, 0xC7, 0xCC],
            Color::Black => [0x9A, 0xA0, 0xA6],
        }
    }
}

/// The colours of the HUD for one state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HudColors {
    pub background: Rgb,
    pub glyph: Rgb,
    /// The stripe on the left edge, which belongs to the input method.
    pub stripe: Rgb,
    /// The input method's label on the stripe.
    pub stripe_text: Rgb,
}

impl HudColors {
    /// Colours for a background and a stripe colour.
    pub fn new(background: Color, stripe: Color) -> HudColors {
        HudColors {
            background: background.fill(),
            glyph: [0xFF, 0xFF, 0xFF],
            stripe: stripe.accent(),
            stripe_text: [0x20, 0x21, 0x24],
        }
    }
}

/// Everything the HUD draws for one state, besides the state's glyph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HudStyle {
    pub colors: HudColors,
    /// The big text on the stripe that names the input method.
    pub label: String,
}

/// The colours an unknown input method is given by its name, none of which means a mode.
const NAMED_COLORS: [Color; 6] = [
    Color::Purple,
    Color::Pink,
    Color::Brown,
    Color::Teal,
    Color::Green,
    Color::Orange,
];

/// A stable colour for an input method name.
pub fn color_for_name(name: &str) -> Color {
    // FNV-1a, so the colour does not change between runs.
    let hash = name.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
        (h ^ b as u64).wrapping_mul(0x0100_0000_01b3)
    });
    NAMED_COLORS[(hash % NAMED_COLORS.len() as u64) as usize]
}

/// A short label for an input method name: the first character when it is not ASCII ("简体拼音"
/// gives "简"), otherwise the first two letters or digits in upper case ("Hiragana" gives "HI").
pub fn label_for_name(name: &str) -> String {
    let name = name.trim();
    match name.chars().next() {
        Some(first) if !first.is_ascii() => first.to_string(),
        _ => {
            let label: String = name
                .chars()
                .filter(char::is_ascii_alphanumeric)
                .take(2)
                .map(|c| c.to_ascii_uppercase())
                .collect();
            if label.is_empty() { "?".into() } else { label }
        }
    }
}

/// The observable state of the foreground window's input method.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Snapshot {
    pub ime: Ime,
    /// `Some(true)` for Chinese mode, `Some(false)` for English mode, `None` when it cannot be
    /// told (the US layout has no such mode, and some input methods do not expose it).
    pub chinese: Option<bool>,
    /// The name of an input method imetap does not know (`Ime::Other`), when the system gave one.
    other: Option<NonZeroU32>,
}

/// The names of the unknown input methods seen so far, so that [`Snapshot`] can stay `Copy`.
static OTHER_NAMES: Mutex<Vec<String>> = Mutex::new(Vec::new());

impl Snapshot {
    pub fn new(ime: Ime, chinese: Option<bool>) -> Self {
        Self {
            ime,
            chinese,
            other: None,
        }
    }

    /// Records the name the system gives to an input method imetap does not know. The HUD derives
    /// a label and a colour from it. Two different names make two different snapshots.
    pub fn with_other_name(mut self, name: &str) -> Self {
        let name = name.trim();
        if !name.is_empty() {
            let mut names = OTHER_NAMES.lock().unwrap();
            let index = names.iter().position(|n| n == name).unwrap_or_else(|| {
                names.push(name.to_string());
                names.len() - 1
            });
            self.other = NonZeroU32::new(index as u32 + 1);
        }
        self
    }

    /// The name recorded with [`with_other_name`](Self::with_other_name).
    pub fn other_name(&self) -> Option<String> {
        let index = self.other?.get() as usize - 1;
        OTHER_NAMES.lock().unwrap().get(index).cloned()
    }

    /// The big text on the HUD and in the tray icon.
    pub fn glyph(&self) -> &'static str {
        match (self.ime, self.chinese) {
            (Ime::Us, _) => "EN",
            (Ime::Other, None) => "?",
            (_, Some(true)) => "中",
            (_, Some(false)) => "EN",
            (_, None) => "中",
        }
    }

    /// The input method and mode in words, for `status` output.
    pub fn caption(&self) -> String {
        match (self.ime, self.chinese) {
            (Ime::Other, _) => self
                .other_name()
                .unwrap_or_else(|| self.ime.display_name().to_string()),
            (Ime::Us, _) => self.ime.display_name().to_string(),
            (ime, Some(true)) => format!("{} · 中文", ime.display_name()),
            (ime, Some(false)) => format!("{} · English", ime.display_name()),
            (ime, None) => ime.display_name().to_string(),
        }
    }
}

impl fmt::Display for Snapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.glyph(), self.caption())
    }
}

/// Decides whether a window switch (or any unprompted change) deserves a HUD when only text fields
/// count. `baseline` is the state the last time focus was in a text field. Outside a text field
/// nothing shows and the baseline stays, so leaving a Chinese text box for a window without one and
/// coming back is no change. Inside one, the HUD is due when the state differs from the baseline,
/// which is then updated.
pub fn text_field_change(
    baseline: &mut Option<Snapshot>,
    now: Snapshot,
    in_text_field: bool,
) -> bool {
    if !in_text_field {
        return false;
    }
    let due = baseline.is_some_and(|before| before != now);
    *baseline = Some(now);
    due
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glyph_and_caption_follow_the_mode() {
        let s = Snapshot::new(Ime::Pinyin, Some(true));
        assert_eq!((s.glyph(), s.caption().as_str()), ("中", "微软拼音 · 中文"));
        let s = Snapshot::new(Ime::Doubao, Some(false));
        assert_eq!((s.glyph(), s.caption().as_str()), ("EN", "豆包 · English"));
    }

    #[test]
    fn the_us_layout_has_no_mode() {
        let s = Snapshot::new(Ime::Us, None);
        assert_eq!((s.glyph(), s.caption().as_str()), ("EN", "US keyboard"));
    }

    #[test]
    fn an_unknown_mode_falls_back_to_the_name() {
        let s = Snapshot::new(Ime::Shuangpin, None);
        assert_eq!((s.glyph(), s.caption().as_str()), ("中", "双拼"));
    }

    #[test]
    fn names_parse_case_insensitively() {
        assert_eq!(Ime::parse("US"), Some(Ime::Us));
        assert_eq!(Ime::parse("Doubao"), Some(Ime::Doubao));
        assert_eq!(Ime::parse("nope"), None);
    }

    #[test]
    fn only_text_fields_count_for_window_switch_huds() {
        let zh = Snapshot::new(Ime::Doubao, Some(true));
        let en = Snapshot::new(Ime::Us, None);
        let mut baseline = None;
        // The first reading only sets the baseline.
        assert!(!text_field_change(&mut baseline, zh, true));
        // A Chinese text box, then a window without a text field, then the Chinese text box again.
        assert!(!text_field_change(&mut baseline, en, false));
        assert!(!text_field_change(&mut baseline, zh, true));
        // Another text field with a different state does show.
        assert!(text_field_change(&mut baseline, en, true));
        // And back.
        assert!(text_field_change(&mut baseline, zh, true));
        assert!(!text_field_change(&mut baseline, zh, true));
    }

    #[test]
    fn names_make_labels_and_stable_colours() {
        assert_eq!(label_for_name("简体拼音"), "简");
        assert_eq!(label_for_name("Hiragana"), "HI");
        assert_eq!(label_for_name("  Dvorak "), "DV");
        assert_eq!(label_for_name("A"), "A");
        assert_eq!(label_for_name("-"), "?");
        assert_eq!(color_for_name("Hiragana"), color_for_name("Hiragana"));
        assert!(!matches!(
            color_for_name("anything"),
            Color::Red | Color::Blue
        ));
    }

    #[test]
    fn different_unknown_input_methods_are_different_snapshots() {
        let a = Snapshot::new(Ime::Other, None).with_other_name("Hiragana");
        let b = Snapshot::new(Ime::Other, None).with_other_name("Dvorak");
        let c = Snapshot::new(Ime::Other, None).with_other_name("Hiragana");
        assert_ne!(a, b);
        assert_eq!(a, c);
        assert_ne!(a, Snapshot::new(Ime::Other, None));
        assert_eq!(a.other_name().as_deref(), Some("Hiragana"));
        assert_eq!(a.caption(), "Hiragana");
        assert_eq!(Snapshot::new(Ime::Other, None).caption(), "Other");
    }
}
