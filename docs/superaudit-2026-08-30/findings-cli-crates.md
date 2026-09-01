> **ARCHIVAL SNAPSHOT — NOT CURRENT STATE.** Captured against `25cfcaf` on 2026-08-30. Line numbers,
> live service state, severities and open/fixed claims below are contemporaneous evidence only. Consult
> `../HANDOFF-2026-08-31-clw-TL-superaudit-campaign.md` and the WP catalogue before acting.

## Agente CLI/TEST CRATES (clw-cli, conformance, integration-tests, e2e-evidence) — completo. 0 CRITICAL/HIGH, 6 MED.
Nota: agente verificou empiricamente 2 suspeitas (1 refutada — process::exit flush ok; 1 CONFIRMADA com binário real, 1 round-trip live em prod, inofensivo: Bearer vazio → auth failed).

MEDs:
1. main.rs:28-74 + config.rs:84-149,285-288: env CLW_* VAZIA tratada como presente (≠ CORELINK_* que filtra vazio). CLW_TOKEN="" → request real com `Bearer ` vazio + "authentication failed" em vez de erro acionável "missing token" (CONFIRMADO live); CLW_ENDPOINT="" → "refusing plaintext (got '')" em vez de fallback default; CLW_TENANT="" idem. Secret ausente em CI aparece exatamente como var vazia (o próprio harness de conformance documenta e contorna, live_conformance.rs:56-60). Fix: filter(!is_empty) pós-clap.
2. e2e-evidence lib.rs:416-424: Runner NÃO isola env ambiente (só remove RUST_LOG) — CLW_REF_DOMAIN=runner ambiente flipa personas p/ keyspace runner; em box corelink o trio CLW_CRED_TICKET/LEASE/FABRIC empurraria persona pro caminho C2c e queimaria ticket single-use real. clw-cli tests strip 11 vars (e2e.rs:421-439); evidence (que roda contra PROD) não. Fix: env_remove namespace completo + HOME sandbox.
3. story_ship_service.rs: Marina não é crash-reentrante nem concurrency-safe — nome fixo `orders-api`; morte entre Beat 6 (--force v2) e Beat 11 (cleanup) → próxima run 409 create-only → journey vermelha sem bug real. Outras personas são reentrantes; Marina exceção. Fix: rm best-effort pré-flight ou sufixo por-run.
4. Cleanup do live-suite sistematicamente parcial: AC refs limpos, CAS blobs e run-memo NUNCA (ac-roundtrip-value-* foram exatamente as entradas foreign que forçaram o classificador do #238!). ~5MiB únicos/fixture/run + memo records — tenant canário acresce a cada run agendada; em tenant metered = dinheiro real. Fix: delete blobs conhecidos + AC value + memo key.
5. erase.rs:253-296 + prune.rs:157-169: sweeps destrutivos SERIAIS (1 round-trip/objeto, até 3 passes) — com floor ~1.2s/req, erase do exemplo de usage.md ≈ 10min; GDPR erase de tenant grande = horas. ls --all ganhou buffer_unordered(16) no #221; deletes não. Fix: mesma concorrência bounded por fase.
6. CLI-CONTRACT.md CLW_REF_DOMAIN "PROPOSED" (duplica achado docs — doc de contrato que consumidores pinam).

LOWs:
- main.rs:363-384 command_redeems_credential inclui init/auth login/rotate (zero rede) — queima ticket C2c single-use que o workload precisa; auth login pós-redeem persistiria PAT curto no config.toml. Violação de invariante declarada.
- ls --name emite "bytes" vs snapshot/hydrate "bytes_total" — inconsistência JSON.
- snapshot --json omite secret_warnings (humano tem no stderr; precedente skipped_external_symlinks existe).
- rm.rs:95-100 GET-before-DELETE TOCTOU + round-trip extra (server ask; comentário).
- rm.rs:52-57 msg "64-char lowercase hex" mas uppercase aceito.
- clw-cli Cargo.toml feature Win32_System_Memory MORTA (FIX 5 substituiu por Vec<u64>; grep zero usos).
- auth rotate exit 0 fazendo nada (rotated:false no JSON mas exit é o contrato; declined rm/erase são non-zero).
- --json ignorado silenciosamente em completions (run tem hint P6; completions não).
- conformance: cache dirs hardcoded /tmp/clw-*-cache compartilhados (colisão multi-user, não-hermético; resto usa tempfile).
- live_journeys.rs:721-727 cred_ticket redeem: QUALQUER falha soft-skip ("maybe burned") — regressão client-side nunca avermelharia. Fix: distinguir 410 de transport/4xx.
- e2e.rs adversarial: asserts wall-clock <20s load-sensitive (2× headroom fino). Fix: derivar do deadline.
- ls --all concorrência 16 hardcoded; --concurrency documentado como snapshot/hydrate-only — sem knob p/ 429.
- config.rs:445-471 chave de config com typo silenciosamente ignorada se ≥1 chave conhecida parseia (endpont= → default prod sem aviso). Fix: warn por chave desconhecida.

INFOs: marker_lines usize::MAX sentinel — hit-branch MAX==MAX espúrio; smoke-offline cache sem pid-suffix; expand_tilde só ~/ (não ~ nem ~user); integration-tests deps em [dependencies] em vez de dev; e2e.rs #![cfg(unix)] → legs do contrato de exit-code (125/137/verbatim/128+sig) SEM cobertura Windows no gate required (windows.yml só e2e_portable); hostile.rs DELETE responde 200 vs contrato 204; DUPLICAÇÃO: is_capability_unavailable/map_auth_error 4 cópias (rm.rs reimplementa local), harness wiremock quadruplicado (~700 linhas deletáveis via tests/common), json_str/json_u64/is_hex64/short/dirs_identical copiados em 4-5 story files.

SÓLIDO: exit-code freeze enforced end-to-end (constantes single-source, panic hook com seam de injeção debug-only, provado com binário real); secrets hygiene defense-in-depth não-vácua (leaked_secret computado no RAW pré-scrub); disciplina destrutiva uniforme adversarialmente testada; pirâmide de testes ataca própria vacuidade (pass-2 de-vacuous, .expect(1) no-retry-storm, 429 positive control); evidence suite artefato real (full-shape JSON, replica-lag polling, renderer HTML-escape).

TEST GAPS: clw-cli — bare `clw ls` guidance, completions purity, auth logout --all real, uninstall dry-run humano, status --json dirty, prune humano, TODOS os caminhos TTY interativos (sem pty em lugar nenhum), single -v, Windows exit-contract; conformance — AC create-only 409 live nunca provado (a semântica que o UX de re-snapshot inteiro depende!), paginação multi-página live, 402/413 fora do quota journey; integration — 3 suites HTTP + property #![cfg(unix)] zero Windows lib-level; e2e-evidence — J8 pendente (conhecido), hydrate --manifest-digest happy path sem persona, corrupt-body integrity só integration-level.

DRIFT CLI vs DOCS: hydrate imprime `source:` vs README:132/usage.md:72 `name:`; README config table incompleta (--ref-domain, CORELINK_*, -v/-q, CLW_OP_DEADLINE_SECS — knob que o próprio erro do client manda subir, em NENHUM doc de usuário); usage.md:537 mostra prefixo "transport error:" já corrigido; usage.md:546 "storage → 429" contradiz contrato 402 non-retryable do código; usage.md ls --all --json shape incompleto (foreign_ac_entries/skipped/errors); rm --ref-key e hydrate --manifest-digest sem receitas em usage.md; README "every command except run" — completions tb sem JSON; split P5 stdout/stderr documentado só no fonte; erase example sem sufixo "; tenant verified empty" (a claim de verificação GDPR).
