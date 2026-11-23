//! User settings read from `drawn_arts.ini` next to the DLL.

use std::{fs, io, path::Path, str::FromStr};

use anyhow::{Context, anyhow, bail};
use log::LevelFilter;
use windows::Win32::UI::Input::KeyboardAndMouse::*;

use crate::{arts::CombatArt, game::Uid};

/// Settings of the mod.
#[derive(Debug)]
pub struct Config {
    /// Key to hold to have the selected combat art equipped.
    pub hold_key: VIRTUAL_KEY,
    /// Key to hold to show the combat art wheel.
    pub wheel_key: VIRTUAL_KEY,
    /// Multiplier for the raw mouse movement that steers the wheel.
    pub wheel_sensitivity: f32,
    /// Art selected when the loaded save has no combat art equipped.
    pub default_art: Uid,
    /// Arts the wheel shows, clockwise from the top. Empty means every art the save owns.
    pub wheel_arts: Vec<Uid>,
    /// Most detailed messages written to the log.
    pub log_level: LevelFilter,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            hold_key: VK_SHIFT,
            wheel_key: VK_TAB,
            wheel_sensitivity: 0.3,
            default_art: Uid(5700), // Mortal Draw
            wheel_arts: Vec::new(),
            log_level: LevelFilter::Info,
        }
    }
}

impl Config {
    /// Reads the settings from `path`, falling back to the defaults if the file doesn't exist.
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        match fs::read_to_string(path) {
            Ok(text) => text.parse().with_context(|| format!("invalid config {}", path.display())),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                log::info!("{} not found, using default settings", path.display());
                Ok(Self::default())
            }
            Err(e) => Err(e).with_context(|| format!("can't read {}", path.display())),
        }
    }
}

impl FromStr for Config {
    type Err = anyhow::Error;

    /// Parses `key = value` lines; `#` starts a comment and unknown keys are rejected.
    fn from_str(text: &str) -> anyhow::Result<Self> {
        let mut config = Self::default();
        for (number, line) in text.lines().enumerate() {
            let line = line.split('#').next().unwrap_or_default().trim();
            if line.is_empty() {
                continue;
            }
            let (key, value) = line
                .split_once('=')
                .map(|(key, value)| (key.trim(), value.trim()))
                .ok_or_else(|| anyhow!("line {}: expected `key = value`", number + 1))?;
            let result = match key {
                "hold_key" => parse_key(value).map(|k| config.hold_key = k),
                "wheel_key" => parse_key(value).map(|k| config.wheel_key = k),
                "wheel_sensitivity" => value
                    .parse()
                    .ok()
                    .filter(|&s: &f32| s.is_finite() && s > 0.0)
                    .map(|s| config.wheel_sensitivity = s)
                    .ok_or_else(|| anyhow!("`{value}` is not a positive number")),
                "default_art" => parse_art(value).map(|uid| config.default_art = uid),
                "wheel_arts" => value
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(parse_art)
                    .collect::<anyhow::Result<_>>()
                    .map(|uids| config.wheel_arts = uids),
                "log_level" => value
                    .parse()
                    .map(|level| config.log_level = level)
                    .map_err(|_| anyhow!("`{value}` is not one of off, error, warn, info, debug, trace")),
                _ => Err(anyhow!("unknown setting `{key}`")),
            };
            result.with_context(|| format!("line {}", number + 1))?;
        }
        Ok(config)
    }
}

fn parse_art(value: &str) -> anyhow::Result<Uid> {
    let uid = Uid(value.parse().with_context(|| format!("`{value}` is not a combat art uid"))?);
    match CombatArt::find(uid) {
        Some(_) => Ok(uid),
        None => bail!("`{value}` is not a combat art uid"),
    }
}

/// Parses a key name like `shift` or `f5`, or a virtual-key code like `0x10`.
fn parse_key(value: &str) -> anyhow::Result<VIRTUAL_KEY> {
    let name = value.to_ascii_lowercase();
    if let Some(hex) = name.strip_prefix("0x") {
        return u16::from_str_radix(hex, 16)
            .map(VIRTUAL_KEY)
            .with_context(|| format!("`{value}` is not a virtual-key code"));
    }
    let key = match name.as_str() {
        "shift" => VK_SHIFT,
        "lshift" => VK_LSHIFT,
        "rshift" => VK_RSHIFT,
        "ctrl" => VK_CONTROL,
        "lctrl" => VK_LCONTROL,
        "rctrl" => VK_RCONTROL,
        "alt" => VK_MENU,
        "lalt" => VK_LMENU,
        "ralt" => VK_RMENU,
        "tab" => VK_TAB,
        "capslock" => VK_CAPITAL,
        "space" => VK_SPACE,
        "mouse4" | "x1" => VK_XBUTTON1,
        "mouse5" | "x2" => VK_XBUTTON2,
        "mmb" => VK_MBUTTON,
        _ => match name.as_bytes() {
            // Letters and digits share their virtual-key codes with their uppercase ASCII codes.
            [c @ (b'a'..=b'z' | b'0'..=b'9')] => VIRTUAL_KEY(c.to_ascii_uppercase().into()),
            [b'f', digits @ ..] => match std::str::from_utf8(digits).ok().and_then(|d| d.parse().ok()) {
                Some(n @ 1..=12u16) => VIRTUAL_KEY(VK_F1.0 + n - 1),
                _ => bail!("unknown key `{value}`"),
            },
            _ => bail!("unknown key `{value}`"),
        },
    };
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_all_settings() {
        let config: Config = "
            # comment
            hold_key = LShift
            wheel_key = 0x14   # caps lock
            wheel_sensitivity = 0.75
            default_art = 7100
            wheel_arts = 5700, 7700,7100
            log_level = trace
        "
        .parse()
        .unwrap();
        assert_eq!(config.hold_key, VK_LSHIFT);
        assert_eq!(config.wheel_key, VK_CAPITAL);
        assert_eq!(config.wheel_sensitivity, 0.75);
        assert_eq!(config.default_art, Uid(7100));
        assert_eq!(config.wheel_arts, [Uid(5700), Uid(7700), Uid(7100)]);
        assert_eq!(config.log_level, LevelFilter::Trace);
    }

    #[test]
    fn shipped_config_matches_defaults() {
        let config: Config = include_str!("../drawn_arts/drawn_arts.ini").parse().unwrap();
        let default = Config::default();
        assert_eq!(config.hold_key, default.hold_key);
        assert_eq!(config.wheel_key, default.wheel_key);
        assert_eq!(config.wheel_sensitivity, default.wheel_sensitivity);
        assert_eq!(config.default_art, default.default_art);
        assert!(config.wheel_arts.is_empty());
        assert_eq!(config.log_level, default.log_level);
    }

    #[test]
    fn parses_letter_and_function_keys() {
        assert_eq!(parse_key("q").unwrap(), VIRTUAL_KEY(b'Q'.into()));
        assert_eq!(parse_key("F5").unwrap(), VK_F5);
        assert!(parse_key("f13").is_err());
    }

    #[test]
    fn rejects_unknown_arts_and_settings() {
        assert!("default_art = 1234".parse::<Config>().is_err());
        assert!("speed = 2".parse::<Config>().is_err());
        assert!("wheel_sensitivity = 0".parse::<Config>().is_err());
        assert!("hold_key".parse::<Config>().is_err());
    }
}
