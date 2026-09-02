# Catálogo executável de WPs — v3

> **OPERATIONAL SPECIFICATION.** For WPs that have not landed, this catalogue defines the intended
> statement, preconditions, change, invariants, tests, acceptance and rollback. It is not the completion
> ledger: use `../HANDOFF-2026-08-31-clw-TL-superaudit-campaign.md` plus freshly fetched git/GitHub state
> for status. All line numbers and external preconditions were measured at `25cfcaf` and must be
> revalidated before implementation.

**Objeto:** `HuGR-Labs/corelink-workspaces` @ `25cfcaf`
**Procedência:** superaudit (117 achados) → plano v1 (47 WPs) → 5 cold reviews adversariais (58 achados, 7 BLOCKER) → v2 (correções estruturais) → **v3: revisão individual de cada WP, com verificação no repo por WP.**

**O que a v3 acrescenta.** A v2 reescreveu os WPs que a review derrubou e deixou ~30 listados de forma compacta em §4, sem reexame. Foi exatamente esse padrão — WPs sem as 7 cláusulas — que escondeu os BLOCKER na v1. A v3 escreve **todos** com o esquema completo e verifica cada um contra o código. Achados novos surgidos nesta passada estão marcados **[v3]**.

## Contrato normativo do catálogo

Esta seção é autocontida: nenhuma versão anterior do plano é necessária para executar um WP.

### Esquema de um WP

Cada WP tem sete cláusulas, em ordem fixa, nenhuma opcional:

| Cláusula | Símbolo | Conteúdo |
|---|---|---|
| Statement | **S** | O que é verdade depois, escrito no presente como invariante. |
| Precondition | **P** | O que precisa ser verdade antes; se for falso, o WP não começa. |
| Change | **Δ** | Mudanças exatas, em nível de arquivo/linha quando conhecido. |
| Invariant | **I** | O que não pode mudar; precisa constranger a parte perigosa de **Δ**. |
| Test | **T** | Obrigação de teste; “nenhuma” só é válida para mudança puramente documental. |
| Acceptance | **A** | Critérios executáveis por máquina ou comando reproduzível. |
| Rollback | **R** | Como reverter com segurança e o que não volta ou fica inconsistente numa reversão parcial. |

O cabeçalho declara **Closes**, **Depends** (inclusive dependências impostas por arquivos compartilhados),
**Risk** e **Class**.

### Invariantes globais G1–G9

Valem para todos os WPs. Uma violação reprova o PR mesmo quando o seu próprio **A** passa.

- **G1 — Contrato de exit-code congelado.** Valores e condições de emissão não mudam.
  `docs/CLI-SURFACE-EXIT-FREEZE.md` é normativo: exit do filho verbatim; `125` reservado para filho que
  não rodou; `137` para processo morto por limite de output; `128+n` por sinal; non-zero nunca cacheado.
  Um sinal entregue ao grupo de processos do clw continua alcançando o filho.
- **G2 — Superfície CLI congelada.** Nomes de flags, subcomandos e chaves JSON existentes não são
  renomeados nem removidos, e o significado de uma chave existente não muda. Adicionar uma chave é
  permitido; quebrar consumidores que pinam nome ou semântica não é.
- **G3 — Determinismo do digest.** Nenhuma mudança altera o digest de um manifest para o mesmo conteúdo
  de workspace, exceto onde o catálogo declara isso expressamente (WP-32 e WP-41a), com CHANGELOG,
  migração e coordenação cross-team antes do merge.
- **G4 — Fluxo de merge.** Branch curta → PR → cold review independente → gate FULL local verde →
  squash-merge. Commits usam `-s` (DCO) e títulos Conventional Commits. Nunca `--no-verify`, force-push
  ou push direto em `main`.
- **G5 — Integridade de release.** Nenhum caminho publica binário não assinado, release parcial ou trust
  root ausente numa tag real. Mudança em `release.yml` não merge sem um `vX.Y.Z-rc.N` dry-run verde,
  próprio ou de lote, antes da próxima tag real. Um caminho pulado hoje não pode ser deixado defeituoso
  para quando sua condição voltar a ativá-lo.
- **G6 — Fail-closed sobre fail-open.** Diante de condição inesperada ou evidência ausente, falhar alto é
  preferível a continuar em silêncio. Uma correção não converte rejeição, falha de segurança ou resultado
  desconhecido em skip/sucesso silencioso.
- **G7 — Teste red-on-revert.** Toda correção de código entra com pelo menos um teste que falha quando a
  correção é revertida. O teste precisa provar a propriedade e sua própria não-vacuidade, não apenas obter
  um run verde.
- **G8 — Legibilidade de artefato armazenado.** Nenhuma mudança torna ilegível por um clw posterior um
  `Manifest`, `RefRecord` ou `RunRecord` já gravado num tenant. G3 protege o digest; G8 protege leitura e
  compatibilidade. Endurecimento que invalide artefato antes aceito é mudança quebrante e exige migração,
  CHANGELOG e eventual bump de versão do formato.
- **G9 — Segurança do caminho destrutivo.** Nenhuma deleção ocorre sobre live set não provado. Preservam-se
  dry-run por default, recusa de wipe total, abort fail-safe quando a marcação é incompleta e veracidade das
  contagens de progresso parcial. O conjunto exato de alvos precisa ser inspecionável antes da deleção.

### Convenções de execução e definição de pronto

- Branch: `claude/wp-NN-<slug>`. Título: `<tipo>(<escopo>): <resumo> (WP-NN)`, com tipo em
  `feat|fix|chore|docs|ci|refactor|test`.
- Antes de implementar, buscar `origin`, revalidar **P**, dependências, linhas e o diff real contra
  `origin/main`. Stage somente arquivos nomeados; nunca `git add -A`.
- **A** verifica integralmente; `bash scripts/gates.sh` passa na lane FULL; o cold reviewer é diferente do
  implementador e confere **A**, os invariantes e o diff completo; então o PR pode ser squash-merged.
- Um WP só está pronto quando o merge está confirmado pelo estado git/GitHub fresco e pelo header de estado
  da campanha. Um run verde, branch pronta ou review em andamento não conta como landed.
- Mudanças de digest são contrato entre repositórios: o time de runners fixa `toolchain_ref` ao digest do
  manifest. WP-32/WP-41a exigem coordenação com esse TL antes do merge, não só nota posterior.

---

# SESSÃO 1 — Verdade de release e desbloqueio

---

## WP-00 — Remover a claim falsa de SLSA das release notes

**Closes:** H6 (metade de código) · **Depends:** — · **Risk:** NENHUM · **Class:** ci/texto

**S.** O texto que toda release publica descreve o que ela realmente traz.

**P.** Nenhuma.

**Δ.** `release.yml:933`: na string de `--notes`, remover "SLSA provenance". Fica: "signed binaries (minisign SHA256SUMS.minisig + minisign.pub), CycloneDX SBOM". Reintroduzir só quando a attestation deixar de ser pulada (hoje gated a repo público em `:1008`, waiver do dono de 2026-06-18).

**I.** Nenhum comportamento muda — só a string publicada. Não toca nenhum passo condicional.

**T.** Nenhuma; é texto. Sem obrigação de rc: não altera lógica.

**A.**
1. `sed -n '933p' .github/workflows/release.yml | grep -c SLSA` → `0`.
2. As notas da v0.1.12 (WP-01) não mencionam proveniência — conferir no mirror após publicar.

**R.** Reverter o commit. Volta a publicar a claim falsa; nada mais.

**[v3] Nota de escopo.** `grep -n SLSA release.yml` dá 5 hits: `:733` e `:989` são comentários descritivos do passo, `:1003` é o texto do waiver, `:1008` é o nome do step. Só `:933` é **texto publicado ao usuário**. Os outros quatro ficam — descrevem corretamente um passo que existe e está gated. O WP toca **uma** linha.

---

## WP-0A — Extrair os guards do `release.yml` para função testável

**Closes:** habilita WP-04, WP-05, WP-13b · **Depends:** — · **Risk:** BAIXO · **Class:** ci/refactor

**S.** As decisões de segurança do pipeline de release são funções puras, testadas por unidade em todo PR, sem tag.

**P.** Nenhuma. É refatoração sem mudança de comportamento.

**Δ.**
1. `scripts/release-guards.sh`: `release_kind <tag>` → `real|rc|invalid` (a lógica hoje inline em `release.yml:125`, `grep -Eq -- '-rc\.?[0-9]+$'`); `require_trust_root <kind> <path>`; `require_selfverify_ok <kind> <rc>`; `assert_tag_is_ancestor <sha> <base>`; `parse_quota_body <json>`.
2. `release.yml` chama as funções.
3. `scripts/release-guards-selftest.sh` + job em `ci.yml`, espelhando o `gate-check-selftest` que já existe em `ci.yml:65`.
4. Definição **única** de "release real" no arquivo.

**I.** G5, G6. Refatoração pura: as mesmas entradas produzem os mesmos resultados. Nenhuma política muda neste WP — as mudanças de política são WP-04 e WP-05, e chegam depois, já testáveis.

**T.** O selftest, em todo PR:
```
A1 release_kind v0.1.13 == real
A2 release_kind v0.1.13-rc.1 == rc  e  v0.1.13-rc1 == rc   (as duas grafias que o regex aceita)
A3 require_trust_root real /inexistente -> exit 1, stderr nomeia "minisign.pub"
A4 require_trust_root rc   /inexistente -> exit 0
A5 require_selfverify_ok real 1 -> exit 1
A6 assert_tag_is_ancestor em repo sintético: commit de main -> 0; órfão -> 1 ("not an ancestor");
   branch não-mergeado -> 1
A7 assert_tag_is_ancestor com origin/main inalcançável -> exit 1 (FAIL-CLOSED)
A8 parse_quota_body pretty-printed E minificado -> mesmo veredito
A9 reverter o ramo estrito -> A3, A5, A6 viram exit 0
```
A7 é a falha mais provável do mundo real (fetch falhando e desarmando o guard em silêncio) e não existia em nenhuma versão anterior do plano.

**A.** O job existe em `ci.yml`, roda em todo PR, verde · A9 demonstrado num commit de rascunho, link no PR · `grep -c 'release_kind' release.yml` mostra chamada, não redefinição.

**R.** Reverter. Volta a lógica embutida e não testável.

**[v3] Risco de refatoração que o WP precisa assumir.** Extrair lógica de um arquivo de 1030 linhas que só roda por tag é a definição de mudança não exercitável — o próprio problema que o WP resolve. Mitigação: o PR faz **apenas** extração (nenhum `if` muda de sentido), e o rc da sessão 1 (que WP-01 já ia gastar) confirma o caminho tolerante ponta a ponta antes de WP-05 endurecer o estrito.

---

## WP-05 — Trust root fail-closed na release real

**Closes:** M5 · **Depends:** WP-0A · **Risk:** BAIXO · **Class:** ci/release

**S.** Numa tag real, a ausência de `docs/minisign.pub` ou a falha de verificação de `SHA256SUMS` contra ele aborta antes de publicar.

**P.** WP-0A merged (senão não há como provar o ramo estrito sem empurrar tag real).

**Δ.**
1. `release.yml:798`: `[ -f docs/minisign.pub ] && cp … || true` passa a chamar `require_trust_root "$KIND" docs/minisign.pub` — hard-fail em `real`, tolerante em `rc`.
2. `:873-878`: o ramo `else` que emite `::warning::` passa por `require_selfverify_ok`, que sai 1 em `real`.
3. **[v3] Decisão explícita sobre os vizinhos.** As linhas `:799` e `:800` fazem o mesmo padrão `|| true` para `EULA.md` e `NOTICE`. Recomendação: **mesmo tratamento** — os dois são obrigações de licenciamento numa release real, e uma release sem EULA é um problema jurídico, não cosmético. Se ficarem tolerantes, o comentário diz por quê. A v2 deixava isso como "decidir no PR"; a v3 recomenda.

**I.** G5, G6. Nenhum caminho hoje fatal vira tolerado. O ramo `rc` continua tolerante — é o que permite dry-run sem segredo.

**T.** Coberto por A3/A4/A5/A9 do selftest de WP-0A. **Nenhuma dependência de rc para provar o ramo estrito** — que era o defeito BLOCKER da v2.

**A.**
1. Selftest de WP-0A verde com os casos estritos.
2. `grep -c '|| true' release.yml` **na saída de `git diff`**, não "na região" — a v2 pedia um limite que nenhum comando define. Forma correta: o diff do PR não deixa nenhum `|| true` nos três `cp` de staging.
3. Nenhum `::warning::` no passo de self-verify (contar no diff, não no arquivo — há 17 no arquivo, a maioria legítimos).
4. rc de lote verde (por G5, não como prova do ramo estrito).

**R.** Reverter + novo rc antes da próxima tag real.

---

## WP-04 — Assert de que a tag é ancestral de `main`

**Closes:** M4 · **Depends:** WP-0A · **Risk:** BAIXO · **Class:** ci/release

**S.** Uma tag real cujo commit não seja ancestral de `origin/main` reprova no preflight, antes de qualquer build, assinatura ou publicação.

**P.** WP-0A merged.

**Δ.**
1. Preflight chama `assert_tag_is_ancestor "$GITHUB_SHA" origin/main`, hard-fail em `real`.
2. Tags `-rc.N` **isentas**, explicitamente e comentado (o rc existe para exercitar o pipeline, inclusive de branch).
3. **[v3] O Δ.3 da v2 sai.** Ele mandava "verificar e ajustar `fetch-depth: 0`" — já é `fetch-depth: 0`. O WP tinha sido escrito sem ler o arquivo; é justamente esse tipo de item que faz o revisor perder confiança no resto.

**I.** G5. Só adiciona barreira: nenhum caminho que passava deixa de passar, exceto o que se quer barrar.

**T.** A6 e A7 do selftest de WP-0A, em repositórios sintéticos locais — grátis, offline, em todo PR. **A v2 usava como controle negativo empurrar uma tag real num commit não-ancestral: se o assert estivesse sutilmente errado, o pipeline seguiria pelo caminho de release real e publicaria no mirror público como `latest`, bumpando a fórmula. Não se testa o guard que protege tags reais com uma tag real.**

**A.**
1. A6/A7 verdes; A9 (reverter → A6 vira 0) demonstrado.
2. No pipeline real: o passo aparece no job de preflight **antes** de qualquer job com `needs:` de build — verificável no grafo do run do próximo rc.

**R.** Reverter + novo rc.

**Nota de complementaridade.** Se WP-02 seguir o caminho (a), há também ruleset de tag no servidor — redundância intencional: um controle no servidor, um no pipeline. Se seguir (b), **este WP é o único controle existente** contra tag fora de `main`, e sua prioridade sobe.

---

## WP-01 — Publicar v0.1.12

**Closes:** H2 (as duas metades) · **Depends:** WP-00, WP-0A, WP-05, WP-04 · **Risk:** BAIXO · **Class:** release

**S.** A release mais recente é `v0.1.12` com `h2 ≥ 0.4.17`, publicada por um pipeline honesto e fail-closed; **e existe tripwire que torna visível o próximo lag de release**.

**P.**
1. `origin/main` verde no gate FULL.
2. Segredo minisign presente (senão a assinatura Linux hard-falha em tag real, por design de #182 — `release.yml:616`).
3. Tenant de CI unmetered — **e verificado de fato**: `:158` e `:182` emitem `::warning::` e **pulam** se a chave de quota estiver ausente ou rotacionada, então a precondição pode ficar não-verificada com o run verde.

**Δ.**
1. CHANGELOG: promover `[Unreleased]` para `## [0.1.12]`, listando #220, #221, #209, #238, #200, #237 e o bump #239. **[v3]** O `[Unreleased]` atual já tem conteúdo real (a entrada do `pre-merge-gate-check.sh`) — a promoção move esse bloco, não cria um vazio.
2. `v0.1.12-rc.1` → observar → `v0.1.12`.
3. **Cadência:** regra em `docs/RELEASE.md` (WP-46) + linha `days-since-last-tag` na matriz do deadman (WP-09).
4. **Argumento de exploitabilidade do `h2`** registrado no PR: RUSTSEC-2026-0258 é explorável contra um **cliente** HTTP CLI? A maioria dos advisories de `h2` é exaustão de recurso contra servidor aceitando peer não-confiável. Se a exploitabilidade for baixa, a release continua certa — pelos outros 52 commits — mas a ordenação deixa de ser refém de uma urgência não demonstrada.

**I.** G5. O rc não bumpa fórmula, é `--prerelease`, é reversível.

**T.** O próprio rc.

**A. [v3 — enumeração corrigida contra os assets reais da v0.1.11].** A v2 dizia "26 assets, igual ou maior" — não detecta asset faltando (adicione um novo, perca o `.minisig`, o número fica). Conferi a lista real do mirror; o conjunto é:
```
5 binários:  clw-0.1.12-{aarch64,x86_64}-apple-darwin
             clw-0.1.12-{aarch64,x86_64}-unknown-linux-gnu
             clw-0.1.12-x86_64-pc-windows-gnu.exe
2 minisig:   clw-0.1.12-{aarch64,x86_64}-unknown-linux-gnu.minisig   <- Linux APENAS, por design
12 SBOM:     clw-0.1.12.cdx.json + clw-{cache,chunk,cli,client,conformance,hydrate,
             integration-tests,manifest,run,snapshot,types}.cdx.json
7 outros:    SHA256SUMS  SHA256SUMS.minisig  minisign.pub  install.sh
             EULA.md  NOTICE  THIRD-PARTY-LICENSES.md
= 26
```
1. Cada nome acima existe no release e tem `size > 0` (loop `gh release view --json assets`).
2. `sha256sum -c SHA256SUMS` passa sobre o conjunto baixado — o `minisign -V` prova que a **lista** está assinada, não que os binários batem com ela.
3. `minisign -V -p docs/minisign.pub -m SHA256SUMS` verde num download limpo.
4. `isDraft,isPrerelease` → `false,false`.
5. `h2`: `printf '0.4.17\n%s\n' "$got" | sort -V | head -1` == `0.4.17`.
6. Fórmula do tap em `0.1.12` com sha256 batendo com os assets.
7. Log do run contém `"CI tenant unmetered"` ou `"no quota row (404)"` — prova de que o guard executou.
8. A linha de cadência existe no deadman.

**[v3] Assimetria descoberta nesta passada.** Só os dois binários **Linux** têm `.minisig` individual (`release.yml:598-602`, passo "sign (Linux, minisign detached)"). Mac (Developer-ID) e Windows (Authenticode) estão waived, então para esses três binários a **única** cadeia de integridade é `SHA256SUMS` + `SHA256SUMS.minisig`. Isso é criptograficamente sólido — o manifest assinado cobre os binários — mas **muda o texto que o WP-17 precisa escrever**: em Windows e macOS a verificação suportada não é "verifique a assinatura do binário", é "verifique o checksum contra o manifest assinado". Nenhuma versão anterior do plano dizia isso.

**R.** Release não se reverte — corrige-se com a próxima; é para isso que o rc existe. Bump da fórmula e `latest` no mirror são irreversíveis na prática: quem der `brew upgrade` na janela leva o build ruim.

---

## WP-02-DEC / WP-02-IMP — Postura de proteção de `main`

**Closes:** H1 · **Depends:** — · **Risk:** MÉDIO · **Class:** governance

**WP-02-DEC** é decisão do dono, sai no dia 1, não bloqueia código (só a redação de WP-19).

**S.** Ou `main` está protegida com checks obrigatórios e ruleset de tag `v*`, ou a ausência é decisão registrada **com controle detectivo operante**.

**P.** Decisão entre (a) plano pago e (b) compensação documentada. **[v3]** A v2 afirmava "~US$4/usuário/mês" sem comando de verificação — num plano em que todo o resto carrega evidência reproduzível, a única afirmação sem evidência é justamente a que pede dinheiro ao dono. Antes de decidir: confirmar qual tier dá branch protection em repo **privado** (proteção clássica e rulesets têm gating diferente) e quantos assentos a org tem.

**Δ (a).** Upgrade; restaurar a configuração de 2026-07-09 (checks `gates`+`dco`, `enforce_admins`, histórico linear, force-push/deleção bloqueados); ruleset barrando criação de `refs/tags/v*` fora de `main`; CLAUDE.md e CONTRIBUTING afirmando o que passa a ser verdade.

**Δ (b).** Bloco "Controles de merge — sem proteção de servidor" nomeando os três controles preventivos (gate FULL local, `pre-merge-gate-check.sh`, convenção de não empurrar direto) **e dizendo em voz alta que nenhum é enforced** · decisão registrada com data e motivo · **workflow detectivo de $0**: `on: push: branches: [main]` afirmando que todo commit novo é squash-merge de PR com checks verdes, vermelho caso contrário. Roda no fabric existente, custo zero, converte convenção não-enforced em convenção **com alarme**. Substitui parcialmente WP-04 e WP-36.

**I.** G4. Nenhum caminho altera o fluxo declarado; (a) o torna enforced, (b) o mantém convencional e admite isso.

**T.** (b): commit de teste direto em `main` num **fork** dispara o alarme. (a): nenhuma.

**A. [v3 — critério impossível da v1 substituído].** A v1 exigia "um push direto de teste a `main` é rejeitado" — teste que G4 proíbe e que o destructive-guard bloqueia; o executor do plano não pode realizá-lo.
- (a): `gh api …/branches/main --jq .protected` → `true`; `…/protection --jq '.required_status_checks.contexts'` contém `gates` e `dco`; `enforce_admins.enabled` → `true`.
- (b): grep de **família** de termos (`branch.?protect|enforce_admins|required (status )?check|protected branch`) sobre `CLAUDE.md CONTRIBUTING.md README.md docs/*.md`, com a lista de hits colada no PR e cada um classificado como verdadeiro ou dentro do bloco de compensação. **A v1 grepava uma grafia num arquivo** — reescrever para "main é protegida" zerava o grep com a afirmação falsa de pé.
- Ambos: a postura viva registrada como comando + saída, datada, no doc — para que o próximo auditor rode uma linha em vez de re-derivar.

**R.** (a) reverte-se **baixando o plano** — o que reintroduz H1 em silêncio, por ação de billing, sem aviso. Se escolher (a), manter o alarme de (b) mesmo assim.

---

## WP-29-DEC — Decidir a garantia de containment de symlink

**Closes:** decide M18, desbloqueia M28 · **Depends:** — · **Risk:** decisão BAIXO · **Class:** decision

Vai ao dono no dia 1, junto de WP-02-DEC. A v1 sequenciava esta decisão para a sessão 7, **depois** do WP-15 (sessão 4) que escreve o parágrafo dependente dela.

**S.** Existe decisão registrada entre (a) endurecer o validador para resolver através dos symlinks do manifest, e (b) manter o containment lexical e rebaixar o texto ao que ele garante.

**Contexto para decidir.** O clw **nunca segue links** (`follow_links(false)` no walk; o hydrate cria sem seguir), então não há traversal de escrita hoje. O dano real é a **promessa errada** para consumidores da árvore hidratada que sigam links. (a) arrisca invalidar manifests existentes e é O(entradas × profundidade); (b) é uma frase precisa em três documentos.

**Recomendação:** (b), coordenada com WP-15, e (a) como backlog com dono.

**A.** Decisão escrita, datada, no PR de WP-15 ou num doc de decisão.

---

## WP-22 — Verificar a promessa de residência de dados

**Closes:** P1 · **Depends:** resposta do server TL · **Risk:** POTENCIALMENTE ALTO · **Class:** docs/external

A **pergunta sai no dia 1** — latência de dias. A v1 a agendava para a sessão 5, o que é tempo morto.

**S.** A promessa publicada em PRIVACY, DPA, GUIDE §21 e DR §8 corresponde ao que o servidor faz.

**Δ.**
1. Pergunta escrita ao server TL com **escopo explícito**: (i) localização dos dados de tenant **em repouso**; (ii) os 5 envs regionais **armazenam** ou só processam?; (iii) réplicas e backups; (iv) **destino de logs e telemetria** — que nenhuma versão anterior do plano pedia, e o trabalho de `audit_outbox`/Server-Timing sugere que cruza região.
2. Confirmada: registrar com data e atribuição nos 4 documentos.
3. Refutada: corrigir os 4 em lockstep e **escalar** — documento de peso legal errado é HIGH, não MED. Nomear a ação: os 4 são corrigidos **antes** da próxima release, e este WP a bloqueia.

**I.** Nenhuma alteração sem confirmação escrita da fonte. Adivinhar aqui é pior que não fazer nada.

**T.** Nenhuma.

**A. [v3].**
1. Resposta arquivada, datada, atribuída, cobrindo os 4 pontos de escopo.
2. **Uma corroboração independente**, não só a asserção: colo do `cf-ray` numa escrita, ou ponteiro para a config de binding (D1/R2) no repo do servidor. Uma linha de evidência, não zero.
3. **Data de re-verificação** em cada um dos 4 documentos ("residência confirmada em 2026-08-30 por \<papel\>; re-verificar até 2027-02-28"), para que a próxima auditoria reabra por agenda em vez de tratar confirmação velha como permanente.

**R.** N/A.

---

# SESSÃO 2 — Cadeia do `gates.sh`

Ordem obrigatória: **WP-06a → WP-06b → WP-14 → WP-14b → WP-03**. Todos editam `scripts/gates.sh`; a v1 declarava os três como independentes.

---

## WP-06a — Extrair `classify_lane` e criar o selftest do gate **(NOVO na v3)**

**Closes:** habilita WP-06b e WP-14b · **Depends:** — · **Risk:** BAIXO · **Class:** ci/refactor

**Por que existe.** As cláusulas **T** de WP-06 e WP-14(L35) apontavam para `pre-merge-gate-check-selftest.sh` — que testa o pré-check de **merge**, nada a ver com seleção de lane. **Não existe selftest do `gates.sh`**, e a lógica de lane é inline em `:52-75`, ativa só sob `CI=true` **e** `GITHUB_EVENT_NAME=pull_request`, chamando `git fetch`/`git diff` contra um `origin/<base>` real. Sem extração, os critérios dos dois WPs viram "um humano rodou à mão uma vez".

**S.** A decisão de lane é uma função pura sobre a lista de arquivos alterados, testada em todo PR.

**Δ.**
1. `classify_lane <arquivos separados por newline>` → `docs-only` | `full` | `scoped:<crates>`. `gates.sh` chama; nada mais muda.
2. `scripts/gates-selftest.sh` + job em `ci.yml` ao lado de `gate-check-selftest` (só bash, ~1 min).

**I.** A classificação produzida para qualquer entrada é **idêntica** à de hoje. Refatoração pura; as mudanças de política são WP-06b e WP-14b.

**T.** O selftest:
```
classify_lane "docs/FOO.md"                    == docs-only
classify_lane "README.md"                      == docs-only
classify_lane ""                               == full      (fail-safe de diff vazio)
classify_lane "Cargo.lock"                     == full      (superfície de contrato)
classify_lane "crates/clw-snapshot/src/lib.rs" == scoped, conjunto contém clw-cli e
                                                  clw-integration-tests (sinks)
```
**A.** Job existe, roda em todo PR, verde · a classificação de cada caso bate com o comportamento pré-refatoração (registrar a comparação no PR).
**R.** Reverter.

---

## WP-06b — Fechar o buraco docs-only e criar o tripwire do trust root

**Closes:** M6 · **Depends:** WP-06a · **Risk:** BAIXO · **Class:** ci

**S.** (i) Um diff que toque qualquer arquivo não-`.md` sob `docs/` roda a lane completa. (ii) Um drift entre `docs/minisign.pub` e a chave embutida em `scripts/install.sh` reprova o gate.

**Δ.**
1. `classify_lane`: docs-only passa a exigir que **todos** os arquivos sejam `.md` (dentro ou fora de `docs/`). Consequência: `docs/minisign.pub`, imagens e `.yml` sob `docs/` escalam para lane completa.
2. Check de igualdade byte a byte entre a linha 2 de `docs/minisign.pub` e `CLW_MINISIGN_PUBKEY_LINE` de `scripts/install.sh:79`, com mensagem explicando a consequência (usuários do curl-fallback deixariam de verificar).
3. **[v3] Posição obrigatória, verificada:** junto do dead-org-slug guard (`gates.sh:38-51`), que roda **antes** de `SCOPE="full"` (`:52`) e portanto antes do `exit 0` de docs-only. É o único lugar do arquivo onde um check roda em **todas** as lanes. A v2 dizia "roda sempre" sem dizer onde; a review mostrou que colocá-lo depois do bloco de SCOPE o tornaria inerte exatamente nos PRs docs-only.

**I.** G6. A lane docs-only continua existindo — muda sua fronteira, não sua existência.

**T.** No `gates-selftest.sh` (WP-06a):
```
classify_lane "docs/minisign.pub"   == full     <- o ponto do WP
classify_lane "docs/diagram.png"    == full
classify_lane "docs/algum.yml"      == full
classify_lane "docs/FOO.md"         == docs-only  (preservado)
reverter a regra -> os três primeiros voltam a docs-only  (red-on-revert)
```
**A.**
1. Os casos acima verdes.
2. **Estado atual: check de chave verde** — verifiquei que as duas chaves são byte-idênticas hoje, então o verde é significativo e não vácuo.
3. Mutação de 1 byte em `docs/minisign.pub` → gate vermelho com a mensagem nova. Controle negativo real.

**R.** Reverter. Volta o buraco.

---

## WP-14 — Higiene de CI e scripts

**Closes:** L27, L29, L36, L37, L38, L40, L59, I11, M34 (WP-11 absorvido) · **Depends:** WP-06a · **Risk:** BAIXO · **Class:** ci

**S.** Os scripts e workflows auxiliares não têm armadilhas conhecidas de shell, de parsing ou de configuração de linter.

**Δ.**
1. **L37** `scripts/family-e2e.sh:120` — `"${missing[@]}"` com array vazio sob `set -euo pipefail` (linha 40, verificado) é "unbound variable" no bash 3.2 do macOS de fábrica: o script **morre no relatório final com a matriz toda verde**. Usar `${missing[@]+"${missing[@]}"}`.
2. **L40** `ci.yml:61,63` — `${{ github.base_ref || 'main' }}` interpolado direto em `run:`; passar por `env:`.
3. **L29** `mutation.yml:85` — `grep -c … || echo 0` emite duas linhas com arquivo vazio.
4. **L59** criar `.github/actionlint.yaml` declarando o label `corelink`. **[v3] E fiar o actionlint na CI:** verifiquei que **actionlint não aparece em lugar nenhum** de `.github/` ou `scripts/` (0 ocorrências) — criar só o arquivo de config produziria um artefato que apodrece e um critério ("actionlint limpo") que um humano afirma uma vez. Entra na lane `shellcheck.yml`, renomeada `lint`.
5. **L27** pinar versão de `cargo-llvm-cov`, `cargo-mutants` (cujo contrato de exit-code depende da versão) e `cargo-fuzz`.
6. **L36** `pre-merge-gate-check.sh` com `--match-head-commit`, fechando o TOCTOU entre ler os checks e mergear.
7. **L38** `provision-release-secrets.sh:69` resolve caminhos por `git rev-parse --show-toplevel` em vez de assumir cwd.
8. **I11** anotar o `SC2086` intencional. **[v3] A referência da v1 (`gates.sh:131`) estava errada:** a linha 131 é comentário; `:134` e `:136` **já** têm `# shellcheck disable=SC2086`; a expansão não-aspada sem anotação é o `$SCOPE` do `crate_set` em `:132`. Descrever pelo conteúdo, não pelo número — WP-14b reescreve essa linha e moveria o número de novo.
9. **M34** rótulos de `coverage.yml`: nome do job `llvm-cov (report-only)` → `llvm-cov (floor 88%)`, texto do step, header "Phase 1a". **[v3] Mais o arquivo que a v2 não citava:** `scripts/pre-merge-gate-check-selftest.sh:28` **e** `:118` contêm a string exata `"llvm-cov (report-only)"` em `HEALTHY_CHECKS`, e esse selftest roda em todo PR (`ci.yml:65`). Sob a regra de substring nada quebra, mas a fixture deixa de espelhar a realidade e **nada pega o drift** — atualizar no mesmo commit.

**I. [v3 — a invariante mais importante deste WP].** **É proibido tocar `REQUIRED_PRESENT` em `pre-merge-gate-check.sh:134`.** A lista é `["dco","gates","llvm-cov"]` casada por **substring**, e o comentário em `:127-128` diz que isso é deliberado ("keeps this robust against job-name edits"). O WP-11 da v1 mandava "atualizar o nome esperado no mesmo commit" — o que **criaria** a dependência de string exata que o WP dizia temer, e o próprio arquivo avisa em `:130-133` que isso vira fail-noisy e leva o próximo apressado a desabilitar o check inteiro.

**T.** `shellcheck` limpo (fora os `disable` anotados) · `actionlint` limpo com o config novo · caso novo no `pre-merge-gate-check-selftest.sh`: um conjunto de checks contendo `{"name":"llvm-cov (floor 88%)"}` + gates + dco → exit 0; e um contendo `{"name":"coverage"}` (sem a substring) → exit não-zero, provando que o matcher ainda recusa rename que derruba a substring.

**A.** Lane de lint verde em todo PR (não "rodei uma vez") · `bash scripts/family-e2e.sh` sem credencial alguma chega ao relatório final sem morrer — que é exatamente o bug de L37 · os dois casos novos do selftest.

**R.** Itens independentes; reverter individualmente.

---

## WP-14b — Dependentes reversos na lane scoped

**Closes:** L35 · **Depends:** WP-06a, WP-14 · **Risk:** BAIXO · **Class:** ci

Extraído do WP-14 porque é **mudança de cobertura do gate obrigatório**, não higiene de shell: um revert conjunto derrubaria os fixes de shell junto.

**S.** Um PR que altere um crate intermediário roda os testes dos crates que dependem dele, não só os dele e os dos sinks.

**Δ.** `classify_lane` (WP-06a) passa a unir: crates alterados + sinks (`clw-cli`, `clw-integration-tests`, que `gates.sh:131` já dobra) + **dependentes reversos via `cargo metadata`**.

**I.** Só amplia cobertura. Nenhum PR que hoje roda um teste deixa de rodá-lo.

**T.** Caso no `gates-selftest.sh`: alterar `crates/clw-manifest/src/lib.rs` → o conjunto contém `clw-snapshot` e `clw-hydrate` (dependentes reversos), além dos sinks. Reverter → o conjunto encolhe (red-on-revert).

**A.** O caso acima verde · conjunto exato afirmado, não "contém pelo menos".

**R.** Reverter. A lane scoped volta a ter o buraco (com backstop da lane full em `main`).

---

## WP-03 — Pinar todas as actions por SHA e ligar Dependabot

**Closes:** H3, L41 · **Depends:** WP-01, WP-06a, WP-14 · **Risk:** BAIXO-MÉDIO · **Class:** ci/supply-chain

**S.** As 32 referências `uses:` estão pinadas por SHA de 40 caracteres, com a tag humana em comentário, e a invariante é mantida por máquina.

**P.** WP-01 merged (não tocar o pipeline antes de ele publicar a v0.1.12).

**Δ.**
1. Resolver o SHA da tag corrente de cada uma das 5 actions únicas via `gh api repos/<owner>/<repo>/commits/<tag> --jq .sha` e substituir todas as ocorrências: `actions/checkout@v4` (×18), `taiki-e/install-action@v2` (×6), `actions/upload-artifact@v4` (×6), `actions/download-artifact@v4` (×1), `actions/attest-build-provenance@v1` (×1).
2. `.github/dependabot.yml` com `package-ecosystem: github-actions`, semanal. **Deliberadamente sem `cargo`**: o repo já tem `deny`/`audit` no gate e a política de bump de deps é manual e revisada.
3. Assert no `gates.sh`, **acima** do `exit 0` de docs-only.

**Forma correta do assert — a da v1 tinha três defeitos independentes** (invertido; `\s` não é ERE POSIX e quebraria no grep do macOS, onde o gate FULL roda por convenção; e enganável por comentário com 40 hex):
```sh
bad="$(grep -rhoE '^[[:space:]]*-?[[:space:]]*uses:[[:space:]]*[^[:space:]#]+' .github/workflows/ \
       | sed 's/.*uses:[[:space:]]*//' \
       | grep -vE '^[A-Za-z0-9._-]+/[A-Za-z0-9._-]+(/[^@]+)?@[0-9a-f]{40}$' || true)"
[ -z "$bad" ] || { printf 'GATE FAILED: unpinned action(s):\n%s\n' "$bad" >&2; exit 1; }
count=$(grep -rcE '^[[:space:]]*-?[[:space:]]*uses:' .github/workflows/ | awk -F: '{s+=$2} END{print s}')
[ "$count" -eq 32 ] || { echo "GATE FAILED: uses: count drifted ($count != 32)" >&2; exit 1; }
```
O `sed` remove o comentário **antes** de casar; o `$` ancorado mata o match por substring; o assert de contagem faz um `uses:` **sumindo** também aparecer. Forma ancorada é segura hoje: 32 linhas, 5 actions únicas, zero refs `./` ou `docker://`, nenhuma composite action.

**I.** G5. Nenhuma versão efetiva muda — o SHA é o da tag em uso. **Este WP não é um upgrade.**

**T.** O assert é o teste. Controle negativo: commit de rascunho reintroduzindo `@v4` → gate vermelho.

**A.** `bad` vazio e `count == 32` · gate verde **com o assert executando** (colar a linha do log) · as 13 lanes que rodam em PR continuam verdes, prova de que os SHAs resolvem · o controle negativo demonstrado.

**R.** Reverter. Volta a tags mutáveis.

**[v3] Nota de manutenção.** O assert de contagem `== 32` precisa ser atualizado deliberadamente quando um workflow legitimamente ganhar ou perder um `uses:`. É intencional: torna a mudança visível em vez de silenciosa. Documentar isso na mensagem de erro.

---

# SESSÃO 3 — Observabilidade e permissões

---

## WP-07 — Ligar o gate do grafo de fuzz e sanear o lockfile

**Closes:** M7 · **Depends:** — · **Risk:** BAIXO-MÉDIO · **Class:** ci/supply-chain

**S.** O grafo de dependências que o fuzz constrói é o mesmo que `cargo-deny`/`cargo-audit` auditam, e o fuzz nightly falha se esse grafo violar a política.

**P.** Nenhuma.

**Δ.**
1. `fuzz/Cargo.lock`: refresh, para que `clw-types` acompanhe o workspace (hoje o lock fixa `0.1.0` contra `0.1.11` — verificado).
2. `fuzz.yml`: build com `--locked` (um lock stale reprova em vez de ser ignorado) e `CLW_FUZZ_GATE=1` no ambiente da lane.
3. **Se o gate não passar hoje**, o PR inclui o conserto — ou, se for grande, o WP para e abre um filho documentando o que falha. **Não é aceitável ligar o gate e deixá-lo vermelho.**
4. Nota de manutenção: o lock precisa de refresh junto de bumps de versão do workspace.

**I.** G6.

**T.** `CLW_FUZZ_GATE=1 bash scripts/gates.sh` verde localmente.

**A. [v3 — o critério da v1 era enganável por comentário].**
1. **Primeiro e principal:** `cd fuzz && cargo metadata --locked --format-version 1 >/dev/null` sai 0. É o único critério do WP que é **vermelho hoje** e verde só depois do fix — para dependência de path, o mismatch de versão faz `--locked` hard-falhar. Exemplar; liderar com ele.
2. **Igualdade exata de versão**, não inspeção visual: `test "$(grep -A1 'name = "clw-types"' fuzz/Cargo.lock | grep '^version' | cut -d'"' -f2)" = "$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)"`.
3. **O gate RODOU, não apenas existe:** o log do run de `fuzz.yml` contém a literal `"FUZZ supply-chain lane GREEN"`, que `gates.sh:185` já imprime. **A v1 usava `grep -c CLW_FUZZ_GATE fuzz.yml >= 1` — e a string já existe num comentário em `gates.sh:170`, então copiar esse comentário para o yml satisfazia o critério com o gate desligado.** Grepar o log do run, não o yml.
4. **Controle negativo:** dep com licença proibida em `fuzz/Cargo.toml` num branch de rascunho → lane vermelha. Sem isso, "o gate está ligado" e "o gate está ligado mas sua config permite tudo" são indistinguíveis.

**R.** Reverter. O grafo volta a não ser auditado.

---

## WP-08a — `permissions:` nos 4 workflows dispatcháveis

**Closes:** M8 (parte) · **Depends:** — · **Risk:** BAIXO · **Class:** ci/security

**S.** `ci`, `conformance-canary`, `e2e-evidence` e `live-journeys` declaram `permissions:` no topo, com escopo mínimo, e cada job que precise de mais eleva localmente.

**P.** **Executar, não só citar:** `gh api repos/HuGR-Labs/corelink-workspaces --jq '.default_workflow_permissions'`. Sem saber o default não se sabe o que se está restringindo. A v1 nomeava essa precondição e nunca a executava.

**Δ.**
1. `permissions: {contents: read}` no topo dos quatro.
2. Auditar cada job pelo que **realmente** usa o token: chamadas `gh`, `actions/*-artifact`, comentários em PR. O deadman precisa de `actions: read`.
3. Elevar por job, nunca no topo, com justificativa escrita.

**I.** Nenhuma lane passa a falhar. Hardening, não mudança funcional.

**T.** Dispatch manual dos quatro após a mudança.

**A. [v3].**
1. Assert estático no gate: `grep -L '^permissions:' .github/workflows/*.yml` vazio (cobre também os já corretos, e impede regressão futura).
2. **Tabela por job no PR**: cada consumidor do `GITHUB_TOKEN` mapeado ao escopo concedido. Uma chamada `gh` cujo escopo não foi concedido é defeito achado **por leitura**, já que rodar não acha — um passo que pula não consome permissão.
3. Os quatro rodaram verdes **com o caminho credenciado EXECUTADO, não pulado**: afirmar `0 skipped` no resumo. O repo documenta o padrão oposto em `release.yml:267` ("CLW_CI_TOKEN absent → SKIP (green). This is an intentional env-skip, NOT a gate") — "verde" sem essa qualificação é satisfeito por um run que não exerceu nada.

**R.** Reverter (volta a herdar o default).

---

## WP-08b — `permissions:` no `release.yml`

**Closes:** M8 (resto) · **Depends:** WP-01, WP-13b · **Risk:** MÉDIO · **Class:** ci/security

Separado porque `release.yml` **não tem `workflow_dispatch`** (verificado: 0 ocorrências; é `on: push: tags: ['v*']`). A aceitação da v1 — "dispatch manual dos 5" — era fisicamente impossível justamente no arquivo onde errar é irrecuperável: o PR mergearia com evidência de 4 em 5 e estrearia numa release real, com bump de fórmula e mirror público, num arquivo cujos modos de falha são `::warning::`, não vermelho.

**S.** `release.yml` declara `permissions: {contents: read}` no topo; os jobs elevam com escopo mínimo; e a mudança foi exercitada antes de qualquer tag real.

**Δ.**
1. `permissions: {contents: read}` no topo.
2. **Adicionar `workflow_dispatch`** guardado para no-op fora de contexto de tag, para que essa classe de mudança volte a ser testável.
3. **[v3] O Δ.4 da v1 é ELIMINADO.** Ele propunha remover `contents: write` do job de publish "se a release usa PAT do mirror". O arquivo responde em `:746-753`: a permissão **não** é para a release no mirror (essa usa `secrets.CLW_RELEASES_TOKEN` em `:904,926-930`) — é para `actions/attest-build-provenance` em `:995`. Esse passo hoje é **pulado** (gate de repo público), então removê-la dá run verde hoje e falha dura na primeira release attestada, isto é, exatamente quando o repo virar público e a integridade de release mais importar. E "cada workflow rodou verde" estruturalmente não detecta passo pulado. Se essa permissão for revisitada, é no WP que desfizer o waiver de SLSA, não aqui.

**I.** G5. **Nenhum passo hoje pulado pode passar a falhar quando for reativado** — invariante que a v1 não tinha e que é exatamente o que o Δ.4 violava.

**T.** rc dry-run dedicado.

**A.** rc verde · a tabela por job de WP-08a, agora para `release.yml`, incluindo prova por grep de que as pernas de publish/formula usam PAT do mirror e não o `GITHUB_TOKEN`.

**R.** Reverter + novo rc.

---

## WP-09 — Deadman sobre todas as lanes agendadas

**Closes:** M9 · **Depends:** — · **Risk:** BAIXO · **Class:** ci/observability

**S.** Cada lane agendada tem vigilância de frescor e de conclusão; uma lane que pare de rodar produz vermelho dentro de uma janela definida por lane.

**P.** Nenhuma.

**Δ.**
1. Matriz `{workflow, max_age_hours}`. **[v3] Inventário real verificado**, com as cadências já deliberadamente escalonadas e comentadas no repo:
   | lane | cron | janela |
   |---|---|---|
   | `conformance-canary` | diário 13:00 | 26h |
   | `fuzz` | diário 08:00 | 26h |
   | `metered-probe-canary` | seg 12:00 | 8d |
   | `live-journeys` | seg 06:17 | 8d |
   | `mutation` | seg 04:00 | 8d |
   | `stress` | seg 02:00 | 8d |
   | `e2e-evidence` | (WP-12 agenda) | 8d |
2. Cada entrada avalia as três condições atuais: existe run concluído? é recente? concluiu com sucesso?
3. Linha `days-since-last-tag` (a metade de cadência de H2, do WP-01).
4. Corrigir o `then=$(…)` de `:50`, armadilha de edição.
5. Janelas como **dado** no topo do arquivo, para que mudar cadência seja uma linha.

**I.** O deadman continua não sendo prova de vida do sistema — vigia que a vigilância roda. Sua **própria** vivacidade fica explicitamente fora de escopo, com uma linha apontando WP-36. Sem essa linha, o WP se lê como se tivesse fechado a auto-cegueira, e não fechou.

**T.** Dispatch com janela artificialmente curta para uma lane → vermelho nomeando-a; depois com a janela real.

**A. [v3 — o critério da v1 era insatisfatível].** A v1 dizia `grep -l "schedule:" .github/workflows/*.yml ⊆ matriz`. **Rodei: o resultado inclui o próprio `canary-deadman.yml`.** Um deadman não pode ser elemento do próprio conjunto vigiado — se ele parar, sua própria entrada para de ser avaliada. O critério ou falha para sempre ou força uma auto-entrada de cobertura nula. Forma correta, embutida **dentro** do próprio deadman (lane agendada nova sem registro = deadman vermelho):
```sh
comm -23 <(grep -l '^\s*schedule:' .github/workflows/*.yml | xargs -n1 basename \
           | grep -v '^canary-deadman.yml$' | sort) \
         <(yq '.jobs.*.strategy.matrix.include[].workflow' .github/workflows/canary-deadman.yml | sort)
```
tem de ser **vazio**.
E "verde exceto onde vermelho é esperado" não é predicado — vira lista `KNOWN_RED` **commitada como dado**; qualquer vermelho fora dela reprova. Mais: `grep -c 'then=\$(' canary-deadman.yml` → 0.

**R.** Reverter para a versão de lane única.

---

## WP-10 — Anular o trio de credenciais nos jobs que executam código de PR

**Closes:** M11 · **Depends:** WP-23 (premissa compartilhada de vazio⇒ausente) · **Risk:** BAIXO · **Class:** ci/security

**S.** Todo job que compila ou executa código proveniente de PR vê `CLW_CRED_TICKET`, `CLW_LEASE_ID` e `CLW_FABRIC_ENDPOINT` vazios, independentemente do que o fabric injete no box.

**Δ.**
1. Bloco `env:` anulando as três, com comentário apontando o precedente (`conformance-canary.yml:82-94`, que já faz isso com justificativa longa).
2. **[v3] Escopo ampliado.** A v1 cobria só o job `gates` de `ci.yml`. Faltavam: **`coverage.yml`** (roda `cargo llvm-cov` sobre código de PR, `on: pull_request`, sem filtro de path) e **`windows.yml`** (compila código de PR e executa sob wine, `on: pull_request`). Se a ameaça é real para `ci.yml`, é idêntica nesses dois. Mais `gate-check-selftest`, que também roda em PR.

**I.** Nenhum teste pode passar a depender do trio estar presente. Os testes de broker setam o trio eles mesmos sobre uma base limpa (`e2e.rs:421-439`).

**T. [v3 — a evidência da v1 não era durável].** A v1 propunha um passo de debug **removido antes do merge**: nada impede alguém deletar o `env:` depois, e não há red-on-revert. Substituir por assert **dentro do `gates.sh`**, acima do exit docs-only:
```sh
for v in CLW_CRED_TICKET CLW_LEASE_ID CLW_FABRIC_ENDPOINT; do
  [ -z "${!v:-}" ] || { echo "GATE FAILED: $v is set in the gate job env" >&2; exit 1; }
done
```
Isso torna a propriedade verificada por máquina em todo PR, para sempre, e falha alto se um edit futuro derrubar o bloco.

**A.**
1. Os quatro workflows carregam o bloco.
2. O assert existe e roda.
3. **Controle negativo em box real:** commit de rascunho removendo o `env:` de `ci.yml` deixa o gate **vermelho** num run `runs-on: corelink`. Colar o link. **[v3] Esse é também o único critério que prova que o fabric injeta o trio** — premissa que o WP assume e que nenhuma versão do plano jamais demonstrou.
4. Os testes existentes **não servem** de evidência: `e2e.rs` já faz `env_remove` das 11 variáveis, então o suite fica verde com ou sem a anulação no job. Contraexemplo que a v1 não cobria: o bloco entra com typo (`CLW_CRED_TICKETS`), tudo verde, o passo de debug mostra a variável certa vazia porque a errada nunca foi checada, o WP shipa, S falso.

**R.** Reverter (volta a herdar o ambiente do box).

**Nota externa.** A correção estrutural — o fabric não injetar env-0 em leases de CI — é do runners TL. Este WP é defesa em profundidade e vale mesmo que aquela mudança aconteça.

---

## WP-34 — Lane de mutação: consertar ou desligar

**Closes:** M1, L30 · **Depends:** WP-09 · **Risk:** BAIXO · **Class:** ci

**Puxado da "sessão 9+" para a sessão 3.** WP-09 põe a `mutation` na matriz do deadman; deixar WP-34 para seis sessões depois é a fadiga de alerta que o próprio plano descreve como risco.

**S.** A lane de mutação está verde e gateia alguma coisa, ou não existe. Não há terceira opção.

**P.** Triagem concluída.

**Δ. [v3 — diagnóstico concreto, substituindo o "provavelmente harness/timeout" da v2].** Investiguei o contrato de exit em `mutation.yml:66-74`: **sobreviventes (exit 2) e timeouts (exit 3) são esperados e NÃO falham o job**; só falha de harness (1=bad-args, 4=baseline-tests-failed, 70=internal). Logo as 4 vermelhas **não são mutantes sobreviventes — são falha de harness**. E o padrão é revelador: em cada run vermelha **exatamente um shard falha e os outros cinco passam**, e o shard **muda** — `0/6` em 2026-07-20, 08-03 e 08-10; `2/6` em 08-24. Não é mutante determinístico nem shard estruturalmente quebrado: é **flakiness de baseline** (exit 4 — a suíte de testes falhando de forma intermitente sob o harness) ou contenção de recurso no fabric.
Portanto: (a) reproduzir localmente rodando o shard afetado com `--baseline` e capturar o exit real; se for 4, o alvo é o teste flaky — que **também afeta o gate obrigatório** e é achado maior que a lane de mutação; ou (b) desligar a lane e remover o parágrafo do `TESTING.md` que a promete.

**I.** Se a causa for teste flaky, o conserto é do teste — **não** relaxar o contrato de exit da lane para engolir exit 4. Isso converteria a única detecção honesta em silêncio.

**T.** Reprodução local do shard falho, exit code registrado no PR.

**A. [v3 — a v1 aceitava uma lane desdentada].** "3 semanas verdes" também descreve uma lane que gateia **nada**.
1. Ou: 3 execuções semanais consecutivas verdes **e** um controle negativo — mutante sobrevivente injetado num branch de rascunho **avermelha** a lane (o que exige a fase 2 `--in-diff` prometida e nunca fiada).
2. Ou: a lane não existe mais e `TESTING.md` concorda.
3. Em qualquer caso: o exit code da falha original está registrado, e se for 4 há issue aberta para o teste flaky.

**R.** N/A (é decisão).

---

## WP-35 — Runbook de contingência do runner

**Closes:** M3 · **Depends:** — · **Risk:** BAIXO · **Class:** docs/ops

**Promovido** da onda E: é 1 arquivo, custo zero, e guarda o único caminho de merge e de release. Não havia razão para ficar atrás de um `grep -c`.

**S.** Existe procedimento escrito **e testado** para devolver as lanes obrigatórias a runners hospedados num outage do fabric.

**Δ.** `docs/RUNBOOK-ci-fallback.md`: o commit exato que troca `runs-on`, o que quebra ao fazê-lo (toolchain do host, cross-build windows-gnu, ausência do cache que `gates.sh:23-25` supõe — ver WP-50), o custo em minutos hospedados, e como voltar.

**[v3] Precisão de escopo.** O fallback só é necessário para as 19 lanes em `corelink`. `release.yml:334` é `[self-hosted, macOS, X64]` — um Mac físico com Xcode e toolchain próprios; **não** existe fallback hospedado trivial para a matriz de release (o runner macOS do GitHub é outra máquina, sem os segredos provisionados). O runbook precisa dizer isso: em outage do Mac, **a release para**; só o merge continua.

**I.** O runbook descreve o que foi **executado**, não o que se imagina.

**T.** Executar o procedimento uma vez.

**A.** Um PR de teste (não merged) demonstra as lanes obrigatórias verdes em `ubuntu-latest`, com link no runbook · as pegadinhas encontradas estão escritas · a limitação do Mac de release está declarada.

**R.** N/A.

---

## WP-36 — Deadman externo

**Closes:** M10 · **Depends:** WP-09 · **Risk:** BAIXO · **Class:** ops

**S.** Existe um observador **fora** do GitHub Actions que avisa quando as lanes param, ou está registrado que ele não existe e por quê.

**Δ.** Em ordem de custo: (a) cron no Mac do dono consultando `gh api` e notificando por canal independente; (b) serviço de heartbeat externo; (c) registrar a limitação em `docs/` e no CLAUDE.md. Qualquer uma fecha o WP — **inclusive (c)**, desde que explícita.

**[v3] Por que a limitação é real.** O deadman roda no mesmo scheduler e no mesmo fabric que vigia, e seu único alerta é `::error::` — que vira e-mail de workflow vermelho do GitHub (verificado: `:46`, `:53`, `:57`). Outage do Actions ou do fabric silencia vigia e vigiado juntos, e o canal de alerta é o mesmo sistema que caiu.

**I.** Se (c), o texto diz sem rodeio que não há detecção externa.

**T.** Se (a) ou (b): desligar uma lane deliberadamente e ver o alerta chegar pelo canal externo.

**A.** Ou existe observador cujo alerta chega por canal independente do GitHub, com o teste acima demonstrado, ou a limitação está escrita e datada.

**R.** N/A.

---

# SESSÃO 4 — Endurecimento do `release.yml` (um rc de lote)

---

## WP-13a — Manuseio do segredo de assinatura

**Closes:** L31, L39 · **Depends:** WP-01, WP-03 · **Risk:** ALTO se malfeito · **Class:** ci/security

**Extraído do cluster** porque é o único item de segurança e o único cujo revert é inaceitável — e porque sua falha é **silenciosa**.

**S.** O material de chave minisign nunca persiste no disco do Mac de longa duração após a assinatura.

**Δ.**
1. `release.yml:628` decodifica `minisign.key` no cwd do Mac (`max-parallel: 3`, mesmo usuário, box físico). Mover para `$RUNNER_TEMP` **junto com os dois literais pareados**: o `trap 'rm -f minisign.key' EXIT` de `:629` e o `rm -f minisign.key` de `:631`. Uma única variável `KEYFILE` referenciada pelos três.
2. `printf '%s'` no lugar de `echo "$MINISIGN_PASSWORD"` em `:630` (backslash mangling).

**[v3] Precisão que a v2 conflava.** `:863` também escreve `../minisign.key`, mas no job de **publish**, que roda em `corelink` (`:744`) — microVM efêmera, onde a premissa de disco compartilhado não vale. São dois sítios com regimes diferentes; tratá-los como um item leva a "consertar" o errado ou a esquecer um trap.

**I.** G5, G6. A assinatura continua fail-closed em tag real (`:616`).

**T.** Passo pós-assinatura que afirma `! test -e` em cada caminho candidato e **falha o job** se sobrar material de chave.

**A.**
1. O passo de asserção existe e executa (linha no log do rc).
2. rc verde.
3. Nenhum literal `minisign.key` fora da variável no diff.

**R.** Reverter — **mas se o WP for aplicado pela metade (redirect movido, trap não), a chave decodificada persiste no Mac e nada fica vermelho, nunca.** É a falha silenciosa de maior consequência do plano inteiro, e é o motivo de o WP ser separado e ter asserção própria em vez de ser o item 3 de um cluster de sete.

---

## WP-13b — Robustez do `release.yml`

**Closes:** L32, L33, L34, L42 · **Depends:** WP-13a · **Risk:** MÉDIO · **Class:** ci/release

**S.** O pipeline não tem fragilidades de parsing nem promessas quebráveis de publicação.

**Δ.**
1. **L32** quota-guard com `jq` (via `parse_quota_body` de WP-0A) em vez de `head -1 | grep`. **[v3] Direção da falha, para calibrar prioridade:** um JSON pretty-printed hoje produz falso "METERED", que **bloqueia** a release — falha **segura**. O risco real de fail-open está em `:158`/`:182`, que **pulam** o guard com `::warning::` quando a chave está ausente ou rotacionada; esse é o caminho que importa e é coberto pela precondição de WP-01.
2. **L33** versão via `cargo metadata --format-version 1 | jq -r` em vez de `grep -m1 '^version = '`.
3. **L42** `retention-days` 7→30 em `:647` e `:725`. São os **binários assinados**, não material de chave: a postura de segurança não muda, é linha de billing.
4. **L34** `git push` da fórmula com rebase-and-retry limitado, para que duas tags próximas não deixem o tap na versão antiga com ambos os runs verdes.

**I.** G5, G6. Nenhum item transforma condição hoje fatal em tolerada.

**T.** A8 do selftest de WP-0A (`parse_quota_body` com corpo pretty-printed e minificado → mesmo veredito).

**A.** rc de lote verde · nenhum `head -1` no guard · nenhum `grep -m1 '^version'` para versão · A8 verde.

**R.** Reverter itens individualmente — **com a ressalva honesta que a v1 omitia:** revert por hunk não existe num PR squash-merged. É por isso que o item de segredo saiu para WP-13a.

---

# SESSÕES 5-7 — Verdade documental e mecanização

Ordem obrigatória em `SECURITY.md` e `THREAT-MODEL.md`: **WP-16 → WP-15 → WP-18**. WP-29-DEC (sessão 1) precede WP-15. WP-21 depende de **WP-05** porque `docs/minisign.pub` é superfície de release morando sob `docs/`.

---

## WP-16 — Tornar a afirmação sobre `unsafe` verdadeira e verificável por máquina

**Closes:** H4b · **Depends:** — · **Risk:** BAIXO · **Class:** code+docs

**S.** Todo crate sem `unsafe` proíbe `unsafe` por lint; o único que tem o permite pontualmente e documenta; e os documentos afirmam exatamente isso.

**Δ. [v3 — caminho principal trocado].**
1. **`[workspace.lints.rust] unsafe_code = "forbid"`** na raiz + `[lints] workspace = true` em cada membro. A v1 propunha `#![forbid]` em 11 `lib.rs` e relegava o workspace-lints a "considerar". Inverti porque o inventário real de crate roots quebra a abordagem por arquivo: **12 crates, 11 `lib.rs`, `clw-cli` NÃO tem `lib.rs`** (só `main.rs` e `config.rs`), e **`clw-e2e-evidence` tem `lib.rs` E `src/bin/e2e.rs`** — dois crate roots, e atributo em `lib.rs` não alcança o bin. Cargo 1.91.1 suporta a tabela; edition 2021 não impede.
2. `clw-cli` sobrescreve: `[lints.rust] unsafe_code = "deny"` + um `#[allow(unsafe_code)]` no bloco DACL de `config.rs:768` (única ocorrência real de `unsafe` no workspace, verificada), com comentário do porquê e da segurança.
3. `SECURITY.md:73-75` e `THREAT-MODEL.md:47`: "zero `unsafe`" → "zero `unsafe` **exceto** um bloco auditado em `clw-cli` (DACL do Windows), garantido por lint de workspace".

**I.** Nenhuma mudança de runtime — só lints. **WP-16 é dono das frases de `unsafe` nos dois documentos**; WP-15 e WP-18 não as tocam (a v1 tinha três WPs editando as mesmas duas linhas sem ordem declarada).

**T.** O compilador.

**A. [v3 — o critério da v1 era malformado e furado].** A v1 dizia `grep -rc "forbid(unsafe_code)" crates/*/src/lib.rs → 11 crates`: `grep -rc` sobre vários arquivos emite `path:count` por arquivo, não o escalar; casa comentário (`// deveríamos adicionar forbid(unsafe_code)` passava); e o glob **estruturalmente não enxerga** `main.rs` nem `src/bin/*.rs`, isto é, não cobre o único crate que tem `unsafe`.
1. `grep -L 'workspace = true' crates/*/Cargo.toml` → vazio (os 12 optaram; cobre roots `main.rs` e `bin/`).
2. `grep -rc 'allow(unsafe_code)' crates/` → exatamente 1, em `clw-cli/src/config.rs`.
3. **Controle negativo durável**, não manual: passo que anexa `pub fn _x(){unsafe{}}` a uma cópia de rascunho de `clw-types/src/lib.rs`, roda `cargo check -p clw-types`, afirma **falha**, restaura. Dez segundos, e é a única coisa que prova que o lint está vivo. `cargo build` verde **não** prova nada — `forbid` só erra quando há `unsafe`.
4. `grep -rn 'zero unsafe' docs/ *.md` → nenhum hit sem a ressalva.

**R.** Remover os lints. A garantia volta a ser social.

---

## WP-15 — Reescrever o THREAT-MODEL

**Closes:** H4a, H4c, M28 (as duas frases), L43 · **Depends:** WP-16, WP-29-DEC · **Risk:** BAIXO técnico, ALTO em valor · **Class:** docs

**S.** `docs/THREAT-MODEL.md` descreve o produto que existe.

**Δ.**
1. **Erasure (H4a):** remover "clw has no erasure command today … the residency promise is non-compliant". Descrever `rm`, `erase` (Art. 17), `prune`, `ls`, com D-1/D-7/D-8 live. Declarar o residual verdadeiro: GC lease/epoch server-side, e a janela de corrida que `prune.rs:203-217` comenta (um 404 mid-walk pode ser `rm` concorrente **ou** replica-lag do AC, indistinguíveis, e o fail-safe é recusar a varredura).
2. **M1.6/M1.7 (H4c):** fundir os bullets contraditórios — um diz "not yet bounded (M1.6)", o seguinte "Now mitigated by M1.6". O implementado vai para a tabela de mitigações; o residual fica nomeado.
3. **Symlink (M28):** conforme WP-29-DEC. **[v3] WP-15 é dono INCONDICIONAL das duas frases** — `THREAT-MODEL` e `SECURITY.md:50-55` ("absolute paths and **any `..`/`.` component are rejected**", citando o validador antigo `validate_symlink_target`). Na v1 a correção do SECURITY.md vivia só no ramo (b) do WP-29; se o dono escolhesse (a), o SECURITY.md **continuaria mentindo**, porque endurecer containment não torna verdadeira a frase "qualquer `..` é rejeitado". Citar o validador vivo (`validate_symlink_target_contained`) pelo nome.
4. **Mitigações shipped (L43):** seção nova listando o que já está fechado, cada uma com ponteiro para o teste que prova: rejeição de userinfo-smuggling, proibição de redirect, caps de leitura de wire, strip do env do filho (H3), DACL, self-heal do cache.
5. Header de status/data/owner (padrão de WP-21).

**I.** Nenhuma afirmação nova sem referência de código verificável.

**T.** Nenhuma automatizada — mas WP-48 passa a impedir a **reintrodução** das frases aposentadas.

**A.** Checklist de contradições zerada (erasure, M1.6/M1.7, symlink; `unsafe` é de WP-16) · cada mitigação com ponteiro, amostra de 5 conferida pelo revisor · nenhuma contradição interna.

**R.** Reverter. Volta a mentir.

---

## WP-17 — INSTALL.md: verificação que funciona

**Closes:** H5, H6 (lado docs), M27 · **Depends:** — · **Risk:** BAIXO · **Class:** docs

**S.** Todo procedimento de verificação do INSTALL.md funciona para um usuário real na release corrente; o que não existe está marcado como planejado, não como requisito.

**Δ.**
1. **Authenticode (H5):** tabela de assets (`:92`) e §4d (`:205-210`) — o `.exe` é unsigned (waiver). Hoje o doc diz que status 'Valid' **é requerido**, então um usuário Windows diligente conclui que **toda** release legítima foi comprometida.
2. **[v3 — o que a verificação em Windows/macOS realmente é.** Descobri nesta passada que a assinatura por binário é **Linux-only por design** (`release.yml:598-602`, passo "sign (Linux, minisign detached)"): só `clw-*-{aarch64,x86_64}-unknown-linux-gnu.minisig` existem. Mac (Developer-ID) e Windows (Authenticode) estão waived, então para esses três binários a **única** cadeia de integridade é `SHA256SUMS` + `SHA256SUMS.minisig`. É criptograficamente sólido — o manifest assinado cobre os binários — mas o texto correto não é "verifique a assinatura do binário", é "verifique o checksum contra o manifest assinado". **Nenhuma versão anterior do plano dizia isso.**]
3. **Attestation (H6):** remover `gh attestation verify` (`:108`, `:187-190`) enquanto o passo estiver gated a repo público, ou marcá-lo explicitamente indisponível e por quê.
4. **Trust root (M27):** corrigir `:61-66` — o installer **não** baixa `minisign.pub`; a chave é embutida em `install.sh:40-44,79` e a publicada não é confiada no momento da instalação. O modelo real é **mais forte** que o descrito; o doc reabre no papel o TOFU que o script fechou. Mencionar `CLW_SKIP_SIGNATURE` com o aviso correspondente.

**I.** Nenhuma instrução de verificação pode falhar para quem a siga corretamente.

**T.** Execução real: seguir o doc do início ao fim em máquina limpa (ou container) para macOS/Linux, e os passos de verificação para Windows. Evidência no PR.

**A.** Cada bloco de comando executado contra a release corrente sai 0 (ou tem resultado esperado documentado) · zero menções a Authenticode 'Valid' como requisito · zero `gh attestation verify` enquanto skipped · o texto de Windows/macOS descreve a via `SHA256SUMS` + `minisign`, não assinatura por binário.

**R.** Reverter.

---

## WP-18 — Uma única política de suporte

**Closes:** H8, L44 · **Depends:** WP-15, WP-16 · **Risk:** BAIXO · **Class:** docs

**S.** Existe exatamente uma política de suporte publicada; o outro documento referencia em vez de reafirmar.

**P. [v3 — precondição que nenhuma versão anterior tinha].** **O `SECURITY.md` é visível no mirror público?** O repo principal é privado; o usuário só enxerga `HuGR-Labs/clw-releases`. Se o SECURITY.md e o canal de reporte só existem no privado, toda esta WP — e o fingerprint PGP de L44 — é publicada para **audiência zero**. Verificar antes de escrever.

**Δ.**
1. Eleger `docs/SUPPORT-AND-EOL.md` como canônico (mais específico e mais recente), ou a decisão contrária do dono pela política estrita de latest-only — nesse caso corrigir o SUPPORT-AND-EOL.
2. `SECURITY.md` §"Supported versions": parágrafo curto + link, sem duplicar.
3. Remover "an EOL policy lands with 1.0" — o documento já existe.
4. **L44:** fingerprint PGP publicado (ou dizer que o canal é e-mail simples), `security.txt`, cláusula de safe-harbor.

**I.** A política escolhida tem de ser **cumprível**. Prometer backport em 7 dias exige capacidade de manter uma linha anterior; se não houver, escolher a estrita e dizer isso.

**T.** Nenhuma.

**A.** Leitura sequencial dos dois documentos sem contradição · nenhum documento anuncia como futuro o que já existe · a resposta da precondição está registrada, e se for "não visível", há item para publicá-lo no mirror.

**R.** Reverter.

---

## WP-19 — CLAUDE.md e handoff: estado real

**Closes:** H7, L52 · **Depends:** WP-02-DEC · **Risk:** BAIXO técnico, alto em efeito composto · **Class:** docs

**S.** `CLAUDE.md` descreve infraestrutura, toolchain, topologia de release e controles como são hoje, e aponta para um documento de estado atual.

**Δ.**
1. **Proteção:** conforme WP-02-DEC.
2. **Runners:** substituir "All on github-hosted ubuntu" — 19 dos 20 `runs-on` são `corelink` (fabric de microVMs efêmeras, **não** runner único) e o box único é o Mac de release. Registrar o trade-off e apontar WP-35.
3. **Toolchain:** remover a receita `$HOME/.clw-ci-rust/…` (0 ocorrências no repo, verificado). Documentar `scripts/ci-use-host-toolchain.sh`.
4. **Release:** 5 alvos num único Mac via `cargo-zigbuild`, não matriz nativa por OS.
5. **Handoff:** criar `docs/HANDOFF-2026-08-30-clw-TL-state.md` e pôr banner `SUPERSEDED` nos três antigos — incluindo o de 06-28, que em `:89` manda rodar `bash verify.sh`, arquivo que não existe.
6. **[v3] L52 migrado do WP-21 para cá:** os mesmos três handoffs também precisam de banner de renomeação de org. Dois WPs escrevendo banners diferentes nos mesmos três arquivos, em sessões diferentes, terminaria com um sobrescrevendo o outro. Um WP, um commit por arquivo.

**I.** Nenhuma afirmação verificável do CLAUDE.md pode ser falsa. É o arquivo com maior alavancagem de erro do repositório — toda sessão futura de agente parte dele.

**T.** Nenhuma automatizada; WP-48 impede a reintrodução de "github-hosted".

**A.** Cada afirmação factual verificada por comando, com a saída colada (a auditoria achou 4 falsas; o revisor repete) · `! grep -rqI --exclude-dir=.git 'clw-ci-rust' .` · o handoff apontado como "latest" existe, é de 2026-08-30 e não cita arquivos inexistentes.

**[v3] Correção de idioma.** A v1 pedia `grep -rc "clw-ci-rust" . → 0`: recursando sobre `.` isso imprime `path:count` para milhares de arquivos, inclusive `.git`, e nunca o escalar `0`. Usar a forma de ausência com `-q` e `--exclude-dir`.

**R.** Reverter.

---

## WP-20a — Docs de contrato

**Closes:** M23, M24, M25, M26 · **Depends:** — · **Risk:** BAIXO · **Class:** docs

Dividido do WP-20 da v1 (11 achados, ~10 documentos — grande demais para revisão atenta).

**S.** Os documentos que outros times pinam descrevem o contrato real.

**Δ.**
1. **M23 TESTING.md:** `:25-27` diz "github-hosted runners", `ubuntu-latest`, `windows-latest` — tudo roda em `corelink`, e `windows.yml` é cross-mingw + smoke wine, não Windows hospedado. O floor de coverage é apresentado como fase futura e está **ativo** em 88% (`coverage.yml:94-95`, enforcing desde #187). Tabela completa com as 13 lanes (faltam conformance-canary, live-journeys, metered-probe, canary-deadman).
2. **M24 CLI-CONTRACT.md:** `:37` e `:71` marcam `CLW_REF_DOMAIN` como PROPOSED; está shipped (`main.rs:63-74`) e congelado no seam com os runners. → FROZEN com data, no formato do EXIT-FREEZE.
3. **M25 SPEC-manifest.md:** `:6` fixa "hash fe251941" do `clw-types/src/lib.rs`; o sha256 atual começa `08087eda`. Atualizar **e** trocar a política: referência a **tag**, que não apodrece, em vez de hash que exige refresh manual. (Se ficar hash, entra no checklist de release e no WP-48.)
4. **M26 ARCHITECTURE.md:** `:117-143` descreve o dataflow do `run` com captura de **output files** e reconstrução a partir do CAS — o run real memoiza só stdout/stderr/exit, com chave = digests de inputs + env + comando + tool-identity + cwd. E `:157-160` diz que conteúdo idêntico "is shared across tenants", contradito por `GUIDE:138-140` ("no clw-level sharing or dedup across different tenants" — o correto).

**I.** G2 — corrigir a documentação de uma saída **nunca** implica mudar a saída. Onde doc e código divergem, este WP corrige o **doc**; os casos em que o código é que está errado pertencem às sessões de código.

**T.** Nenhuma; WP-48 passa a gatear o hash do SPEC e o "github-hosted".

**A.** Nenhum doc de contrato descreve infraestrutura inexistente · o hash do SPEC bate (ou virou referência a tag) · nenhuma contradição entre ARCHITECTURE e GUIDE.

**R.** Reverter por hunk.

---

## WP-20b — Docs de usuário

**Closes:** M29, M30, M33, L53, L54, L55, L57 · **Depends:** WP-01 (as amostras saem do binário publicado) · **Risk:** BAIXO · **Class:** docs

**S.** As amostras de saída batem com o binário e as receitas funcionam.

**Δ.**
1. **M29 usage.md:** rate-limits pós-402 (o $-ceiling é o modo de falha real; GUIDE §13 já está certo) · `:546` "Storage: 10 GB (enforced; exceeding → 429 + Retry-After)" contradiz o contrato 402 non-retryable do código · remover "live feature nunca em CI, rode manual" (roda daily + weekly) · `:537` mostra prefixo "transport error:" já corrigido.
2. **M30:** TEST-JOURNEY-MAP (a suite shipou e foi provada 9/9) · **go-live/RUNBOOK-destructive-ops.md** com banner "até PR #269 deploy", deployado em 2026-06-15 — é runbook **vivo** de operador, prioridade dentro do WP · GUIDE §20/§J (topologia de release) · family-e2e-STATUS (banner de congelado).
3. **M33:** hydrate imprime `source:` (`subcmds/hydrate.rs:125`) e README:132 + usage.md:72 mostram `name:` · shape JSON de `ls --all` completo (`foreign_ac_entries`/skipped/errors) · receitas para `rm --ref-key` e `hydrate --manifest-digest` · tabela de config do README completa (`--ref-domain`, `CORELINK_*`, `-v/-q`, `CLW_OP_DEADLINE_SECS` — ausente do README e do usage.md, documentado só em `GUIDE:513` — e `CLW_CACHE_BUDGET_BYTES`).
4. **[v3] Item novo, das notas de área que nunca chegaram ao relatório:** a receita de `erase` no usage.md **omite o sufixo `; tenant verified empty`** — que é a verificação que *constitui* a claim de conformidade. Corrigir o THREAT-MODEL (H4a) enquanto a receita do usuário omite a verificação é meio conserto.
5. **L53:** data do CHANGELOG 0.1.11 (07-13 vs tag 07-16) · heading `[Unreleased]` duplicado (`:3` e `:623`) · links de comparação por versão.
6. **L54/L55/L57:** SPEC vs Windows shipped (mode 0o644 constante, mask de setuid) · README linkar GUIDE e INSTALL · `GUIDE:194` "or" → "and" · typos de ano nas amostras.

**I.** G2.

**T.** Amostras **geradas do binário**, não editadas à mão.

**A. [v3 — a v1 tinha aqui o critério menos verificável do plano].** "Toda amostra reproduzível com o binário" sobre dezenas de amostras, sem mecanismo, é amostragem humana na WP que fecha 11 achados — sobre exatamente o modo de falha que reincidiu. Vira mecanizado em WP-48: marcação `<!-- clw-sample: <cmd> -->` nos blocos + `scripts/check-doc-samples.sh` na lane de lint. Critérios aqui:
1. Todo bloco de saída de CLI está marcado, ou consta da lista de não-cobertos (visível e encolhível).
2. `check-doc-samples.sh` verde.
3. Tabela de config do README **gerada** de `clw --help`, não comparada linha a linha à mão.
4. Red-on-revert: reintroduzir "transport error:" numa amostra deixa a lane vermelha.

**R.** Reverter por hunk.

---

## WP-21 — Índice, arquivo e headers de status para `docs/`

**Closes:** M31, M32 · **Depends:** **WP-05**, WP-15, WP-17, WP-18, WP-19, WP-20a/b · **Risk:** MÉDIO · **Class:** docs

**[v3] Por que depende de WP-05 e não é onda-C-paralela.** A §1.3 da v1 dizia que ondas B e C tocam superfícies disjuntas. **Falso:** `docs/minisign.pub` é superfície do pipeline de release morando sob `docs/`, lida em `release.yml:798` e `:873` — **e é o único arquivo não-`.md` sob `docs/`** (verificado). Os dois pontos de leitura são fail-open (`|| true`, `::warning::`), então um sweep que o movesse faria a release seguinte publicar **sem trust root, toda verde**. WP-05 fecha os dois fail-open antes.

**S.** `docs/` distingue estruturalmente material vivo de histórico; todo doc vivo declara status, data de revisão e dono.

**Δ.**
1. `docs/README.md`: índice com status (LIVING/FROZEN/ARCHIVE), dono e last-reviewed.
2. `docs/archive/{2026-06-correspondence,handoffs,findings,go-live}/` e `git mv` dos ~67 point-in-time.
3. **Dois arquivados são referenciados por docs vivos** (`REPLY-to-runners-TL-2026-06-16` ← CLI-CONTRACT; `REPLY-clw-TL-toolchain-resolver-seam` ← EXIT-FREEZE): atualizar os links ou deixá-los no lugar.
4. Header de 3 linhas nos vivos: TESTING, THREAT-MODEL, ARCHITECTURE, SECURITY, DISTRIBUTION, TEST-JOURNEY-MAP, usage, GUIDE, INSTALL, CLI-CONTRACT, SPEC-manifest.
5. **[v3] Tornar fail-closed o guard de slug de org.** `gates.sh:38-51` grepa uma **allowlist de caminhos** com `2>/dev/null || true`: um caminho que suma produz **zero erro e zero hit** — o guard degrada silenciosamente para "passa". Como este WP move caminhos, ele precisa **primeiro** fazer o guard afirmar que cada caminho da allowlist existe.
6. **[v3] `docs/go-live/` é misto e não pode ser movido em bloco** (verificado): `LAUNCH-CHECKLIST.md` e `RUNBOOK-key-rotation-revocation.md` estão na allowlist do guard; `RUNBOOK-destructive-ops.md` é LIVING (WP-20b); `WINDOWS-ACL-VERIFICATION.md` é evidência de verificação; só `DISTRIBUTION-REWORK-PLAN-public-brew.md` e `WP-BRIEFS.md` são arquiváveis. Arquivo a arquivo.
7. **[v3] O handoff novo de WP-19 fica FORA do sweep** — é datado, logo "point-in-time" pela regra, mas é o documento que o CLAUDE.md aponta como atual.

**I.** Nenhum link interno quebrado. **Nenhum arquivo não-`.md` sob `docs/` se move** (hoje: exatamente `minisign.pub`). Nenhum documento referenciado por código, workflow ou script muda de caminho sem o referenciador mudar no mesmo commit.

**T.** Verificador de links sobre `docs/` e os `.md` da raiz.

**A.**
1. Zero links quebrados — e **permanente**, não "uma vez no PR": `lychee --offline` na lane de lint em todo PR. Drift de documentação é a maior classe de achado desta auditoria; verificar uma vez é o padrão que produziu o problema.
2. Todo caminho citado por `scripts/`, `.github/`, `crates/` ainda existe — **e o guard agora falha alto se não existir** (Δ.5).
3. Todo doc vivo tem header.
4. **[v3] `git log --follow` sai do critério:** não distingue `git mv` de delete+add (git não grava movimentos; a detecção é por similaridade em tempo de log), então o critério não tinha poder discriminante. Substituído por `git log --diff-filter=R --name-status -1` mostrando entradas `R100`.

**R.** Reverter o commit de movimentação. **Mas:** links externos, bookmarks e referências de outros repos já quebraram, e o revert não os desfaz.

---

## WP-46 — Documentação ausente

**Closes:** L56 · **Depends:** WP-01 (o runbook descreve a release que acabou de acontecer) · **Risk:** BAIXO · **Class:** docs

**S.** Existem os documentos que o repo promete e que o bus-factor exige.

**Δ.** `docs/RELEASE.md` — runbook de release; hoje o procedimento vive na cabeça de uma pessoa e em comentários do yml; é o doc de bus-factor prometido em WP-BRIEFS M2.3, **e é onde a regra de cadência de WP-01 mora** · `docs/SUPPLY-CHAIN.md` (M3.2) · man pages via `clap_mangen` · índice unificado erro→remédio.

**I.** O runbook descreve o procedimento **executado** na v0.1.12, não o idealizado.

**T.** Nenhuma.

**A.** Cada documento existe, é linkado do README, e foi revisado por quem não o escreveu · `RELEASE.md` contém a regra de cadência e o checklist (incluindo "CLAUDE.md revisado?" e "hash do SPEC atualizado?").

**R.** Reverter.

---

## WP-48 — Gate de verdade documental

**Closes:** prevenção de H4–H8 e M23–M33 · **Depends:** WP-15, WP-17, WP-18, WP-19, WP-20a/b, WP-46 · **Risk:** BAIXO · **Class:** ci/docs

**A crítica de fundo das reviews.** O plano corrige 117 instâncias e instala quase nenhum mecanismo. Cinco dos oito HIGH e onze MEDs são **um** modo de falha — documento canônico derivando de código que continuou andando. A resposta da v1 foram headers de status e sugestões de checklist: controles **sociais**, a mesma classe que produziu o drift, num repo cuja doutrina registrada após o v0.1.11 é *mecanizar, não auditar mais forte*.

**S.** As afirmações que esta auditoria corrigiu não podem voltar sem ficar vermelhas.

**Δ.** Check na lane de lint, rodando em **todas** as lanes (posição junto do dead-org guard, acima do `exit 0` de docs-only — senão o PR que reintroduz a mentira é justo o que escapa):
1. Hash do SPEC bate com `clw-types` (ou é referência a tag, conforme WP-20a).
2. `scripts/check-doc-samples.sh`: amostras marcadas regeneram do binário e batem (WP-20b).
3. `last-reviewed` de todo doc LIVING é mais novo que a última tag.
4. **Tripwires de grep nas frases aposentadas:** `"zero unsafe"` sem ressalva · `"no erasure command"` · `"Authenticode … is required"` · `"SLSA provenance"` em texto publicado · `"PROPOSED"` no CLI-CONTRACT · `"github-hosted"` · `"branch-protected"` se WP-02 for (b).
5. `lychee --offline` (WP-21).
6. `days-since-last-tag` na matriz do deadman (WP-09) — a metade de cadência de H2.

**I.** O check roda em todas as lanes, inclusive docs-only.

**T.** Cada tripwire com controle negativo: reintroduzir a frase num commit de rascunho → vermelho.

**A.** Os 7 controles negativos demonstrados, links no PR · a lane roda em todo PR.

**R.** Reverter. Volta a depender de vigilância humana.

---

## WP-49 — Lint de mensagem de commit

**Closes:** L60 · **Depends:** — · **Risk:** BAIXO · **Class:** ci

**Desfaz um dodge da v1**, que dispensou L60 como "passado, não corrigível". Isso responde a pergunta que ninguém fez: história não é corrigível, mas **nada impede o 13º commit fora do padrão**. O repo já roda check de DCO e já tem `gates.sh`; um lint de `commit-msg` ou de título de PR é menor que metade dos itens do WP-14.

**S.** Um commit (ou título de PR) fora de Conventional Commits não entra em `main`.

**Δ.** Workflow dedicado e confiável `pr-title.yml` em `pull_request_target`, validando o título do PR (que é o que vira a mensagem no squash-merge) contra o padrão. Ele faz checkout **somente** do `base.sha`, com pin imutável e sem credenciais persistidas, antes de executar o checker; código do head do PR nunca roda nesse gate. **[v3]** Validar o **título do PR**, não os commits do branch: o fluxo do repo é squash-merge, então a mensagem que sobrevive é a do PR; gatear commits intermediários seria atrito sem retorno. A formulação original “no `ci.yml` ao lado do `dco`” foi substituída por esta exceção segura: naquele workflow `pull_request`, um PR poderia alterar o checker e auto-certificar o próprio título. O primeiro PR que introduz `pr-title.yml` precisa de bootstrap waiver/cold review, porque um workflow `pull_request_target` só fica ativo depois de chegar à branch base.

**I.** Não bloqueia commits WIP no branch. G4.

**T.** Título inválido num PR de rascunho → vermelho.

**A.** O check existe, roda em todo PR, e o controle negativo foi demonstrado.

**R.** Reverter.

---

## WP-50 — Cache de build da CI

**Closes:** achado novo (surgido na cold review, não estava na auditoria) · **Depends:** — · **Risk:** BAIXO · **Class:** ci

**S.** Ou a CI reaproveita build entre execuções, ou o comentário que afirma que reaproveita é corrigido.

**[v3] O achado.** `gates.sh:23-25` exporta `CARGO_TARGET_DIR="$HOME/ci-cache/clw-target"` sob o comentário *"Persistent target dir on CI (outside the runner's `_work` checkout, immune to clean)"* — mas todo job `corelink` roda em **microVM efêmera**: `$HOME` não sobrevive ao lease. É otimização morta, e não há `actions/cache` em `ci.yml`, `coverage.yml`, `mutation.yml` nem `stress.yml`. Todo gate é build frio.

**Consequências que o plano tinha errado:** o Δ.4 do WP-12 propunha "reaproveitar cache de build" que **não existe**; e o `timeout-minutes: 30` daquele job vira risco real quando o perfil virar `--release`.

**Δ.** Ou (a) `actions/cache` de verdade nas lanes que compilam, com chave por `Cargo.lock` + toolchain; ou (b) remover a exportação e o comentário, registrando que build frio é a escolha. **(a) tem retorno alto** — é a maior alavanca de tempo de CI do repo e afeta o cronograma de todas as sessões seguintes.

**I.** Cache não pode mascarar mudança de toolchain nem de lock: a chave inclui ambos.

**T.** Dois runs consecutivos: o segundo mais rápido, com hit registrado no log.

**A.** (a): hit demonstrado, ganho medido em minutos; (b): o comentário falso não existe mais.

**R.** Reverter.

---

# SESSÕES 8-9 — Suites herméticas

Ordem obrigatória em `clw-e2e-evidence/src/lib.rs`: **WP-24 → WP-26** (a v1 declarava os dois independentes e os punha na mesma sessão, no mesmo arquivo de 710 linhas).

---

## WP-23 — Variável de ambiente vazia é ausente

**Closes:** M12 · **Depends:** — · **Bloqueia:** WP-10 · **Risk:** BAIXO-MÉDIO · **Class:** code

**S.** Uma `CLW_*` setada e vazia é indistinguível de ausente: a resolução segue para o próximo elo e a mensagem de erro é a acionável.

**Δ.** Filtro `filter(|s| !s.is_empty())` após o parse do clap, num **único** ponto, nos cinco campos globais (`endpoint`, `tenant`, `token`, `cache_dir`, `ref_domain`). Espelha `corelink_env()` (`config.rs:285-287`), que já faz exatamente isso para as `CORELINK_*` e comenta o motivo.

**I. [v3 — a cláusula da v1 era factualmente falsa sobre o código].** A v1 dizia: *"ticket vazio + ref-domain runner deve **continuar** não caindo silenciosamente no token estático"*. Verifiquei: `config.rs:133` lê o ticket via `corelink_env("CLW_CRED_TICKET")`, que **já filtra vazio para `None`**, então `resolve_credential_inputs` cai no braço `_ =>` (`:330`) e vai para o token estático — comportamento **atual e intencional** (`:123-128` fala do "runner still on the legacy env-PAT rollout"). O "continuar" era falso, e implementar a I literalmente hard-falharia todo runner-domain onde o fabric injeta ticket vazio. Pior: **WP-10 deliberadamente seta `CLW_CRED_TICKET: ""`** e depende de vazio⇒ausente; honrar a I escrita deixaria o gate obrigatório vermelho em todo PR aberto ao mesmo tempo.
**Invariante correta:** vazio é indistinguível de ausente — isso já é o comportamento. O que **não pode** enfraquecer é a propriedade de `config.rs:308-320`: *ticket **não-vazio** com `CLW_LEASE_ID` ou `CLW_FABRIC_ENDPOINT` faltando é erro duro ("fail-closed") e nunca cai no token estático.*

**T.** Três estados por variável (ausente / vazia / preenchida), com a mensagem exata pinada nos dois primeiros; mais um teste próprio para o caso "ticket não-vazio, lease ausente" → erro duro. Red-on-revert por construção.

**A.**
1. `CLW_TOKEN="" clw ls` → "missing token: set --token, CLW_TOKEN/CORELINK_TOKEN, or add token to ~/.clw/config.toml", exit 2, **sem request de rede**.
2. `CLW_ENDPOINT="" clw ls` → usa o endpoint default (hoje: "refusing plaintext (got '')").
3. `CLW_TENANT=""` → "missing tenant: …".
4. Os testes de broker continuam verdes; o caso fail-closed tem teste próprio.
5. **[v3]** `CLW_CACHE_DIR=""` e `CLW_REF_DOMAIN=""` também cobertos — a v1 os incluía no Δ e os omitia do A.

**R.** Reverter. Registrar no CHANGELOG: mensagens de erro mudam em caminhos hoje quebrados.

---

## WP-24 — Isolar o ambiente na suite de evidência

**Closes:** M13 · **Depends:** — · **Bloqueia:** WP-26 · **Risk:** MÉDIO (a v1 dizia BAIXO) · **Class:** code/test-infra

**S.** Cada step da suite roda com ambiente conhecido: a identidade que o próprio step seta, e nada herdado que mude o caminho de código.

**Δ.**
1. `clw-e2e-evidence/src/lib.rs:409-426` (`Runner::step`) faz `env_remove` **só de `RUST_LOG`** (verificado). Passar a remover o namespace completo, espelhando `clw-cli/tests/e2e.rs:421-439`, que já lista 11 variáveis **e explica o motivo em comentário**: um `CLW_REF_DOMAIN=runner` ambiente flipa as personas para o keyspace de runner, e num box `corelink` o trio empurraria a persona para o caminho C2c, **queimando um ticket single-use real**.
2. `HOME` sandbox, para que `~/.clw/config.toml` da máquina não influencie.
3. Fatorar a lista num lugar único.

**I. [v3 — cerca que a v1 não tinha e que era um caminho direto para regredir H3].** Existem **três** listas, com propósitos diferentes:
- `clw-run/src/lib.rs:54-63` — `SECRET_ENV_VARS`, **8 entradas**, controle de **segurança**: o que é removido do filho não-confiável antes do spawn (`:575-577`). Única a conter `CORELINK_CRED_TICKET`, `CORELINK_LEASE_ID`, `CORELINK_FABRIC_ENDPOINT`.
- `clw-cli/tests/e2e.rs:421-439` — 11 entradas, **hermeticidade de teste**. Única a conter `CLW_ENDPOINT`, `CLW_TENANT`, `CLW_CACHE_DIR`, `CLW_REF_DOMAIN`.
- a da suite de evidência — 1 entrada.

O Δ.3 da v1 dizia "fatorar num único lugar" e só mencionava "uma tem 11 e a outra tem 1" — **nunca mencionava `SECRET_ENV_VARS`**. Quem unificasse na lista de 11 **removeria as três `CORELINK_*` do strip do filho e regrediria o fix H3 com gate verde**, porque nenhum teste hoje afirma que `CORELINK_CRED_TICKET` não chega ao filho.
**Invariante:** `SECRET_ENV_VARS` é controle de segurança e **não** é fundida na lista de higiene de teste. A constante compartilhada pode ser **superconjunto consumido** pelos testes, nunca substituição.

**T.**
1. Teste que injeta `CLW_REF_DOMAIN=runner` no ambiente do processo pai e verifica que a persona continua no keyspace de usuário.
2. **Red-on-revert de H3:** spawnar um filho com as 8 de `SECRET_ENV_VARS` setadas e afirmar que **nenhuma** chega.

**A. [v3 — "journeys idênticas" não era comparável].** A v1 mandava comparar journeys de dois runs; relatórios carregam timestamps, digests e ids de tenant, e — pior — se ambos os runs pulassem, seriam "idênticos" trivialmente.
1. Dois runs (ambiente limpo; ambiente envenenado com `CLW_REF_DOMAIN=runner`, `CLW_ENDPOINT=http://127.0.0.1:1`, `CLW_TOKEN=bogus`, `HOME=/inexistente`), **normalizando** timestamps/uuids/digests, `diff` vazio.
2. **`0 skipped` em ambos** (ver a regra transversal abaixo).
3. A lista de variáveis tem **uma** definição: o array literal de `e2e.rs:421-439` passa a referenciar a constante compartilhada, de modo que as duas **não possam** divergir. Era o objetivo do Δ.3 e o A da v1 não verificava.

**R.** Reverter.

---

### Regra transversal das suites live — **"verde" satisfeito por "pulado"**

Aplica-se a WP-12, WP-24, WP-25 e WP-26. O próprio repo documenta o padrão em `release.yml:267`: *"CLW_CI_TOKEN absent — live conformance cases will SKIP (green). This is an intentional env-skip, NOT a gate."*

**[v3] O mecanismo já existe, parcialmente.** `clw-e2e-evidence/src/bin/e2e.rs:14-17` define `CLW_E2E_REQUIRE_PROD=1`, que transforma um skip de credencial **primária** em falha dura — **mas carrega uma isenção**: *"an OPTIONAL skip (J4's 2nd identity, J5's throwaway erase tenant) is reported but never trips REQUIRE"*. Essa isenção é exatamente o buraco: a 2ª identidade da Priya, o tenant descartável do Omar, o tenant de quota da Nadia e a persona J8 de cred-ticket são todos "opcionais". A memória do projeto registra que os 6 personas foram auto-provisionados e provados **9 passed / 0 skipped** em 2026-07-20 — ou seja, o estado sem skips é alcançável e já foi alcançado.

**Regra:** toda cláusula que diga "verde" passa a dizer **"verde E `0 skipped` E N == \<contagem de personas commitada\>"**, com a lane agendada setando `REQUIRE_PROD=1` **e** um modo que faz skip opcional também reprovar.

---

## WP-25 — Tornar a persona Marina reentrante

**Closes:** M14 · **Depends:** WP-24 · **Risk:** BAIXO · **Class:** code/test-infra

**S.** Toda persona é reentrante: uma execução que morra no meio não faz a seguinte falhar.

**Δ.**
1. `journeys/story_ship_service.rs` (registrada em `bin/e2e.rs:63` como `marina-ships-orders-api`, com o nome fixo `orders-api` vindo de `journeys/realistic.rs:25`): limpeza best-effort pré-flight (`rm --yes` ignorando "não existe"). Preferir isso a sufixo por run — o realismo do nome é o ponto da suite.
2. Auditar as outras 5 personas pelo mesmo critério e **registrar no PR que foram verificadas** — a auditoria as considerou reentrantes por análise, não por teste.

**I.** A limpeza é **setup**, não um "Beat" da narrativa, e não aparece no relatório como ação do usuário.

**T.** Crash simulado: interromper após o beat do `--force` e rodar de novo.

**A.** Duas execuções consecutivas sem limpeza manual, ambas verdes **com `0 skipped`** · execução interrompida seguida de execução completa, verde.

**R.** Reverter.

---

## WP-26 — Teardown das suites live

**Closes:** M15 · **Depends:** WP-24 · **Bloqueia:** WP-12 · **Risk:** ALTO — destrutivo contra prod · **Class:** code/test-infra

**[v3] Por que a v1 era BLOCKER.** Ela mandava "deletar os digests que este run criou". **Criar um digest não é possuí-lo:** CAS é deduplicado, então se duas personas do mesmo run, ou a suite e o canário de conformance, ou um usuário real no mesmo tenant produzirem bytes idênticos, produzem **o mesmo digest** — e o primeiro teardown apaga um blob que outro ref vivo referencia, deixando manifest root ou chunk pendurado. `clw prune` existe exatamente porque deleção tem de ser por **alcançabilidade**. A cláusula I da v1 enunciava a regra insegura como se fosse a propriedade de segurança.
Segundo defeito: **não existe prefixo de suite.** `ref_key()`/`ref_key_in()` (`clw-types/src/lib.rs:425-432`) são BLAKE3 sobre `domain || name` — chaves AC são hashes opacos de 32 bytes. O critério "contagem de entradas com prefixo da suite = 0" era inimplementável, e o contorno óbvio (varrer **valores** AC por padrão, como o Δ.5 da v1 fazia) é a deleção por padrão amplo que a própria I proibia.
Terceiro: os alvos do Δ.5 não são lixo da suite de evidência — `ac-roundtrip-value-*` é escrito por `clw-conformance` (outra suite, tenant compartilhado) e **recriado a cada canário agendado**.

**S.** Cada execução remove os **refs** que criou; os blobs são reclamados por alcançabilidade. O tenant não cresce com o número de execuções.

**Δ.**
1. Teardown deleta **só refs**, por `ref_key_in(domain, name)`, para o conjunto **fechado e enumerável** de nomes de persona do run. **[v3]** As journeys já fazem `rm <nome> --yes` no beat de cleanup (verificado em `story_security.rs:224-227`) — ou seja, a metade dos refs já existe; o que falta é torná-la sistemática e cobrir os nomes que escapam.
2. Blobs reclamados por `clw prune --yes` no tenant cercado, que já tem live-set fail-safe e recusa de wipe total (`prune.rs:100-110`).
3. Reconciliação = `list_refs` menos a baseline conhecida-boa, **não** contagem por prefixo.
4. Falha de limpeza reporta e conta órfãos; não avermelha a journey; não trava.

**I.** **G9.** Nenhuma deleção por padrão amplo, por prefixo ou por varredura de valor. Nenhum blob deletado sem live set provado.

**T.** Contra mock: cria N objetos, roda teardown, afirma que os N sumiram **e que um objeto pré-existente não foi tocado**. **[v3]** O controle negativo é reforçado: o objeto de controle é criado por **identidade diferente**, verificado presente após o teardown **por listagem**, e o teste afirma sobre o **log de operações do mock** que nenhum delete teve alvo fora do conjunto registrado — não só sobre o estado final.

**A.** `list_refs` volta à baseline · contagem de entradas não relacionadas inalterada (listagens antes/depois anexadas) · falha de rede injetada: reporta órfãos, não trava · **`0 skipped` no run que serve de evidência** (senão o teardown não foi exercitado).

**R.** Reverter volta a vazar. **O que foi deletado não volta.**

---

## WP-26b — Limpeza única do passivo no tenant canário

**Closes:** M15 (metade operacional) · **Depends:** WP-26 · **Risk:** ALTO · **Class:** ops

**Extraído do Δ.5 da v1**, que embutia uma **ação de prod irreversível** num PR de código cujo **R** era "reverter".

**S.** As entradas acumuladas de execuções passadas foram removidas, com evidência.

**Δ.** Dry-run listando exatamente o que seria apagado → revisão de quem é dono do canário de conformance (que é quem **produz** as entradas `ac-roundtrip-value-*`, e vai continuar produzindo) → execução → evidência.

**I.** G9. Nada é apagado sem que a lista tenha sido lida por um humano.

**T.** O dry-run é o teste.

**A.** Lista do dry-run anexada e aprovada · execução com listagem antes/depois · nenhuma entrada não-relacionada tocada.

**R.** **Nenhum.** É por isso que é WP separado, com sign-off.

---

## WP-12 — e2e-evidence: binário de release e execução agendada

**Closes:** M35, M2 · **Depends:** WP-24, WP-26, WP-09, WP-50 · **Risk:** MÉDIO · **Class:** ci

**S.** A suite compila e dirige o binário `--release`, roda semanalmente, e sua ausência de execução é vigiada.

**Δ.**
1. `cargo build --release` e `CLW_E2E_BIN=$(pwd)/target/release/clw`.
2. `schedule:` semanal, com horário fora dos já escalonados (stress 02:00, mutation 04:00, live-journeys 06:17, fuzz 08:00, metered 12:00, conformance 13:00, deadman 16:00 — todos comentados como offsets deliberados).
3. Registrar a lane na matriz do deadman.
4. **[v3] A persona J8 (cred-ticket) nunca rodou** — é a única do suite nessa condição, exercita o caminho C2c (o mesmo que WP-10 e WP-24 endurecem), e a v1 queria **agendar** o suite com esse buraco sem mencioná-lo. O WP declara explicitamente qual journey a suite não cobre, e por quê (hand-off externo pendente).
5. **[v3] O Δ.4 da v1 sai:** ele dizia "se necessário, reaproveitar cache de build" — cache que **não existe** (WP-50). Com `--release` numa microVM efêmera, é build frio completo, e o `timeout-minutes: 30` da lane vira risco real. Ou WP-50(a) entra antes, ou o timeout sobe deliberadamente.

**I.** A suite continua fail-loud. Nenhum soft-skip novo.

**T.** Dispatch manual verde com o binário release **antes** de ligar o `schedule:` — não se agenda o que nunca se viu passar no perfil novo.

**A.**
1. Run verde com `0 skipped` e N == contagem de personas.
2. O relatório contém o caminho resolvido do binário e ele termina em `/target/release/clw` (acrescentar o echo se não existir).
3. `schedule:` presente; a lane aparece no deadman.
4. Duas execuções agendadas consecutivas verdes antes de fechar o WP.

**R.** Remover o `schedule:` (volta a manual). **Mas** isso não desfaz o que as execuções já agendadas deletaram de prod via o teardown do WP-26 — o rollback restaura a config, não o tenant.

---

# SESSÕES 10-11 — Deleção, symlinks, cache

---

## WP-27 — Concorrência bounded nos sweeps destrutivos

**Closes:** M16 · **Depends:** WP-38b (o teste de sinal, para não mexer no caminho destrutivo sem cobertura) · **Risk:** MÉDIO · **Class:** code

**S.** `erase` e `prune` deletam com concorrência bounded dentro de cada fase, preservando a ordem entre fases e a veracidade do relatório parcial.

**Δ.** `buffer_unordered(16)` dentro da fase de refs e, separadamente, da fase de blobs, em `subcmds/erase.rs:~255-275` e `subcmds/prune.rs:~158-169`. A ordem refs→blobs é **requisito de correção**, não estilo.

**I. [v3 — a I da v1 apontava para o lugar errado].** Ela citava G1 e a ordem do GC; o perigo está na **veracidade do relatório sob falha**. Os loops seriais dão hoje uma propriedade de **prefixo** que o próprio código chama de load-bearing: `erase.rs:227-232` diz *"that total is the audit number and must be truthful"* (Art. 17), e `prune.rs` reporta `failed_at` como **o** digest que falhou. Sob `buffer_unordered`, no instante do primeiro erro há até N−1 DELETEs em voo de desfecho desconhecido: o conjunto deletado sob falha **não pode** ser idêntico ao serial, `refs_deleted` vira limite inferior, e `failed_at` vira "um dentre vários" — mudança silenciosa de **significado** de chave JSON existente, que **G2 agora protege**.
**Invariante:** as contagens reportadas continuam sendo contagem **verídica** de deleções comprovadamente concluídas, e `failed_at` continua sendo um único digest nomeado. Concretamente: no primeiro erro, **aguardar** as em voo até concluir (não cancelar) e contar só as que retornaram `Ok`.

**T. [v3 — "comparar com o serial" é impossível depois da mudança].** A v1 pedia igualdade de conjunto com a versão serial — que deixa de existir; o teste compararia contra um conjunto escrito à mão, isto é, contra a crença do autor.
1. **Golden set capturado ANTES**, commitado no mesmo PR: um teste no commit pré-mudança grava o conjunto deletado exato para a fixture F; pós-mudança, a implementação concorrente reproduz o arquivo byte a byte.
2. **Ordem de fase provável:** o mock registra log de operações com timestamp e afirma `max(t_ref_delete) < min(t_blob_delete)` para 200 entradas em N=16. Reverter a barreira → vermelho.
3. **Semântica de falha determinística:** o PR **escreve primeiro** a semântica escolhida (cancelar em voo vs. drenar), depois afirma os invariantes que sobrevivem ao nondeterminismo — exit code idêntico ao serial (número pinado); `refs_deleted` == o que o transporte viu concluir; todo digest reportado como deletado ausente numa re-listagem; nenhum delete de blob emitido após a primeira falha na fase de refs. Rodar 50×; flake = falha.

**A.** Os três acima · **limiar de desempenho nomeado**: ≥4× em fixture de 500 refs contra o harness wiremock em N=16 — a v1 pedia "ganho medido", satisfeito por uma regressão. Se não atingir, o WP não vale o risco e é fechado.

**R.** Reverter.

---

## WP-28 — Guarda contra cópia autorreferencial no fallback do Windows

**Closes:** M17 · **Depends:** — · **Risk:** BAIXO · **Class:** code

**S.** No hydrate em Windows, um symlink cujo alvo resolvido contenha o próprio link é rejeitado com erro claro, em vez de disparar cópia recursiva até MAX_PATH.

**Δ. [v3 — reestruturado, porque o teste da v1 era morto na chegada].**
1. **O predicado de containment é função comum, NÃO `cfg(windows)`**, em `clw-hydrate`, com teste de tabela rodando no `cargo test` normal em unix. Motivo: `gates.sh:151` é `cargo check --workspace --target x86_64-pc-windows-gnu` **sem `--all-targets`** (verificado), então um teste `cfg(windows)` **não é sequer compilado**; e `windows.yml` roda apenas `wine clw.exe --version` e `--json doctor`, com o próprio cabeçalho dizendo que o harness `assert_cmd` e a semântica de FS "não rodam confiavelmente sob Wine". A v1 entregaria uma guarda contra encher disco com **zero cobertura executada em lugar nenhum**.
2. O materializador `cfg(windows)` (`clw-hydrate/src/lib.rs:688-730`) chama o predicado antes de `copy_dir_recursive`.
3. `gates.sh` passa a `cargo check --workspace --all-targets --target x86_64-pc-windows-gnu`, para que o código `cfg(windows)` seja ao menos **compilado** em todo PR. Barato, e fecha um buraco sistêmico, não só o deste WP. Pode revelar quebra pré-existente — é o ponto.
4. `windows.yml` ganha um passo wine que hidrata uma fixture com `d/up -> ..`, afirmando saída não-zero e diretório abaixo de 1 MiB. **Se o wine não der conta, o PR diz isso em voz alta**: "o materializador Windows é não-provado; só o predicado é testado".
5. **Não** endurecer o validador geral: `link → ..` relativo intra-workspace é legítimo em unix.

**I.** G3, G8 — nenhum manifest válido no unix passa a ser inválido. A rejeição é do **materializador** do Windows, não do formato.

**T.** Tabela do predicado incluindo o falso positivo de prefixo de string (`/a/b` vs `/a/bc`, e as variantes de drive/UNC insensíveis a caixa); red-on-revert por construção.

**A.** A tabela verde no `cargo test` de unix · o cross-check com `--all-targets` verde · o passo wine (ou a declaração explícita de não-cobertura).

**R.** Reverter.

---

## WP-29-IMP — Implementar a decisão de containment de symlink

**Closes:** M18 · **Depends:** WP-29-DEC, WP-15 · **Risk:** (a) MÉDIO / (b) BAIXO · **Class:** code ou docs

**Δ (a) — endurecer.** Resolução em ponto fixo através dos symlinks conhecidos do manifest (o grafo está disponível na validação canônica), com limite de profundidade e detecção de ciclo. **I:** G8 — pode invalidar manifests hoje aceitos; se algum real for afetado, é mudança quebrante com CHANGELOG e provável bump de `MANIFEST_VERSION`. **A:** a cadeia de 2 saltos do achado (`d/up → ..` + `d/esc → up/../../secret`) rejeitada; symlinks legítimos aceitos; custo medido (é O(entradas × profundidade) no pior caso).
**Δ (b) — rebaixar o texto.** Uma frase precisa em THREAT-MODEL, SECURITY e GUIDE (coordenada com WP-15) mais nota no doc de contrato para consumidores. **A:** as três descrições concordam entre si e com o código, e a limitação está nomeada onde um consumidor a leria.

**Recomendação: (b)**, porque o clw não segue links e o dano real hoje é a promessa errada — que se conserta com texto, não com risco de invalidar manifests existentes; (a) fica como backlog com dono.

---

## WP-33 — Fazer a eviction do cache ser de fato LRU

**Closes:** M22 · **Depends:** — · **Risk:** BAIXO · **Class:** code

**S.** A eviction remove os blobs menos recentemente **usados**, como o código e o GUIDE afirmam, não os mais antigos por data de escrita.

**Δ.**
1. Bump de mtime em `get()` com hit (`clw-cache/src/lib.rs:297-313`) e em `put()` de blob já existente (`:320-324`, hoje um no-op silencioso), best-effort.
2. **[v3] Usar `std::fs::File::set_modified`** (estável desde 1.75; toolchain é 1.91.1), **não** `filetime`: este último é **dev-dependency** e promovê-lo a dep de runtime mexe em `deny.toml`, `about.toml` e `THIRD-PARTY-LICENSES` — mudança de supply chain que o WP não previa.
3. Manter a ordenação do gc por mtime (`:425`), que passa a significar LRU de verdade.
4. Se o custo do syscall no caminho quente for relevante, amortizar — e então o doc diz "LRU **aproximado**".

**I.** É cache: nenhuma mudança pode transformar miss em erro. Falha de `set_modified` é ignorada com log em nível debug.

**T. [v3 — armadilha de granularidade que a v1 não previa].** `sort_by_key` é **estável**, então mtimes iguais empatam na ordem de walk dos 256 shards — arbitrária. Em FS de mtime grosso (HFS+ 1s, alguns overlays, NFS), escrever N blobs em laço apertado faz vários compartilharem mtime e a asserção vira cara-ou-coroa. Portanto o setup **fixa** mtimes explicitamente com `filetime::set_file_mtime` (já é dev-dep e já é usado em `lib.rs:544`), espaçados 60s, o mais antigo sendo o blob H.

**A.**
1. `get(H)` → `gc()` → `has(H)` verdadeiro e o segundo-mais-antigo evictado.
2. O mtime de H **estritamente aumentou** através do `get()` — prova que o bump ocorreu, não que H sobreviveu por outro motivo.
3. Reverter o bump → (1) vermelho. Rodar 20× sem flake.
4. **`put()` de blob existente também bumpa** — o Δ.1 nomeia isso e a v1 não tinha critério nenhum.
5. **Sem nova dependência de runtime:** `git diff Cargo.toml crates/clw-cache/Cargo.toml` não acrescenta nada em `[dependencies]`.
6. Limiar: <5% de regressão no bench de hit existente, ou o caminho amortizado é adotado e o GUIDE diz "aproximado".

**R.** Reverter.

---

# SESSÕES 12-13 — Cobertura de contrato e o trem de mudança quebrante

---

## WP-38a — Contrato de exit-code em Windows · **WP-38b — 128+n por sinal** · **WP-38c — AC create-only 409 live**

**Closes:** §9 itens 1-3 · **Risk:** BAIXO · **Class:** test

**[v3] Promovidos a WPs próprios.** A **A** do WP-38 da v1 era *"cada item vira teste; itens não feitos ficam como issue com dono"* — cláusula que **se dissolve**: qualquer subconjunto, inclusive o vazio, a satisfaz. Estes três cobrem contratos que o produto **publica e congela**; não podem ser sub-bullets com escape.

- **WP-38a:** o contrato (125/137/verbatim) não tem cobertura no gate — `e2e.rs` é `#![cfg(unix)]` e nenhuma lane roda teste Windows. Depende do `--all-targets` de WP-28 para ao menos compilar. **A:** ou o teste roda sob wine, ou o PR declara "compilado, não executado" — explicitamente, nunca por omissão.
- **WP-38b:** o mapeamento existe (`clw-run/src/lib.rs:682-694`) e **nenhum teste mata um filho para conferir 143**. **A:** `clw run <cmd>` sob `kill -TERM` produz 143; red-on-revert. **Bloqueia WP-42.**
- **WP-38c:** a semântica de create-only 409 — sobre a qual o UX inteiro de re-snapshot se apoia — nunca foi provada contra prod (o fix #220 tem teste unitário). **A:** caso na lane de conformance agendada, `0 skipped`, vermelho se o servidor mudar o contrato.

## WP-38d — Lacunas de teste restantes

**Closes:** §9 itens 4-10 **+ os 10 que morreram na consolidação do relatório** · **Risk:** BAIXO

**[v3] Os itens que nenhuma versão anterior carregava**, recuperados das notas de área: AC-get oversize sem teste direto (`client:1271`); fallback HEAD→GET do `exists` "não lê o body" não testado; mutação mid-snapshot; sem proptest sobre árvores geradas; **a leniência de `RefRecord::from_bytes` não pinada por teste algum** — crítico porque WP-39 quer **apertar** o parse e ninguém sabe qual é a leniência atual; `clw ls` sem argumento; `auth logout --all` real; dry-run humano do `uninstall`; **`hydrate --manifest-digest` sem persona** (a flag que o time de runners pina); integridade de corpo corrompido provada só em nível de integração. Mais §9.9 (caminhos TTY — zero pty no repo) e §9.10 (paginação multi-página live; 402/413 fora da journey de quota), que a v1 **omitia**.
Mais: chunker nos limites (o proptest domina 0..=3MiB e nunca atinge CHUNK_MAX 4MiB); cache multithread **e** multiprocesso (a v1 estreitava para multiprocesso); sentinel ilegível; abort do gc; fifo/socket/device; hardlinks; nomes unicode; streaming real >16MiB; disco cheio.

**A.** Cada item com status **DONE** ou **DEFERRED(dono, data)** explícito no PR — "não feito" vira decisão registrada, não aprovação implícita.

---

## WP-42 — Ciclo de vida de processo em `run`

**Closes:** L11, L12, L13 · **Depends:** **WP-38b** · **Risk:** ALTO (a v1 dizia MÉDIO) · **Class:** code

**Δ.** Hydrate: mover I/O bloqueante (`sync_all`, rename) para `spawn_blocking`, como o snapshot já faz (`clw-snapshot/src/lib.rs:234,272,411,427`) · `run`: digest streaming na chave de memo em vez de ler arquivos inteiros (`lib.rs:242,321,361` — rustc 200MB + input 4GB ≈ 4,2GB só para a chave) mais cache do digest da ferramenta por (path, len, mtime) · `run`: `kill_on_drop`.

**I. [v3 — G1 é sobre condições de emissão, não só valores].** Pôr o filho em **grupo de processos próprio** muda **entrega de sinal**, não o mapeamento: hoje um SIGINT/SIGTERM de terminal alcança o grupo em foreground, o filho morre por sinal e `status.signal().map(|s| 128 + s)` produz 130/143. Com grupo novo, o filho **não recebe** o sinal — o clw morre, o filho fica órfão, e `128+n` **nunca é emitido**. É mudança de condição de emissão de um código congelado sem alterar constante alguma, e a I e a A da v1 ("nenhum código de saída alterado") seriam ambas satisfeitas por essa quebra. Também mexeria no design deliberado de órfão/SIGPIPE (`:591-612`) e no grace bounded de H2 (`:625-668`).
**Invariante:** um sinal entregue ao grupo do clw continua alcançando o filho, e o clw continua saindo 128+n; `clw run <cmd>` sob `kill -TERM $$` num shell continua dando 143.
**Recomendação:** manter só `kill_on_drop` e **descartar a metade de process-group**.

**A.** O cenário de sinal como critério explícito (herdado de WP-38b) · benchmark antes/depois para L12 · nenhum código de saída nem condição de emissão alterados.

---

## Trem de mudança quebrante — WP-30, WP-31, WP-39, WP-41a, WP-32

Estes cinco saem **numa única release**, com nota destacada, coordenação cross-team e CHANGELOG. Agrupá-los era o propósito do RT-1 da v1, que depois os espalhava por ondas diferentes.

### WP-30 — Colisão case-fold e NFC · WP-31 — Nomes ilegais no Windows · WP-39 — Endurecimento de validação em `clw-types`

**Closes:** M19, M20, L1, L2, L3, L5, L6 · **Depends:** WP-16 · **Risk:** ALTO · **Class:** code

**[v3] Por que a v1 era BLOCKER — e a correção estrutural.** Tratei essas mudanças como do lado produtor. **Não são:** `validate_rel_path` está no caminho de **desserialização** — `clw-types/src/lib.rs:189` (`validate_canonical` o chama) ← `:216` (`from_bytes` chama `validate_canonical`). Três efeitos retroativos:
1. **`hydrate` de um snapshot do mês passado falha com exit 2** se o workspace tinha `aux.log`, `report.` ou `COM1.txt` — dado existente tornado ilegível.
2. **`prune` fica permanentemente inutilizado para aquele tenant:** `prune.rs:275-283` — falha de `from_bytes` na marcação retorna `live_set_incomplete`, que **por design aborta o sweep inteiro e não deleta nada** (`:203-217`: live set incompleto varreria blobs vivos como falsos órfãos). Um manifest legado e o tenant **nunca mais** coleta lixo. Idem para `deny_unknown_fields` do WP-39.
3. `ls --all` reclassifica aqueles refs.
E a I da v1 dizia "G3 para workspaces não afetados" — G3 é sobre **digests**, e nenhuma dessas mudanças toca digest: **a invariante declarada não constrangia a parte perigosa do Δ.** É a lacuna que **G8** fecha.

**Δ.**
1. Restrições novas **exclusivamente no caminho de produção** (`clw-manifest`, o walk/snapshot). `validate_canonical`/`from_bytes` continuam aceitando **tudo** que aceitam hoje.
2. Rejeição do lado consumidor, se desejada, é **gated em `MANIFEST_VERSION`**: bumpar e manter a antiga legível.
3. **Simetria obrigatória.** A v1 recomendava "aviso no snapshot, erro no hydrate" — isso fabrica snapshots **inidratáveis por construção**: `snapshot` sai 0, grava o ref, e o ref nunca pode ser hidratado; por `CLI-SURFACE-EXIT-FREEZE.md:46-49` exit 0 de snapshot significa sucesso, e o seam com os runners dirige `snapshot → hydrate → run`. **Regra: rejeitar no produtor, aceitar no consumidor.** Nunca o inverso.
4. **L1 completo:** a v1 cobria só `deny_unknown_fields` e omitia as duas metades que quebram identidade de digest — **chaves duplicadas last-wins** e **hex uppercase aceito** (`digest()` recomputado ≠ digest de origem). Normalizar-ou-rejeitar uppercase no parse; rejeitar chaves duplicadas. Sob a mesma regra de lado-produtor.
5. **L2/L3/L5/L6:** checagens numéricas em `validate_canonical` (soma de chunks vs. tamanho, tetos) · rejeitar control chars em componentes (escape de terminal via `ls`/`diff`) — **[v3] este item some do WP-39 e vai para o WP-31**, onde já estava na lista NTFS; a v1 o tinha duplicado em dois WPs de sessões diferentes, sobre a mesma função · truncar input ecoado nas mensagens (hoje um digest multi-MB é clonado para dentro da mensagem) · gating de versão antes do parse · newtype para `tenant`.
6. **[v3] Da lista de INFO que nunca chegou ao relatório:** sem limite de NAME_MAX / comprimento de path → `ENAMETOOLONG` tardio. Mesma classe de M20; entra no WP-31.

**I.** **G8** + G1 + G3.

**A. [v3 — o critério decisivo que a v1 não tinha].** *Toda fixture de manifest das versões v0.1.0–v0.1.12 continua parseando por `from_bytes` e continua hidratando.* Sem ele, todos os checks da v1 passavam verdes enquanto o produto perdia a capacidade de ler dados dos próprios usuários. Mais: tabelas de casos com controles positivos escolhidos para pegar over-matching (`console.log`, `communication.txt` contra `CON`/`COM1`); `CONFIG`/`config` e `é` em NFC/NFD; e o diretório de saída **vazio** após um hydrate rejeitado (substancia o "antes de escrever qualquer arquivo").

**[v3] Nota sobre WP-31 `Depends: WP-30`.** A v1 justificava com "mesmo validador, mesmos testes". **Não é o mesmo validador:** WP-31 estende `validate_rel_path`, predicado **por caminho, sem contexto**; a detecção de colisão do WP-30 é inerentemente **por diretório** — precisa do conjunto de irmãos, que `validate_rel_path` nunca vê, e portanto mora no walk do snapshot e no pre-flight do hydrate. Arquivos diferentes, sem conflito textual. Mantém-se a exigência de **coerência de política** (ambos escolhem avisar-vs-rejeitar do mesmo jeito), que é real; cai a serialização desnecessária.

### WP-41a — Diretório vazio no manifest · WP-41b — Política de dois-pontos **(rejeitado como escrito)**

**Closes:** L9 (41a); L10 **não fechado** · **Risk:** 41a MÉDIO

**41a.** Diretório cujos únicos filhos foram pulados some do manifest, quebrando a idempotência `snapshot→hydrate→snapshot`. Corrigir **adiciona entradas ⇒ muda digests**. A v1 punha isso na onda E com um "atenção a G3" e seguia adiante. Na v3 é **exceção explícita a G3**, entra neste trem de release e herda a coordenação cross-team.

**41b — rejeitado.** A v1 propunha uniformizar tratando arquivo com `:` no nome como "skip com aviso". Isso converteria uma rejeição dura (`clw-types/src/lib.rs:245-254`, deliberadamente barrando **qualquer** dois-pontos como vetor de NTFS ADS) num skip silencioso: `snapshot` sairia 0 tendo omitido arquivos e `hydrate` restauraria workspace incompleto. É **fail-open, viola G6**, no validador que existe como defesa de ADS. Se arquivos com dois-pontos não devem derrubar o snapshot, a resposta é opt-in nomeado (`--skip-unportable`, reportado no resumo **e** no JSON), nunca skip por default.

### WP-32 — Determinismo do walk

**Closes:** M21 · **Depends:** aviso ao runners TL · **Risk:** ALTO · **Class:** code

**Δ.** `.git_global(false).git_exclude(false).parents(false)` em `clw-manifest/src/lib.rs:65-70`. Hoje o builder usa `hidden(false)`, `.clwignore`, `follow_links(false)`, `require_git(false)` e herda os defaults `git_global(true)`/`parents(true)`/`git_exclude(true)` — e `require_git(false)` faz as regras de gitignore valerem **mesmo fora de repo git**, então o vazamento é real, não teórico.

**I. [v3 — o maior blast radius do plano, que a v1 não identificou].** Digest de manifest é **contrato cross-repo**: o time de runners fixou `toolchain_ref = <manifest digest>` (opção (b) ratificada pelo produtor hugit em 2026-06-28, `CheckDef.toolchain_ref = SnapshotReport.root`), viabilizada por `hydrate --manifest-digest` da v0.1.5. Mudar a semântica do walk invalida um digest que **outro time fixou em produção**, e reverter não desfaz a quebra do pin deles. **Aviso escrito ao runners TL antes do merge, com o novo digest.**
**Segundo efeito, omitido pela v1: o 409.** Por `CLI-SURFACE-EXIT-FREEZE.md:14-19`, re-snapshot do mesmo nome com conteúdo idêntico é no-op de sucesso; com conteúdo diferente é 409 salvo `--force`. Depois deste WP, **todo ref cujo workspace era filtrado pelo gitignore global vira 409 no dia em que o usuário atualiza** — o caminho feliz "já está atualizado" vira erro.

**A. [v3 — o critério da v1 não era executável].** "Mesma fixture em duas máquinas com git diferente" não tem harness e a CI tem uma classe de runner; seria humano fazendo à mão uma vez, na única WP autorizada a mudar digests. Substituído por algo **mais forte numa máquina só**:
1. Dois sandboxes com gitignores globais **contraditórios** (`*.log` vs `*.rs`) mais o baseline: `digest(A) == digest(B) == digest(sem config)`. Antes do fix, os três diferem — vermelho de três vias.
2. Vazamento de diretório pai: fixture em `$TMP/outer/ws` com `$TMP/outer/.gitignore` contendo o nome do arquivo; o arquivo **está** no manifest.
3. Dirigido pelo **binário** (`assert_cmd .env(...)`), não `std::env::set_var` — que muta o processo e corre com o pool de threads do `cargo test`; neutralizar `HOME`, `XDG_CONFIG_HOME`, `GIT_CONFIG_GLOBAL`, `GIT_CONFIG_SYSTEM`.
4. **Controle de estabilidade:** fixture sem ignores global/pai mantém o **mesmo digest** antes e depois (hex pinado) — é o que transforma "só os afetados mudam" de afirmação em prova.
5. Confirmação escrita do runners TL anexada · CHANGELOG nomeando a mudança quebrante **e o 409** · a mensagem de 409 nomeia esta mudança como causa provável.

---

# BACKLOG — com as 7 cláusulas, porque é onde os BLOCKER se escondem

A v1 entregou 14 WPs de onda E com só S/Δ/A. Foi exatamente esse padrão que escondeu os achados 1 e 15 das reviews. Aqui vão com esquema completo; os de `Class: code` **não podem ser despachados** sem I e T.

**WP-37 — Fuzz: corpus persistente e alvos vivos.** *Closes L28, L4.* **Δ:** `actions/cache` + corpus semente; alvos novos para `validate_symlink_target_contained`, `RefRecord::from_bytes`, `validate_tenant`; decidir o destino de `validate_symlink_target` (código morto que é o alvo atual). **I:** nenhum alvo aponta para código morto. **A [v3]:** "corpus cresce" **não** é profundidade — vale a contagem de `cov:` do libFuzzer maior que a partida a frio; e `grep -rn 'validate_symlink_target\b' crates/ --include=*.rs | grep -v test` mostra o callee do alvo em código de produto.

**WP-40 — Durabilidade e observabilidade do cache.** *Closes L7, L8.* **Δ:** `fsync` antes do rename (queda de energia pode deixar blob truncado com nome final; o `get` se cura, mas o `has` mente); log nas falhas de gc engolidas por `let _ =` (verificado: `lib.rs:358` engole o resultado do `gc`); gc que continua o sweep após erro num blob em vez de abortar tudo — hoje um erro **desativa a eviction em silêncio**. **I:** é cache; nada vira erro fatal. **T:** crash simulado (truncar blob, verificar autocura); gc com blob problemático continua e reporta. **A:** os dois testes, red-on-revert.

**WP-43 — Coerência da CLI.** *Closes L15-L26 exceto L14.* **Δ:** shapes JSON (`bytes` vs `bytes_total`; expor `secret_warnings`) · TOCTOU do `rm` e mensagem sobre hex uppercase · `auth rotate` saindo 0 sem fazer nada (verificado: emite `"rotated": false` e o exit é o contrato; os declined de rm/erase são non-zero) · `--json` ignorado em `completions` · soft-skip do redeem que engole regressão · asserts de wall-clock sensíveis a carga · knob de concorrência para `ls --all` · avisar em chave de config desconhecida · código morto (`Win32_System_Memory`) e fixtures com `/tmp/clw-cache` literal em `auth.rs:371,402,440` e `doctor.rs:229`. **I:** G2 — adicionar chave JSON é permitido, **renomear não**; onde a incoerência é de nome, adicionar o novo e manter o antigo por um ciclo de depreciação. **A:** teste por item; **arquivo golden com o conjunto de chaves de todo subcomando `--json`** — adicionar exige atualizar o golden, **remover falha**. Isso torna G2 um gate em vez de convenção de review.

**WP-43b — L14: comandos sem rede não queimam o ticket.** *Extraído porque é o item de maior risco do cluster e a v1 lhe dava "teste por item", satisfatível com um teste que afirma `true`.* **Δ:** `command_redeems_credential` (`main.rs:363-384`) inclui `init`, `auth login`, `auth rotate` — zero rede — e queima o ticket C2c single-use de que o workload precisa; `auth login` pós-redeem persistiria PAT curto no config. **I:** G6; a invariante declarada do C2c. **T/A:** com um fabric wiremock apontado, `init`/`auth login`/`auth rotate`/`completions`/`--help` produzem **zero** requisições de redeem; `snapshot`/`hydrate`/`run` produzem exatamente **uma**; red-on-revert por subcomando.

**WP-44 — Higiene de supply chain.** *Closes L48-L51, I4, I5.* **Δ:** remover `anyhow` morto de `Cargo.toml:73` + entrada órfã do lock · `futures` para `workspace.dependencies` (hoje inline em 4 crates) · declarar `rust-version` (MSRV) · `.gitignore` ganha `__pycache__/` e `.claude/settings.local.json` (verificado: **0 hits hoje** — só o gitignore global da máquina do dono cobre, então o clone de outro contribuidor está desprotegido) · corrigir o comentário do `deny.toml` (11 → 12 crates) · reavaliar `wiremock`/`tokio` como deps normais em `clw-e2e-evidence`, que fazem o `THIRD-PARTY-LICENSES` superatribuir. **I:** o binário shipped não ganha dependência. **A:** `cargo metadata` sem `anyhow`; gate verde; MSRV declarado.

**WP-45 — Segredos fora de argv e scrub.** *Closes L45-L47.* **P [v3]:** confirmar que o label `corelink` é **exclusivamente** Firecracker efêmero — se um box persistente pegar jobs, L45/L46 escalam de "janela curta" para "durável". A v1 carregava L45/L46 e **não** carregava a verificação que decide a severidade deles. **Δ:** token do tap fora de argv e do `.git/config` do clone, com `trap` removendo o diretório · segredos por stdin/env nos 4 sítios · scrub dos identificadores reais em `BUGREPORT-prod-ac-500.md` (tenant UUID + Clerk user id) e no RELAY. **I:** **L47 é bloqueador de qualquer mudança de visibilidade do repo** — marcar como tal. **[v3] L47 promovido** para junto do WP-21 (mesmos arquivos, mesma sessão): escrubar dois arquivos é 20 minutos e é bloqueador declarado. **A [v3]:** `ps` durante uma release é praticamente inexecutável e racy — vira critério **estático** (nenhum segredo em posição de argv nos 4 sítios, por grep) mais allowlist de identificadores sintéticos, falhando em qualquer UUID fora dela.

**WP-47 — Cluster INFO.** *Closes I1, I3, I7, I8, I9, I12.* **[v3] I8 era linha de matriz sem Δ** — o `Closes` da v1 listava 4 itens e o Δ implementava **zero**. Agora explícitos: sentinel `usize::MAX` de `marker_lines`; `expand_tilde` só `~/`; deps de integration-tests em `[dependencies]` em vez de dev; **`hostile.rs` DELETE responde 200 onde o contrato diz 204** (divergência de contrato, não nit). Mais: documentar o proxy de ambiente honrado por default (não vaza PAT — CONNECT tunela — mas surpreende); `TooLarge{size:0}` descartando o tamanho real; **`Retry-After` clampado a 10s** (I3, que a v1 também omitia do Δ); dedup de `map_auth_error`/`is_capability_unavailable` (verificado: usados em 5 subcomandos, com definições próprias em `rm.rs` e `status.rs`) e do harness wiremock (~700 linhas); comentários rotted. **A:** cada item com critério concreto ou fechado como wontfix — "sem regressão de gate" passa fazendo nada.

---

# Resumo da revisão individual

**56 WPs revisados um a um**, cada um verificado contra o código. Achados novos desta passada (marcados **[v3]**), por classe:

**Correções de fato sobre o repo:** enumeração de assets da release estava errada (26 assets, com 12 SBOMs e `install.sh` que eu não listava) · assinatura por binário é **Linux-only por design**, o que muda o texto de verificação do WP-17 para Windows/macOS · `docs/minisign.pub` é o **único** não-`.md` sob `docs/`, o que torna a regra do WP-06b cirúrgica · `docs/go-live/` é misto e não move em bloco · a falha da lane de mutação é **de harness, com shard que muda** (0/6 três vezes, 2/6 uma), logo flakiness de baseline, não mutante · `map_auth_error` tem definição duplicada em `rm.rs` e `status.rs` e uso em 5 · o cleanup atual das journeys já faz `rm --yes` do ref, então falta só o lado dos blobs.

**Defeitos de especificação corrigidos:** WP-04 mandava ajustar `fetch-depth` que já está correto · WP-14 apontava `gates.sh:131` quando a linha é a `:132` · WP-19 usava `grep -rc` sobre `.` · WP-16 tinha glob que não alcança o único crate com `unsafe` · WP-31 dependia de WP-30 por um motivo falso · o item de control chars estava duplicado em WP-31 e WP-39 · WP-21 usava `git log --follow` como prova de `git mv`.

**Itens recuperados que nenhuma versão anterior carregava:** a receita de `erase` sem o sufixo de verificação GDPR · a persona J8 nunca executada · a pergunta de visibilidade do SECURITY.md no mirror · o destino de logs/telemetria na pergunta de residência · o limite de NAME_MAX · `Retry-After` clampado · §9.9 e §9.10.
