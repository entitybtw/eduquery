# eduquery

A local LLM platform for analysing students' academic performance.
Web panel + SQLite + a language model that runs **on your own server**:
student data never leaves your machine.

- Tiny image: the llama.cpp engine and the models are **downloaded automatically** on first run.
- One container, no external services, no API keys.
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

1. The database `data/eduquery.db` is created and filled with **demo students** (if empty).
2. In the background, the **llama.cpp** engine (`llama-server`, ~25 MB) is downloaded to `data/engine/`.
3. In the **Models** section you pick a model from the catalog, click "Download", then "Activate".
4. The first request loads the model into memory (seconds to minutes); after that it is fast.

You need internet access to `github.com` (engine) and `huggingface.co` (models).
Everything downloaded is stored in `./data` and survives container rebuilds and updates.

## How it works

```
 browser ──HTTP──► eduquery (Rust, axum) ──HTTP localhost──► llama-server (llama.cpp)
                        │                                          │
                   SQLite (data/)                           GGUF model (data/models/)
```

- `eduquery` is the only process you run. It starts `llama-server` as a child
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
| Import | Upload CSV files (including cp1251 encoding) |
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

## Configuration (environment variables / `.env`)

| Variable | Default | Description |
|---|---|---|
| `WEB_PORT` | `8080` | external panel port (compose) |
| `EDUQUERY_ADMIN_USER` | `admin` | admin login |
| `EDUQUERY_ADMIN_PASS` | `admin` | admin password — **change it!** |
| `EDUQUERY_ALLOW_ANONYMOUS` | `false` | passwordless login (debugging only) |
| `EDUQUERY_N_THREADS` | `0` | CPU threads, 0 = number of cores |
| `EDUQUERY_N_CTX` | `4096` | model context size in tokens |
| `EDUQUERY_ALLOW_REMOTE_DOWNLOAD` | `true` | allow downloading models and the engine |
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
├── eduquery.db     # students, analysis history, settings, sessions
├── models/         # downloaded GGUF files
└── engine/         # downloaded llama.cpp
```

Back up by copying `data/eduquery.db` (preferably while the container is stopped).
Update: `docker compose pull && docker compose up -d`.

## API

Auth: `POST /api/v1/login` → cookie or Bearer token.

```
GET  /health                         liveness check (no auth)
POST /api/v1/login | /logout         GET /api/v1/me
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
`src/students.rs` / `analysis.rs` — data and prompts, `web/` — the UI.

## Troubleshooting

- **"no active model selected"** — download and activate a model in the Models section.
- **Engine did not download** — check access to github.com; see `docker logs eduquery`.
- **llama-server exited on startup** — the model file is corrupt / not a GGUF, or there is not enough RAM.
- **Model outputs garbage or repeats itself** — use a larger model (1.5B), raise
  `repeat_penalty`/DRY in Parameters, increase the context.
- **Container killed for memory** — raise `EDUQUERY_MEMORY_LIMIT` or use a smaller model.

## License

MIT
