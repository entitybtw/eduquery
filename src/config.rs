//! Конфигурация приложения через переменные окружения.

use anyhow::{Context, Result};
use std::path::PathBuf;

/// Конфигурация, собранная из ENV.
#[derive(Debug, Clone)]
pub struct Config {
    pub host: String,
    pub port: u16,
    /// Директория с данными (SQLite, модели, загруженные файлы).
    pub data_dir: PathBuf,
    pub admin_user: String,
    pub admin_pass: String,
    /// Разрешить вход без пароля (только для локальной разработки).
    pub allow_anonymous: bool,
    /// Количество потоков CPU для инференса.
    pub n_threads: i32,
    /// Размер контекста модели в токенах.
    pub n_ctx: u32,
    /// Разрешить загрузку моделей из внешней сети.
    pub allow_remote_download: bool,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let data_dir = PathBuf::from(env_or("EDUQUERY_DATA_DIR", "./data"));
        std::fs::create_dir_all(&data_dir)
            .with_context(|| format!("не удалось создать каталог данных {}", data_dir.display()))?;
        std::fs::create_dir_all(data_dir.join("models")).ok();

        Ok(Self {
            host: env_or("EDUQUERY_HOST", "0.0.0.0"),
            port: env_or("EDUQUERY_PORT", "8080").parse().unwrap_or(8080),
            data_dir,
            admin_user: env_or("EDUQUERY_ADMIN_USER", "admin"),
            admin_pass: env_or("EDUQUERY_ADMIN_PASS", "admin"),
            allow_anonymous: env_bool("EDUQUERY_ALLOW_ANONYMOUS", false),
            n_threads: env_or("EDUQUERY_N_THREADS", "0")
                .parse()
                .unwrap_or(0)
                .max(0),
            n_ctx: env_or("EDUQUERY_N_CTX", "4096").parse().unwrap_or(4096),
            allow_remote_download: env_bool("EDUQUERY_ALLOW_REMOTE_DOWNLOAD", true),
        })
    }

    pub fn db_path(&self) -> PathBuf {
        self.data_dir.join("eduquery.db")
    }

    pub fn models_dir(&self) -> PathBuf {
        self.data_dir.join("models")
    }

    /// Число потоков по умолчанию (авто, если 0).
    pub fn threads(&self) -> i32 {
        if self.n_threads > 0 {
            self.n_threads
        } else {
            std::thread::available_parallelism()
                .map(|n| n.get() as i32)
                .unwrap_or(4)
        }
    }
}

fn env_or(key: &str, default: &str) -> String {
    match std::env::var(key) {
        Ok(v) if !v.trim().is_empty() => v,
        _ => default.to_string(),
    }
}

fn env_bool(key: &str, default: bool) -> bool {
    match std::env::var(key) {
        Ok(v) => matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"),
        Err(_) => default,
    }
}
