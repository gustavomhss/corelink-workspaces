# Log de execução do catálogo — 2026-08-30

> **ARCHIVAL INTERMEDIATE LOG — NOT CURRENT STATE.** Entries below preserve what the session believed at
> each review round, including branches, pending verdicts and tooling later withdrawn. They are useful as
> rationale, but they are not a landed-work ledger. Use
> `../HANDOFF-2026-08-31-clw-TL-superaudit-campaign.md` and live git/GitHub state for status.

Registro por WP: o que foi feito, o que a cold review derrubou, e os achados
adjacentes descobertos durante a execução (que NÃO entram no WP corrente, para
preservar atomicidade, mas não podem se perder).

---

## WP-00 — Remover a claim falsa de provenance

**Branch:** `claude/wp-00-release-notes-slsa`

### Cold review #1 → REQUEST CHANGES

Dois BLOCKER, ambos aceitos:

1. **Nada estava commitado.** A edição existia só como modificação não-staged.
2. **A claim sobrevivia, e pior.** As notas terminam em `See docs/INSTALL.md.`, e
   esse documento não só repetia a claim (`:108`) como a transformava num comando
   acionável que erra (`:186-191`, `gh attestation verify`). Trocar uma mentira
   passiva por uma instrução que falha num caminho de verificação de segurança é
   pior que o estado inicial: treina o usuário a ignorar falha de verificação.
   O statement do WP ("o texto que toda release publica descreve o que ela traz")
   não era cumprido.

Mais três achados aceitos:

3. **MAJOR — o caminho `--clobber` nunca atualiza as notas.** Um re-run (que a
   CLAUDE.md registra como rotineiro, por `ENOTFOUND` no upload) só re-sobe assets.
   Uma release publicada com notas erradas as mantém para sempre.
4. **MAJOR — o guard era um comentário.** A doutrina registrada do repo após o
   v0.1.11 é *mecanizar, não auditar mais forte*; um comentário não teria pego o
   defeito original.
5. **MINOR — `docs/DISTRIBUTION.md`** afirmava provenance incondicionalmente em
   3 lugares.

### Escopo final (ampliado deliberadamente, por causa do achado 2)

WP-00 passou a ser "a falsidade de provenance, em toda superfície visível ao
usuário, mais o mecanismo que a mantém fora" — unidade coerente e atômica:

- `release.yml`: claim removida das notas; texto hoisted para `NOTES=` única,
  consumida pelo caminho de criação **e** pelo de re-run (`gh release edit`).
- `scripts/gates.sh`: guard mecanizado que inspeciona o **valor** de `NOTES=`
  (não o arquivo, senão o próprio comentário o dispararia) e falha se aparecer
  `slsa|provenance|attestation`; mais fail-closed se a atribuição for renomeada.
- `docs/INSTALL.md`: diz que attestation não é publicada e por quê; mantém o
  comando como nota explicitamente desabilitada, para que quem o tenha visto em
  outro lugar aprenda que a falha é **esperada**, não sinal de comprometimento.
- `docs/DISTRIBUTION.md`: 3 correções.
- `CHANGELOG.md`: entrada em `[Unreleased]`.

### Verificação executada

- **Nível argv, contra `origin/main`, com stub do `gh`:** o passo de publish passa
  11 argumentos idênticos exceto o valor das notas; base contém a claim (1), novo
  não (0). O caminho clobber vai de `view upload` para `view upload edit`, e o
  `edit` carrega as notas corretas sem a claim.
- **Red-on-revert do guard novo, dois modos:** reintroduzir a claim → GATE FAILED;
  renomear `NOTES=` para cegar o guard → GATE FAILED (fail-closed).
- YAML válido (5 jobs), `bash -n` no bloco extraído, shellcheck sem achados novos.
- Âncoras internas do INSTALL.md verificadas.

### Erros meus durante a execução (registrados por honestidade)

- Inseri o comentário-guard **dentro** de uma continuação `\`, o que o teria
  transformado em argumento do `gh`. Pego antes de commitar.
- Meu primeiro critério de aceitação (`grep -- '--notes' | grep -c SLSA`) era
  **vácuo**: a própria linha de comentário contém `--notes`. Exatamente a classe
  de defeito que as cold reviews acharam no plano.
- Meu teste de argv era **vácuo**: o log usa `[release] [create]` com colchetes e
  eu grepava `release create`, então os dois lados vinham vazios e o "argv
  idêntico" não provava nada. Corrigido com `grep -F '[release] [create]'`.
- Meu verificador de âncoras estava errado (colapsava espaços múltiplos num
  hífen; o GitHub converte cada espaço).

---

## Achados adjacentes — descobertos na execução, NÃO corrigidos aqui

| # | Achado | Evidência | Destino |
|---|---|---|---|
| A1 | `docs/INSTALL.md:82` linka `#windows-powershell-1`; o heading é `### Windows (PowerShell)` → âncora `#windows-powershell`. Link quebrado. **Pré-existente** (está em `origin/main`), sem relação com a claim. | verificador de âncoras | **WP-17 / WP-20b** |
| A2 | As notas apontam `See docs/INSTALL.md.`, mas o repo-fonte é privado e o arquivo não é publicado como asset — só `install.sh` vai para o mirror. Para o downloader público, o ponteiro é 404. Corrigir exige decidir: shipar `INSTALL.md` como asset (muda o conjunto de assets, afeta o critério do WP-01) ou apontar para uma URL do mirror. É decisão, não conserto de passagem. | `release.yml:790-800` stage só `minisign.pub`/`EULA.md`/`NOTICE`; `:955-992` empurra só `install.sh` | **WP-17** |
| A3 | `shellcheck` SC2086 em `gates.sh:164` (`$SCOPE` não-aspado no `crate_set`). Pré-existente; já mapeado. | `shellcheck scripts/gates.sh` | **WP-14** (o catálogo v3 já corrigiu a referência de linha) |
| A4 | Releases já publicadas (v0.1.0–v0.1.11) mantêm a claim falsa nas notas. O fix do `--clobber` só age em re-run da mesma tag; corrigir o passado exige editar notas de releases públicas — **ação outward-facing que precisa de autorização do dono**, não executada. | mirror `HuGR-Labs/clw-releases` | **decisão do dono** |

---

## WP-00 — auto-ataque ao guard (achado meu, antes da 2a review)

Ataquei os modos de bypass do guard que escrevi. Resultado:

| caso | esperado | real |
|---|---|---|
| `NOTES="… SLSA provenance …"` | pegar | **pegou** |
| `NOTES='… SLSA …'` (aspas simples) | pegar | **pegou** (via fail-closed) |
| `NOTES="…\n  SLSA provenance …"` (multi-linha) | pegar | **PASSOU — BURACO** |
| `NOTES="$NOTES SLSA provenance"` (concatenação) | pegar | **pegou** |
| limpo | passar | passou |

**O buraco.** O `sed -n 's/^…NOTES="\(.*\)"$/\1/p'` exige a aspa de fechamento na
MESMA linha. Numa atribuição multi-linha, a checagem de existência
(`grep -q '^\s*NOTES="'`) casa a primeira linha e passa, mas a extração do valor
devolve vazio — então o `grep` da claim não tem o que examinar e o guard aprova
vaciuamente. É exatamente a classe de vacuidade que as cold reviews acharam no
plano, agora no código que eu escrevi para impedi-la.

**Correção a aplicar (no commit de revisão, junto dos achados da 2a review):**
contar as declarações e os valores extraídos e exigir igualdade — se um `NOTES=`
não pôde ser lido como valor de linha única, o guard não está guardando nada e
tem de falhar:

```sh
decls=$(grep -c '^[[:space:]]*NOTES="' FILE || true)
values="$(sed -n 's/^[[:space:]]*NOTES="\(.*\)"$/\1/p' FILE)"
vals_n=$(printf '%s' "$values" | grep -c . || true)
[ "$decls" -gt 0 ]          || fail "sem atribuição NOTES="
[ "$decls" -eq "$vals_n" ]  || fail "NOTES= não legível como valor de linha única (multi-linha/heredoc?) — guard cego"
```

**Limite conhecido, a documentar no próprio guard:** o matcher
`slsa|provenance|attestation` é bounded — pega variações lexicais (`SLSA-3`,
`attestations`) mas não uma claim não-shipada expressa com outro vocabulário
(ex.: `in-toto`, ou uma promessa de Authenticode). O guard cobre a classe que
causou este incidente, não "toda afirmação falsa possível". Dizer isso no
comentário, para o próximo leitor não superestimar a garantia.

---

## WP-00 — Cold review #2 → REQUEST CHANGES (e revisão aplicada)

### O BLOCKER, verificado ao vivo pelo revisor

`v0.1.11` — marcada **Latest**, a página onde todo download e toda pergunta sobre
proveniência aterrissa — ainda serve a claim falsa AGORA. O fix do `release.yml` só
alcança tags cujo workflow seja re-executado, e ninguém re-roda `v0.1.9`. Enumerei:
**20 de 20** releases publicadas afetadas.

A frase do revisor que aceito por inteiro: shipar o fix enquanto o CHANGELOG diz "a
claim foi removida" **fecha o achado no papel**.

**Estado intermediário daquela rodada:** um draft de `scripts/backfill-release-notes.sh`
foi commitado numa revisão não-merged. Reviews posteriores provaram que ele não era
fail-closed e o retiraram. Nenhum backfill existe no tip; o draft sobrevive apenas no
histórico git anterior, é conhecido-defeituoso e **não deve ser recuperado nem executado**.
O CHANGELOG passou a dizer "**New** releases no longer advertise…" mais um
"**Still outstanding**" explícito nomeando as 20 releases e que qualquer backfill futuro é ação do dono.

### Quatro bypasses do guard (eu tinha achado um)

| bypass | eu | revisor |
|---|---|---|
| `NOTES=` multi-linha | achei | achou |
| `NOTES+=` append | — | achou |
| heredoc | — | achou |
| variável separada concatenada no argumento | — | achou |

O quarto é o mais instrutivo: `--notes "$NOTES $EXTRA"` rota o texto inteiramente em
volta de um guard que só inspeciona `NOTES`. E ele notou que minha alegação de
"provado red-on-revert nos dois modos" era **auto-confirmante** — testei exatamente
os dois modos para os quais escrevi o guard.

**Guard reescrito:** pina a FORMA antes de ler o valor (exatamente uma linha `NOTES`,
extraível como linha única, e todo `--notes` sendo a variável nua). Os 4 fecham.

### Over-correction que eu introduzi

A referência cruzada que adicionei para `[§4d]` criava um caminho **novo** do texto
correto para um texto falso idêntico: §4d exigia status Authenticode `Valid` enquanto
todo `.exe` shipado é unsigned. Meu commit passou a dirigir tráfego para o mesmo
defeito que ele existe para corrigir. Corrigido §4d (espelhando o hedge que §4c já
tinha) e a tabela de assets, que fazia a mesma afirmação.

### O mesmo erro, duas vezes, em dois arquivos — e o que ele ensina

Reusei o vocabulário do gate (`…|authenticode|notariz|…`) para detectar mentiras em
texto já publicado. O draft removido (somente no histórico anterior; não recuperar nem executar)
recusou **6 releases corretas**: o termo
sobrevivente era `notariz`, da frase **"binary not yet Apple-notarized"** — negativa
VERDADEIRA e exatamente a informação de que um usuário macOS travado no Gatekeeper
precisa.

Naquela revisão corrigi o draft... e **cometi o mesmo erro no gate**, que então reprovou o texto
honesto do próprio repo. O gate pegou meu erro — prova de que não é vácuo.

**Lição, escrita nos dois arquivos para não se perder:** um *removedor de claims* tem
de distinguir afirmação falsa de negativa honesta; um *gate de texto* não tem essa
obrigação, mas se banir palavras cegamente rejeita texto verdadeiro e útil. Solução:
lista explícita de frases conhecidas-verdadeiras, removidas antes do match. Qualquer
uso NOVO da palavra — inclusive uma reescrita da frase isenta — continua vermelho.

### Achados menores aceitos

- comentário do workflow exagerava o alcance ("reaches an already-published release")
  → corrigido, nomeando o limite; a referência ao draft de backfill foi posteriormente removida;
- `gh release edit` era best-effort com `::warning::` → agora **fatal**: um re-run que
  não corrige as notas não alcançou a idempotência que alega. Documentado o efeito
  colateral: o workflow passa a ser autoritativo sobre o corpo da release, então
  hand-edits na UI do GitHub são sobrescritos;
- `### Fixed` duplicado sob `[Unreleased]` → fundido no existente;
- "cryptographic provenance" em DISTRIBUTION → "cryptographic assurance", com a
  distinção explicada (minisign prova integridade e autenticidade, não proveniência);
- "signature of record" com dois referentes → adotei o termo do próprio workflow
  ("cross-platform signature-of-record");
- `grep -in` numerava linhas do stream do sed, não do arquivo → `-n` removido;
- âncora `#windows-powershell-1` quebrada (pré-existente) → corrigida, fix de um
  caractere num arquivo que eu já estava editando, declarado no commit.

## WP-00 — auto-ataque #2 ao guard endurecido (antes da rodada 3)

| caso | resultado | veredito |
|---|---|---|
| comentário mencionando `--notes` | VERMELHO | **falso positivo — defeito meu** |
| `NOTES = ` com espaços | VERMELHO | correto (fail-closed, forma não reconhecida) |
| frase isenta embutida em mentira ("not yet Apple-notarized **but SLSA provenance included**") | VERMELHO | correto — scrub remove só a ocorrência exata |
| frase isenta como cobertura ("**Apple-notarized**. not yet Apple-notarized") | VERMELHO | correto — a afirmativa fica exposta |
| `--notes-file` em vez de `--notes` | VERMELHO | correto (fail-closed) |

**A corrigir:** contar `--notes` ignorando linhas de comentário —
`grep -v '^[[:space:]]*#' "$rel_wf" | grep -c -- '--notes'` nos dois contadores.
Sem isso, a próxima pessoa que escrever um comentário sobre `--notes` leva uma
falha de gate confusa. A direção é segura, mas um guard que reprova texto inocente
gasta a credibilidade de que depende para ser levado a sério.

**Confirmado robusto:** a lista de frases conhecidas-verdadeiras **não** é abusável
como cobertura — o scrub é por ocorrência exata, então qualquer afirmação em volta
dela continua exposta ao matcher.

---

# Leva paralela 1 — WP-16, WP-23, WP-33

Executados por agentes em worktrees isolados, simultaneamente ao WP-00.

## Estado

| WP | branch | commit | gate | cold review |
|---|---|---|---|---|
| WP-00 | `claude/wp-00-release-notes-slsa` | 4 commits | ALL GATES GREEN | **pendente** (rate limit) |
| WP-16 | `claude/wp-16-forbid-unsafe` | `6e5bd9c` | ALL GATES GREEN (agente) | **pendente** |
| WP-23 | `claude/wp-23-empty-env` | `24b0235` | ALL GATES GREEN (agente) | **pendente** |
| WP-33 | `claude/wp-33-cache-lru` | `d76127b` | rodando | **pendente** |

**Rate limit de sessão (429) matou os 4 agentes às ~18:5x; reseta 19:40 (São Paulo).**
Nenhum merge feito — a meta exige cold review adversarial em tudo, e auto-review
não é cold review. Merges seguram até as reviews rodarem.

## WP-33 — terminado por mim (o agente morreu antes de commitar)

Trabalho estava completo na árvore, sem commit. Verifiquei antes de commitar:
- 27 testes verdes (eram 23 → 4 novos).
- **Red-on-revert nos dois sítios, com isolamento correto:** remover o refresh do
  `get()` derruba `get_refreshes_mtime_so_eviction_is_lru_not_fifo` e
  `mtime_refresh_is_amortised_within_the_interval`, mas NÃO o de `put`; remover o do
  `put()` derruba só `put_of_present_blob_refreshes_mtime`. Cada teste falha pela
  sua própria razão — não são vácuos.
- Zero dependência nova (`git diff Cargo.toml crates/clw-cache/Cargo.toml` vazio):
  produção usa `File::set_modified`, `filetime` fica dev-only.

## Auto-review do WP-16 (meu, não cold)

- **Suspeita levantada e REFUTADA:** o agente testou roots lib/bin/test mas não
  benches, e existem 4 (`clw-types`, `clw-chunk`, `clw-cache`, `clw-manifest`).
  Injetei `unsafe{}` em `crates/clw-cache/benches/cache_io.rs` e
  `cargo check -p clw-cache --benches` falhou com `-F unsafe-code`. **Benches são
  cobertos.** O lint alcança lib, main, bin, test e bench.
- **Escopo do `#[allow]`:** está no bloco `unsafe { … }`, não na função nem no
  módulo. Correto — o mínimo possível.

## Follow-ups registrados (NÃO feitos, para preservar atomicidade)

| # | item | por quê não agora | destino |
|---|---|---|---|
| F1 | Mecanizar "exatamente um `allow(unsafe_code)`" no `gates.sh`. Hoje a invariante depende de review — e esta sessão provou repetidamente que invariante não-mecanizada apodrece. O agente registrou em `REVIEWER-BRIEF.md` como reject-on-sight, que é controle social. | Tocaria a mesma região do `gates.sh` que o WP-00 já modifica → conflito de merge. Fazer depois do WP-00 mergear. | WP novo |
| F2 | Valores VAZIOS no arquivo de config ainda não são filtrados: `token = ""` em `~/.clw/config.toml` continua dando "authentication failed". Mesma classe do M12, sobrevive ao WP-23. | Fora do escopo declarado do WP-23 (que trata a superfície clap/env). | WP novo |
| F3 | `clw-cli` usa `deny`, não `forbid`, e fica FORA da herança de lints do workspace (o Cargo rejeita misturar `workspace = true` com override). Um lint futuro de workspace terá de ser espelhado à mão. | Limitação do Cargo, não do WP. Comentado no `Cargo.toml`. | registrar |

---

## WP-00 — round 7: the guard was abandoned, not repaired

Rounds 1-6 all returned REQUEST CHANGES. Round 6 named the pattern rather than
an instance:

> each version fixes the specific instances the previous review executed and
> then asserts the CLASS is closed, in prose, in three places.

Two of its five BLOCKERs made continuing indefensible regardless of the class
argument. The guard had grown to 386 lines that *executed* the release
workflow's steps to render their argv, which meant `sudo apt-get`,
`brew install`, `cargo install cargo-about`, and a call to the **production**
CoreLink quota endpoint carrying `x-corelink-internal-auth` — on every
developer's laptop, in every gate lane, for 4m22s of a 7m38s gate, leaking
orphaned processes. That is a worse artifact than the defect it was chasing.

The redesign is the one the round-4 reviewer proposed and I did not take:

* the published body moved to `.github/release-notes.md`, a committed file;
* `release.yml` renders it with `sed "s/__VERSION__/${VERSION}/g"` and passes
  `--notes-file`;
* the guard shrank to ~120 lines, executes nothing, and runs in 0.07s. It
  asserts five shape properties of `release.yml` and judges no text.

**What I gave up, deliberately.** The guard no longer claims to prove that
nothing anywhere publishes a false release body. It cannot: `curl` to the REST
API, `gh` reached through a variable or `xargs`, another workflow, or a person
with a browser all get there with it clean. Six rounds established that
enumerating those idioms is not winnable. The control is now that the words
live in a file review reads.

Every one of the five invariants is red on revert (ten sabotages run). One
sabotage I initially recorded as a vacuous GREEN was my own mislabelled test —
re-pointing `$RUNNER_TEMP` at another path is legitimate, because the body
still comes from the reviewed file. The correct sabotage (deleting the render
line while keeping the assignment) is red.

**The transferable lesson**, and the third time this session has produced it:
a guard that must enumerate what an adversary might write has already lost.
WP-16 reached the same place by delegating `unsafe` detection to the compiler.
Here the move is structural rather than delegated — put the text where review
already looks, then guard only the structure that keeps it there.

---

## H1 CONFIRMED WITH HARD EVIDENCE — `main` is not protected, and cannot be

While reviewing WP-35 an agent reported that branch protection was
unverifiable. I checked it directly rather than take the report:

```
$ gh api repos/HuGR-Labs/corelink-workspaces/branches/main/protection
{"message":"Upgrade to GitHub Pro or make this repository public to enable
 this feature.", "status":"403"}
$ gh api repos/HuGR-Labs/corelink-workspaces/rulesets
{"message":"Upgrade to GitHub Pro ...", "status":"403"}
$ gh api orgs/HuGR-Labs --jq '.plan.name'   ->  free
$ gh api repos/HuGR-Labs/corelink-workspaces --jq '.private'  ->  true
```

A private repository on a free organisation plan cannot have branch
protection or rulesets AT ALL. This is not "protection was switched off" —
the feature is unavailable on the current plan.

`CLAUDE.md` states, as a repo convention: *"`main` is branch-protected
(`gates`+`dco` required, enforce_admins)"* and *"as of 2026-07-09 `main`
requires the `gates` + `dco` status checks GREEN and enforces admins,
force-push/deletion blocked, linear history."* None of that is in force.

This was H1 of the superaudit, recorded then as "branch protection OFF".
The true statement is stronger and worth writing down precisely, because the
weaker one invites the wrong fix: nobody can turn it back on. The options are
to make the repo public, to pay for Pro/Team, or to stop asserting a control
that does not exist and rely on the process discipline that is actually
holding the line today.

**It also changes what every gate in this catalogue is worth.** `gates.sh` is
not a merge blocker; it is a convention. Someone can push straight to `main`.
Every WP whose acceptance rests on "the gate catches it" rests instead on
"the gate catches it, provided a person runs it and honours the result." That
is worth saying out loud in the WP-02 decision rather than leaving implied.

Owner decision, and it is genuinely the owner's: this is a plan/visibility
question with a cost, not an engineering one.
