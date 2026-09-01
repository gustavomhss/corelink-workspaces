> **ARCHIVAL SNAPSHOT — NOT CURRENT STATE.** Captured against `25cfcaf` on 2026-08-30. Line numbers,
> severities and open/fixed claims below are contemporaneous evidence only. Consult
> `../HANDOFF-2026-08-31-clw-TL-superaudit-campaign.md` and the WP catalogue before acting.

## Agente CORE CRATES (clw-types/chunk/manifest/cache) — completo. 0 CRITICAL/HIGH, 6 MED.

MEDs (2 temas: validação de path/symlink lexical+unix-cêntrica; drift política-vs-doc):
1. types lib.rs:316-366 validate_symlink_target_contained aceita target que resolve para ANCESTRAL do próprio link (`d/up -> ..`) → no Windows hydrate copy-fallback (hydrate lib.rs:688-727) copy_dir_recursive com dst dentro de src → cópia auto-referencial até MAX_PATH (DoS).
2. types lib.rs:316-366 containment lexical unsound com symlinks irmãos: `d/up -> ..` + `d/esc -> up/../../secret` → em runtime escapa do workspace; quebra garantia documentada "nunca symlink externo" p/ consumidores que seguem links (MED pq clw não segue).
3. types lib.rs:225-257 sem defesa contra colisão case-fold/NFC-NFD → em fs case-insensitive (Win/mac) duas entradas → mesmo arquivo, last-write-wins, digest não reproduz.
4. types lib.rs:225-257 nomes ilegais no Windows passam (CON/NUL/COM1, trailing dot/space, ctrl chars, `< > " | ? *`) → falha tardia no hydrate Windows.
5. manifest lib.rs:65-70 walk: require_git(false) + defaults do crate ignore (git_global=true, parents=true) → gitignore GLOBAL do usuário + .gitignore de diretórios PAIS moldam snapshot → digest difere entre máquinas, arquivos somem sem aviso. Fix: .git_global(false)+.parents(false).
6. cache lib.rs:376-444+320-324+297-313 eviction é FIFO-by-write-time, não "LRU(mtime)" documentado: get/re-put nunca refrescam mtime → blobs mais quentes evictados primeiro (thrash perpétuo). Fix: mtime bump em get-hit e put idempotente.

LOWs:
- types sem deny_unknown_fields nos structs persistidos (Manifest/Entry/ChunkRef/RefRecord); dup keys last-wins; hex uppercase aceito → digest() recomputado ≠ digest de origem.
- validate_canonical sem checagem numérica (File.size vs Σchunks, total_size, chunk.size ≤ CHUNK_MAX, mode mask) — hydrate reimplementa caps (safe) mas ls/diff/status renderizam números inconsistentes.
- Componentes aceitam ctrl chars (`\n`, ESC) → escape de terminal via ls/diff; erros ecoam input inteiro (digest multi-MB clonado na msg).
- validate_symlink_target (strict) é código MORTO porém é o fuzzado; validate_symlink_target_contained (vivo) SEM fuzz target.
- Version gating pós-parse (erro serde opaco p/ manifest futuro); RefRecord sem campo version.
- Config.tenant String crua interpolada em cas_url/ac_url — sem newtype validante (parse-don't-validate).
- cache: sem fsync antes de rename (crash-only atomicity; power loss → blob zero-length com nome final; has() reporta presente sem integridade).
- cache: falhas de gc engolidas (`let _=`) sem log; gc aborta sweep inteiro no 1º erro não-NotFound → eviction silenciosamente desativada.
- manifest: dir cujos únicos filhos são skipped (symlink externo/fifo) SOME do manifest → snapshot→hydrate→snapshot não idempotente.
- manifest: política inconsistente — symlink externo = skip gracioso, arquivo com `:` no nome = falha TOTAL do snapshot (rotineiro em unix, ex. `build:12:30.log`).

INFOs: LocalCache::new ~770 syscalls todo open (gate no sentinel); sentinel ilegível → Some(u32::MAX) in-band + write não-atômico; put não confere hash (debug_assert grátis); walker erra ErrorKind (io::Error::other); casts `as u32` de CHUNK_* sem const assert; Digest deserialize aloca String+Vec por digest (visitor alloc-free possível); doc "lowercase" vs uppercase aceito; sem bound NAME_MAX/path-length (falha tardia ENAMETOOLONG); diff HashMap last-wins silencioso p/ manifests não-validados.

SÓLIDO: parse posture release-mode strict; zero panic alcançável em não-teste (varrido); clw-chunk mínimo+proptest equivalência FastCDC/StreamCDC; cache temp+rename com PID+tid+nanos, .tmp- excluído de gc/clear, get self-healing, 0700/0600; io_ctx preserva ErrorKind; Debug do Config redige PAT (testado); testes de regressão anotados com incidente.

TEST GAPS: fuzz não cobre validadores vivos (contained, RefRecord::from_bytes, validate_tenant); chunk proptest domínio 0..=3MiB nunca atinge CHUNK_MAX 4MiB (regime forced-cut não testado), sem boundary CHUNK_MIN/MAX ±1, mid-stream reader-error não testado; zero testes negativos case-collision/NFC/reserved-names; cache sem teste multithread/multiprocesso, mtime-refresh não pinado, sentinel ilegível não testado, gc abort não testado; manifest sem proptest de árvores geradas, gitignore global/parents não testado, zero testes Windows executáveis em CI; leniência de from_bytes sem teste pinando.
