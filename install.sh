#!/usr/bin/env bash
# eduquery — быстрая установка.
#
#   ./install.sh
#   docker compose up -d --build
#
# Скрипт создаёт .env с случайным паролем администратора.

set -euo pipefail
cd "$(dirname "$0")"

if ! command -v docker >/dev/null 2>&1; then
  echo "ОШИБКА: не найден docker. Установите Docker Engine (>=20) и повторите." >&2
  exit 1
fi

if [ -f .env ]; then
  echo ".env уже существует — не трогаю."
  echo "Текущие настройки:"
  sed 's/^EDUQUERY_ADMIN_PASS=.*/EDUQUERY_ADMIN_PASS=<скрыт>/' .env | grep -E '^(WEB_PORT|EDUQUERY_ADMIN_USER|EDUQUERY_ADMIN_PASS)=' || true
  exit 0
fi

PASS="$(head -c 18 /dev/urandom | base64 | tr -d '/+=' | cut -c1-18)"
USER_NAME="${EDUQUERY_ADMIN_USER:-admin}"
PORT="${WEB_PORT:-8080}"

cat > .env <<EOF
# Создано install.sh $(date +%Y-%m-%d)

WEB_PORT=${PORT}

EDUQUERY_ADMIN_USER=${USER_NAME}
EDUQUERY_ADMIN_PASS=${PASS}

EDUQUERY_N_THREADS=0
EDUQUERY_N_CTX=4096
EDUQUERY_ALLOW_REMOTE_DOWNLOAD=true
EDUQUERY_ALLOW_ANONYMOUS=false
EDUQUERY_MEMORY_LIMIT=8g
RUST_LOG=eduquery=info,warn
EOF

chmod 600 .env
mkdir -p data

echo
echo "Готово. Создан файл .env со случайным паролем."
echo
echo "  Логин:  ${USER_NAME}"
echo "  Пароль: ${PASS}"
echo "  Адрес:  http://localhost:${PORT}"
echo
echo "Дальше:"
echo "  docker compose up -d --build"
echo
