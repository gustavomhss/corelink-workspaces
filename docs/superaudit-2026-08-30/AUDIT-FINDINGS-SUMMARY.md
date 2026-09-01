# Estado da sessão — superaudit clw (2026-08-30)

> **ARCHIVAL SNAPSHOT — NOT CURRENT STATE.** This file records the audit session against `25cfcaf`.
> “Complete”, “open”, “green”, counts and next steps below are contemporaneous claims, not the campaign's
> live status. The `scratchpad/...`, `memory/...`, `MEMORY.md` and worktree references are dead
> session-local pointers retained only as provenance. Use
> `../HANDOFF-2026-08-31-clw-TL-superaudit-campaign.md` and live git/GitHub state before acting.

## O que esta sessão fez (COMPLETO — nada pendente de execução)
Superaudit integral do repo corelink-workspaces @ 25cfcaf (= origin/main, worktree limpo, NADA modificado no repo — auditoria read-only).
Método: 7 subagentes paralelos (core crates / net crates / cli+test crates / CI+scripts / docs / security posture / deps+build) + checks mecânicos (fmt ✓, clippy ✓, cargo-deny ✓, cargo-audit ✓ 0 vulns/274 deps, shellcheck, actionlint) + verificação live de estado externo (mirror, branch protection API, run history).

## Entregável
- Relatório publicado como Artifact: **https://claude.ai/code/artifact/eb5c1690-ff83-48b4-92dd-f7338833d894** (título "Superaudit clw", favicon 🔬)
  - **Dead session-local pointer:** `scratchpad/superaudit-clw.html` was the source HTML; it is not in this repo.
- **Dead session-local pointers:** `memory/clw-superaudit-2026-08-30.md` and `MEMORY.md` are not in this repo.

## Placar final
0 CRITICAL · 8 HIGH · 31 MED · ~70 LOW/INFO. Nenhum HIGH em código Rust — código central "unusually well-hardened" (4 auditores convergiram). Problemas reais: governança/CI/docs.

## Os 8 HIGH (resumo)
1. **Branch protection de main OFF** — `protected:false` via API; org HuGR-Labs plano FREE (protection/rulesets 403 plan-gated em repo privado); perdida na migração de org #229 (agosto); CLAUDE.md ainda declara vigente.
2. **53 commits sem release desde v0.1.11** (tag 2026-07-16; mirror confirma v0.1.11 latest) — inclui bump h2 RUSTSEC-2026-0258 (#239) NÃO shipped. Ação: v0.1.12-rc.1 → v0.1.12.
3. **0/33 actions SHA-pinned**; taiki-e/install-action@v2 instala cargo-zigbuild no caminho do binário de release (release.yml:403).
4. THREAT-MODEL.md: "no erasure/non-compliant" falso (rm/prune/erase shipped); auto-contradição M1.6; "zero unsafe" falso (1 bloco DACL clw-cli/config.rs:768-885); nenhum crate tem forbid(unsafe_code).
5. INSTALL.md exige Authenticode 'Valid' — todo .exe unsigned (waiver).
6. INSTALL+DISTRIBUTION prometem gh attestation — skipped (waiver privado); release notes (release.yml:933) também mentem "SLSA provenance".
7. CLAUDE.md stale: "github-hosted ubuntu" (tudo é corelink desde #233), receita ~/.clw-ci-rust morta (atual: scripts/ci-use-host-toolchain.sh), topologia release antiga (hoje 5 targets em 1 Mac via zigbuild), aponta handoff 06-28 c/ verify.sh inexistente.
8. SECURITY.md (latest-only, sem backport) vs SUPPORT-AND-EOL.md (security-only 90d + backport 7d) — políticas contraditórias.

## MEDs-chave (31 total; detalhe completo nos arquivos abaixo)
- Operação: mutation lane vermelha 4/6 últimas semanas (report-only, ninguém notificado); e2e-evidence parada desde 07-20 (manual-only); SPOF runner corelink reintroduzido (#233, custo deliberado); deadman vigia só 1/6 lanes + roda no mesmo fabric.
- Release: sem assert tag-ancestral-de-main; minisign.pub fail-open (`|| true`) + gates.sh docs-only skip cobre o trust root (PR só tocando minisign.pub pula gates; sem check de igualdade com install.sh:79 — igual hoje).
- CI: CLW_FUZZ_GATE nunca wired + fuzz/Cargo.lock stale (--locked falha); permissions: ausente nos 5 workflows com secrets; trio CLW_CRED_TICKET/LEASE/FABRIC visível p/ código de PR no job gates (VERIFY c/ runners TL).
- Código: env CLW_* VAZIA tratada como presente (CONFIRMADO live; Bearer vazio → auth failed; fix filter !is_empty pós-clap); e2e Runner não isola env ambiente (só RUST_LOG); Marina não crash-reentrante (orders-api fixo + --force Beat 6); live-suites vazam CAS blobs+memo no tenant canário (origem dos foreign do #238); erase/prune seriais (~1.2s/obj → GDPR-erase horas; ls ganhou buffer_unordered no #221, deletes não); clw-types: symlink target ancestral (d/up->..; Windows copy-fallback auto-referencial), escape via symlinks irmãos, case-fold/NFC collision, nomes Windows reservados aceitos; manifest walk honra gitignore GLOBAL+parents (digest não-determinístico entre máquinas; fix .git_global(false)+.parents(false)); cache eviction FIFO não LRU documentado (get/re-put não refrescam mtime → thrash dos blobs quentes).

## Arquivos de estado nesta scratchpad (detalhe completo por área)
- findings-consolidated.md (orquestrador + deps)
- findings-core-crates.md · findings-net-crates.md · findings-cli-crates.md
- findings-cicd.md · findings-security.md · findings-docs.md
- mech-checks.log (fmt/clippy/deny/audit/shellcheck/actionlint raw)
- superaudit-clw.html (fonte do artifact)

## Fatos verificados úteis
- Canários verdes (conformance diário, fuzz nightly, deadman); sem PRs/issues abertos; tags até v0.1.11(-rc.1); mirror clw-releases latest = v0.1.11 (26 assets, 2026-07-17).
- Org plan: `gh api orgs/HuGR-Labs --jq .plan` → "free". Branch: `gh api repos/HuGR-Labs/corelink-workspaces/branches/main` → protected:false.
- GitHub Actions API sofreu rate-limit secundário intermitente durante a sessão (agentes compartilham quota) — espaçar chamadas.
- 1 request live a prod feito por subagente (verificação empírica env vazia; inofensivo). Nada mais tocou prod.

## Próximos passos (se o usuário pedir — plano de 5 ondas no relatório)
1. Cortar v0.1.12 (rc primeiro) + decidir postura de proteção de main (plano pago vs compensação documentada; atualizar CLAUDE.md).
2. PR de CI: SHA-pins + dependabot(github-actions) + tag-on-main assert + fechar fail-open minisign + permissions×5 + CLW_FUZZ_GATE + fuzz lock refresh + deadman matriz + anular trio no gates.
3. PR de docs: H4–H8 + forbid(unsafe_code) + docs/README.md índice + docs/archive/ (78% de docs/ é correspondência jun/2026; só 2 refs de docs evergreen: REPLY-to-runners-TL-2026-06-16 ← CLI-CONTRACT, REPLY-clw-TL-toolchain-resolver-seam ← EXIT-FREEZE).
4. PRs de código: os 11 MED de código (lista acima).
5. Backlog: mutation lane (consertar ou matar), corpus fuzz persistente, gaps de teste (Windows RUNS, 128+n child-signal, AC 409 live, fuzz dos validadores vivos, case-collision negativos).

## Convenções a lembrar
Commits com -s (DCO) + Conventional Commits; nunca force-push main; fluxo branch→PR→cold review→squash; fetch-first (clone local envelhece); worktree atual: .claude/worktrees/repo-comprehensive-audit-d3e415 (branch claude/repo-comprehensive-audit-d3e415 — sem commits próprios).
