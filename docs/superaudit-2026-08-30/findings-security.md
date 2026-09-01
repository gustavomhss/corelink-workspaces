> **ARCHIVAL SNAPSHOT — NOT CURRENT STATE.** Captured against `25cfcaf` on 2026-08-30. Line numbers,
> live security posture, severities and open/fixed claims below are contemporaneous evidence only. Consult
> `../HANDOFF-2026-08-31-clw-TL-superaudit-campaign.md` and the WP catalogue before acting.

## Agente SECURITY POSTURE (completo) — 0 CRITICAL/HIGH. Core forte; achados = drift de docs + hardening CI.

MEDs (todos documentais/compliance):
1. THREAT-MODEL.md:47 + SECURITY.md:73-75 afirmam "zero unsafe" — FALSO: 1 bloco unsafe em clw-cli/src/config.rs:768-885 (DACL Windows via windows-sys, bem documentado). Fix: corrigir docs + #![forbid(unsafe_code)] em todos os crates exceto clw-cli (deny + allow pontual). Hoje NENHUM crate tem lint unsafe_code.
2. THREAT-MODEL.md:65-73 stale: afirma "no erasure command today / residency promise non-compliant" — FALSO: rm/erase/prune/ls existem e D-1/D-7/D-8 live. Doc canônico asserta não-conformidade GDPR que não é mais verdade.
3. THREAT-MODEL.md:54-64 auto-contraditório: bullet diz resource exhaustion "not yet bounded (M1.6)" e o seguinte "Now mitigated by M1.6"; M1.7 listado como futuro mas implementado (mode mask hydrate:649, files-before-symlinks :458).
4. SECURITY.md:35-41 vs SUPPORT-AND-EOL.md:22-37 CONTRADIÇÃO: "só latest suportada, sem backports" vs "minor anterior: security-only 90 dias + backport em 7d". Duas promessas públicas divergentes.

LOWs:
- THREAT-MODEL não lista mitigações shipped (userinfo-smuggling reject, redirect ban, caps de wire, H3 child-env strip, DACL, cache self-heal) → risco de regressão silenciosa.
- SECURITY.md:12 sem fingerprint PGP publicado, sem security.txt, sem safe-harbor.
- install.sh:57-58 CLW_SKIP_SIGNATURE=1 permite install só por checksum (mesma superfície do binário) — opt-in, loud warning, aceito.
- release.yml:1081-1086 tap token em argv + persiste em .git/config do clone; $TAP_DIR não removido (runner efêmero mitiga).
- Secrets em argv: release.yml:162 (CLW_CI_QUOTA_READ_KEY), metered-probe-canary.yml:72 (PROBE_TOKEN), release.yml:517 security import -P no Mac PERSISTENTE (self-hosted macOS X64, linha 334 — waiver, skip hoje); dogfood-signup.sh:28,35,51 CLERK_SECRET_KEY em argv.
- LOW-MED: 5 workflows sem `permissions:` top-level (ci, conformance-canary, e2e-evidence, live-journeys, release top) — 8 de 13 têm. Fix: contents: read top-level. VERIFY: default do GITHUB_TOKEN no repo.
- Docs com identificadores reais (não credenciais): BUGREPORT-prod-ac-500.md:11 tenant UUID real + Clerk user id; RELAY-…ROUND2-DELIVERED.md:16 tenant sandbox + token_id revogado. Scrub antes de qualquer mudança de visibilidade do repo.

INFOs: reqwest com proxy-from-env default (sem .no_proxy()); sem min_tls_version explícito (default rustls 1.2 ok); secret-heuristic do snapshot é warn-only (by design); bootstrap trust curl|sh inerente.

VERIFY: default GITHUB_TOKEN; label corelink exclusivamente Firecracker efêmero (se box persistente pegar job, LOWs de argv escalam); SECURITY.md visível no mirror público?; CI tripwire para byte-igualdade minisign.pub ↔ install.sh.

SÓLIDO (verificado): zero credenciais commitadas; lifecycle do token estanque (Debug redigido em 5 structs, sem Serialize, erro nunca carrega token, doctor/auth/init presence-only, ~/.clw/config.toml 0600 atômico + DACL, H3 strip com teste red-on-revert); TODA leitura de wire capada (tabela completa: blob 5MiB, list 16MiB/página + 10M digests + 1M páginas + cursor strict, cred-ticket 64KiB, hydrate bounds, child stdout kill-on-exceed, cache re-verify BLAKE3); redirects OFF; https-only + loopback exact-host + userinfo reject; rustls puro, zero danger_ flags; install.sh ordem de verificação correta, chave embutida, --proto '=https'; minisign na release fail-closed exemplar; deny.toml sources crates.io-only.
