# Observability boundary

**Status recorded:** 2026-09-01

This document records the known external-observer limitation for WP-36. It is
transparency about an uncovered failure domain, not an operational mitigation.

## What the in-platform deadman checks

[`canary-deadman.yml`](../.github/workflows/canary-deadman.yml) runs on a schedule and
checks the registered scheduled lanes through [`deadman-registry.sh`](../scripts/deadman-registry.sh).
For each registered lane it checks whether a completed run exists, whether it is within
that lane's freshness window, and whether it concluded successfully. This can expose a
lane that is scheduled but has stopped reporting.

It does not prove that the system is alive. In particular, it cannot prove GitHub Actions
scheduler liveness, CoreLink fabric liveness or ingestion liveness: the deadman itself
uses the same Actions/fabric execution path. If that path cannot schedule or start the
deadman, the watcher cannot evaluate its own silence. Its failure signal also remains in
the GitHub Actions run/notification path.

## External observer status

As of 2026-09-01, **no observer outside GitHub Actions exists or is configured**. There
is no owner-Mac cron, third-party heartbeat service, webhook/pager, or independent alert
channel that has been deployed or tested for this repository. Therefore there is no
external detection or alerting guarantee for an Actions scheduler outage, a CoreLink
fabric outage, or a dead deadman.

A future Mac cron or third-party heartbeat is only a possible implementation path. It is
not implemented, and this document must not be read as saying that it is a current
mitigation. To establish one, an independently scheduled observer would need to deliver
an alert through a channel independent of GitHub and demonstrate the alert by
deliberately stopping a watched lane.

For the catalog context, see [WP-CATALOG-v3.md](superaudit-2026-08-30/WP-CATALOG-v3.md)
(`WP-36 — Deadman externo`).
