> **ARCHIVAL SNAPSHOT — NOT CURRENT STATE.** Captured against `25cfcaf` on 2026-08-30. Line numbers,
> live GitHub state, severities and open/fixed claims below are contemporaneous evidence only. Consult
> `../HANDOFF-2026-08-31-clw-TL-superaudit-campaign.md` and the WP catalogue before acting.

## Agente CI/CD + SCRIPTS (completo) — 0 CRITICAL, 1 HIGH.

HIGH: taiki-e/install-action@v2 (tag mutável, não SHA) em release.yml:403 instala cargo-zigbuild = ferramenta que PRODUZ os binários shipped, no host de release (tb ci:42, coverage:50, mutation:52, release:193,687). Tag comprometida = backdoor no binário assinado. 0/33 uses: pinados por SHA.

MEDs:
1. release.yml:46-49 sem assert de que o commit taggeado é ancestral de main — tag em commit nunca-revisado passa preflight, assina e publica (bypass do fluxo PR). Fix: git merge-base --is-ancestor + tag ruleset (VERIFY-EXTERNAL).
2. release.yml:798+873-878 trust-root FAIL-OPEN se docs/minisign.pub ausente (`[ -f … ] && cp … || true`; self-verify só ::warning::) — release real pode publicar minisig assinado por chave nunca provada contra o trust root, sem shipping do pub. Fix: hard-fail em real tag.
3. CLW_FUZZ_GATE nunca wired em workflow nenhum (confirma achado do agente deps).
4. permissions: ausente em ci, conformance-canary, e2e-evidence, live-journeys, release top-level (só publish job tem; 8/13 corretos). Exatamente os workflows com secrets prod. `contents: write` do publish desnecessário p/ attest.
5. canary-deadman.yml:40 vigia SÓ conformance-canary — metered-probe ($-ceiling!), fuzz, mutation, stress, live-journeys sem freshness guard. Canário que para de RODAR = cegueira sem red.
6. VERIFY: deadman roda no MESMO scheduler/fabric que o vigiado + alerta = email do GitHub → outage global = silêncio. Único deadman real = prober externo.
7. VERIFY: ci.yml gates executa código de PR no runner corelink que carrega trio ambiente CLW_CRED_TICKET/LEASE/FABRIC_ENDPOINT (conformance-canary.yml:82-94) — lanes prod anulam, gates NÃO anula → código de PR vê o ticket. Fix: anular no job ou fabric não injetar em leases CI.

LOWs:
- fuzz.yml:59 cargo-fuzz sem --version; mutation.yml:54 cargo-mutants unpinned (contrato de exit-codes 0/2/3 vs 1/4/70 depende de versão).
- fuzz.yml SEM persistência de corpus (todo nightly começa do zero, 5min/target — profundidade nunca acumula). Fix: actions/cache + seed corpus.
- mutation.yml:85 `grep -c … || echo 0` → n com 2 linhas quando arquivo vazio (summary malformado).
- mutation report-only p/ sempre: `--in-diff` Phase-2 prometido nunca wired — survivors não gateiam nada.
- e2e-evidence.yml:74,111 dirige binário DEBUG, não release — evidência "real binary" com perfil diferente do shipped.
- release.yml:628-631 minisign.key decodificado no WORKDIR do Mac compartilhado multi-runner (max-parallel 3) — janela de leitura por job concorrente mesmo user. Fix: $RUNNER_TEMP.
- release.yml:164-176 quota-guard parseia só 1ª linha do body (head -1 + grep) — JSON pretty-printed → falso METERED → bloqueia release real. Fix: jq.
- release.yml:136 versão via `grep -m1 '^version = '` — frágil a reordenação. Fix: cargo metadata|jq.
- release.yml:1126-1135 formula bump git push sem rebase/retry; concurrency group por-tag → 2 tags quase simultâneas podem deixar tap na versão VELHA com ambos verdes.
- release.yml:933 vs 1008 notes prometem "SLSA provenance" mas attestation gated a repo público (privado) → promessa nunca cumprida nos notes.
- gates.sh:64-67 docs-only skip trata docs/minisign.pub (trust root!) como inerte → PR só tocando trust root pula TODOS os gates; nenhum check byte-equal com install.sh:79 (igual hoje, verificado). Fix: escalar não-.md de docs/ + assert de igualdade.
- gates.sh:121-137 scoped lane: crates dependentes intermediários (mid-graph) não rodam unit tests próprios (nem changed nem sink) — gap real bounded, backstop = full lane em main. Fix: reverse-deps via cargo metadata.
- pre-merge-gate-check.sh:79-158 TOCTOU: checks lidos sem pin de head SHA antes de merge. Fix: --match-head-commit.
- family-e2e.sh:120 `"${missing[@]}"` vazio sob set -u = "unbound variable" em bash 3.2 (macOS stock) — morre no REPORT com matriz toda verde. Fix: idiom ${missing[@]+…}.
- provision-release-secrets.sh:69 assume cwd=repo root — rodado de fora, docs/minisign.pub stray, trust root não atualizado com secret rotacionado.
- release.yml:630,865 `echo "$MINISIGN_PASSWORD"` — backslash mangling. Fix: printf.
- ci.yml:61,63 `${{ github.base_ref || 'main' }}` interpolado direto em run: (padrão injection-prone; explorável só com write). Fix: via env.
- Sem dependabot.yml/renovate p/ github-actions.
- release.yml:641-647 retention-days:7 p/ artefatos assinados — janela de recovery curta. Fix: 30.

INFOs: coverage.yml header ainda "report-only Phase 1a" mas FLOOR=88 enforcing; ratchet manual atrasa ~0.8%/ciclo (auto-ratchet); canary-deadman `then=$()` booby trap; release.yml comments "Apple Silicon host" vs X64 real; provision script referencia release.yml:546 (rotted); metered-probe EP hardcoded vs secret; e2e-evidence manual-only (rots); windows.yml paths sem rust-toolchain.toml; cancel-in-progress em main pode cancelar full-lane intermediário (aceitável); mac/win sem assinatura por-binário (waiver documentado, promessa fail-closed registrada); dco --no-merges moot; registry/src compartilhado nunca re-verificado no Mac (defense-in-depth: wipe periódico); install.sh muito forte (residual: skip-signature checksum auto-referencial, opt-in).

VEREDITOS: release.yml = arquivo mais forte (fail-closed real verificado); nenhuma release parcial/não-assinada pode sair pelos caminhos escritos. Top fixes: SHA-pin taiki-e → tudo; assert tag-on-main; fechar fail-open minisign.pub; permissions nos 5; CLW_FUZZ_GATE; deadman p/ todas as lanes agendadas.
