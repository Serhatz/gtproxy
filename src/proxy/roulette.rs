//! Roulette spin checker.
//!
//! The server announces a spin as
//! `` `7[```wName`` spun the wheel and got `47``!`7]`` ``
//! (colour code `4`/`2`/`b`, then the number) with a delay so it appears when
//! the wheel stops. We show it at once, mark messages in exactly the server's
//! format with `[REAL]`, and add the QQ / Reme value for the chosen game.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpinMode {
    /// Plain roulette: just the number.
    Csn,
    /// Last digit: 23 -> 3.
    Qq,
    /// Digit sum, last digit: 23 -> 5, 29 -> 1, 19 -> 0.
    Reme,
}

impl SpinMode {
    pub fn name(self) -> &'static str {
        match self {
            Self::Csn => "CSN",
            Self::Qq => "QQ",
            Self::Reme => "Reme",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "csn" => Some(Self::Csn),
            "qq" => Some(Self::Qq),
            "reme" => Some(Self::Reme),
            _ => None,
        }
    }

    /// The game value of a wheel number, if the mode has one.
    pub fn value(self, number: u8) -> Option<u8> {
        match self {
            Self::Csn => None,
            Self::Qq => Some(number % 10),
            Self::Reme => Some((number / 10 + number % 10) % 10),
        }
    }
}

#[derive(Debug, Clone)]
pub struct RouletteSettings {
    pub enabled: bool,
    /// Drop the server's delay so the result shows while the wheel spins.
    pub instant: bool,
    pub mode: SpinMode,
}

impl Default for RouletteSettings {
    fn default() -> Self {
        Self { enabled: true, instant: true, mode: SpinMode::Csn }
    }
}

#[derive(Debug, PartialEq)]
pub struct Spin {
    pub player: String,
    pub number: u8,
    /// Colour code of the number: `4` red, `b` black, `2` green.
    pub colour: char,
}

const HEAD: &str = "`7[``";
const MIDDLE: &str = "`` spun the wheel and got `";
const TAIL: &str = "``!`7]``";

/// Parses a spin message in exactly the server's format; anything else is `None`.
pub fn parse_spin(text: &str) -> Option<Spin> {
    let body = text.strip_prefix(HEAD)?.strip_suffix(TAIL)?;
    let (name, result) = body.split_once(MIDDLE)?;
    // Name is usually prefixed by its own colour code (`` `w ``, `` `2 ``, ...).
    let player = name.strip_prefix('`').map(|n| n.get(1..).unwrap_or_default()).unwrap_or(name);
    let mut chars = result.chars();
    let colour = chars.next()?;
    let digits = chars.as_str();
    if player.is_empty() || !colour.is_ascii_alphanumeric() || digits.is_empty() || digits.len() > 2 {
        return None;
    }
    if !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let number: u8 = digits.parse().ok()?;
    (number <= 36).then(|| Spin { player: player.to_string(), number, colour })
}

/// Text shown to the player for a recognised spin:
/// `[REAL] <original> [REME 5]`: the words REAL / REME / QQ and a 0 value are
/// green, the brackets keep the normal text colour.
pub fn annotate(original: &str, spin: &Spin, mode: SpinMode) -> String {
    let mut out = format!("[`2REAL``] {original}");
    if let Some(v) = mode.value(spin.number) {
        let value = if v == 0 { "`20``".to_string() } else { v.to_string() };
        out.push_str(&format!(" [`2{}`` {value}]", mode.name().to_ascii_uppercase()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_server_format() {
        assert_eq!(
            parse_spin("`7[```wGrowtopiaProxy`` spun the wheel and got `20``!`7]``"),
            Some(Spin { player: "GrowtopiaProxy".into(), number: 0, colour: '2' })
        );
        assert_eq!(
            parse_spin("`7[```2Dr.Serhatzzz`` spun the wheel and got `436``!`7]``"),
            Some(Spin { player: "Dr.Serhatzzz".into(), number: 36, colour: '4' })
        );
    }

    #[test]
    fn rejects_lookalikes() {
        // chat copy without the server's brackets/colours
        assert!(parse_spin("[Bob spun the wheel and got 7!]").is_none());
        assert!(parse_spin("`7[```wBob`` spun the wheel and got `47``!`7]`` lol").is_none());
        assert!(parse_spin("`7[```wBob`` spun the wheel and got `499``!`7]``").is_none());
        assert!(parse_spin("`7[```wBob`` spun the wheel and got `4x``!`7]``").is_none());
    }

    #[test]
    fn game_values() {
        assert_eq!(SpinMode::Qq.value(23), Some(3));
        assert_eq!(SpinMode::Reme.value(23), Some(5));
        assert_eq!(SpinMode::Reme.value(29), Some(1));
        assert_eq!(SpinMode::Reme.value(19), Some(0));
        assert_eq!(SpinMode::Reme.value(0), Some(0));
        assert_eq!(SpinMode::Csn.value(23), None);
    }

    #[test]
    fn annotation() {
        let spin = Spin { player: "Bob".into(), number: 23, colour: '4' };
        assert_eq!(annotate("X", &spin, SpinMode::Qq), "[`2REAL``] X [`2QQ`` 3]");
        assert_eq!(annotate("X", &spin, SpinMode::Csn), "[`2REAL``] X");
        let zero = Spin { player: "Bob".into(), number: 19, colour: '4' };
        assert_eq!(annotate("X", &zero, SpinMode::Reme), "[`2REAL``] X [`2REME`` `20``]");
    }
}
