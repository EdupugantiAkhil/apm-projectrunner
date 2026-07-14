#!/bin/sh
set -eu

PORTLESS_SYNC_HOSTS=0 npx portless alias --remove app-one >/dev/null 2>&1 || true
PORTLESS_SYNC_HOSTS=0 npx portless alias --remove app-two >/dev/null 2>&1 || true
PORTLESS_SYNC_HOSTS=0 npx portless proxy stop >/dev/null 2>&1 || true
docker compose down
