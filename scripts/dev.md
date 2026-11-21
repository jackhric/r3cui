# Dev loop (milestone 1 — desktop spike, no device)

## 1. Run the core simulator

Clone and start Unfolded Circle's dockerized core:

```sh
git clone https://github.com/unfoldedcircle/core-simulator
cd core-simulator/docker
chmod 777 ui-env upload        # avoid permission errors on the bind mounts
docker compose up              # REST+WS on :8080, wss on :8443, configurator at /configurator
```

It writes the UI token to `core-simulator/docker/ui-env/ws-token` and defaults `UC_MODEL=UCR3`.
A WS test console is at `http://localhost:8080/ws.html`. Demo auth also works with Basic
`web-configurator`/`1234` (or an admin API key from the simulator README).

## 2. Run our UI against it

```sh
export UC_SOCKET_URL=ws://127.0.0.1:8080/ws
export UC_TOKEN_PATH=/absolute/path/to/core-simulator/docker/ui-env/ws-token
export RUST_LOG=info          # or debug to see every event
cargo run -p remote-ui
```

A 480×800 window should open, authenticate, and list the simulator's entities.
(`UC_TOKEN=<raw token>` also works instead of `UC_TOKEN_PATH`.)

## Notes / limits

- The simulator can't install custom UIs and has no suspend/BT/mic; it's for the WS + UI loop only.
- Device build (later): switch `crates/ui` slint features to `backend-linuxkms`, cross-compile for
  aarch64, then `packaging/pack.sh`. See docs/plan.md and docs/device-contract.md.
