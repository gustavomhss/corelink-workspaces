> **ARCHIVAL SNAPSHOT — NOT CURRENT STATE.** Captured against `25cfcaf` on 2026-08-30. Line numbers,
> corpus counts, severities and open/fixed claims below are contemporaneous evidence only. Consult
> `../HANDOFF-2026-08-31-clw-TL-superaudit-campaign.md` and the WP catalogue before acting.

## Agente DOCS (completo) — corpus 91 arquivos docs/ + 8 top-level.

HIGHs (staleness/contradição):
1. THREAT-MODEL.md:65-73 afirma "no erasure command / non-compliant" — rm/prune/erase shipped há 2.5 meses; pior staleness do repo (doc que SECURITY.md aponta). + auto-contradição M1.6 (54-64).
2. INSTALL.md §2/§4d: Authenticode "is required" + Get-AuthenticodeSignature 'Valid' — TODO .exe shipped é UNSIGNED (waiver). Usuário seguindo §4d conclui que toda release é comprometida.
3. INSTALL.md + DISTRIBUTION.md prometem gh attestation/SLSA publicado — SKIPPED (private-repo waiver, release.yml:997-1025). `gh attestation verify` falha p/ todo usuário. (+ release notes texto "SLSA provenance" falso — INFO code-side.)
4. CLAUDE.md stale: (a) "coverage/fuzz/mutation on github-hosted ubuntu" — tudo é corelink agora; (b) toolchain ".clw-ci-rust ISOLADO" não existe mais (0 hits; padrão atual = scripts/ci-use-host-toolchain.sh); (c) release "mac×2 native, linux×2 + windows-gnu cross" — hoje 5 targets num único Mac via zigbuild. Misdireciona toda sessão futura de agente.
5. SECURITY.md vs SUPPORT-AND-EOL.md: políticas de suporte CONTRADITÓRIAS (latest-only sem backport vs security-only 90d + backport 7d).

MEDs:
- TESTING.md: "github-hosted / windows-latest" falso (tudo corelink; windows.yml = mingw+wine smoke); "coverage report-only Phase 1b futuro" — floor 88% ATIVO desde #187; tabela omite 4 workflows (conformance-canary, live-journeys, metered-probe, canary-deadman).
- CLI-CONTRACT.md:37 CLW_REF_DOMAIN "PROPOSED" — shipped/frozen (main.rs:63-74).
- SPEC-manifest.md:6 + REVIEWER-BRIEF §1.1: hash frozen fe251941 ≠ clw-types atual (mudou em ≥5 PRs; sha atual 08087eda). Reviewer que verifica hash recebe mismatch sem explicação.
- TEST-JOURNEY-MAP.md descreve como futuro suite que shipped (live_journeys semanal + e2e-evidence 9/9).
- go-live/RUNBOOK-destructive-ops.md banner "até PR #269 deploy" — deployado 2026-06-15; runbook VIVO de operador.
- usage.md rate-limits pré-402 (nunca menciona 402 $-ceiling; GUIDE §13 correto); "live feature nunca em CI, rode manual" — roda daily+weekly.
- GUIDE §20/§J topologia release stale ("runner per-OS matrix nativo") — zigbuild em 1 Mac.
- CLAUDE.md aponta HANDOFF-2026-06-28 como "latest" — 2 meses stale, cita v0.1.3 e `bash verify.sh` (não existe). Fix: handoff novo ou banner SUPERSEDED.
- family-e2e-STATUS.md "ledger atualizado" congelado 2026-06-21.
- SECURITY.md "EOL policy lands with 1.0" — SUPPORT-AND-EOL.md já existe.
- VERIFY (server TL): residency "US-only (enam)" em PRIVACY/DPA/GUIDE§21/DR§8 pode ter sido superado (5 envs regionais live); atualizar em lockstep se sim.
- ARCHITECTURE.md:117-143 dataflow de `clw run` descreve output-file capture + manifest-keyed cache que NÃO existem (key real = input digests+env+cmd+tool-identity+cwd; só stdout/stderr/exit memoizados).
- ARCHITECTURE.md:157-160 "shared across tenants" vs GUIDE:138-140 "no cross-tenant dedup" — GUIDE correto.
- INSTALL.md:61-66 descreve installer baixando minisign.pub — trust root é EMBUTIDO (install.sh:40-44); INSTALL reabre no papel o buraco TOFU que o script fechou; omite CLW_SKIP_SIGNATURE.
- SECURITY/THREAT-MODEL/REVIEWER-BRIEF over-statement: "qualquer `..` em symlink target rejeitado" — desde #127/#133 `..` intra-workspace é PERMITIDO (validate_symlink_target_contained); GUIDE:1264 correto.

LOWs: tap-name arqueologia (3 grafias; canônica hugr-labs consistente; 3 HANDOFFs sem banner de org-rename); CHANGELOG [0.1.11] data 07-13 vs tag 07-16; README tabela config incompleta (--ref-domain, -v/-q, CORELINK_* aliases, CLW_OP_DEADLINE_SECS, CLW_CACHE_BUDGET_BYTES) + "auth rotate stub" reframed; SPEC vs Windows shipped (mode 0o644 constante, mask setuid ausentes do SPEC; validação target SHOULD vs MUST real); README gates lista só 3 lanes (CONTRIBUTING correto); CHANGELOG sem links por versão; sem man pages (clap_mangen 1 step); GUIDE:194 "or" deveria ser "and" (bypass installer requer AMBOS); CHANGELOG 2× heading [Unreleased] (linha 3 e 623); AUDIT-2026-06-09 H1 com entity HTML literal; usage.md:98/GUIDE:1151 sample "2025-06-04" typo ano; coverage.yml job name "llvm-cov (report-only)" MENTIRA + é dependência de nome no pre-merge-gate-check.sh (rename precisa coordenação, documentado em lugar nenhum).

ARCHIVE (MED): 91 arquivos flat, ~67 = correspondência point-in-time (78%), tudo junho/2026, afogando ~19 evergreen. Só 2 correspondências são referenciadas por docs evergreen (REPLY-to-runners-TL-2026-06-16 ← CLI-CONTRACT; REPLY-clw-TL-toolchain-resolver-seam ← EXIT-FREEZE). Proposta: docs/README.md índice (status LIVING/FROZEN/ARCHIVE + owner + last-reviewed) + docs/archive/{2026-06-correspondence,handoffs,findings,go-live} + runbooks/. git mv preserva blame. Padrão de banner SUPERSEDED já existe no repo — aplicar uniforme.

GAPS: README não linka GUIDE.md (melhor doc do repo, 1368 linhas) nem INSTALL.md na seção Documentation; sem docs/RELEASE.md (runbook de release — bus-factor doc, prometido em WP-BRIEFS M2.3); sem docs/SUPPLY-CHAIN.md (M3.2); man pages; error-code→remedy index unificado. N/A calibrado: CoC, migration guide, versioned site. Completions + per-subcommand docs + FAQ presentes (GUIDE genuinamente SOTA).

QUALITY (MED): docs vivos sem header status/data/owner (TESTING, THREAT-MODEL, ARCHITECTURE, SECURITY, DISTRIBUTION, TEST-JOURNEY-MAP, usage) — exatamente os achados stale; os datados envelheceram bem.

CHANGELOG ✓ estrutura completa 0.1.0→0.1.11 + [Unreleased] preciso. Entidade legal ✓ consistente (Human Guardrail LTDA, 6 docs; nit: EULA único sem CNPJ).

Top-5 docs: THREAT-MODEL rewrite; INSTALL Authenticode+attestation; SECURITY↔SUPPORT-EOL; CLAUDE.md CI/toolchain; TESTING.md refresh.
