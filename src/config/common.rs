// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

use serde::Deserialize;

use super::capture::CaptureConfig;
use super::display::DisplayConfig;
use crate::source::amdsb::AmDsbConfig;
use crate::source::cofdm::CofdmConfig;
use crate::source::cw::CwConfig;
use crate::source::dvbt::DvbTConfig;
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
    pub dvbt: Option<DvbTConfig>,
}

/// The `view:` block of a config file.
///
/// **Unknown keys are ignored.**  Every field here is `Option<T>` and nothing
/// sets `serde(deny_unknown_fields)`, so a typo — or a key from an older
/// release — loads silently and takes no effect.  That is a deliberate
/// trade: blanket `deny_unknown_fields` would turn every unrelated typo into a
/// hard startup failure, which is a bigger behaviour change than the schema is
/// worth while it is still pre-alpha and unversioned.
///
/// The cost is real, though, and worth remembering the next time a key is
/// renamed: the 0.0.23 impairment change carried a dedicated rejection field
/// for two releases precisely so it could not be absorbed in silence.  A rename
/// that matters needs the same scaffolding again — the schema will not do it.
#[derive(Debug, Deserialize)]
pub struct ViewConfig {
    pub display: Option<DisplayConfig>,
    pub sources: Option<SourcesConfig>,
    pub capture: Option<CaptureConfig>,
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
        match serde_yaml::from_str::<ConfigFile>(&content) {
            Ok(cf) => cf.view.unwrap_or_else(Self::empty),
            Err(e) => {
                eprintln!("orion-sdr-view: error parsing config {:?}: {}", path, e);
                if hard_fail {
                    std::process::exit(1);
                }
                Self::empty()
            }
        }
    }

    /// A config with no keys set, so every accessor returns its built-in
    /// default.  Public because it is what a test wants as a baseline:
    /// [`load`](Self::load) with no explicit path falls back to `.orionsdr.yaml`
    /// in the working directory, which would make a test's result depend on
    /// where it was run from.
    pub fn empty() -> Self {
        ViewConfig {
            display: None,
            sources: None,
            capture: None,
        }
    }
}
