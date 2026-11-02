//! Каталог GGUF-моделей: встроенный список, загрузка по URL, обслуживание файлов.

use anyhow::{bail, Context, Result};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::Mutex;

/// Прес��топределённая модель из каталога.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogEntry {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub url: &'static str,
    /// Примерный размер файла в байтах.
    pub size_approx: u64,
    pub params: &'static str,
    /// Минимальная память для комфортной работы.
    pub min_ram_mb: u32,
    pub recommended: bool,
}

/// Встроенный каталог: от совсем крошечных моделей до 1.5B.
pub const CATALOG: &[CatalogEntry] = &[
    CatalogEntry {
        id: "stories260k",
        name: "TinyStories 260K",
        description: "Минимальная модель (~1 МБ). Только для проверки связки: пишет короткие истории, на русском отвечает слабо.",
        url: "https://huggingface.co/ggml-org/models/resolve/main/tinyllamas/stories260K.gguf",
        size_approx: 1_185_376,
        params: "≈0.26M",
        min_ram_mb: 64,
        recommended: false,
    },
    CatalogEntry {
        id: "stories15m",
        name: "TinyStories 15M",
        description: "Крошечная обучаемая модель (~19 МБ). Быстрая, помещается в любой контейнер, но качество — игрушечное.",
        url: "https://huggingface.co/ggml-org/models/resolve/main/tinyllamas/stories15M-q4_0.gguf",
        size_approx: 19_077_344,
        params: "≈15M",
        min_ram_mb: 128,
        recommended: false,
    },
    CatalogEntry {
        id: "gemma3-270m",
        name: "Gemma 3 270M Instruct",
        description: "Маленькая instruct-модель (~250 МБ) с chat-шаблоном. Работает по-русски, отвечает кратко.",
        url: "https://huggingface.co/unsloth/gemma-3-270m-it-GGUF/resolve/main/gemma-3-270m-it-Q4_K_M.gguf",
        size_approx: 253_115_424,
        params: "≈270M",
        min_ram_mb: 512,
        recommended: true,
    },
    CatalogEntry {
        id: "qwen2.5-0.5b",
        name: "Qwen2.5 0.5B Instruct",
        description: "Компактная универсальная модель (~490 МБ). Хороший баланс скорости и качества, уверенно работает по-русски.",
        url: "https://huggingface.co/Qwen/Qwen2.5-0.5B-Instruct-GGUF/resolve/main/qwen2.5-0.5b-instruct-q4_k_m.gguf",
        size_approx: 491_400_032,
        params: "≈0.5B",
        min_ram_mb: 1024,
        recommended: true,
    },
    CatalogEntry {
        id: "llama3.2-1b",
        name: "Llama 3.2 1B Instruct",
        description: "Модель на 1 миллиард параметров (~800 МБ). Заметно качественнее мелких, подходит для развёрнутого анализа учеников.",
        url: "https://huggingface.co/unsloth/Llama-3.2-1B-Instruct-GGUF/resolve/main/Llama-3.2-1B-Instruct-Q4_K_M.gguf",
        size_approx: 807_694_368,
        params: "≈1.1B",
        min_ram_mb: 2048,
        recommended: true,
    },
    CatalogEntry {
        id: "qwen2.5-1.5b",
        name: "Qwen2.5 1.5B Instruct",
        description: "Самая качественная модель в каталоге (~990 МБ). Медленнее, но даёт связные развёрнутые ответы по-русски.",
        url: "https://huggingface.co/bartowski/Qwen2.5-1.5B-Instruct-GGUF/resolve/main/Qwen2.5-1.5B-Instruct-Q4_K_M.gguf",
        size_approx: 986_048_768,
        params: "≈1.5B",
        min_ram_mb: 3072,
        recommended: false,
    },
];

/// Найти запись каталога по id.
pub fn catalog_entry(id: &str) -> Option<&'static CatalogEntry> {
    CATALOG.iter().find(|e| e.id == id)
}

/// Зарегистрированная в БД модель.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelRecord {
    pub id: String,
    pub name: String,
    pub file_path: String,
    pub size_bytes: i64,
    pub sha256: String,
    pub params_json: String,
    pub source: String,
    pub created_at: String,
}

/// Менеджер файлов моделей: скачивание, проверка, удаление.
pub struct ModelStore {
    dir: PathBuf,
    /// Активные загрузки (id -> прогресс), чтобы не качать дважды.
    downloads: Arc<Mutex<Vec<DownloadState>>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DownloadState {
    pub id: String,
    pub name: String,
    pub received: u64,
    pub total: u64,
    pub status: String,
    pub error: Option<String>,
    pub file: Option<String>,
}

impl ModelStore {
    pub fn new(dir: PathBuf) -> Self {
        std::fs::create_dir_all(&dir).ok();
        Self {
            dir,
            downloads: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Путь файла модели по id (без проверки существования).
    pub fn path_for(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.gguf"))
    }

    /// Список скачанных файлов.
    pub fn list_files(&self) -> Vec<(String, u64)> {
        let mut out = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&self.dir) {
            for e in entries.flatten() {
                let p = e.path();
                if p.extension().and_then(|s| s.to_str()) == Some("gguf") {
                    let size = e.metadata().map(|m| m.len()).unwrap_or(0);
                    let name = p
                        .file_stem()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_default();
                    out.push((name, size));
                }
            }
        }
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    /// Удалить файл модели.
    pub fn delete(&self, id: &str) -> Result<()> {
        let p = self.path_for(id);
        if p.exists() {
            std::fs::remove_file(&p).with_context(|| format!("не удалось удалить {}", p.display()))?;
        }
        Ok(())
    }

    pub fn downloads(&self) -> Vec<DownloadState> {
        self.downloads.lock().unwrap().clone()
    }

    fn update_download(&self, id: &str, f: impl FnOnce(&mut DownloadState)) {
        let mut list = self.downloads.lock().unwrap();
        if let Some(d) = list.iter_mut().find(|d| d.id == id) {
            f(d);
        }
    }

    /// Скачать модель по URL в файл `<id>.gguf`.
    pub async fn download(&self, id: &str, name: &str, url: &str) -> Result<PathBuf> {
        let target = self.path_for(id);
        if target.exists() {
            return Ok(target);
        }

        let tmp = target.with_extension("gguf.part");
        {
            let mut list = self.downloads.lock().unwrap();
            if list.iter().any(|d| d.id == id && d.status == "running") {
                bail!("модель уже скачивается");
            }
            list.retain(|d| d.id != id);
            list.push(DownloadState {
                id: id.to_string(),
                name: name.to_string(),
                received: 0,
                total: 0,
                status: "running".into(),
                error: None,
                file: None,
            });
        }

        let client = reqwest::Client::builder()
            .user_agent("eduquery/0.1")
            .timeout(std::time::Duration::from_secs(60 * 60))
            .build()
            .context("не удалось создать HTTP-клиент")?;

        let resp = match client.get(url).send().await {
            Ok(r) => r,
            Err(e) => {
                self.update_download(id, |d| {
                    d.status = "error".into();
                    d.error = Some(format!("сетевая ошибка: {e}"));
                });
                return Err(e).context("не удалось скачать модель");
            }
        };

        if !resp.status().is_success() {
            let code = resp.status();
            self.update_download(id, |d| {
                d.status = "error".into();
                d.error = Some(format!("HTTP {code}"));
            });
            bail!("сервер вернул HTTP {code}");
        }

        let total = resp.content_length().unwrap_or(0);
        self.update_download(id, |d| d.total = total);

        let mut file = tokio::fs::File::create(&tmp)
            .await
            .with_context(|| format!("не удалось создать {}", tmp.display()))?;
        let mut stream = resp.bytes_stream();
        let mut received = 0u64;

        use tokio::io::AsyncWriteExt;
        while let Some(chunk) = stream.next().await {
            let chunk = match chunk {
                Ok(c) => c,
                Err(e) => {
                    self.update_download(id, |d| {
                        d.status = "error".into();
                        d.error = Some(e.to_string());
                    });
                    let _ = tokio::fs::remove_file(&tmp).await;
                    return Err(e).context("обрыв загрузки");
                }
            };
            file.write_all(&chunk).await.context("ошибка записи")?;
            received += chunk.len() as u64;
            self.update_download(id, |d| d.received = received);
        }
        file.flush().await.context("ошибка сброса")?;
        drop(file);

        // Проверяем GGUF-магию.
        let mut magic = [0u8; 4];
        {
            use tokio::io::AsyncReadExt;
            let mut f = tokio::fs::File::open(&tmp)
                .await
                .context("ошибка чтения скачанного файла")?;
            let _ = f.read_exact(&mut magic).await;
        }
        if &magic != b"GGUF" {
            let _ = tokio::fs::remove_file(&tmp).await;
            self.update_download(id, |d| {
                d.status = "error".into();
                d.error = Some("файл не GGUF".into());
            });
            bail!("скачанный файл не является GGUF-моделью");
        }

        tokio::fs::rename(&tmp, &target)
            .await
            .with_context(|| format!("не удалось переместить {}", tmp.display()))?;

        self.update_download(id, |d| {
            d.status = "done".into();
            d.file = Some(target.to_string_lossy().to_string());
        });

        Ok(target)
    }

    /// sha256 файла.
    pub fn sha256(path: &Path) -> Result<String> {
        use sha2::{Digest, Sha256};
        use std::io::Read;
        let mut f = std::fs::File::open(path)
            .with_context(|| format!("не удалось прочитать {}", path.display()))?;
        let mut h = Sha256::new();
        let mut buf = vec![0u8; 1 << 20];
        loop {
            let n = f.read(&mut buf)?;
            if n == 0 {
                break;
            }
            h.update(&buf[..n]);
        }
        Ok(format!("{:x}", h.finalize()))
    }
}
