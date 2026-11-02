# eduquery

Локальная LLM-платформа для анализа учебной деятельности учеников.
Веб-панель + SQLite + языковая модель, которая работает **у вас на сервере**:
данные учеников никуда не уходят.

- Образ крошечный: движок llama.cpp и модели **скачиваются сами** при первом запуске.
- Один контейнер, без внешних сервисов и API-ключей.
- Интерфейс на русском, логин по паролю, демо-данные для знакомства.

## Быстрый старт

```bash
git clone https://git.entitybtw.ru/entitybtw/eduquery.git
cd eduquery
./install.sh                 # создаст .env со случайным паролем админа
docker compose up -d
```

Откройте `http://localhost:8080` (порт — `WEB_PORT` в `.env`), войдите логином и
паролем, которые вывел `install.sh`.

Без клонирования репозитория (готовый образ с Docker Hub):

```bash
docker run -d --name eduquery -p 8080:8080 -v $PWD/data:/data \
  -e EDUQUERY_ADMIN_PASS=придумайте-пароль entbtw/eduquery:latest
```

## Первый запуск: что произойдёт

1. Создаётся БД `data/eduquery.db` и заполняется **демо-учениками** (если пусто).
2. В фоне скачивается движок **llama.cpp** (`llama-server`, ~25 МБ) в `data/engine/`.
3. В разделе **«Модели»** вы выбираете модель из каталога и жмёте «Скачать», затем «Активировать».
4. Первый запрос к модели загружает её в память (секунды–минуты, дальше быстро).

Нужен доступ в интернет к `github.com` (движок) и `huggingface.co` (модели).
Всё скачанное хранится в `./data` и переживает пересборку/обновление контейнера.

## Как это устроено

```
 браузер ──HTTP──► eduquery (Rust, axum) ──HTTP localhost──► llama-server (llama.cpp)
                       │                                          │
                  SQLite (data/)                           GGUF-модель (data/models/)
```

- `eduquery` — единственный процесс, который вы запускаете. Он сам поднимает
  `llama-server` как дочерний процесс на `127.0.0.1` со случайным портом и
  перезапускает его при смене модели или размера контекста.
- Генерация идёт потоком; если слабая модель зацикливается (повторяет одну и ту
  же строку), ответ прерывается и помечается `loop_detected`.
- Слишком длинный промпт не падает с ошибкой: llama-server работает в рамках
  заданного `n_ctx`, а длинные отчёты стоит сокращать/увеличивать контекст.

## Возможности

| Раздел | Что делает |
|---|---|
| Обзор | Сводка по классам/группам, динамика активности |
| Ученики | Список, карточка, контекст (оценки, посещаемость, активности) |
| Анализ ученика | LLM-разбор по данным ученика, результат сохраняется в историю |
| Сравнение | Сравнение нескольких учеников |
| Вопрос по данным | Свободный вопрос к LLM по выбранным данным |
| Импорт | Загрузка CSV (в т. ч. cp1251) |
| Модели | Каталог, скачивание по ссылке, активация, удаление |
| Параметры | temperature, top-p/top-k, штрафы за повтор, DRY, max tokens, контекст, системный промпт |

### Каталог моделей

| Модель | Размер | Для чего |
|---|---|---|
| TinyStories 260K / 15M | 1–19 МБ | только проверка, что всё работает |
| Gemma 3 270M Instruct | ~250 МБ | минимум, русский слабый |
| Qwen2.5 0.5B Instruct | ~490 МБ | быстрый вариант для слабого железа |
| Llama 3.2 1B Instruct | ~810 МБ | разумный баланс |
| Qwen2.5 1.5B Instruct | ~990 МБ | лучшее качество из каталога, нужно ≥4 ГБ RAM |

Любой другой **instruct-GGUF** можно скачать по прямой ссылке (раздел «Модели» →
«Своя ссылка»). Базовые (не instruct) модели не подойдут — нужен chat-шаблон.

## Настройка (переменные окружения / `.env`)

| Переменная | По умолчанию | Описание |
|---|---|---|
| `WEB_PORT` | `8080` | внешний порт панели (compose) |
| `EDUQUERY_ADMIN_USER` | `admin` | логин администратора |
| `EDUQUERY_ADMIN_PASS` | `admin` | пароль — **смените!** |
| `EDUQUERY_ALLOW_ANONYMOUS` | `false` | вход без пароля (только для отладки) |
| `EDUQUERY_N_THREADS` | `0` | потоки CPU, 0 = по числу ядер |
| `EDUQUERY_N_CTX` | `4096` | контекст модели в токенах |
| `EDUQUERY_ALLOW_REMOTE_DOWNLOAD` | `true` | разрешить скачивать модели и движок |
| `EDUQUERY_ENGINE_VARIANT` | пусто | вариант llama.cpp: пусто = CPU, `vulkan` = GPU (Vulkan) |
| `EDUQUERY_LLAMA_SERVER` | — | путь к своему `llama-server` вместо скачанного |
| `EDUQUERY_MEMORY_LIMIT` | `8g` | лимит памяти контейнера (compose) |
| `EDUQUERY_DATA_DIR` | `/data` | каталог данных |
| `RUST_LOG` | `eduquery=info,warn` | уровень логов |

Офлайн-режим: положите `llama-server` (со всеми `.so` из релиза llama.cpp) в
`data/engine/`, GGUF-файлы — в `data/models/`, и поставьте
`EDUQUERY_ALLOW_REMOTE_DOWNLOAD=false`.

## Данные и бэкап

Всё состояние — в каталоге `./data`:

```
data/
├── eduquery.db     # ученики, история анализов, настройки, сессии
├── models/         # скачанные GGUF
└── engine/         # скачанный llama.cpp
```

Бэкап — копия `data/eduquery.db` (желательно при остановленном контейнере).
Обновление: `docker compose pull && docker compose up -d`.

## API

Авторизация: `POST /api/v1/login` → cookie или Bearer-токен.

```
GET  /health                         проверка живости (без авторизации)
POST /api/v1/login | /logout         GET /api/v1/me
GET  /api/v1/overview | /groups
GET  /api/v1/students                /{id}  /{id}/context
POST /api/v1/seed | /import          демо-данные / импорт CSV
GET  /api/v1/models/catalog | /models | /models/status
POST /api/v1/models/download | /download/custom | /activate
DELETE /api/v1/models/{id}
POST /api/v1/generate                свободная генерация
POST /api/v1/analyze/student | /compare | /overview | /ask
GET/POST /api/v1/params              параметры генерации
```

Пример:

```bash
curl -s -X POST http://localhost:8080/api/v1/generate \
  -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  -d '{"prompt":"Кратко объясни, что такое среднее арифметическое"}'
```

Ответ: `text`, `prompt_tokens`, `completion_tokens`, `tokens_per_sec`, `loop_detected`.

## Сборка из исходников

```bash
cargo build --release            # ~1 минута, C++ собирать не нужно
EDUQUERY_DATA_DIR=./data ./target/release/eduquery

docker build -t entbtw/eduquery:latest .
```

Нужен Rust ≥ 1.80. Структура: `src/llm.rs` — управление llama-server,
`src/server.rs` — HTTP API, `src/models.rs` — каталог и загрузка моделей,
`src/students.rs`/`analysis.rs` — данные и промпты, `web/` — интерфейс.

## Решение проблем

- **«не выбрана активная модель»** — скачайте и активируйте модель в разделе «Модели».
- **Движок не скачался** — проверьте доступ к github.com; смотрите `docker logs eduquery`.
- **llama-server завершился при запуске** — файл модели повреждён/не GGUF, либо не хватает RAM.
- **Модель отвечает мусором или повторяется** — возьмите модель крупнее (1.5B), поднимите
  `repeat_penalty`/DRY в «Параметрах», увеличьте контекст.
- **Контейнер убивается по памяти** — увеличьте `EDUQUERY_MEMORY_LIMIT` или возьмите модель меньше.

## Лицензия

MIT
