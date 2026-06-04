# corelink-workspaces

**The workspace as a content-addressed object; compute as a disposable cursor.**

`clw` — the client-side primitive of [CoreLink Workspaces](https://github.com/humangr-labs/corelink-server/blob/main/marketing/expansion/corelink-workspaces.md):
snapshot a directory tree into CoreLink's CAS, hydrate it anywhere, resume where you
left off, and memoize deterministic commands via the AC.

Plugs into the LIVE CoreLink surfaces (`/v1/cas`, `/v1/ac` at `corelink-api.humangr.com`)
with a CoreLink PAT. Pure client: zero server changes.

> Status: phase-1 under construction (wave plan in `.techlead/`). W13 owns the real README.

## Gates

`bash scripts/gates.sh` — fmt + clippy `-D warnings` + tests. DCO trailer required on every commit.
