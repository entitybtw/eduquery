//! Каталог GGUF-моделей: встроенный список, загрузка по URL, обслуживание файлов.

use anyhow::{anyhow, bail, Context, Result};
use futures_util::StreamExt;
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

/// Предопределённая модель из каталога.
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
    CatalogEntry {
        id: "gigachat3-lightning",
        name: "GigaChat 3 Lightning (Q6_K)",
        description: "Открытая русскоязычная модель Сбера GigaChat 3 Lightning (MoE, 10B параметров, активных ≈1.8B) от ai-sage, квантование Q6_K (~8.8 ГБ). Сильный русский язык, быстрая за счёт MoE. Нужно ≥10 ГБ свободной RAM (поднимите EDUQUERY_MEMORY_LIMIT до 12g).",
        url: "https://huggingface.co/ai-sage/GigaChat3-10B-A1.8B-GGUF/resolve/main/GigaChat3-10B-A1.8B-q6_k.gguf",
        size_approx: 8_782_240_064,
        params: "10B (A1.8B)",
        min_ram_mb: 10240,
        recommended: false,
    },
    CatalogEntry {
        id: "gigachat3.1-lightning",
        name: "GigaChat 3.1 Lightning (Q4_K_M)",
        description: "Обновлённая GigaChat 3.1 Lightning (MoE, 10B параметров, активных ≈1.8B) от ai-sage, компактное квантование Q4_K_M (~6.5 ГБ). Лучший вариант из GigaChat для сервера с 8–12 ГБ RAM (EDUQUERY_MEMORY_LIMIT не меньше 10g).",
        url: "https://huggingface.co/ai-sage/GigaChat3.1-10B-A1.8B-GGUF/resolve/main/GigaChat3.1-10B-A1.8B-q4_K_M.gguf",
        size_approx: 6_474_702_976,
        params: "10B (A1.8B)",
        min_ram_mb: 8192,
        recommended: false,
    },
];

/// Найти запись каталога по id.
pub fn catalog_entry(id: &str) -> Option<&'static CatalogEntry> {
    CATALOG.iter().find(|e| e.id == id)
}

/// Откуда скачивать модель: основной адрес Hugging Face, зеркало из
/// `EDUQUERY_HF_MIRROR` (если задано) и публичное зеркало hf-mirror.com.
/// Порядок: сначала настроенное зеркало, затем оригинал, затем запасное.
pub fn candidate_urls(entry: &CatalogEntry) -> Vec<String> {
    let mut urls: Vec<String> = Vec::new();
    let rest = entry.url.strip_prefix("https://huggingface.co/");
    if let Ok(mirror) = std::env::var("EDUQUERY_HF_MIRROR") {
        let mirror = mirror.trim().trim_end_matches('/');
        if !mirror.is_empty() {
            if let Some(rest) = rest {
                urls.push(format!("{mirror}/{rest}"));
            }
        }
    }
    urls.push(entry.url.to_string());
    if let Some(rest) = rest {
        let fallback = format!("https://hf-mirror.com/{rest}");
        if !urls.iter().any(|u| u == &fallback) {
            urls.push(fallback);
        }
    }
    urls.dedup();
    urls
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
    /// Текущее зеркало (для диагностики).
    pub url: Option<String>,
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

    /// Недокачанные модели (`.gguf.part`): их можно продолжить, не начиная заново.
    pub fn partials(&self) -> Vec<(String, u64)> {
        let mut out = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&self.dir) {
            for e in entries.flatten() {
                let p = e.path();
                let name = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
                if let Some(id) = name.strip_suffix(".gguf.part") {
                    let size = e.metadata().map(|m| m.len()).unwrap_or(0);
                    out.push((id.to_string(), size));
                }
            }
        }
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    /// Сверить файлы на диске и таблицу `models`:
    /// найденные GGUF регистрируются, записи без файла удаляются.
    pub fn reconcile(&self, db: &crate::db::Db) -> Result<()> {
        let conn = db.conn();
        for (id, size) in self.list_files() {
            let known: Option<String> = conn
                .query_row("SELECT id FROM models WHERE id = ?1", [&id], |r| r.get(0))
                .optional()?;
            if known.is_none() {
                let path = self.path_for(&id).to_string_lossy().to_string();
                conn.execute(
                    "INSERT INTO models(id, name, file_path, size_bytes, sha256, params_json, source)
                     VALUES (?1, ?1, ?2, ?3, '', '{}', 'local')",
                    rusqlite::params![id, path, size],
                )?;
            }
        }
        let rows: Vec<(String, String)> = {
            let mut stmt = conn.prepare("SELECT id, file_path FROM models")?;
            let it = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
            it.collect::<rusqlite::Result<Vec<_>>>()?
        };
        for (id, path) in rows {
            if !Path::new(&path).exists() {
                conn.execute("DELETE FROM models WHERE id = ?1", [&id])?;
            }
        }
        Ok(())
    }

    /// Удалить файл модели.
    pub fn delete(&self, id: &str) -> Result<()> {
        let p = self.path_for(id);
        if p.exists() {
            std::fs::remove_file(&p).with_context(|| format!("не удалось удалить {}", p.display()))?;
        }
        // Недокачанный файл тоже удаляем, иначе он останется в списке «Продолжить».
        let part = self.dir.join(format!("{id}.gguf.part"));
        if part.exists() {
            std::fs::remove_file(&part)
                .with_context(|| format!("не удалось удалить {}", part.display()))?;
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

    /// Скачать модель в файл `<id>.gguf`.
    ///
    /// `urls` — список зеркал в порядке приоритета. Недокачанный файл
    /// `.gguf.part` сохраняется между попытками и перезапусками: скачивание
    /// продолжается с места обрыва (HTTP Range), а не начинается заново.
    pub async fn download(
        &self,
        id: &str,
        name: &str,
        urls: &[String],
        expected_size: Option<u64>,
    ) -> Result<PathBuf> {
        let target = self.path_for(id);
        if target.exists() {
            self.update_download(id, |d| {
                d.status = "done".into();
                d.file = Some(target.to_string_lossy().to_string());
            });
            return Ok(target);
        }
        if urls.is_empty() {
            bail!("нет ссылки для скачивания модели");
        }

        let tmp = target.with_extension("gguf.part");
        {
            let mut list = self.downloads.lock().unwrap();
            if list.iter().any(|d| d.id == id && d.status == "running") {
                bail!("модель уже скачивается");
            }
            list.retain(|d| d.id != id);
            let partial = std::fs::metadata(&tmp).map(|m| m.len()).unwrap_or(0);
            list.push(DownloadState {
                id: id.to_string(),
                name: name.to_string(),
                received: partial,
                total: 0,
                status: "running".into(),
                error: None,
                file: None,
                url: urls.first().cloned(),
            });
        }

        let client = reqwest::Client::builder()
            .user_agent(concat!("eduquery/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(30))
            .timeout(Duration::from_secs(60 * 60))
            .build()
            .context("не удалось создать HTTP-клиент")?;

        let mut last_err: Option<anyhow::Error> = None;
        // Несколько кругов по зеркалам: сеть может моргнуть на любом из них.
        for round in 0..3u32 {
            for url in urls {
                self.update_download(id, |d| {
                    d.status = "running".into();
                    d.error = None;
                    d.url = Some(url.clone());
                });
                match self.download_one(&client, url, &tmp, id).await {
                    Ok(()) => {
                        // Проверяем GGUF-магию — иначе на диск попадёт HTML-страница.
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
                            let msg = "скачанный файл не является GGUF-моделью (возможно, ссылка устарела)";
                            self.update_download(id, |d| {
                                d.status = "error".into();
                                d.error = Some(msg.into());
                            });
                            bail!("{msg}");
                        }

                        let size = tokio::fs::metadata(&tmp).await.map(|m| m.len()).unwrap_or(0);
                        if let Some(expected) = expected_size.filter(|s| *s > 0) {
                            if size * 10 < expected * 9 {
                                let _ = tokio::fs::remove_file(&tmp).await;
                                let msg = format!(
                                    "файл слишком мал: {size} байт вместо ожидаемых ~{expected}; ссылка могла устареть"
                                );
                                self.update_download(id, |d| {
                                    d.status = "error".into();
                                    d.error = Some(msg.clone());
                                });
                                bail!("{msg}");
                            }
                        }

                        tokio::fs::rename(&tmp, &target)
                            .await
                            .with_context(|| format!("не удалось переместить {}", tmp.display()))?;
                        self.update_download(id, |d| {
                            d.status = "done".into();
                            d.file = Some(target.to_string_lossy().to_string());
                            d.received = size;
                            d.total = size;
                        });
                        tracing::info!("модель {id} скачана ({size} байт)");
                        return Ok(target);
                    }
                    Err(e) => {
                        tracing::warn!("не удалось скачать {id} с {url}: {e:#}");
                        let msg = format!("{e:#}");
                        last_err = Some(e);
                        self.update_download(id, |d| d.error = Some(msg));
                    }
                }
            }
            if round < 2 {
                let pause = Duration::from_secs(2 * (round as u64 + 1));
                self.update_download(id, |d| {
                    d.status = "running".into();
                    d.error = Some("обрыв связи, повторяю попытку…".into());
                });
                tokio::time::sleep(pause).await;
            }
        }

        let msg = last_err
            .map(|e| format!("{e:#}"))
            .unwrap_or_else(|| "не удалось скачать модель".into());
        self.update_download(id, |d| {
            d.status = "error".into();
            d.error = Some(msg.clone());
        });
        Err(anyhow!(msg).context("не удалось скачать модель"))
    }

    /// Одна попытка скачивания с докачкой. Не удаляет `.part` при обрыве.
    async fn download_one(
        &self,
        client: &reqwest::Client,
        url: &str,
        dest: &Path,
        id: &str,
    ) -> Result<()> {
        use tokio::io::AsyncWriteExt;

        let have = tokio::fs::metadata(dest).await.map(|m| m.len()).unwrap_or(0);
        let mut req = client.get(url);
        if have > 0 {
            req = req.header(reqwest::header::RANGE, format!("bytes={have}-"));
        }
        let resp = req
            .send()
            .await
            .with_context(|| format!("не удалось подключиться к {url}"))?;
        let status = resp.status();
        if status.as_u16() == 416 {
            return Ok(()); // файл уже скачан целиком
        }
        if !status.is_success() {
            bail!("{url}: HTTP {status}");
        }

        let resumed = status.as_u16() == 206;
        let total = if resumed {
            resp.headers()
                .get(reqwest::header::CONTENT_RANGE)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.rsplit('/').next())
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(0)
        } else {
            resp.content_length().unwrap_or(0)
        };
        let mut received = if resumed { have } else { 0 };
        let mut file = if resumed && have > 0 {
            tokio::fs::OpenOptions::new().append(true).open(dest).await?
        } else {
            tokio::fs::File::create(dest).await?
        };

        self.update_download(id, |d| {
            d.received = received;
            d.total = total;
        });

        let mut stream = resp.bytes_stream();
        loop {
            let next = tokio::time::timeout(Duration::from_secs(60), stream.next())
                .await
                .map_err(|_| anyhow!("таймаут: нет данных 60 секунд"))?;
            let Some(chunk) = next else { break };
            let chunk = chunk?;
            file.write_all(&chunk).await.context("ошибка записи файла модели")?;
            received += chunk.len() as u64;
            self.update_download(id, |d| d.received = received);
        }
        file.flush().await.context("ошибка сброса файла модели")?;
        drop(file);

        if total > 0 && received < total {
            bail!("соединение оборвалось: получено {received} из {total} байт");
        }
        Ok(())
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
