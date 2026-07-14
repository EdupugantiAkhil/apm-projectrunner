#!/bin/sh
set -eu

portless_port=${PORTLESS_PORT:-1355}
app_one_url=${APP_ONE_URL:-http://app-one.localhost:${portless_port}}
app_two_url=${APP_TWO_URL:-http://app-two.localhost:${portless_port}}

first=$(curl --noproxy '*' --fail --silent --show-error "$app_one_url")
second=$(curl --noproxy '*' --fail --silent --show-error "$app_two_url")

FIRST=$first SECOND=$second node - <<'NODE'
const first = JSON.parse(process.env.FIRST);
const second = JSON.parse(process.env.SECOND);
const firstVisitIsShared = second.visits.some(
  (visit) => visit.id === first.currentVisitId,
);

if (first.servedBy !== 'app-one') throw new Error('app-one address reached the wrong instance');
if (second.servedBy !== 'app-two') throw new Error('app-two address reached the wrong instance');
if (!firstVisitIsShared) throw new Error('app-two did not see the visit written through app-one');

console.log(`OK: both instances share the database (${second.visits.length} visits stored)`);
NODE
