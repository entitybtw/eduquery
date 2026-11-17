# API

Базовый префикс — `/api/v1`. Сервер по умолчанию слушает `http://localhost:8080`.
Формат — JSON, кроме импорта (CSV/JSON) и экспорта (CSV).

## Авторизация

```http
POST /api/v1/login
Content-Type: application/json

{"username": "admin", "password": "..."}
```

Ответ: `{"status":"ok","token":"..."}` и cookie `eq_token` (`HttpOnly`, `SameSite=Lax`).
Дальше можно передавать cookie или заголовок:

```http
Authorization: Bearer <token>
```

`POST /api/v1/logout` завершает сессию. `GET /api/v1/me` возвращает учётную запись, права,
активную модель и текущее использование лимитов.

Коды: `400` — некорректный запрос, `401` — нет сессии, `403` — нет права, `404` — объект не найден,
`409` — конфликт (бенчмарк уже идёт, запуск нельзя удалить), `429` — превышен лимит ИИ,
`500` — внутренняя ошибка. Ошибки: `{"error":"текст"}`.

## Права

| Право | Что разрешает |
|---|---|
| `view` | обзор, ученики, статистика, чтение бенчмарков |
| `analyze` | запросы к ИИ |
| `import` | импорт, демо-данные, добавление записей, профиль ученика |
| `export` | скачивание CSV |
| `models` | движок, модели, параметры, запуск/оценка/удаление бенчмарков |
| `users` | пользователи и запись настроек |

Роль `admin` имеет все права.

## Ученики

```
GET    /api/v1/students?q=&group=&sort=
GET    /api/v1/students/{id}
GET    /api/v1/students/{id}/context      # текстовый контекст для ИИ
POST   /api/v1/students/{id}/profile      # право import
POST   /api/v1/students/{id}/records      # право import
DELETE /api/v1/records/{id}               # право import
GET    /api/v1/groups
GET    /api/v1/overview
GET    /api/v1/stats?group=
```

`sort`: пусто / `name` / `ratio` / `ratio_asc` / `activities` / `recent`.

Профиль (все поля необязательны; пустая строка очищает текстовое поле):

```json
{
  "full_name": "Иван Петров",
  "group_name": "7А",
  "email": "student@example.invalid",
  "grade": 7,
  "birth_date": "2012-05-01",
  "address": "г. Примерск, ул. Школьная, д. 1",
  "family": "Полная семья. Мать, отец, младшая сестра."
}
```

Запись журнала (`ImportRow`): `kind`, `occurred_at`, `subject`, `title`, `score`, `max_score`,
`grade`, `status`, `duration_min`, `amount`, `reason`, `excused`, `category`, `level`,
`comment`. Обязательные поля зависят от `kind`; подробнее — [`DATA.md`](DATA.md).

## Импорт и экспорт

```
POST /api/v1/import?mode=replace    # Content-Type: text/csv или application/json
GET  /api/v1/import/template.csv
GET  /api/v1/export/students.csv?q=&group=&sort=
GET  /api/v1/export/activities.csv
GET  /api/v1/export/analyses.csv
GET  /api/v1/benchmarks/export.csv
```

Экспорт требует `export`. CSV — UTF-8 с BOM, разделитель `;`, десятичная запятая.
Шаблон импорта доступен с правом `import`.

## Модели и движок

```
GET  /api/v1/models/catalog
GET  /api/v1/models
GET  /api/v1/models/status
POST /api/v1/models/download          {"id":"qwen2.5-0.5b"}
POST /api/v1/models/download/custom   {"id":"my-model","name":"Моя модель","url":"https://…/model.gguf"}
POST /api/v1/models/download/cancel   {"id":"qwen2.5-0.5b"}
POST /api/v1/models/activate          {"id":"qwen2.5-0.5b"}
DELETE /api/v1/models/{id}
DELETE /api/v1/models/{id}/partial
GET  /api/v1/engine/status
POST /api/v1/engine/install
```

`GET /api/v1/models` возвращает `models`, `files` (`[[id, size], …]`), `partials`
(`[[id, size], …]`), `active` (путь), `loaded`, `downloads`.

Загрузка идёт в фоне. `status`: `running` / `done` / `error` / `cancelled`. Прогресс:
`{"id","name","received","total","status","error","file","url"}`.

`POST /api/v1/models/download/cancel` останавливает загрузку, сохраняя `.gguf.part`.
Обработчик ждёт до 5 секунд, пока задача действительно завершится, и отвечает `409`,
если загрузка уже не выполняется. `DELETE /api/v1/models/{id}/partial` удаляет только
недокачанный файл; готовая модель не затрагивается. `DELETE /api/v1/models/{id}` во время
активной загрузки возвращает `409`.

Своя ссылка (`download/custom`) сохраняется в настройках как `download_url:<id>` и
`download_name:<id>`, поэтому после перезапуска такую загрузку тоже можно продолжить.

## Генерация и анализ

```
POST /api/v1/generate
POST /api/v1/analyze/student
POST /api/v1/analyze/compare
POST /api/v1/analyze/overview
POST /api/v1/analyze/ask
POST /api/v1/analyze/characteristic
GET  /api/v1/params
POST /api/v1/params
```

`POST /api/v1/generate`:

```json
{
  "model_id": "qwen2.5-0.5b",
  "system": "необязательная системная инструкция",
  "prompt": "текст запроса",
  "messages": [{"role": "user", "content": "текст"}],
  "params": {
    "temperature": 0.7,
    "top_p": 0.9,
    "top_k": 40,
    "repeat_penalty": 1.1,
    "dry_multiplier": 0.8,
    "max_tokens": 1024,
    "n_ctx": 8192,
    "n_threads": 0,
    "seed": 0,
    "system_prompt": "правила",
    "example_input": "пример запроса",
    "example_answer": "пример ответа"
  }
}
```

`prompt` и `messages` взаимозаменяемы; `messages` имеет приоритет. `model_id` необязателен —
берётся активная модель. Параметры из запроса переопределяют сохранённые, но пустые
`system_prompt` / `example_*` не затирают сохранённые значения.

Ответ:

```json
{
  "text": "…",
  "prompt_tokens": 512,
  "completion_tokens": 300,
  "tokens_per_sec": 21.4,
  "loop_detected": false,
  "duration_ms": 14000
}
```

Анализ:

| Маршрут | Тело |
|---|---|
| `/analyze/student` | `{"student_id":1,"focus":"…","model_id":"…","params":{…}}` |
| `/analyze/compare` | `{"a":1,"b":2,"model_id":"…","params":{…}}` |
| `/analyze/overview` | `{"model_id":"…","params":{…}}` |
| `/analyze/ask` | `{"student_id":1,"question":"…","model_id":"…","params":{…}}` |
| `/analyze/characteristic` | `{"student_id":1,"model_id":"…","params":{…}}` |

Все анализаторы требуют `analyze` и возвращают тот же объект, что `generate`. Результаты с
`student_id` сохраняются в `analyses`; каждая успешная генерация пишет `usage_metrics`.

## Бенчмарки

```
GET    /api/v1/benchmarks
POST   /api/v1/benchmarks
GET    /api/v1/benchmarks/status
GET    /api/v1/benchmarks/{id}
POST   /api/v1/benchmarks/results/{id}/rating   {"rating":1..5}   # 0 — сброс
DELETE /api/v1/benchmarks/{id}
GET    /api/v1/benchmarks/export.csv
```

Запуск (право `models`):

```json
{
  "mode": "student",
  "student_id": 1,
  "title": "Сравнение на разборе",
  "prompt": "",
  "model_ids": ["qwen2.5-0.5b", "llama-3.2-1b"],
  "system_prompt": "необязательно",
  "example_input": "необязательно",
  "example_answer": "необязательно",
  "params": {}
}
```

`mode`: `student` (полный разбор), `characteristic` (официальная характеристика), `custom`
(свой `prompt`). Если `system_prompt` / `example_*` не переданы, берутся сохранённые настройки.
Один запуск — до 12 моделей, только установленные файлы. Второй одновременный запуск — `409`.

`GET /api/v1/benchmarks` возвращает:

```json
{
  "running": 0,
  "runs": [
    {"id":1,"created_at":"…","finished_at":"…","status":"done","mode":"student",
     "title":"…","student_id":1,"total":2,"ok":2,"errors":0,
     "avg_rating":4.5,"avg_tps":18.2,"tokens":640}
  ],
  "usage": [
    {"model_id":"qwen2.5-0.5b","calls":12,"prompt_tokens":5000,
     "completion_tokens":3000,"avg_tps":19.1,"loops":0,"errors":0,"last_used":"…"}
  ]
}
```

`GET /api/v1/benchmarks/status`:

```json
{"running":1,"progress":{"run_id":1,"title":"…","mode":"student",
 "created_at":"…","total":2,"done":1,"current":"Llama 3.2 1B"}}
```

`GET /api/v1/benchmarks/{id}` добавляет `results`: `id`, `model_id`, `model_name`, `status`,
`error`, `answer`, `prompt_tokens`, `completion_tokens`, `tokens_per_sec`, `duration_ms`,
`loop_detected`, `rating`, `created_at`.

## Пользователи и настройки

```
GET/POST      /api/v1/users
PUT/DELETE    /api/v1/users/{id}
POST          /api/v1/me/password
GET/POST      /api/v1/params
GET/POST      /api/v1/settings/{key}
POST          /api/v1/seed
GET           /health
```

`/users` требует `users`; `/settings/{key}` читается всем, записывается с `users`; демо-данные —
`import`. `/health` публичен и возвращает `{"status":"ok"}`.

## Долгие запросы

Запросы к ИИ могут длиться минуты. Сервер отвечает `200` и периодически отправляет пробелы, чтобы
прокси не срабатывал по таймауту. Клиент должен дождаться конца тела и затем разобрать JSON.
