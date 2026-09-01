# Superaudit clw — consolidação em progresso (2026-08-30, HEAD 25cfcaf = origin/main)

> **ARCHIVAL SNAPSHOT — NOT CURRENT STATE.** This consolidation records the audit at `25cfcaf`.
> Counts, live GitHub/release state and open/fixed claims below are contemporaneous evidence only. Consult
> `../HANDOFF-2026-08-31-clw-TL-superaudit-campaign.md` and the WP catalogue before acting.

## Achados do orquestrador (verificados por mim)

- HIGH (processo/release): 53 commits em main desde tag v0.1.11 (2026-07-16) SEM release — inclui fixes de usuário (#220 create-only client-side, #221 ls N+1, #209 classificação 5xx, #238 ls --all, #200 CLW_CACHE_BUDGET_BYTES) e bump de segurança h2 RUSTSEC-2026-0258 (#239) que NÃO está no binário publicado (brew/mirror ainda v0.1.11, confirmado no mirror público, 26 assets, 2026-07-17). v0.1.12 atrasada ~6 semanas.
- MED (resiliência CI): TODAS as lanes (incl. required `gates`+`dco` e release) voltaram para runner self-hosted único `corelink` (#233–#236, "zero hosted spend") — reintroduz o merge/release SPOF que #225 tinha matado. Decisão deliberada de custo, mas sem fallback documentado.
- LOW: lib.rs monolíticos — clw-client 3819 linhas, clw-run 2441, clw-hydrate 2374, clw-snapshot 1979, clw-cli/config.rs 1858, clw-cli/tests/e2e.rs ~164KB. Manutenibilidade/navegação.
- LOW: falta `.github/actionlint.yaml` declarando label `corelink` → actionlint inutilizável sem ruído (20 falsos positivos).
- INFO: shellcheck: gates.sh:131 SC2086 (split intencional de $SCOPE? verificar), pre-merge-gate-check-selftest.sh 4× SC2001 style.
- INFO: 12/200 commits recentes fora de Conventional Commits.
- Mecânicos: fmt limpo, clippy limpo, cargo-deny ok, cargo-audit 0 vulns (274 deps).
- VERIFY-EXTERNAL: GitHub Actions API rate-limited — não consegui listar saúde de runs recentes (canários verdes?) nem branch protection ao vivo. Sem PRs abertos, sem issues abertas. Tags até v0.1.11(-rc.1).

## Agente DEPS/SUPPLY-CHAIN (completo)

Veredito: postura muito disciplinada; nada HIGH.
- MED: fuzz/Cargo.lock STALE (`cargo metadata --locked` FALHA; clw-types 0.1.0 vs 0.1.11; cc/memchr/syn/wasip2 divergem) + gate deny/audit do fuzz (`CLW_FUZZ_GATE=1`, gates.sh:171-183) NUNCA roda em CI (fuzz.yml não seta) → grafo auditado ≠ grafo construído. Fix: refresh lock + `--locked` no build fuzz + setar CLW_FUZZ_GATE=1 em fuzz.yml.
- LOW: `anyhow` morto em workspace.dependencies (Cargo.toml:73) + entrada órfã no Cargo.lock (audita crate que não builda).
- LOW: `futures = "0.3"` inline em 4 crates (snapshot, hydrate, cli, integration-tests) em vez de workspace.dependencies.
- LOW: sem `rust-version` (MSRV) em nenhum manifest (toolchain pin 1.91.1 existe mas MSRV não machine-checkable).
- LOW: actions pinadas por tag mutável, não SHA: actions/checkout@v4, taiki-e/install-action@v2 (roda na release lane — edge mais afiado), upload-artifact@v4, attest-build-provenance@v1. Cargo tools SÃO pinados.
- LOW: tools flutuantes nas lanes aditivas: coverage.yml:52 cargo-llvm-cov sem versão, mutation.yml:54 cargo-mutants sem versão, fuzz.yml:59 cargo-fuzz sem --version.
- INFO: deny.toml:16,74 comenta "11 first-party crates" — são 12 (clw-e2e-evidence).
- INFO: wiremock+tokio como deps normais em clw-e2e-evidence (deliberado) → THIRD-PARTY-LICENSES.md superatribui deps de harness que o binário não linka.
- INFO: edition 2021 (2024 disponível); criterion 0.5 (0.7 atual); deny.toml:9 comentário `--all-features` vs gates.sh:91 invocação plain (redundante via [graph] all-features=true).
- INFO gitignore: falta `__pycache__/`; `.claude/settings.local.json` só ignorado via gitignore global da máquina (`*.local` não casa) — clone de outro contribuidor desprotegido.
- Inventário: 274 pkgs root lock (262 third-party); binário ~151 host crates; duplicatas só ecosystem-lag (getrandom trio, hashbrown, windows-sys ×3, r-efi, wit-bindgen); rustls puro (zero openssl); zero crates abandonados; deny.toml exemplar (yanked=deny, ignore=[], skip=[], sources crates.io-only); toolchain sem drift (release.yml:92-97 assert); about/NOTICE frescos e espelhados; sem junk commitado (maior arquivo docs/go-live/WP-BRIEFS.md 208KB).
