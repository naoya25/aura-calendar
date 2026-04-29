use std::{
    env, fs, io,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppConfig {
    pub calendars: Vec<CalendarConfig>,
    pub display: DisplayConfig,
    pub refresh_interval_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CalendarConfig {
    pub name: String,
    pub ical_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DisplayConfig {
    pub normal_format: String,
    pub stealth_format: String,
    pub show_title: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            calendars: Vec::new(),
            display: DisplayConfig::default(),
            refresh_interval_seconds: 300,
        }
    }
}

impl Default for DisplayConfig {
    fn default() -> Self {
        Self {
            normal_format: "{minutes_until}分後 {title}".to_string(),
            stealth_format: "***".to_string(),
            show_title: true,
        }
    }
}

impl AppConfig {
    pub fn load_or_create() -> Result<Self, Box<dyn std::error::Error>> {
        Self::load_or_create_at(&config_path()?)
    }

    fn load_or_create_at(path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        if path.exists() {
            let raw_config = fs::read_to_string(path)?;
            let loaded = Self::from_json(&raw_config)?;
            if loaded.needs_normalization {
                loaded.config.save_to(path)?;
            }
            return Ok(loaded.config);
        }

        let config = Self::default();
        config.save_to(path)?;
        Ok(config)
    }

    fn from_json(raw_config: &str) -> Result<LoadedConfig, Box<dyn std::error::Error>> {
        let raw_config: RawAppConfig = serde_json::from_str(raw_config)?;
        Ok(raw_config.into())
    }

    fn save_to(&self, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let raw_config = serde_json::to_string_pretty(self)?;
        fs::write(path, format!("{raw_config}\n"))?;
        Ok(())
    }

    pub fn normal_title(&self) -> String {
        if self.calendars.is_empty() {
            "Aura: no calendar".to_string()
        } else {
            "Aura: calendar ready".to_string()
        }
    }

    pub fn stealth_title(&self) -> &str {
        &self.display.stealth_format
    }
}

#[derive(Debug, Clone, Deserialize)]
struct RawAppConfig {
    #[serde(default)]
    calendars: Vec<RawCalendarConfig>,
    #[serde(default)]
    display: DisplayConfig,
    #[serde(default = "default_refresh_interval_seconds")]
    refresh_interval_seconds: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum RawCalendarConfig {
    Legacy(String),
    Current(CalendarConfig),
}

#[derive(Debug, Clone)]
struct LoadedConfig {
    config: AppConfig,
    needs_normalization: bool,
}

impl From<RawAppConfig> for LoadedConfig {
    fn from(raw: RawAppConfig) -> Self {
        let mut needs_normalization = false;
        let calendars = raw
            .calendars
            .into_iter()
            .enumerate()
            .map(|(index, entry)| match entry {
                RawCalendarConfig::Legacy(ical_url) => {
                    needs_normalization = true;
                    CalendarConfig {
                        name: format!("Calendar {}", index + 1),
                        ical_url,
                    }
                }
                RawCalendarConfig::Current(calendar) => calendar,
            })
            .collect();

        Self {
            config: AppConfig {
                calendars,
                display: raw.display,
                refresh_interval_seconds: raw.refresh_interval_seconds,
            },
            needs_normalization,
        }
    }
}

fn default_refresh_interval_seconds() -> u64 {
    300
}

pub fn config_path() -> Result<PathBuf, io::Error> {
    let home = env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "HOME is not set"))?;

    Ok(config_path_for_home(&home))
}

fn config_path_for_home(home: &Path) -> PathBuf {
    home.join("Library")
        .join("Application Support")
        .join("AuraCalendar")
        .join("config.json")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn config_path_should_use_application_support_dir() {
        let path = config_path_for_home(Path::new("/Users/example"));

        assert_eq!(
            path,
            PathBuf::from("/Users/example/Library/Application Support/AuraCalendar/config.json")
        );
    }

    #[test]
    fn load_or_create_should_write_default_config_when_missing() {
        let path = temp_config_path();

        let config = AppConfig::load_or_create_at(&path).expect("config should be created");
        let raw_config = fs::read_to_string(&path).expect("config file should exist");

        assert_eq!(config, AppConfig::default());
        assert!(raw_config.contains("\"calendars\": []"));

        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn normal_title_should_reflect_calendar_state() {
        let mut config = AppConfig::default();

        assert_eq!(config.normal_title(), "Aura: no calendar");

        config.calendars.push(CalendarConfig {
            name: "Main".to_string(),
            ical_url: "https://example.com/calendar.ics".to_string(),
        });

        assert_eq!(config.normal_title(), "Aura: calendar ready");
    }

    fn temp_config_path() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after unix epoch")
            .as_nanos();

        env::temp_dir()
            .join(format!("aura-calendar-test-{nanos}"))
            .join("config.json")
    }
}
