# Shared database, duplicated service

This is a working implementation of [`features.md`](features.md). It runs three
containers:

1. `database` — one PostgreSQL database, available only on the private Compose network.
2. `app-one` — the first copy of the service.
3. `app-two` — a second copy built from exactly the same image.

Both application copies listen on container port `8080`. The Vercel Labs
[Portless](https://github.com/vercel-labs/portless) development proxy gives them stable
virtual hostnames on the same host port without adding another runtime container:

- <http://app-one.localhost:1355> (`app-one`)
- <http://app-two.localhost:1355> (`app-two`)

Each request to `/` records which copy handled it and returns all visits. Calling one
address and then the other demonstrates that both copies use the same persistent data.

## Run it

```sh
cp .env.example .env       # optional; development defaults are built in
npm install
npm run compose:up
npm run smoke
npm run compose:down
```

This configuration does not need root. Portless uses plain HTTP on unprivileged port
`1355`, disables `/etc/hosts` synchronization, and relies on the special `.localhost`
domain. It does not use mDNS or an external DNS service. The applications' direct
fallback URLs are `http://127.0.0.1:18081` and `http://127.0.0.1:18082`.

Use `docker compose down --volumes` when you also want to delete the database data.

## Configuration

Copy `.env.example` to `.env` and change values as needed. The checked-in credentials
are local-development defaults only. The database port is not published to the host.

`PORTLESS_PORT` controls the shared public proxy port. `APP_ONE_HOST_PORT` and
`APP_TWO_HOST_PORT` are loopback-only upstream ports used by Portless.

## Verify

```sh
npm install
npm test
docker compose config --quiet
npm run compose:up && npm run smoke
```
