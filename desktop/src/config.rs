
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub server: ServerConfig,
    pub playback: PlaybackConfig,
    pub queue: QueueConfig,
    pub commands: CommandConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PlaybackConfig {
    pub max_video_height: u32,
    pub volume: f64,
    pub media_dir: String,
    pub ffmpeg_path: String,
    pub mpv_path: String,
    pub yt_dlp_path: String,
    pub radio_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct QueueConfig {
    pub limit: usize,
    pub per_user_limit: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CommandConfig {
    pub prefix: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            server: ServerConfig::default(),
            playback: PlaybackConfig::default(),
            queue: QueueConfig::default(),
            commands: CommandConfig::default(),
        }
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".into(),
            port: 8765,
        }
    }
}

impl Default for PlaybackConfig {
    fn default() -> Self {
        Self {
            max_video_height: 240,
            volume: 0.4,
            media_dir: String::new(),
            ffmpeg_path: String::new(),
            mpv_path: "backend/vendor/mpv/mpv.exe".into(),
            yt_dlp_path: "yt-dlp".into(),
            radio_url: String::new(),
        }
    }
}

impl Default for QueueConfig {
    fn default() -> Self {
        Self {
            limit: 10,
            per_user_limit: 2,
        }
    }
}

impl Default for CommandConfig {
    fn default() -> Self {
        Self {
            prefix: "!".into(),
        }
    }
}

impl AppConfig {
    pub fn load_or_create() -> Result<Self, Box<dyn std::error::Error>> {
        let path = Self::config_path()?;

        if !path.exists() {
            let config = Self::default();
            config.save()?;

            println!("[Config] Created {}", path.display());

            return Ok(config);
        }

        let contents = fs::read_to_string(&path)?;
        let mut config: Self = toml::from_str(&contents)?;

        let bundled_mpv_exists = path
            .parent()
            .is_some_and(|directory| {
                directory
                    .join("backend")
                    .join("vendor")
                    .join("mpv")
                    .join("mpv.exe")
                    .is_file()
            });

        if bundled_mpv_exists
            && [
            "resources/mpv/mpv.exe",
            "vendor/mpv/mpv.exe",
        ]
        .iter()
        .any(|legacy_path| config.playback.mpv_path.eq_ignore_ascii_case(legacy_path))
        {
            config.playback.mpv_path = "backend/vendor/mpv/mpv.exe".into();
            config.save()?;
        }

        config.validate()?;

        println!("[Config] Loaded {}", path.display());

        Ok(config)
    }

    pub fn save(&self) -> Result<(), Box<dyn std::error::Error>> {
        self.validate()?;

        let path = Self::config_path()?;
        let contents = toml::to_string_pretty(self)?;

        fs::write(&path, contents)?;

        Ok(())
    }

    pub fn config_path() -> Result<PathBuf, io::Error> {
        let executable = std::env::current_exe()?;

        let directory = executable.parent().ok_or_else(|| {
            io::Error::other("Could not determine executable directory")
        })?;

        Ok(directory.join("config.toml"))
    }

    pub fn api_base_url(&self) -> String {
        format!("http://{}:{}", self.server.host, self.server.port)
    }

    fn validate(&self) -> Result<(), Box<dyn std::error::Error>> {
        if self.server.host.trim().is_empty() {
            return Err("Server host cannot be empty.".into());
        }

        if self.server.port == 0 {
            return Err("Server port must be between 1 and 65535.".into());
        }

        if self.playback.max_video_height == 0 {
            return Err("Maximum video height must be positive.".into());
        }

        if !self.playback.volume.is_finite()
            || !(0.0..=1.0).contains(&self.playback.volume)
        {
            return Err("Playback volume must be between 0 and 1.".into());
        }

        if self.queue.limit == 0 {
            return Err("Queue limit must be positive.".into());
        }

        if self.queue.per_user_limit == 0 {
            return Err("Per-user queue limit must be positive.".into());
        }

        if self.queue.per_user_limit > self.queue.limit {
            return Err(
                "Per-user queue limit cannot exceed the total queue limit.".into(),
            );
        }

        if self.commands.prefix.trim().is_empty() {
            return Err("Command prefix cannot be empty.".into());
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::AppConfig;

    #[test]
    fn packaged_config_uses_the_bundled_mpv_path() {
        let config: AppConfig = toml::from_str(include_str!("../config.toml"))
            .expect("packaged config.toml should be valid");

        assert_eq!(config.playback.mpv_path, "backend/vendor/mpv/mpv.exe");
        assert_eq!(config.playback.volume, 0.4);
        config
            .validate()
            .expect("packaged defaults should be valid");
    }

    #[test]
    fn missing_playback_volume_uses_40_percent_default() {
        let config: AppConfig = toml::from_str(
            "[playback]\nmpv_path = \"backend/vendor/mpv/mpv.exe\"\n",
        )
        .expect("legacy config.toml should use playback defaults");

        assert_eq!(config.playback.volume, 0.4);
    }
}
