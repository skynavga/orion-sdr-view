use serde::Deserialize;
use super::Defaults;

#[derive(Debug, Deserialize)]
pub struct DisplayConfig {
    pub db_min:    Option<f32>,
    pub db_max:    Option<f32>,
    pub time_zone: Option<String>,
}

impl super::ViewConfig {
    pub fn db_min(&self) -> f32 {
        self.display.as_ref().and_then(|d| d.db_min).unwrap_or(Defaults::DB_MIN)
    }
    pub fn db_max(&self) -> f32 {
        self.display.as_ref().and_then(|d| d.db_max).unwrap_or(Defaults::DB_MAX)
    }
    /// Returns true if timestamps should be displayed in UTC (default).
    pub fn time_zone_utc(&self) -> bool {
        match self.display.as_ref().and_then(|d| d.time_zone.as_deref()) {
            Some(s) => s.to_ascii_lowercase() != "local",
            None    => true,
        }
    }
}
