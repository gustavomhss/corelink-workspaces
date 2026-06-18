# Distribution rework — public Homebrew tap + GitHub Releases (PLAN, paranoid/SOTA)

> **Decision (owner, 2026-06-17):** open beta distributes `clw` via a **public Homebrew tap**
> (`brew install humangr-labs/clw/clw`) with a `curl | sh` `install.sh` fallback. Supersedes the prior
> "proprietary, authenticated R2-only, NOT a public GitHub Release" posture (`docs/DISTRIBUTION.md`).
> **`clw` stays proprietary-LICENSED (EULA governs use); it just becomes publicly DOWNLOADABLE** — the
> two are independent (many proprietary CLIs are `brew install`-able but not open-source).
>
> **Status:** PLAN — not yet executed. Owner authorized execution; this is the meticulous design + the
> work-packages + the paranoid risk register + the verification strategy, to be cold-reviewed before any code.

---

## 1. Target architecture

```
                    tag vX.Y.Z (real) / vX.Y.Z-rc.N (dry-run)
                                 │  release.yml
                                 ▼
   preflight ─ conformance ─ build×5 (signed) ─ SBOM ─┐
                                                       ▼
                                         publish job (REWORKED)
                                         ├─ GitHub Release  (PUBLIC binary host)
                                         │    • 5 binaries (mac arm/intel, linux arm/intel, windows .exe)
                                         │    • SHA256SUMS + SHA256SUMS.minisig + minisign.pub
                                         │    • SBOM (clw-<ver>.cdx.json)
                                         │    • rc tags ⇒ --prerelease; real vX.Y.Z ⇒ latest
                                         └─ bump Homebrew formula (only on REAL tags)
                                              → humangr-labs/homebrew-clw : Formula/clw.rb
   consumers:
     • macOS/Linux devs : `brew install humangr-labs/clw/clw`   (brew strips quarantine)
     • any platform     : `curl … install.sh | sh`              (fallback; macOS quarantine caveat)
     • Windows          : direct .exe download from the Release  (no brew on Windows)
     • the runner       : pins a binary by digest from the Release URL (digest = its sha256)
```

**Binary host = GitHub Releases** (public, free, immutable per-tag URLs, standard for brew taps).
**Tap = a separate public repo** `humangr-labs/homebrew-clw` holding `Formula/clw.rb`.

---

## 2. Paranoid / SOTA risk register (the gotchas — this is the meat)

| # | Risk | Verdict / mitigation |
|---|---|---|
| R1 | **macOS Gatekeeper quarantine** on an UNSIGNED/un-notarized binary | `brew install` **strips** `com.apple.quarantine` → brew-installed binaries run with NO Gatekeeper prompt even unsigned. So macOS notarization (G1, deferred) is **NOT a brew blocker**. BUT a **direct `curl install.sh`** download IS quarantined → Gatekeeper blocks. Mitigation: install.sh on macOS does `xattr -d com.apple.quarantine` on the downloaded binary (documented), and the README pushes `brew` as the primary path. Notarization closes the curl-path gap later (GA). |
| R2 | **rc dry-run tags creating public Releases** | The publish job must mark `vX.Y.Z-rc.N` as **`--prerelease`** (and NOT bump the brew formula) — only a real `vX.Y.Z` publishes "latest" + bumps the formula. Else every dry-run spams a public release and could point brew at an rc. Gate on the tag shape (the preflight already classifies). |
| R3 | **Cross-repo formula-bump token** (least privilege) | The release pipeline pushes to `homebrew-clw` → needs a **fine-grained PAT scoped to ONLY `homebrew-clw`, contents:write** (NOT a broad classic PAT, NOT the default `GITHUB_TOKEN` which can't cross repos). Stored as secret `HOMEBREW_TAP_TOKEN`. Paranoid: scope to the single repo, single permission; rotate per the runbook. |
| R4 | **sha256 single source of truth** | The formula's per-binary `sha256` MUST be derived from the SAME `SHA256SUMS` the pipeline already computes — never recomputed in a second place (drift = a formula that rejects the published binary). The formula-bump step parses `SHA256SUMS`. |
| R5 | **Atomicity / ordering** | Formula bump happens **only after** the GitHub Release + all 5 binaries upload successfully and are verified. Order: build→sign→release-upload→verify-downloadable→bump-formula. A failed upload must NOT leave the formula pointing at missing binaries. |
| R6 | **Independent verifiability** | Publish `minisign.pub` + `SHA256SUMS.minisig` in the Release so a user can verify the binary WITHOUT trusting brew (brew does its own sha256; minisign is the cross-platform signature-of-record). SBOM published too (supply-chain transparency). |
| R7 | **Formula correctness across 4 platforms** | `Formula/clw.rb` needs `on_macos`+`on_arm`/`on_intel` and `on_linux`+`on_arm`/`on_intel` url+sha256 blocks (Windows is not brew). `brew audit --strict` + `brew style` must pass in the tap repo's CI. Test `brew install` against an rc **prerelease** before the first real tag. |
| R8 | **Legal posture drift** | ToS/DPA/EULA/NOTICE/DISTRIBUTION currently assert "proprietary, authenticated, NOT a public GitHub Release." Public distribution contradicts that. Reconcile: keep the **proprietary license** (EULA still governs use), drop the **private-distribution** claims; state the binary is publicly downloadable, source remains closed. (Counsel deferred per waiver — factual reconciliation now, lawyer ratifies later.) |
| R9 | **Tap repo is a new public attack surface** | `homebrew-clw` is public + the pipeline auto-pushes to it. Protect it: branch ruleset, the bump is a PR-or-direct-push by the scoped token only, `brew audit` gate. A compromised tap = supply-chain risk → the minisign sig (R6) is the backstop users can verify. |
| R10 | **Runner digest pin (relay #1)** | The runner-TL needs a binary `sha256:` to pin. With public GH Releases, the runner pins the sha256 of `clw-<ver>-<target>` from `SHA256SUMS` (now public + stable per tag). Closes the open digest question — feed them the Release URL + SHA256SUMS once a real tag ships. |
| R11 | **`publish=false` / crates.io** | clw crates stay `publish=false` (not on crates.io); brew tap is the only package channel. No change — just confirm the rework doesn't accidentally enable crates publish. |
| R12 | **Reproducibility** | Formula points at immutable tag-pinned Release asset URLs + pinned sha256. The build is already pinned (toolchain, zigbuild, deps via Cargo.lock). SBOM rides along. |

---

## 3. Work-packages (sliced; disjoint owner-files; owner-gated flagged)

| WP | Scope | Owner-files | Gated? |
|---|---|---|---|
| **WP-A — legal reconcile** | ToS/DPA/EULA/NOTICE/DISTRIBUTION: proprietary-license-but-public-download; drop "authenticated/not-public" claims | `docs/*.md`, `EULA.md`, `NOTICE` | agent (counsel ratifies later) |
| **WP-B — publish→GitHub Releases** | Rework release.yml publish job: `gh release create/upload` (rc⇒prerelease), `permissions: contents: write`, keep minisign signing, publish minisign.pub+SHA256SUMS+.minisig+SBOM | `.github/workflows/release.yml` | agent (signing key still owner) |
| **WP-C — Homebrew tap** | Create `humangr-labs/homebrew-clw` repo + `Formula/clw.rb` (4 platforms) + tap CI (`brew audit/style`); release.yml step bumps the formula on real tags via `HOMEBREW_TAP_TOKEN`, sha256 from SHA256SUMS | NEW repo + `.github/workflows/release.yml` | agent creates repo (authorized) + owner provisions `HOMEBREW_TAP_TOKEN` |
| **WP-D — install.sh + Windows + docs** | install.sh → GH Release URL + macOS quarantine strip; INSTALL.md brew-first + curl fallback + Windows .exe; runner digest note | `scripts/install.sh`, `docs/INSTALL.md`, `docs/DISTRIBUTION.md` | agent |

Disjointness: A=docs, B+C share release.yml (sequence B→C, not parallel), D=scripts+docs. A and D both touch docs but different files → fine.

---

## 4. Verification strategy (dry-run, no real publish)

1. **release.yml dry-run via rc tag** (as proven for the build matrix): an `rc` tag must create a **prerelease** GitHub Release with the 5 binaries + checksums + SBOM, and must **NOT** bump the formula (rc gate). Verify the assets are attached + downloadable.
2. **Formula offline checks:** `brew audit --strict --online Formula/clw.rb` + `brew style` in the tap repo CI.
3. **End-to-end brew test:** against the rc **prerelease**, `brew install --formula ./Formula/clw.rb` on macOS + linux, run `clw --version`, confirm sha256 match + no Gatekeeper prompt.
4. **install.sh:** test against the rc prerelease URL (happy path + tamper + quarantine-strip on macOS).
5. **minisign verify:** `minisign -V -p minisign.pub -m SHA256SUMS` against the published assets (once the key exists).
6. Only after all green on an rc → the first real `vX.Y.Z` does the public "latest" release + formula bump.

---

## 5. Owner-gated (the short list)
1. **minisign keypair** — generate, commit `docs/minisign.pub`, provision `MINISIGN_SECRET_KEY`(b64)+`MINISIGN_PASSWORD`. (Signs binaries — even public, for integrity.)
2. **`HOMEBREW_TAP_TOKEN`** — fine-grained PAT scoped to `homebrew-clw` only, contents:write.
3. (Deferred to GA) macOS Developer-ID + notarization (closes the curl-install macOS quarantine gap; brew path doesn't need it), Azure Windows signing.
4. (Optional) confirm I may create the `humargr-labs/homebrew-clw` repo via `gh` (authorized 2026-06-17).

## 6. Rollout sequence
WP-A (legal) ∥ WP-D (install/docs) → WP-B (GH Releases publish) → WP-C (tap + formula bump) → rc dry-run verifies all → owner provisions minisign + tap token → first real `v0.1.0` → beta live.

## 7. Open questions for the owner / decisions still needed
- **Q1:** GitHub Release on the MAIN repo (`corelink-workspaces`, public-visible releases) OK? (The repo itself can stay private while Releases are... no — a private repo's Releases aren't public. **If the repo is private, public brew needs the binaries on a PUBLIC host** — either make Releases public via a public mirror repo, or host binaries in a public R2 bucket. **This is load-bearing:** is `corelink-workspaces` public, or do we need a public release/mirror repo?)
- **Q2:** tap repo name: `humangr-labs/homebrew-clw` → `brew install humangr-labs/clw/clw`. Confirm the org/name.

---

## 8. v2 — corrections after the adversarial cold review (REWORK verdict)

The paranoid review returned **REWORK** and was right: the v1 premise ("GitHub Releases on the main repo
as the public host") was invalid. Resolutions, all folded into the binding plan:

- **B1 RESOLVED — `corelink-workspaces` is PRIVATE; its Release assets are NOT public.** Decision
  (owner 2026-06-17): stand up a **public mirror repo `humangr-labs/clw-releases`** holding ONLY the
  release assets (binaries + `SHA256SUMS`(+`.minisig`) + `minisign.pub` + SBOM + attestation). Source
  stays in the private `corelink-workspaces`. **Every WP re-targets the mirror:** the publish job creates
  the GitHub **Release on `clw-releases`** (not the main repo); the formula `url`, `install.sh`
  `DEFAULT_BASE_URL`, and the runner's pin all point at the mirror's public asset URLs.
- **B2 RESOLVED — proprietary license vs `brew audit`.** The formula sets `license :cannot_represent`
  (closed source) and tap CI gates on **`brew style` only, NOT `brew audit --strict`** (which rejects a
  non-SPDX/closed license). Formula header states the proprietary posture + EULA pointer.
- **B3 RESOLVED — macOS signing is NEAR-TERM, not GA-deferred.** Real mechanism: a *formula*-installed
  binary isn't quarantined (only *casks* are), so it *currently* runs unsigned — but arm64 ad-hoc
  binaries can be `kill -9`'d on launch and Homebrew (5.0) is tightening Gatekeeper. So: **macOS
  Developer-ID signing is pulled forward to near-term** (full notarization can still wait); and the rc
  dry-run gate MUST run `clw --version` from a clean `brew install` on a **real arm64 Mac** (hard gate,
  not assumption).
- **B4 RESOLVED — e2e tests the REAL fetch.** The gate is `brew tap humangr-labs/clw && brew install
  humangr-labs/clw/clw` from the **public mirror, with NO repo credentials**, on arm64+intel macOS +
  linux (local `./Formula` install is insufficient — it masks the fetch path).
- **🟡 folded into the plan:**
  - **SLSA provenance:** add `gh attestation` on the release assets (verifiable with
    `gh attestation verify`); keep minisign as the offline signature-of-record. Provenance is the SOTA bar.
  - **rc classification:** pin ONE canonical rc shape and have the publish job reuse the preflight's
    `prerelease?` output — never re-parse the tag (a re-parse keying on `-rc.` with the dot would treat
    `v0.1.0-rc6` as a real release). rc ⇒ `--prerelease` + NO formula bump.
  - **Formula completeness:** `version` stanza, `livecheck` (points at the mirror's latest),
    `test do { assert_match version, shell_output("#{bin}/clw --version") }`.
  - **Rollback runbook:** bad asset already up + formula bumped + users' brew cache holding a bad sha →
    delete-release + revert-formula-commit + **bump to a NEW patch (never reuse a tag/URL)**. Add to the
    key-rotation/release runbook.
  - **Ordering:** provision the mirror+tap **branch protection BEFORE** wiring the push token.
  - **UX:** INSTALL.md documents `brew uninstall clw` + `brew install clw@<version>` / pinning.
  - **Token:** a **deploy key** on `clw-releases`+`homebrew-clw` (repo-bound by construction) is tighter
    than a fine-grained PAT; either is acceptable, deploy key preferred. NOT the default `GITHUB_TOKEN`
    (can't cross repos — confirmed).
- **🟢 confirmed sound:** cross-repo token genuinely required; sha256 single-source from `SHA256SUMS`;
  minisign+SBOM independent verifiability; `publish=false` unaffected; WP disjointness/sequencing.

**Revised owner-gated:** (1) **minisign keypair** + `docs/minisign.pub` + secrets — near-term; (2)
**macOS Developer-ID signing** — pulled forward from "GA-last" to near-term (notarization still later);
(3) **deploy key (or fine-grained PAT)** for `clw-releases` + `homebrew-clw` write; (4) I create the two
PUBLIC repos via `gh` (authorized) + apply branch protection. Azure/Windows signing stays last.

**Revised rollout:** create `clw-releases` + `homebrew-clw` (public, protected) → WP-A (legal) ∥ WP-D
(install/docs) → WP-B (publish → mirror Release + attestation) → WP-C (tap formula + bump on real tags)
→ rc dry-run proves real `brew install` from the mirror (incl. arm64 Mac) → owner provisions minisign +
macOS signing + deploy key → first real `v0.1.0` → beta live.
