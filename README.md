# eduquery

A local LLM platform for analysing students' academic performance.
Web panel + SQLite + a language model that runs **on your own server**:
student data never leaves your machine.

- Tiny image: the llama.cpp engine and the models are **downloaded automatically** on first run.
- One container, no external services, no API keys.
- Multi-user: accounts stored in SQLite, per-user permissions and AI request limits.
- Tables in and out: upload CSV/JSON (several files at once), download the table you are looking at as CSV.
- Password login and demo data included so you can try it right away.

## Quick start

```bash
git clone https://git.entitybtw.ru/entitybtw/eduquery.git
cd eduquery
./install.sh                 # creates .env with a random admin password
docker compose up -d
```

Open `http://localhost:8080` (port is `WEB_PORT` in `.env`) and log in with the
credentials printed by `install.sh`.

Without cloning the repo (prebuilt image from Docker Hub):

```bash
docker run -d --name eduquery -p 8080:8080 -v $PWD/data:/data \
  -e EDUQUERY_ADMIN_PASS=choose-a-password entbtw/eduquery:latest
```

## What happens on first run

1. The database `data/eduquery.db` is created, filled with **demo students** (if empty), and the first
   administrator is created from `EDUQUERY_ADMIN_USER` / `EDUQUERY_ADMIN_PASS`.
2. After you log in, a **"First run"** window offers to install the dependencies:
   - **llama.cpp engine** (`llama-server`, ~25 MB from GitHub) — one button;
   - **a language model** — one button downloads the recommended one and activates it.
3. Progress is shown as a **progress bar at the top of the page** (engine install and model downloads).
   Interrupted downloads are resumed automatically.
4. The first AI request loads the model into memory (seconds to minutes); after that it is fast.

You can skip the window ("Later") and install everything later from the **Models** tab.
You need internet access to `github.com` (engine) and `huggingface.co` (models).
Everything downloaded is stored in `./data` and survives container rebuilds and updates.

## How it works

```
 browser ──HTTP──► eduquery (Rust, axum) ──HTTP localhost──► llama-server (llama.cpp)
                        │                                          │
                   SQLite (data/)                           GGUF model (data/models/)
```

- `eduquery` is the only process you run. After you install the engine from the UI, it starts `llama-server` as a child
  process on `127.0.0.1` with a random port, and restarts it when the model or
  context size changes.
- Generation is streamed. If a weak model falls into a loop (repeating the same
  line), the answer is cut off and flagged with `loop_detected`.
- The context window is limited by `n_ctx`; for long reports, shorten the input
  or raise the context size.

## Features

| Section | What it does |
|---|---|
| Overview | Summary per class/group, activity trends |
| Students | List, student card, context (grades, attendance, activities) |
| Student analysis | LLM review of one student's data; result is saved to history |
| Comparison | Compare several students |
| Ask about data | Free-form question to the LLM about selected data |
| Import | Upload one or several CSV/JSON tables; optionally replace all current data |
| Export | Download the current students table (with your search/filter/sort), all activity records, or the analysis history as CSV |
| Users | Create accounts, set permissions and AI request limits (admins only) |
| Models | Catalog, download by URL, activate, delete |
| Parameters | temperature, top-p/top-k, repeat penalty, DRY, max tokens, context, system prompt |

### Model catalog

| Model | Size | Use |
|---|---|---|
| TinyStories 260K / 15M | 1–19 MB | smoke test only |
| Gemma 3 270M Instruct | ~250 MB | bare minimum, weak multilingual quality |
| Qwen2.5 0.5B Instruct | ~490 MB | fast option for weak hardware |
| Llama 3.2 1B Instruct | ~810 MB | reasonable balance |
| Qwen2.5 1.5B Instruct | ~990 MB | best quality in the catalog, needs ≥4 GB RAM |

Any other **instruct GGUF** can be downloaded by direct link (Models → custom URL).
Base (non-instruct) models will not work: a chat template is required.

## Users, permissions and limits

Accounts live in the SQLite database. `EDUQUERY_ADMIN_USER` / `EDUQUERY_ADMIN_PASS` only create the
**first** administrator when the database has no users; afterwards manage accounts in the **Users** tab
(passwords are stored as Argon2 hashes; everyone can change their own password by clicking their name).

| Permission | Allows |
|---|---|
| `view` | See overview, students and their cards |
| `analyze` | Send requests to the AI (counted against limits) |
| `import` | Upload tables, generate demo data |
| `export` | Download CSV tables |
| `models` | Install the engine, download/activate/delete models, change generation parameters |
| `users` | Manage users (and write app settings) |

- Role **admin** has every permission; role **user** has the checked ones.
- **Limits** — maximum AI requests per rolling hour and per rolling 24 hours (`0` = unlimited). When a limit is
  hit the API answers `429` with a readable message. The Users tab shows current usage.
- You cannot delete, disable or demote yourself, and the last active admin cannot be removed.

## Tables: import and export

- **Import** (Students tab): choose one or more `.csv` / `.json` files. Delimiter `,` or `;`, UTF-8 or
  Windows-1251, decimal comma accepted (`8,5`). Tick *replace all current data* to wipe students, activities
  and analyses first.
- **Export**: *Download table (CSV)* saves exactly the table on screen (current search, group filter and sort);
  *Download all records* exports every activity in the same format the importer reads, so a file can be
  edited in Excel and uploaded back; the Analysis tab exports the history of AI analyses.
  Files are UTF-8 with BOM, so Excel opens them correctly.

## Configuration (environment variables / `.env`)

| Variable | Default | Description |
|---|---|---|
| `WEB_PORT` | `8080` | external panel port (compose) |
| `EDUQUERY_ADMIN_USER` | `admin` | login of the first admin (created only on an empty user table) |
| `EDUQUERY_ADMIN_PASS` | `admin` | password of the first admin — **change it!** |
| `EDUQUERY_ALLOW_ANONYMOUS` | `false` | passwordless login (debugging only) |
| `EDUQUERY_N_THREADS` | `0` | CPU threads, 0 = number of cores |
| `EDUQUERY_N_CTX` | `4096` | model context size in tokens |
| `EDUQUERY_ALLOW_REMOTE_DOWNLOAD` | `true` | allow installing the engine and downloading models |
| `EDUQUERY_ENGINE_VARIANT` | empty | llama.cpp build: empty = CPU, `vulkan` = GPU via Vulkan |
| `EDUQUERY_LLAMA_SERVER` | — | path to your own `llama-server` instead of the downloaded one |
| `EDUQUERY_MEMORY_LIMIT` | `8g` | container memory limit (compose) |
| `EDUQUERY_DATA_DIR` | `/data` | data directory |
| `RUST_LOG` | `eduquery=info,warn` | log level |

Offline mode: put `llama-server` (with all `.so` files from a llama.cpp release)
into `data/engine/`, GGUF files into `data/models/`, and set
`EDUQUERY_ALLOW_REMOTE_DOWNLOAD=false`.

## Data and backup

All state lives in `./data`:

```
data/
├── eduquery.db     # users, students, analysis history, settings, sessions
├── models/         # downloaded GGUF files
└── engine/         # downloaded llama.cpp
```

Back up by copying `data/eduquery.db` (preferably while the container is stopped).
Update: `docker compose pull && docker compose up -d`.

## API

Auth: `POST /api/v1/login` → cookie or Bearer token. Missing permission → `403`, exceeded AI limit → `429`.
`POST /api/v1/import` accepts `?mode=replace`.

```
GET  /health                         liveness check (no auth)
POST /api/v1/login | /logout         GET /api/v1/me      POST /api/v1/me/password
GET/POST /api/v1/users               PUT/DELETE /api/v1/users/{id}      (permission: users)
GET  /api/v1/export/students.csv?q=&group=&sort=   /activities.csv   /analyses.csv   (permission: export)
GET  /api/v1/engine/status           POST /api/v1/engine/install        (install: models)
GET  /api/v1/overview | /groups
GET  /api/v1/students                /{id}  /{id}/context
POST /api/v1/seed | /import          demo data / CSV import
GET  /api/v1/models/catalog | /models | /models/status
POST /api/v1/models/download | /download/custom | /activate
DELETE /api/v1/models/{id}
POST /api/v1/generate                free-form generation
POST /api/v1/analyze/student | /compare | /overview | /ask
GET/POST /api/v1/params              generation parameters
```

Example:

```bash
curl -s -X POST http://localhost:8080/api/v1/generate \
  -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  -d '{"prompt":"Briefly explain what an arithmetic mean is"}'
```

Response: `text`, `prompt_tokens`, `completion_tokens`, `tokens_per_sec`, `loop_detected`.

## Building from source

```bash
cargo build --release            # ~1 minute, no C++ to compile
EDUQUERY_DATA_DIR=./data ./target/release/eduquery

docker build -t entbtw/eduquery:latest .
```

Requires Rust ≥ 1.80. Layout: `src/llm.rs` — llama-server management,
`src/server.rs` — HTTP API, `src/models.rs` — model catalog and downloads,
`src/users.rs` / `auth.rs` — accounts, permissions, limits, sessions,
`src/admin_api.rs` — users, CSV export, engine endpoints, `src/students.rs` / `analysis.rs` — data and prompts, `web/` — the UI.

## Troubleshooting

- **"engine is not installed"** — click *Install* in the First-run window or on the Models tab.
- **"no active model selected"** — download and activate a model in the Models section.
- **Forgot the admin password** — stop the container, delete the `users` rows (`sqlite3 data/eduquery.db "DELETE FROM users"`) and start again: the admin from `EDUQUERY_ADMIN_*` is recreated.
- **Engine did not download** — check access to github.com (the error is shown in the UI; downloads resume after drops); see `docker logs eduquery`.
- **llama-server exited on startup** — the model file is corrupt / not a GGUF, or there is not enough RAM.
- **Model outputs garbage or repeats itself** — use a larger model (1.5B), raise
  `repeat_penalty`/DRY in Parameters, increase the context.
- **Container killed for memory** — raise `EDUQUERY_MEMORY_LIMIT` or use a smaller model.

## License

MIT
