// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Exercises the three-tier config loading logic without launching the GUI.

use std::io::Write;
use tempfile::NamedTempFile;

use orion_sdr_view::config::{Defaults, TzMode, ViewConfig};
use orion_sdr_view::source;

fn defaults_all_match(cfg: &ViewConfig) {
    assert_eq!(cfg.db_min(), Defaults::DB_MIN, "db_min");
    assert_eq!(cfg.db_max(), Defaults::DB_MAX, "db_max");
    assert_eq!(
        cfg.spec_time_range_secs(),
        Defaults::SPEC_TIME_RANGE_SECS,
        "spec_time_range_secs"
    );
    assert_eq!(cfg.zoom(), Defaults::ZOOM, "zoom");
    assert_eq!(cfg.freq_hz(), Defaults::FREQ_HZ, "freq_hz");
    assert_eq!(cfg.cn_db(), source::tone::TONE_DEFAULT_CN_DB, "cn_db");
    assert_eq!(cfg.amp_max(), Defaults::AMP_MAX, "amp_max");
    assert_eq!(cfg.ramp_secs(), Defaults::RAMP_SECS, "ramp_secs");
    assert_eq!(cfg.pause_secs(), Defaults::PAUSE_SECS, "pause_secs");
    assert_eq!(cfg.carrier_hz(), Defaults::CARRIER_HZ, "carrier_hz");
    assert_eq!(cfg.mod_index(), Defaults::MOD_INDEX, "mod_index");
    assert_eq!(cfg.am_gap_secs(), Defaults::AM_GAP_SECS, "am_gap_secs");
    assert_eq!(cfg.am_cn_db(), source::amdsb::AM_DEFAULT_CN_DB, "am_cn_db");
    assert_eq!(cfg.am_msg_repeat(), 1, "am_msg_repeat");
    assert_eq!(cfg.psk31_mode(), "BPSK31", "psk31_mode");
    assert_eq!(
        cfg.psk31_carrier_hz(),
        Defaults::CARRIER_HZ,
        "psk31_carrier_hz"
    );
    assert_eq!(
        cfg.psk31_cn_db(),
        source::psk31::PSK31_DEFAULT_CN_DB,
        "psk31_cn_db"
    );
    assert_eq!(
        cfg.psk31_canned_text(),
        "CQ CQ CQ DE N0GNR",
        "psk31_canned_text"
    );
    assert_eq!(
        cfg.psk31_msg_repeat(),
        orion_sdr_view::source::psk31::PSK31_DEFAULT_REPEAT,
        "psk31_msg_repeat"
    );
}

// ── Scenario 1: explicit --config with full YAML ─────────────────────────────

#[test]
fn explicit_config_full() {
    let yaml = r#"
view:
  display:
    db_min: -100.0
    db_max: -10.0
  sources:
    test_tone:
      freq_hz:    5000.0
      cn_db:      30.0
      amp_max:    0.80
      ramp_secs:  2.0
      pause_secs: 5.0
    am_dsb:
      carrier_hz: 15000.0
      mod_index:  0.5
      gap_secs:   3.0
      cn_db:      25.0
"#;
    let mut f = NamedTempFile::new().unwrap();
    f.write_all(yaml.as_bytes()).unwrap();

    let cfg = ViewConfig::load(Some(f.path().to_path_buf()));
    assert_eq!(cfg.db_min(), -100.0);
    assert_eq!(cfg.db_max(), -10.0);
    assert_eq!(cfg.freq_hz(), 5000.0);
    assert_eq!(cfg.cn_db(), 30.0);
    assert_eq!(cfg.amp_max(), 0.80);
    assert_eq!(cfg.ramp_secs(), 2.0);
    assert_eq!(cfg.pause_secs(), 5.0);
    assert_eq!(cfg.carrier_hz(), 15000.0);
    assert_eq!(cfg.mod_index(), 0.5);
    assert_eq!(cfg.am_gap_secs(), 3.0);
    assert_eq!(cfg.am_cn_db(), 25.0);
}

// ── Spectrogram display fields: explicit override + partial defaults ─────────

#[test]
fn spectrogram_display_full() {
    let yaml = r#"
view:
  display:
    spec_time_range_secs: 15.0
"#;
    let mut f = NamedTempFile::new().unwrap();
    f.write_all(yaml.as_bytes()).unwrap();

    let cfg = ViewConfig::load(Some(f.path().to_path_buf()));
    assert_eq!(cfg.spec_time_range_secs(), 15.0);
}

#[test]
fn spectrogram_display_defaults_when_absent() {
    let yaml = "view:\n  display:\n    db_min: -80.0\n";
    let mut f = NamedTempFile::new().unwrap();
    f.write_all(yaml.as_bytes()).unwrap();
    let cfg = ViewConfig::load(Some(f.path().to_path_buf()));
    assert_eq!(cfg.spec_time_range_secs(), Defaults::SPEC_TIME_RANGE_SECS);
}

#[test]
fn an_unrecognised_key_is_ignored_rather_than_refused() {
    // The schema's standing policy, and the one thing `ViewConfig`'s doc comment
    // claims that nothing else checks: every field is `Option<T>` and nothing
    // sets `deny_unknown_fields`, so a stale or misspelled key loads silently
    // and takes no effect.
    //
    // The trade is deliberate — blanket `deny_unknown_fields` would turn every
    // typo into a hard startup failure — but it means a *renamed* key needs its
    // own scaffolding to be noticed, as the 0.0.23 impairment change had for two
    // releases.  Anything that flips this policy has to delete this test to do
    // it, which is the point.
    let yaml = r#"
view:
  display:
    db_min: -80.0
    nonsense_key: 12
  sources:
    cofdm:
      cn_db: 30.0
      no_such_key: 0.5
"#;
    let mut f = NamedTempFile::new().unwrap();
    f.write_all(yaml.as_bytes()).unwrap();

    // Via the real loader, which hard-fails on a parse error for an explicit
    // `--config`: reaching the assertions at all is half the point.
    let cfg = ViewConfig::load(Some(f.path().to_path_buf()));
    assert_eq!(cfg.db_min(), -80.0, "the surviving keys must take effect");
    assert_eq!(cfg.cofdm_cn_db(), 30.0);
}

#[test]
fn pan_direction_config() {
    // Default (absent) → "spectrum" (false).
    let cfg = ViewConfig::load(None);
    assert!(!cfg.pan_signal_follows());

    // Explicit "signal" → true.
    let yaml = "view:\n  display:\n    pan: signal\n";
    let mut f = NamedTempFile::new().unwrap();
    f.write_all(yaml.as_bytes()).unwrap();
    assert!(ViewConfig::load(Some(f.path().to_path_buf())).pan_signal_follows());

    // Explicit "spectrum" and unrecognized both → false.
    for v in ["spectrum", "bogus"] {
        let yaml = format!("view:\n  display:\n    pan: {v}\n");
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(yaml.as_bytes()).unwrap();
        assert!(!ViewConfig::load(Some(f.path().to_path_buf())).pan_signal_follows());
    }
}

// ── Scenario 3: explicit --config with partial YAML → overrides + defaults ────

#[test]
fn explicit_config_partial() {
    let yaml = "view:\n  display:\n    db_min: -120.0\n";
    let mut f = NamedTempFile::new().unwrap();
    f.write_all(yaml.as_bytes()).unwrap();

    let cfg = ViewConfig::load(Some(f.path().to_path_buf()));
    assert_eq!(cfg.db_min(), -120.0);
    // everything else falls back to defaults
    assert_eq!(cfg.db_max(), Defaults::DB_MAX);
    assert_eq!(cfg.freq_hz(), Defaults::FREQ_HZ);
    assert_eq!(cfg.carrier_hz(), Defaults::CARRIER_HZ);
}

// ── Scenario 4: explicit --config missing file → exit(1) ──────────────────────
// (Can't test process::exit in-process; verified manually via CLI)

// ── Scenario 5: explicit --config invalid YAML → exit(1) ─────────────────────
// (Same — tested manually)

// ── Scenario 6: CWD .orionsdr.yaml present and valid ─────────────────────────
// ── Scenario 7: CWD .orionsdr.yaml invalid YAML → soft-warn, use defaults ────
//
// CWD tests mutate the process working directory, so they must run serially.
// We combine them under one test guarded by a static mutex.

#[test]
fn cwd_config_scenarios() {
    use std::sync::Mutex;
    static CWD_LOCK: Mutex<()> = Mutex::new(());
    let _guard = CWD_LOCK.lock().unwrap();

    let orig = std::env::current_dir().unwrap();

    // 6a: valid .orionsdr.yaml
    {
        let dir = tempfile::tempdir().unwrap();
        let yaml = "view:\n  display:\n    db_max: -5.0\n";
        std::fs::write(dir.path().join(".orionsdr.yaml"), yaml).unwrap();
        std::env::set_current_dir(&dir).unwrap();

        let cfg = ViewConfig::load(None);
        std::env::set_current_dir(&orig).unwrap();

        assert_eq!(cfg.db_max(), -5.0, "CWD config: db_max should be -5.0");
        assert_eq!(
            cfg.db_min(),
            Defaults::DB_MIN,
            "CWD config: db_min should be default"
        );
    }

    // 6b: invalid .orionsdr.yaml → soft-warn, fall back to defaults
    {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(".orionsdr.yaml"),
            b"{ this is not: [valid yaml",
        )
        .unwrap();
        std::env::set_current_dir(&dir).unwrap();

        let cfg = ViewConfig::load(None);
        std::env::set_current_dir(&orig).unwrap();

        defaults_all_match(&cfg);
    }

    // 6c: no .orionsdr.yaml in CWD → all defaults
    {
        let dir = tempfile::tempdir().unwrap();
        std::env::set_current_dir(&dir).unwrap();

        let cfg = ViewConfig::load(None);
        std::env::set_current_dir(&orig).unwrap();

        defaults_all_match(&cfg);
    }
}

// ── PSK31 config: full YAML with all PSK31 fields ────────────────────────────

#[test]
fn psk31_config_full() {
    let yaml = r#"
view:
  sources:
    psk31:
      mode: QPSK31
      carrier_hz: 1500.0
      gap_secs: 5.0
      cn_db: 40.0
      canned_text: "TEST MSG"
      custom_text: "CUSTOM MSG"
      msg_repeat: 7
"#;
    let mut f = NamedTempFile::new().unwrap();
    f.write_all(yaml.as_bytes()).unwrap();

    let cfg = ViewConfig::load(Some(f.path().to_path_buf()));
    assert_eq!(cfg.psk31_mode(), "QPSK31");
    assert_eq!(cfg.psk31_carrier_hz(), 1500.0);
    assert_eq!(cfg.psk31_gap_secs(), 5.0);
    assert_eq!(cfg.psk31_cn_db(), 40.0);
    assert_eq!(cfg.psk31_canned_text(), "TEST MSG");
    assert_eq!(cfg.psk31_custom_text(), "CUSTOM MSG");
    assert_eq!(cfg.psk31_msg_repeat(), 7);
}

// ── PSK31 config: partial YAML → defaults for missing fields ─────────────────

#[test]
fn psk31_config_partial() {
    let yaml = r#"
view:
  sources:
    psk31:
      mode: QPSK31
"#;
    let mut f = NamedTempFile::new().unwrap();
    f.write_all(yaml.as_bytes()).unwrap();

    let cfg = ViewConfig::load(Some(f.path().to_path_buf()));
    assert_eq!(cfg.psk31_mode(), "QPSK31");
    // Everything else falls back to defaults
    assert_eq!(cfg.psk31_carrier_hz(), Defaults::CARRIER_HZ);
    assert_eq!(cfg.psk31_canned_text(), "CQ CQ CQ DE N0GNR");
    assert_eq!(
        cfg.psk31_msg_repeat(),
        orion_sdr_view::source::psk31::PSK31_DEFAULT_REPEAT
    );
}

// ── AM DSB config: msg_repeat field ──────────────────────────────────────────

#[test]
fn am_dsb_msg_repeat() {
    let yaml = r#"
view:
  sources:
    am_dsb:
      msg_repeat: 5
"#;
    let mut f = NamedTempFile::new().unwrap();
    f.write_all(yaml.as_bytes()).unwrap();

    let cfg = ViewConfig::load(Some(f.path().to_path_buf()));
    assert_eq!(cfg.am_msg_repeat(), 5);
}

#[test]
fn am_dsb_msg_repeat_zero_clamps_to_one() {
    let yaml = r#"
view:
  sources:
    am_dsb:
      msg_repeat: 0
"#;
    let mut f = NamedTempFile::new().unwrap();
    f.write_all(yaml.as_bytes()).unwrap();

    let cfg = ViewConfig::load(Some(f.path().to_path_buf()));
    assert_eq!(cfg.am_msg_repeat(), 1);
}

// ── Scenario 8: YAML with unknown top-level keys → silently ignored ───────────

#[test]
fn unknown_keys_ignored() {
    let yaml = r#"
view:
  display:
    db_min: -90.0
  future_key: ignored_value
library:
  some_setting: 42
"#;
    let mut f = NamedTempFile::new().unwrap();
    f.write_all(yaml.as_bytes()).unwrap();

    let cfg = ViewConfig::load(Some(f.path().to_path_buf()));
    assert_eq!(cfg.db_min(), -90.0);
    assert_eq!(cfg.db_max(), Defaults::DB_MAX);
}

// ── FT8 config: full YAML with all FT8 fields ────────────────────────────────

#[test]
fn ft8_config_full() {
    let yaml = r#"
view:
  sources:
    ft8:
      mode: FT4
      carrier_hz: 1200.0
      gap_secs: 30.0
      cn_db: 48.0
      call_to: W1AW
      call_de: K0KE
      grid: DN70
      free_text: 73 DE K0KE
"#;
    let mut f = NamedTempFile::new().unwrap();
    f.write_all(yaml.as_bytes()).unwrap();

    let cfg = ViewConfig::load(Some(f.path().to_path_buf()));
    assert_eq!(cfg.ft8_mode(), "FT4");
    assert_eq!(cfg.ft8_carrier_hz(), 1200.0);
    assert_eq!(cfg.ft8_gap_secs(), 30.0);
    assert_eq!(cfg.ft8_cn_db(), 48.0);
    assert_eq!(cfg.ft8_call_to(), "W1AW");
    assert_eq!(cfg.ft8_call_de(), "K0KE");
    assert_eq!(cfg.ft8_grid(), "DN70");
    assert_eq!(cfg.ft8_free_text(), "73 DE K0KE");
}

// ── FT8 config: partial YAML → defaults for missing fields ───────────────────

#[test]
fn ft8_config_partial() {
    let yaml = r#"
view:
  sources:
    ft8:
      carrier_hz: 900.0
"#;
    let mut f = NamedTempFile::new().unwrap();
    f.write_all(yaml.as_bytes()).unwrap();

    let cfg = ViewConfig::load(Some(f.path().to_path_buf()));
    assert_eq!(cfg.ft8_carrier_hz(), 900.0);
    // Everything else falls back to defaults.
    assert_eq!(cfg.ft8_mode(), "FT8");
    assert_eq!(
        cfg.ft8_gap_secs(),
        orion_sdr_view::source::ft8::FT8_DEFAULT_GAP_SECS
    );
    assert_eq!(cfg.ft8_cn_db(), source::ft8::FT8_DEFAULT_CN_DB);
    assert_eq!(
        cfg.ft8_call_to(),
        orion_sdr_view::source::ft8::FT8_DEFAULT_CALL_TO
    );
    assert_eq!(
        cfg.ft8_call_de(),
        orion_sdr_view::source::ft8::FT8_DEFAULT_CALL_DE
    );
    assert_eq!(
        cfg.ft8_grid(),
        orion_sdr_view::source::ft8::FT8_DEFAULT_GRID
    );
    assert_eq!(
        cfg.ft8_free_text(),
        orion_sdr_view::source::ft8::FT8_DEFAULT_FREE_TEXT
    );
}

// ── FT8 config: no ft8 section → all defaults ────────────────────────────────

#[test]
fn ft8_config_defaults_when_absent() {
    let yaml = "view:\n  display:\n    db_min: -80.0\n";
    let mut f = NamedTempFile::new().unwrap();
    f.write_all(yaml.as_bytes()).unwrap();

    let cfg = ViewConfig::load(Some(f.path().to_path_buf()));
    assert_eq!(cfg.ft8_mode(), "FT8");
    assert_eq!(
        cfg.ft8_carrier_hz(),
        orion_sdr_view::source::ft8::FT8_DEFAULT_CARRIER_HZ
    );
    assert_eq!(
        cfg.ft8_gap_secs(),
        orion_sdr_view::source::ft8::FT8_DEFAULT_GAP_SECS
    );
    assert_eq!(
        cfg.ft8_call_to(),
        orion_sdr_view::source::ft8::FT8_DEFAULT_CALL_TO
    );
    assert_eq!(
        cfg.ft8_call_de(),
        orion_sdr_view::source::ft8::FT8_DEFAULT_CALL_DE
    );
    assert_eq!(
        cfg.ft8_grid(),
        orion_sdr_view::source::ft8::FT8_DEFAULT_GRID
    );
}

// ── Scenario 9: YAML with missing `view:` key → all defaults ─────────────────

#[test]
fn missing_view_key_uses_defaults() {
    let yaml = "# no view key here\nlibrary:\n  x: 1\n";
    let mut f = NamedTempFile::new().unwrap();
    f.write_all(yaml.as_bytes()).unwrap();

    let cfg = ViewConfig::load(Some(f.path().to_path_buf()));
    defaults_all_match(&cfg);
}

// ── time_zone parsing ────────────────────────────────────────────────────────

fn tz_cfg(yaml_value: &str) -> ViewConfig {
    let yaml = format!("view:\n  display:\n    time_zone: {yaml_value}\n");
    let mut f = NamedTempFile::new().unwrap();
    f.write_all(yaml.as_bytes()).unwrap();
    ViewConfig::load(Some(f.path().to_path_buf()))
}

#[test]
fn time_zone_missing_is_utc() {
    let yaml = "view:\n  display:\n    db_min: -90.0\n";
    let mut f = NamedTempFile::new().unwrap();
    f.write_all(yaml.as_bytes()).unwrap();
    let cfg = ViewConfig::load(Some(f.path().to_path_buf()));
    assert_eq!(cfg.time_zone_offset_min(), 0);
}

#[test]
fn time_zone_utc_keyword() {
    assert_eq!(tz_cfg("utc").time_zone_offset_min(), 0);
    assert_eq!(tz_cfg("UTC").time_zone_offset_min(), 0);
}

#[test]
fn time_zone_explicit_positive() {
    assert_eq!(tz_cfg("\"+00:00\"").time_zone_offset_min(), 0);
    assert_eq!(tz_cfg("\"+05:30\"").time_zone_offset_min(), 330);
    assert_eq!(tz_cfg("\"+14:00\"").time_zone_offset_min(), 840);
    assert_eq!(tz_cfg("\"+12:45\"").time_zone_offset_min(), 765);
}

#[test]
fn time_zone_explicit_negative() {
    assert_eq!(tz_cfg("\"-00:00\"").time_zone_offset_min(), 0);
    assert_eq!(tz_cfg("\"-08:00\"").time_zone_offset_min(), -480);
    assert_eq!(tz_cfg("\"-12:00\"").time_zone_offset_min(), -720);
    assert_eq!(tz_cfg("\"-03:30\"").time_zone_offset_min(), -210);
}

#[test]
fn time_zone_out_of_range_falls_back_to_utc() {
    // Outside -12..+14 range, parser returns None and we fall back to UTC.
    assert_eq!(tz_cfg("\"+15:00\"").time_zone_offset_min(), 0);
    assert_eq!(tz_cfg("\"-13:00\"").time_zone_offset_min(), 0);
    assert_eq!(tz_cfg("\"+05:99\"").time_zone_offset_min(), 0);
    assert_eq!(tz_cfg("garbage").time_zone_offset_min(), 0);
}

#[test]
fn time_zone_local_is_in_display_range() {
    // "local" resolves at query time — we can't pin the value, but it must be
    // inside the supported display range.
    let v = tz_cfg("local").time_zone_offset_min();
    assert!(
        (-12 * 60..=14 * 60).contains(&v),
        "local offset {v} min outside display range"
    );
}

#[test]
fn time_zone_mode_parses_all_variants() {
    // Missing field → Utc.
    let yaml = "view:\n  display:\n    db_min: -90.0\n";
    let mut f = NamedTempFile::new().unwrap();
    f.write_all(yaml.as_bytes()).unwrap();
    let cfg = ViewConfig::load(Some(f.path().to_path_buf()));
    assert_eq!(cfg.time_zone_mode(), TzMode::Utc);

    // Literal keywords.
    assert_eq!(tz_cfg("utc").time_zone_mode(), TzMode::Utc);
    assert_eq!(tz_cfg("UTC").time_zone_mode(), TzMode::Utc);
    assert_eq!(tz_cfg("local").time_zone_mode(), TzMode::Local);

    // Explicit offsets.
    assert_eq!(tz_cfg("\"+05:30\"").time_zone_mode(), TzMode::Explicit(330));
    assert_eq!(
        tz_cfg("\"-08:00\"").time_zone_mode(),
        TzMode::Explicit(-480)
    );
    assert_eq!(tz_cfg("\"+14:00\"").time_zone_mode(), TzMode::Explicit(840));

    // Garbage falls back to Utc.
    assert_eq!(tz_cfg("garbage").time_zone_mode(), TzMode::Utc);
    assert_eq!(tz_cfg("\"+15:00\"").time_zone_mode(), TzMode::Utc);
}

// ── COFDM spectral shaping ────────────────────────────────────────────────

use orion_sdr_view::source::{CofdmBwFraction, CofdmMask, CofdmTaper, cofdm_edge_guard_for};

/// Load a config from a `sources: cofdm:` block body (already indented).
fn cofdm_cfg(body: &str) -> ViewConfig {
    let yaml = format!("view:\n  sources:\n    cofdm:\n{body}");
    let mut f = NamedTempFile::new().unwrap();
    f.write_all(yaml.as_bytes()).unwrap();
    ViewConfig::load(Some(f.path().to_path_buf()))
}

#[test]
fn cofdm_shaping_defaults_when_absent() {
    // A `cofdm:` block that mentions none of the shaping keys still gets the
    // enabled defaults — shaping is on out of the box.
    let cfg = cofdm_cfg("      bandwidth: 1/2\n");
    assert_eq!(cfg.cofdm_bw_fraction(), CofdmBwFraction::OneHalf);
    assert!(cfg.cofdm_shaping_enabled());
    assert_eq!(cfg.cofdm_taper(), CofdmTaper::Quarter);
    assert_eq!(cfg.cofdm_mask(), CofdmMask::Db60);
    assert!(!cfg.cofdm_include_dc());
    // No `edge_guard` key means "derive it from the bandwidth fraction".
    assert_eq!(cfg.cofdm_edge_guard(), None);
}

#[test]
fn cofdm_shaping_keys_parse() {
    let cfg = cofdm_cfg(concat!(
        "      bandwidth:  7/8\n",
        "      shaping:    false\n",
        "      edge_guard: 90\n",
        "      include_dc: true\n",
        "      taper:      3/8\n",
        "      mask:       80\n",
    ));
    assert_eq!(cfg.cofdm_bw_fraction(), CofdmBwFraction::SevenEighths);
    assert!(!cfg.cofdm_shaping_enabled());
    assert_eq!(cfg.cofdm_edge_guard(), Some(90));
    assert!(cfg.cofdm_include_dc());
    assert_eq!(cfg.cofdm_taper(), CofdmTaper::ThreeEighths);
    assert_eq!(cfg.cofdm_mask(), CofdmMask::Db80);
}

#[test]
fn cofdm_mask_accepts_bare_and_labelled_depths() {
    // YAML authors write `mask: 60`; the settings row's label is "60 dB".
    assert_eq!(cofdm_cfg("      mask: 40\n").cofdm_mask(), CofdmMask::Db40);
    assert_eq!(
        cofdm_cfg("      mask: \"80 dB\"\n").cofdm_mask(),
        CofdmMask::Db80
    );
    assert_eq!(cofdm_cfg("      mask: off\n").cofdm_mask(), CofdmMask::Off);
    // Unparseable values fall back to the default rather than failing the load.
    assert_eq!(
        cofdm_cfg("      mask: \"120 dB\"\n").cofdm_mask(),
        CofdmMask::Db60
    );
    assert_eq!(
        cofdm_cfg("      taper: 5/8\n").cofdm_taper(),
        CofdmTaper::Quarter
    );
}

#[test]
fn cofdm_edge_guard_derives_from_every_fraction() {
    for &fr in CofdmBwFraction::ALL {
        let cfg = cofdm_cfg(&format!("      bandwidth: {}\n", fr.label()));
        assert_eq!(cfg.cofdm_bw_fraction(), fr);
        assert_eq!(cfg.cofdm_edge_guard(), None, "{}", fr.label());
        // The derived guard is what the settings row seeds from.
        assert!(cofdm_edge_guard_for(fr) > 0);
    }
}

// ── COFDM tuning: band centre and sample rate ─────────────────────────────

use orion_sdr_view::source::{
    COFDM_DEFAULT_FS, COFDM_MAX_FS, COFDM_MIN_FS, cofdm_center_bounds, cofdm_default_center_hz,
};

#[test]
fn cofdm_tuning_defaults_when_absent() {
    let cfg = cofdm_cfg("      bandwidth: 1/4\n");
    assert_eq!(cfg.cofdm_fs_hz(), COFDM_DEFAULT_FS);
    assert_eq!(
        cfg.cofdm_center_hz(),
        cofdm_default_center_hz(COFDM_DEFAULT_FS)
    );
}

#[test]
fn cofdm_center_and_rate_parse() {
    let cfg = cofdm_cfg(concat!(
        "      center_hz: 300000\n",
        "      fs_hz:     1920000\n",
    ));
    assert_eq!(cfg.cofdm_fs_hz(), 1_920_000.0);
    assert_eq!(cfg.cofdm_center_hz(), 300_000.0);
}

#[test]
fn a_configured_rate_re_derives_the_default_centre() {
    // The centre defaults to Nyquist/2, so configuring only the rate must still
    // put the band mid-display rather than leaving it at the old rate's `fs/4` —
    // which at a lower rate would be outside the band entirely.
    let cfg = cofdm_cfg("      fs_hz: 480000\n");
    assert_eq!(cfg.cofdm_fs_hz(), 480_000.0);
    assert_eq!(cfg.cofdm_center_hz(), 120_000.0);
}

#[test]
fn an_out_of_range_rate_or_centre_is_clamped() {
    // Both arrive from a hand-edited YAML file, and every derived frequency is
    // proportional to the rate — so a typo like `fs_hz: 1920` (meant as MHz)
    // must land somewhere usable rather than collapsing the band.
    assert_eq!(cofdm_cfg("      fs_hz: 1920\n").cofdm_fs_hz(), COFDM_MIN_FS);
    assert_eq!(
        cofdm_cfg("      fs_hz: 1.0e12\n").cofdm_fs_hz(),
        COFDM_MAX_FS
    );

    let (lo, hi) = cofdm_center_bounds(COFDM_DEFAULT_FS);
    assert_eq!(cofdm_cfg("      center_hz: 0\n").cofdm_center_hz(), lo);
    assert_eq!(
        cofdm_cfg("      center_hz: 5000000\n").cofdm_center_hz(),
        hi
    );
    // A centre is clamped against the *configured* rate, not the default one.
    let cfg = cofdm_cfg(concat!(
        "      fs_hz:     480000\n",
        "      center_hz: 480000\n",
    ));
    assert_eq!(cfg.cofdm_center_hz(), cofdm_center_bounds(480_000.0).1);
}

// ── Display: viewport zoom ────────────────────────────────────────────────

#[test]
fn zoom_defaults_and_parses() {
    assert_eq!(display_cfg("").zoom(), Defaults::ZOOM);
    assert_eq!(display_cfg("    zoom: 4.0\n").zoom(), 4.0);
    // Floored at full span: a ratio below 1.0 would mean a window wider than
    // the band.  The *upper* bound is per-source (`nyquist / MIN_SPAN_HZ`) so
    // the viewport applies it, not the config.
    assert_eq!(display_cfg("    zoom: 0.25\n").zoom(), 1.0);
    assert_eq!(display_cfg("    zoom: -8\n").zoom(), 1.0);
    assert_eq!(display_cfg("    zoom: 960\n").zoom(), 960.0);
}

/// Load a config from a `display:` block body (already indented).
fn display_cfg(body: &str) -> ViewConfig {
    let yaml = format!("view:\n  display:\n    db_max: -15\n{body}");
    let mut f = NamedTempFile::new().unwrap();
    f.write_all(yaml.as_bytes()).unwrap();
    ViewConfig::load(Some(f.path().to_path_buf()))
}
