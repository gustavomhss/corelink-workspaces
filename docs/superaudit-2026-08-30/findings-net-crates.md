> **ARCHIVAL SNAPSHOT — NOT CURRENT STATE.** Captured against `25cfcaf` on 2026-08-30. Line numbers,
> severities and open/fixed claims below are contemporaneous evidence only. Consult
> `../HANDOFF-2026-08-31-clw-TL-superaudit-campaign.md` and the WP catalogue before acting.

## Agente NET/EXEC CRATES (client/snapshot/hydrate/run) — completo. 0 CRITICAL/HIGH/MED. 4 LOW + 5 INFO.
"Unusually well-hardened" — superfícies HTTP/auth/traversal/OOM todas fechadas e com teste red-on-revert.

LOWs:
1. hydrate lib.rs:588-655 (via :452): std::fs blocking + sync_all() fsync + rename DIRETO no worker tokio, até `concurrency` em voo — inconsistente com snapshot (que usa spawn_blocking :234/:272). hydrate --concurrency 8 pode pinar todos os workers em fsync → timers/futures starved. Fix: spawn_blocking ou tokio::fs.
2. run lib.rs:242,321-322,361-362: compute_key lê arquivo inteiro em memória (inputs + binário command[0]) em TODO run, hit incluso, sem streaming (snapshot streama >16MiB). rustc 200MB + input 4GB = ~4.2GB alocados só p/ chave de memo. Fix: blake3 Hasher streaming + cache do tool digest por (path,len,mtime).
3. hydrate :418-455/:463-493: colisão case-fold em fs case-insensitive sobrescreve SILENCIOSO (byte-order permite "CONFIG"+"config") — árvore infiel, sem escape mas perda de dados silenciosa. Fix: detectar colisão lowercased-set up-front e rejeitar/warn. (Casa com MED #3 do agente core — mesma raiz em types.)
4. run lib.rs:565-578: sem signal handlers, sem kill_on_drop, sem process group — SIGTERM programático direto no clw órfã o child (não sinalizado, não reaped). Ctrl-C funciona só via foreground group. Fix: kill_on_drop(true)/forwarder ou documentar dependência de process-group.

INFOs: proxy ambiente honrado por default (não vaza PAT — CONNECT tunela TLS — mas surpreendente/não-documentado); snapshot sem cap de CONTAGEM de entradas (só manifest 5MiB pós-hoc — árvore 10M arquivos sobe tudo antes de falhar; ok sob threat model local-trusted); Retry-After clampado a 10s (60s sob-honrado; trade-off intencional); put 413 → TooLarge{size:0} placeholder descarta len real; TOCTOU classify-vs-read symlink (documentado fora do threat model v0.1, O_NOFOLLOW tracked).

SÓLIDO: auth/URL hardening exemplar (mesma lib url do reqwest, userinfo smuggle morto, redirect none, redeem fail-closed single-shot); todo body capado (Content-Length short-circuit + running ceiling, ambos testados); verbos retried idempotentes por construção + AC PUT lost-ACK reconcile via is_conflict; hydrate trata manifest como hostil e PROVA (caps antes de FS work, setuid strip, files-before-symlinks, RAII temp + fsync-before-rename, cada um red-on-revert); contrato exit-code honrado e tripwired (125/137 constantes, non-zero nunca cacheado).

TEST GAPS: client — AC get oversize sem teste direto (:1271), exists HEAD→GET fallback "não lê body" não testado; snapshot — fifo/socket/device sem pin, hardlinks, unicode filenames, caminho streaming real >16MiB nunca end-to-end, mutação mid-snapshot; hydrate — case-collision, disk-full; run — child morto por sinal → 128+n NUNCA dirigido por teste (código :685-694 existe, nada mata child e asserta 143), Windows path arms só compile-gate.
