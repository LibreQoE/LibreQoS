//! `[power]` — the host CPU power posture (`docs/power_profile_work_order.md`
//! §5). The SINGLE SOURCE for the profile; there is deliberately **no turbo
//! knob** (turbo is forced off for every profile).

use allocative::Allocative;
use serde::{Deserialize, Serialize};

/// The host CPU power profile. `Balanced` is the default and is deliberately
/// minimal ("turbo off, change nothing else"); `Eco` caps frequency and biases
/// for power; `Performance` favors jitter determinism. **Every** profile forces
/// turbo off.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize, Allocative)]
#[serde(rename_all = "snake_case")]
pub enum PowerProfile {
    /// Frequency-capped, power-biased idle.
    Eco,
    /// Turbo off only (the default).
    #[default]
    Balanced,
    /// Deterministic frequency + shallow C-states.
    Performance,
}

impl PowerProfile {
    /// The stable config-file spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Eco => "eco",
            Self::Balanced => "balanced",
            Self::Performance => "performance",
        }
    }

    /// Every profile, in display order.
    pub const ALL: [PowerProfile; 3] = [Self::Eco, Self::Balanced, Self::Performance];
}

impl std::fmt::Display for PowerProfile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for PowerProfile {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "eco" => Ok(Self::Eco),
            "balanced" => Ok(Self::Balanced),
            "performance" => Ok(Self::Performance),
            other => Err(format!(
                "unknown power profile '{other}' (expected one of: eco, balanced, performance)"
            )),
        }
    }
}

/// The `[power]` section.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Allocative)]
#[serde(default)]
pub struct PowerConfig {
    /// The host CPU power profile (there is no turbo knob).
    pub profile: PowerProfile,
    /// `eco`'s frequency cap as a percent of the turbo-off maximum (default
    /// **65**). Operator-set so the cap is not a hidden magic constant
    /// (`docs/power_profile_work_order.md` §2). Clamped to `[10, 100]` at use.
    pub eco_max_freq_pct: u64,
}

impl Default for PowerConfig {
    fn default() -> Self {
        Self {
            profile: PowerProfile::Balanced,
            eco_max_freq_pct: 65,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_balanced() {
        let parsed: PowerConfig = toml::from_str("").expect("empty section is defaulted");
        assert_eq!(parsed.profile, PowerProfile::Balanced);
        assert_eq!(parsed.eco_max_freq_pct, 65);
    }

    #[test]
    fn eco_cap_is_operator_settable() {
        let parsed: PowerConfig =
            toml::from_str("profile = \"eco\"\neco_max_freq_pct = 55").expect("parses");
        assert_eq!(parsed.profile, PowerProfile::Eco);
        assert_eq!(parsed.eco_max_freq_pct, 55);
    }

    #[test]
    fn parses_the_profile_snake_case() {
        let parsed: PowerConfig =
            toml::from_str("profile = \"performance\"").expect("parses");
        assert_eq!(parsed.profile, PowerProfile::Performance);
    }

    #[test]
    fn unknown_profile_is_a_parse_error() {
        // No turbo knob exists: a `turbo` key is not a profile.
        assert!(toml::from_str::<PowerConfig>("profile = \"turbo\"").is_err());
    }
}
