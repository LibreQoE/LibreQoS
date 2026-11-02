#!/bin/bash
# Publish the built manual to LQOS_MANUAL_HOST ("user@host:/path"); it has no default.
mdbook build
pushd book > /dev/null
scp -r * "${LQOS_MANUAL_HOST:?set LQOS_MANUAL_HOST to user@host:/path}"
popd > /dev/null
