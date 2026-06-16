# Owner action — name the live data-plane container block (`wrangler.toml`, D-6a)

> **For:** the owner / infra. **From:** clw TL, relaying a corelink-server TL question. **Priority:**
> LOW — non-blocking for the clw GA tag; it's a campaign-#1 (CI-acceleration) reconcile, not a
> launch gate. **One decision, then the server TL acts.**

## The question
The corelink-server TL confirmed the **D-6a config drift** is real but cannot fix it safely without
the owner naming the intended live container. From their response (code-verified):

> `wrangler.toml` does **not** have a single `max_instances` — it has **7 container blocks** with
> mixed values: `:40 = 5`, `:382 = 20`, and `:537 / :699 / :822 / :944 / :1066 = 10`. The "5 vs live
> 20" drift is real between two of those blocks. **They need to know which block is the live
> data-plane container** to reconcile to one intended value — they won't touch prod `wrangler.toml`
> mid-flight without that confirmation.

## What the owner needs to answer
1. **Which of the 7 container blocks is the live data-plane CAS/AC container?** (line `:40`, `:382`,
   `:537`, `:699`, `:822`, `:944`, or `:1066`.)
2. **What is the intended `max_instances` for it** (the live value is observed as `20`)?

## Then
Relay the answer to the corelink-server TL; they reconcile the drift to the single intended value.
clw is **not** in this data path (sccache speaks its own protocol to the cargo route) — this is purely
a server config reconcile. Tracked under campaign #1 / D-6.

> Source: `docs/RESPONSE-from-server-TL-2026-06-16.md` (P3) · `docs/FINDING-per-tenant-write-ceiling.md`.
