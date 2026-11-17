# Разработка

## Требования

- Rust ≥ 1.80 и Cargo.
- Node.js не обязателен: интерфейс — обычные HTML/CSS/JS без сборщика.
- Docker и Docker Compose — для образа.
- Для ИИ: интернет до GitHub (движок) и Hugging Face / зеркала (модели) либо заранее
  подготовленные файлы.

## Локальный запуск

```bash
cargo check
cargo build --release

EDUQUERY_DATA_DIR=./data \
EDUQUERY_ADMIN_USER=admin \
EDUQUERY_ADMIN_PASS=change-me \
./target/release/eduquery
```

Панель: `http://localhost:8080`. Для отладки без пароля:

```bash
EDUQUERY_ALLOW_ANONYMOUS=true EDUQUERY_DATA_DIR=./data ./target/debug/eduquery
```

Временный каталог для проверок:

```bash
EDUQUERY_DATA_DIR=/tmp/eq-check EDUQUERY_ALLOW_ANONYMOUS=true \
EDUQUERY_HOST=127.0.0.1 EDUQUERY_PORT=18080 ./target/debug/eduquery
```

## Переменные окружения

| Переменная | По умолчанию | Назначение |
|---|---|---|
| `EDUQUERY_HOST` | `0.0.0.0` | адрес прослушивания |
| `EDUQUERY_PORT` | `8080` | порт |
| `EDUQUERY_DATA_DIR` | `./data` | БД, модели, движок |
| `EDUQUERY_ADMIN_USER` | `admin` | первый администратор |
| `EDUQUERY_ADMIN_PASS` | `admin` | пароль первого администратора — менять |
| `EDUQUERY_ALLOW_ANONYMOUS` | `false` | вход без пароля, только для отладки |
| `EDUQUERY_N_THREADS` | `0` | потоки CPU, 0 — авто |
| `EDUQUERY_N_CTX` | `8192` | размер контекста |
| `EDUQUERY_ALLOW_REMOTE_DOWNLOAD` | `true` | разрешить скачивание движка и моделей |
| `EDUQUERY_ENGINE_VARIANT` | пусто | `vulkan` для GPU |
| `EDUQUERY_ENGINE_URL` | — | прямая ссылка на архив llama.cpp |
| `EDUQUERY_LLAMA_SERVER` | — | свой бинарник `llama-server` |
| `EDUQUERY_HF_MIRROR` | — | зеркало Hugging Face для каталога |
| `EDUQUERY_MEMORY_LIMIT` | `8g` | лимит памяти compose |
| `EDUQUERY_MTU` | `1500` | MTU docker-сети |
| `RUST_LOG` | `eduquery=info,warn` | логирование |

## Проверка перед коммитом

```bash
cargo check                       # без предупреждений
cargo build --release
node --check web/assets/app.js
node --check web/assets/stats.js
```

Затем вручную:

1. Запустить с временным `EDUQUERY_DATA_DIR`.
2. Войти, нажать «Сгенерировать демо-данные».
3. Проверить таблицу учеников, карточку, профиль (дата рождения, адрес, семья).
4. Скачать модель (например, `qwen2.5-0.5b`), активировать, сделать разбор.
5. Сгенерировать характеристику с примером в настройках и без него.
6. Запустить бенчмарк на двух моделях, дождаться, открыть ответы, поставить оценку, скачать CSV.
7. Проверить импорт/экспорт CSV.
8. Убедиться, что после перезапуска активная модель восстановилась.

## Docker

```bash
docker build -t <реестр>/eduquery:latest .
docker compose up -d --build
docker push <реестр>/eduquery:latest
```

Образ двухступенчатый: сборка на `rust:1-bookworm`, запуск на `debian:bookworm-slim`.
llama.cpp и модели в образ не входят — они скачиваются в `/data`.

При сборке Dockerfile сначала собирает пустую заглушку для кэша зависимостей, затем сбрасывает
fingerprint пакета и проверяет размер итогового бинарника. Без этого Docker может сохранить
stub-бинарник: `COPY` приносит старые mtime, и Cargo считает сборку свежей.

`docker-compose.yml` монтирует `./data` и задаёт лимит памяти. Для GigaChat Lightning поднимите
`EDUQUERY_MEMORY_LIMIT` до `10g`/`12g`. При MTU хоста меньше 1500 задайте `EDUQUERY_MTU`.

## Соглашения кода

### Rust

- Ошибки — `anyhow::Result`; в HTTP-обработчиках — `json_err(...)`.
- Новый маршрут добавляется в `router()` и в `required_perm()`.
- Доступ к БД — через `state.db.conn()`; SQL — параметризованный (`rusqlite::params!`).
- Миграции — только аддитивные, в `Db::add_columns`; не пересоздавайте таблицы на живой БД.
- Для фоновой работы — `tokio::spawn`, прогресс хранить в состоянии (как `BenchState`,
  `DownloadState`, `InstallState`).
- Любая генерация должна вызывать `record_usage` / `record_usage_error`.

### Frontend

- Классические скрипты, глобальные функции; без модулей и сборки.
- Новый экран: секция `#p-<tab>` в `index.html`, добавление в `TABS`, `TAB_PERM` и `tab()`.
- Все пользовательские строки экранировать через `esc()`.
- Вывод ИИ — `setOutput(el, text, loop_detected)` или `mdToHtml` внутри карточки.
- Для длинных списков использовать существующие классы `.panel`, `.list-row`, `.cards`, `.kv`.
- Кнопки, требующие права, помечать `data-perm="..."`; `applyPerms()` скроет их автоматически.

### Промпты

- Системная инструкция — только в `system_prompt` (или `DEFAULT_SYSTEM_PROMPT`).
- Пример — только в `example_input` / `example_answer`.
- Новые факты в контексте ученика добавлять в `render_context` и в `build_profile_json`.
- Характеристика — отдельный `build_characteristic_prompt`; не смешивать с разбором.

## Как добавить поле ученика

1. Колонка в `students` (`db.rs`) + миграция в `add_columns`.
2. Поле в `StudentSummary`, `SUMMARY_SELECT`, `map_summary` (`students.rs`).
3. Алиасы в `ImportRow`, обновление в `upsert_student`.
4. Поле в `ProfileUpdate` и `update_profile`.
5. `render_context` и `build_profile_json` (`analysis.rs`).
6. CSV-экспорт (`admin_api.rs`), шаблон импорта.
7. Таблица/карточка/модалка профиля в `web/`.
8. Документация: `DATA.md`, при необходимости `ANALYSIS.md`.

## Как добавить модель в каталог

1. Запись в `CATALOG` (`models.rs`): `id`, `name`, `description`, `url`, `size_approx`, `params`,
   `min_ram_mb`, `recommended`.
2. Проверить прямую ссылку и размер.
3. Убедиться, что это instruct-модель с chat template.
4. Обновить таблицу в `docs/MODELS.md` и `README.md`.

## Как добавить endpoint

1. Обработчик в `server.rs` или отдельном модуле.
2. `.route(...)` в `router()`.
3. Правило в `required_perm()`.
4. Запись в `docs/API.md`.
5. Проверка `curl` и, если есть UI, кнопка/экран.

## Отладка

- Логи: `RUST_LOG=eduquery=debug`.
- `GET /health` — жив ли процесс.
- `GET /api/v1/engine/status` — установка движка.
- `GET /api/v1/models` — файлы, `.part`, активная модель, загрузки.
- `GET /api/v1/benchmarks/status` — идёт ли сравнение.
- Если каталог пуст: проверить вход, сеть, `localStorage` и `GET /api/v1/models/catalog`.
- Если после рестарта нет модели: проверить `active_model_id` в `settings` и файлы в `models/`.

## Git

- Одна тема — один коммит.
- Не коммитить `data/`, `models/`, `*.gguf`, `.env` и локальную очередь работ.
- Перед push: `git status`, просмотр диффа, отсутствие секретов и персональных данных.
