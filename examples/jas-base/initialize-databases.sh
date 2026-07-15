#!/bin/sh
set -eu

prefix="${SWITCHYARD_DEPLOYMENT}--${SWITCHYARD_INSTANCE}"
/usr/local/bin/jas-base-fixture initialize \
  "${prefix}--kv-store:9101" \
  "${prefix}--document-store:9102"
