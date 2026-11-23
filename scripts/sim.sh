#!/bin/sh
# Start/stop the core-simulator plus its Home Assistant demo integration (the only source of
# entities in a fresh simulator) and print the env to run against it.
# usage: scripts/sim.sh [up|seed|kodi|down|logs|env]
set -eu

here=$(cd "$(dirname "$0")" && pwd)
SIM_DIR=${SIM_DIR:-$here/../../core-simulator}
SIM_PORT=${SIM_PORT:-8090}
export SIM_PORT

[ -d "$SIM_DIR/docker" ] || {
    echo "core-simulator not found at $SIM_DIR (git clone https://github.com/unfoldedcircle/core-simulator, or set SIM_DIR)" >&2
    exit 1
}
docker_dir=$(cd "$SIM_DIR/docker" && pwd)

compose() {
    docker compose -f "$docker_dir/docker-compose.yml" -f "$here/sim.override.yml" "$@"
}

print_env() {
    echo "export UC_SOCKET_URL=ws://127.0.0.1:$SIM_PORT/ws"
    echo "export UC_TOKEN_PATH=$docker_dir/ui-env/ws-token"
}

# Demo admin account from the core-simulator README.
api() {
    curl -fsS -m 15 -u admin:remote2 -H 'Content-Type: application/json' "$@"
}

# A fresh simulator knows the HA demo entities only as "available"; the UI sees them once configured.
seed() {
    base=http://127.0.0.1:$SIM_PORT/api
    intg=$base/intg/instances/hass.main
    i=0
    until api "$intg" 2>/dev/null | grep -q '"device_state":"CONNECTED"'; do
        i=$((i + 1))
        [ "$i" -le 60 ] || { echo "hass.main integration never connected" >&2; exit 1; }
        sleep 2
    done
    ids=$(api "$intg/entities?limit=100" | grep -o '"entity_id":"[^"]*"' | cut -d: -f2 | paste -sd, -)
    api -X POST "$intg/entities" -d "[$ids]" >/dev/null
    echo "configured $(echo "$ids" | tr ',' '\n' | wc -l) entities" >&2
}

# External drivers are normally found via mDNS, which doesn't cross the compose network, so the
# Kodi driver is registered with the core by hand.
kodi() {
    mkdir -p "$docker_dir/integration-kodi"
    compose --profile kodi up -d integration-kodi
    base=http://127.0.0.1:$SIM_PORT/api
    api "$base/intg/drivers/kodi_driver" >/dev/null 2>&1 || api -X POST "$base/intg/drivers" -d '{
        "driver_id": "kodi_driver",
        "name": {"en": "Kodi"},
        "driver_url": "ws://integration-kodi:9090/ws",
        "version": "0",
        "enabled": true
    }' >/dev/null
    echo "Kodi driver registered — finish setup at http://127.0.0.1:$SIM_PORT/configurator" >&2
}

case "${1:-up}" in
    up)
        chmod 777 "$docker_dir/ui-env" "$docker_dir/upload"
        compose up -d core-simulator integration-hass hassio
        seed
        print_env
        ;;
    seed) seed ;;
    kodi) kodi ;;
    down) compose --profile kodi down ;;
    logs) compose logs -f core-simulator ;;
    env) print_env ;;
    *) echo "usage: $0 [up|seed|kodi|down|logs|env]" >&2; exit 2 ;;
esac
