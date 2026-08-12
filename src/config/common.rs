// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

use serde::Deserialize;

use super::display::DisplayConfig;
use crate::source::amdsb::AmDsbConfig;
use crate::source::cofdm::CofdmConfig;
use crate::source::cw::CwConfig;
use crate::source::ft8::Ft8Config;
use crate::source::psk31::Psk31Config;
use crate::source::tone::TestToneConfig;

#[derive(Debug, Deserialize)]
pub struct SourcesConfig {
    pub test_tone: Option<TestToneConfig>,
    pub cw: Option<CwConfig>,
    pub am_dsb: Option<AmDsbConfig>,
    pub psk31: Option<Psk31Config>,
    pub ft8: Option<Ft8Config>,
    pub cofdm: Option<CofdmConfig>,
}

#[derive(Debug, Deserialize)]
pub struct ViewConfig {
    pub display: Option<DisplayConfig>,
    pub sources: Option<SourcesConfig>,
}

// Private top-level wrapper matching `view:` key
#[derive(Debug, Deserialize)]
struct ConfigFile {
    pub view: Option<ViewConfig>,
}

impl ViewConfig {
    /// Three-tier resolver:
    /// 1. `--config <path>` (hard-fail on error)
    /// 2. `.orionsdr.yaml` in CWD (soft-warn on error, skip if absent)
    /// 3. Built-in defaults (returns empty ViewConfig)
    pub fn load(explicit_path: Option<std::path::PathBuf>) -> Self {
        if let Some(p) = explicit_path {
            return Self::from_path(&p, true);
        }
        let cwd = std::path::PathBuf::from(".orionsdr.yaml");
        if cwd.exists() {
            return Self::from_path(&cwd, false);
        }
        Self::empty()
    }

    fn from_path(path: &std::path::Path, hard_fail: bool) -> Self {
        let content = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("orion-sdr-view: error reading config {:?}: {}", path, e);
                if hard_fail {
                    std::process::exit(1);
                }
                return Self::empty();
            }
        };
        let cfg = match serde_yaml::from_str::<ConfigFile>(&content) {
            Ok(cf) => cf.view.unwrap_or_else(Self::empty),
            Err(e) => {
                eprintln!("orion-sdr-view: error parsing config {:?}: {}", path, e);
                if hard_fail {
                    std::process::exit(1);
                }
                return Self::empty();
            }
        };
        let retired = cfg.retired_key_errors();
        if !retired.is_empty() {
            for msg in &retired {
                eprintln!("orion-sdr-view: config {:?}: {}", path, msg);
            }
            if hard_fail {
                std::process::exit(1);
            }
            return Self::empty();
        }
        cfg
    }

    /// Diagnostics for keys a breaking schema change retired, one per
    /// occurrence.  Empty for a config that carries none.
    ///
    /// **This exists because the schema will not reject them for us.**  Every
    /// field is `Option<T>` and nothing sets `serde(deny_unknown_fields)`, so a
    /// stale key is simply ignored — the user gets a config that appears to
    /// load while the setting they wrote is quietly discarded, which is worse
    /// than either converting it or refusing it.  Blanket
    /// `deny_unknown_fields` was rejected as the alternative: it would turn
    /// every unrelated typo into a hard error in the same commit, a bigger
    /// behaviour change than the one being made.
    ///
    /// Retire the fields themselves a release or two after 0.0.23.
    pub fn retired_key_errors(&self) -> Vec<String> {
        let Some(sources) = self.sources.as_ref() else {
            return Vec::new();
        };
        let present: [(&str, bool); 6] = [
            (
                "test_tone",
                sources
                    .test_tone
                    .as_ref()
                    .is_some_and(|c| c.noise_amp.is_some()),
            ),
            (
                "cw",
                sources.cw.as_ref().is_some_and(|c| c.noise_amp.is_some()),
            ),
            (
                "am_dsb",
                sources
                    .am_dsb
                    .as_ref()
                    .is_some_and(|c| c.noise_amp.is_some()),
            ),
            (
                "psk31",
                sources
                    .psk31
                    .as_ref()
                    .is_some_and(|c| c.noise_amp.is_some()),
            ),
            (
                "ft8",
                sources.ft8.as_ref().is_some_and(|c| c.noise_amp.is_some()),
            ),
            (
                "cofdm",
                sources
                    .cofdm
                    .as_ref()
                    .is_some_and(|c| c.noise_amp.is_some()),
            ),
        ];
        present
            .iter()
            .filter(|(_, found)| *found)
            .map(|(source, _)| {
                format!(
                    "sources.{source}.noise_amp was replaced by cn_db in 0.0.23. \
                     The impairment is now a carrier-to-noise ratio in dB, not an \
                     absolute amplitude; there is no automatic conversion. Remove \
                     noise_amp and set cn_db instead."
                )
            })
            .collect()
    }

    fn empty() -> Self {
        ViewConfig {
            display: None,
            sources: None,
        }
    }
}
