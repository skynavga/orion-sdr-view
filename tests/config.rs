//! Exercises the three-tier config loading logic without launching the GUI.

use std::io::Write;
use tempfile::NamedTempFile;

use orion_sdr_view::config::{ViewConfig, Defaults};

fn defaults_all_match(cfg: &ViewConfig) {
    assert_eq!(cfg.db_min(),        Defaults::DB_MIN,        "db_min");
    assert_eq!(cfg.db_max(),        Defaults::DB_MAX,        "db_max");
    assert_eq!(cfg.freq_hz(),       Defaults::FREQ_HZ,       "freq_hz");
    assert_eq!(cfg.noise_amp(),     Defaults::NOISE_AMP,     "noise_amp");
    assert_eq!(cfg.amp_max(),       Defaults::AMP_MAX,       "amp_max");
    assert_eq!(cfg.ramp_secs(),     Defaults::RAMP_SECS,     "ramp_secs");
    assert_eq!(cfg.pause_secs(),    Defaults::PAUSE_SECS,    "pause_secs");
    assert_eq!(cfg.carrier_hz(),    Defaults::CARRIER_HZ,    "carrier_hz");
    assert_eq!(cfg.mod_index(),     Defaults::MOD_INDEX,     "mod_index");
    assert_eq!(cfg.loop_gap_secs(), Defaults::LOOP_GAP_SECS, "loop_gap_secs");
    assert_eq!(cfg.am_noise_amp(),  Defaults::AM_NOISE_AMP,  "am_noise_amp");
    assert_eq!(cfg.am_msg_repeat(), 1,                       "am_msg_repeat");
    assert_eq!(cfg.psk31_mode(),    "BPSK31",                "psk31_mode");
    assert_eq!(cfg.psk31_carrier_hz(), Defaults::CARRIER_HZ, "psk31_carrier_hz");
    assert_eq!(cfg.psk31_noise_amp(),  Defaults::AM_NOISE_AMP, "psk31_noise_amp");
    assert_eq!(cfg.psk31_message(), "CQ CQ CQ DE N0GNR",    "psk31_message");
    assert_eq!(cfg.psk31_msg_repeat(), orion_sdr_view::source::psk31::PSK31_DEFAULT_REPEAT, "psk31_msg_repeat");
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
      noise_amp:  0.10
      amp_max:    0.80
      ramp_secs:  2.0
      pause_secs: 5.0
    am_dsb:
      carrier_hz:    15000.0
      mod_index:     0.5
      loop_gap_secs: 3.0
      noise_amp:     0.02
"#;
    let mut f = NamedTempFile::new().unwrap();
    f.write_all(yaml.as_bytes()).unwrap();

    let cfg = ViewConfig::load(Some(f.path().to_path_buf()));
    assert_eq!(cfg.db_min(),        -100.0);
    assert_eq!(cfg.db_max(),         -10.0);
    assert_eq!(cfg.freq_hz(),       5000.0);
    assert_eq!(cfg.noise_amp(),       0.10);
    assert_eq!(cfg.amp_max(),         0.80);
    assert_eq!(cfg.ramp_secs(),       2.0);
    assert_eq!(cfg.pause_secs(),      5.0);
    assert_eq!(cfg.carrier_hz(),  15000.0);
    assert_eq!(cfg.mod_index(),       0.5);
    assert_eq!(cfg.loop_gap_secs(),   3.0);
    assert_eq!(cfg.am_noise_amp(),   0.02);
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
    assert_eq!(cfg.db_max(),        Defaults::DB_MAX);
    assert_eq!(cfg.freq_hz(),       Defaults::FREQ_HZ);
    assert_eq!(cfg.carrier_hz(),    Defaults::CARRIER_HZ);
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
        assert_eq!(cfg.db_min(), Defaults::DB_MIN, "CWD config: db_min should be default");
    }

    // 6b: invalid .orionsdr.yaml → soft-warn, fall back to defaults
    {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".orionsdr.yaml"), b"{ this is not: [valid yaml").unwrap();
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
      loop_gap_secs: 5.0
      noise_amp: 0.10
      message: "TEST MSG"
      custom_message: "CUSTOM MSG"
      msg_repeat: 7
"#;
    let mut f = NamedTempFile::new().unwrap();
    f.write_all(yaml.as_bytes()).unwrap();

    let cfg = ViewConfig::load(Some(f.path().to_path_buf()));
    assert_eq!(cfg.psk31_mode(), "QPSK31");
    assert_eq!(cfg.psk31_carrier_hz(), 1500.0);
    assert_eq!(cfg.psk31_loop_gap_secs(), 5.0);
    assert_eq!(cfg.psk31_noise_amp(), 0.10);
    assert_eq!(cfg.psk31_message(), "TEST MSG");
    assert_eq!(cfg.psk31_custom_message(), "CUSTOM MSG");
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
    assert_eq!(cfg.psk31_message(), "CQ CQ CQ DE N0GNR");
    assert_eq!(cfg.psk31_msg_repeat(), orion_sdr_view::source::psk31::PSK31_DEFAULT_REPEAT);
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
      loop_gap_secs: 30.0
      noise_amp: 0.03
      call_to: W1AW
      call_de: K0KE
      grid: DN70
      free_text: 73 DE K0KE
      msg_repeat: 4
"#;
    let mut f = NamedTempFile::new().unwrap();
    f.write_all(yaml.as_bytes()).unwrap();

    let cfg = ViewConfig::load(Some(f.path().to_path_buf()));
    assert_eq!(cfg.ft8_mode(), "FT4");
    assert_eq!(cfg.ft8_carrier_hz(), 1200.0);
    assert_eq!(cfg.ft8_loop_gap_secs(), 30.0);
    assert_eq!(cfg.ft8_noise_amp(), 0.03);
    assert_eq!(cfg.ft8_call_to(), "W1AW");
    assert_eq!(cfg.ft8_call_de(), "K0KE");
    assert_eq!(cfg.ft8_grid(), "DN70");
    assert_eq!(cfg.ft8_free_text(), "73 DE K0KE");
    assert_eq!(cfg.ft8_msg_repeat(), 4);
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
    assert_eq!(cfg.ft8_loop_gap_secs(), orion_sdr_view::source::ft8::FT8_DEFAULT_LOOP_GAP_SECS);
    assert_eq!(cfg.ft8_noise_amp(), Defaults::AM_NOISE_AMP);
    assert_eq!(cfg.ft8_call_to(), orion_sdr_view::source::ft8::FT8_DEFAULT_CALL_TO);
    assert_eq!(cfg.ft8_call_de(), orion_sdr_view::source::ft8::FT8_DEFAULT_CALL_DE);
    assert_eq!(cfg.ft8_grid(), orion_sdr_view::source::ft8::FT8_DEFAULT_GRID);
    assert_eq!(cfg.ft8_free_text(), orion_sdr_view::source::ft8::FT8_DEFAULT_FREE_TEXT);
    assert_eq!(cfg.ft8_msg_repeat(), orion_sdr_view::source::ft8::FT8_DEFAULT_REPEAT);
}

// ── FT8 config: no ft8 section → all defaults ────────────────────────────────

#[test]
fn ft8_config_defaults_when_absent() {
    let yaml = "view:\n  display:\n    db_min: -80.0\n";
    let mut f = NamedTempFile::new().unwrap();
    f.write_all(yaml.as_bytes()).unwrap();

    let cfg = ViewConfig::load(Some(f.path().to_path_buf()));
    assert_eq!(cfg.ft8_mode(), "FT8");
    assert_eq!(cfg.ft8_carrier_hz(), orion_sdr_view::source::ft8::FT8_DEFAULT_CARRIER_HZ);
    assert_eq!(cfg.ft8_loop_gap_secs(), orion_sdr_view::source::ft8::FT8_DEFAULT_LOOP_GAP_SECS);
    assert_eq!(cfg.ft8_call_to(), orion_sdr_view::source::ft8::FT8_DEFAULT_CALL_TO);
    assert_eq!(cfg.ft8_call_de(), orion_sdr_view::source::ft8::FT8_DEFAULT_CALL_DE);
    assert_eq!(cfg.ft8_grid(), orion_sdr_view::source::ft8::FT8_DEFAULT_GRID);
    assert_eq!(cfg.ft8_msg_repeat(), orion_sdr_view::source::ft8::FT8_DEFAULT_REPEAT);
}

// ── FT8 config: msg_repeat = 0 clamps to 1 ───────────────────────────────────

#[test]
fn ft8_msg_repeat_zero_clamps_to_one() {
    let yaml = r#"
view:
  sources:
    ft8:
      msg_repeat: 0
"#;
    let mut f = NamedTempFile::new().unwrap();
    f.write_all(yaml.as_bytes()).unwrap();

    let cfg = ViewConfig::load(Some(f.path().to_path_buf()));
    assert_eq!(cfg.ft8_msg_repeat(), 1);
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
