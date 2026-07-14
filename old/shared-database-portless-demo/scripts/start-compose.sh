#!/bin/sh
# Starts the archived Portless proof-of-concept.
set -eu

portless_port=${PORTLESS_PORT:-1355}
app_one_port=${APP_ONE_HOST_PORT:-18081}
app_two_port=${APP_TWO_HOST_PORT:-18082}

docker compose up --build -d --wait

# A high proxy port, plain HTTP, and disabled hosts-file synchronization keep
# this setup entirely unprivileged and local to the current user.
PORTLESS_SYNC_HOSTS=0 npx portless proxy start --port "$portless_port" --no-tls
PORTLESS_SYNC_HOSTS=0 npx portless alias app-one "$app_one_port" --force
PORTLESS_SYNC_HOSTS=0 npx portless alias app-two "$app_two_port" --force

printf '\nReady:\n  http://app-one.localhost:%s\n  http://app-two.localhost:%s\n' \
  "$portless_port" "$portless_port"
