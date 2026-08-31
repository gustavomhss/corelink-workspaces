//! The Windows symlink copy-fallback's self-reference guard (M17).
//!
//! # Why this exists
//!
//! Windows has no unprivileged symlinks, so [`crate::hydrate`] materialises a
//! `Symlink` entry as a COPY of its target. When the target is a DIRECTORY that
//! *contains the link itself* — canonically `d/up -> ..` — a naive recursive
//! copy walks `d` while writing into `d/up`, re-reads what it just wrote, and
//! recurses until it dies on MAX_PATH (or fills the disk). The manifest is
//! untrusted remote input, so that is a resource-exhaustion vector.
//!
//! # Two layers — a UNION, not authority-plus-garnish
//!
//! [`copy_would_recurse`] is the decision the materializer acts on. It refuses
//! the copy if EITHER of two independent checks says "contained":
//!
//! 1. [`path_contains`] — a cheap, purely lexical model of Win32 path folding.
//!    No syscalls.
//! 2. **`std::fs::canonicalize` on both sides** — the OS's own answer.
//!
//! An earlier draft of this file called layer 1 a "pre-filter" and layer 2 "the
//! authority". That was wrong in both directions, and a cold review was right to
//! reject it. Be precise about what each layer can and cannot do:
//!
//! * **Layer 1's positives are NOT verified by layer 2.** `copy_would_recurse`
//!   short-circuits on a layer-1 hit, so layer 2 never gets to disagree — and
//!   layer 1 is only an approximation of NTFS's `$UpCase` table. Rust's
//!   `to_lowercase` folds U+212A KELVIN SIGN to `k` and U+0130 to `i`+U+0307;
//!   NTFS does not. So layer 1 can OVER-REJECT: on Windows, a legitimate hydrate
//!   whose paths differ only by such a codepoint is refused, and layer 2 cannot
//!   rescue it. That is the deliberate direction — a false refusal is a loud,
//!   recoverable error, while a false accept is an unbounded recursive copy —
//!   but it is a real cost, not a free win.
//! * **Layer 2 alone is NOT sufficient.** When the link does not yet exist its
//!   canonical form has to be composed as `canonicalize(parent) + file_name`,
//!   and that final component is never resolved by the OS. See
//!   [`copy_would_recurse`] for the identity case this affects.
//!
//! Neither layer subsumes the other. The union is the guard.
//!
//! ## What layer 2 buys
//!
//! A lexical model of Windows path identity is a model of what an adversary
//! might write, and the first review of this WP found two ways past exactly that
//! model — both reachable from a manifest the validator accepts:
//!
//! * **trailing spaces and dots.** Win32 path parsing strips them, so
//!   `dest\d\sub\.. ` *is* `dest\d`. A predicate that special-cases only the
//!   literal `".."` reads `".. "` as an ordinary directory name and lets the
//!   recursive copy through — the original vulnerability plus one byte.
//!   `d/sub/up -> ".. "` validates (verified).
//! * **non-ASCII case.** NTFS folds case with a Unicode table, so `ä` and `Ä`
//!   are one directory. ASCII-only folding is case-SENSITIVE there, which
//!   rejects LESS — a fail-open. `Ä/sub/up -> "../../ä/sub"` also validates.
//!
//! Both are now handled lexically too (see [`normalize`]), but `canonicalize`
//! resolves trailing dot/space, Unicode case folding, 8.3 short-name aliases,
//! junctions and directory symlinks *without our having to enumerate them* —
//! it opens the path and asks the filesystem what it actually is. That is why
//! layer 2 exists even though layer 1 now covers the two known shapes.
//!
//! Layer 2 is **fail-closed**: if either side cannot be canonicalised, the
//! materializer refuses the entry rather than copying blind.
//!
//! # Why these are plain functions, not `cfg(windows)`
//!
//! Deliberate, and load-bearing. `cfg(windows)` code in this repo has **zero
//! executed test coverage anywhere**: the PR gate cross-*checks* the
//! windows-gnu target (a type-check, no test run), and `.github/workflows/
//! windows.yml` ran only `clw.exe --version` / `--json doctor` under Wine,
//! because the FS-semantics and `assert_cmd` harnesses do not run reliably
//! there. A `cfg(windows)`-only guard would have been code nobody ever executes.
//!
//! That is why the predicate is a plain function. This change then ALSO added a
//! wine step that runs the `lexical_` tests below on the windows-gnu binary — pure
//! computation over string literals, no filesystem — so the lexical layer now has
//! executed coverage on the real target ABI. The `canonical_guard_*` tests still do
//! not run there, and the `cfg(windows)` materializer body still runs nowhere.
//! `canonicalize` is cross-platform, so keeping both layers target-independent
//! means the whole mechanism — including the fail-closed path — is exercised by
//! a normal `cargo test` on unix.
//!
//! # What this still cannot see
//!
//! * The lexical layer models Windows rules while running on unix, so its
//!   trailing-dot/space and Unicode-case rows assert *our* folding, not NTFS's —
//!   and where the two differ it over-rejects (above).
//! * `canonicalize` is a TOCTOU snapshot: a concurrent writer could re-point a
//!   directory between the check and the copy.
//! * The two tests that exercise layer 2's case-FOLDING behaviour
//!   (`canonical_guard_alone_catches_unicode_case_on_a_folding_fs` and
//!   `canonical_guard_catches_the_identity_case_on_a_folding_fs`) need a
//!   case-insensitive filesystem and therefore **do not run in CI** — every lane
//!   here is Linux, which is case-sensitive, so there they return early and
//!   report `ok` having asserted nothing. Do not read a green CI run as evidence
//!   that they passed.
//!
//!   Layer 2's other six tests run everywhere, including
//!   `canonical_guard_resolves_an_aliased_final_component`, which pins the
//!   `canonicalize(link)` behaviour on ANY filesystem by aliasing the last
//!   component with a directory SYMLINK instead of a case variant. So the
//!   `canonicalize(link)` fix itself IS gated in CI; what CI does not cover is
//!   specifically case folding.
//!
//! The depth cap on `copy_dir_recursive` (see `lib.rs`) is the backstop for all
//! of it — no single oracle is trusted to be complete for untrusted input.

use std::path::Path;

// ---------------------------------------------------------------------------
// Layer 1: our lexical model of Win32 path folding.
// ---------------------------------------------------------------------------

/// A path reduced to comparable, normalised components.
struct Normalized {
    /// Whether the path is rooted (leading separator, or a `C:`-style drive).
    rooted: bool,
    /// How many leading components form the ROOT and so cannot be popped by
    /// `..` (1 for a `C:`-style drive designator, else 0). `C:\..` is `C:\`.
    root_comps: usize,
    /// Case-folded components, with `.` dropped and `..` applied.
    comps: Vec<String>,
}

/// Strip a `\\?\` / `\\.\` verbatim-or-device prefix, folding `\\?\UNC\` back to
/// the plain `\\server\share` spelling. Returns the input unchanged otherwise.
///
/// DIVERGENCE FROM WIN32, deliberate: this strips the prefix and then [`normalize`]
/// goes on to resolve `..` and trailing dots/spaces — whereas a real verbatim
/// (`\\?\`) path *disables* exactly that normalization, and `..` in one is a
/// literal directory name. So this folds two spellings Win32 would keep distinct.
///
/// It is safe here only because of where the guard sits: both arguments are
/// `dest.join(...)` paths, and a `\\?\`-prefixed path assembled this way fails
/// `symlink_metadata` in the materializer before the guard is ever consulted.
/// The divergence errs toward treating more paths as equal, i.e. toward
/// REJECTING — the safe direction. Revisit if this predicate is ever reused
/// somewhere a verbatim path can actually reach the filesystem.
fn strip_verbatim_prefix(s: &str) -> &str {
    let b = s.as_bytes();
    let is_sep = |c: u8| c == b'\\' || c == b'/';
    if b.len() >= 4
        && is_sep(b[0])
        && is_sep(b[1])
        && (b[2] == b'?' || b[2] == b'.')
        && is_sep(b[3])
    {
        let rest = &s[4..];
        let rb = rest.as_bytes();
        // \\?\UNC\server\share  ==  \\server\share.
        // `get(..3)` (not `rest[..3]`) — a byte-slice would PANIC on a non-ASCII
        // path whose 3rd byte is mid-codepoint.
        if rb.len() > 3
            && rest.get(..3).is_some_and(|p| p.eq_ignore_ascii_case("UNC"))
            && is_sep(rb[3])
        {
            // Keep the separator so the result stays rooted.
            return &rest[3..];
        }
        return rest;
    }
    s
}

/// Case-fold one component the way a case-insensitive filesystem would.
///
/// `to_lowercase` (Unicode), NOT `to_ascii_lowercase`: NTFS folds with a Unicode
/// table, so `Ä` and `ä` name one directory. ASCII-only folding would be
/// case-SENSITIVE for non-ASCII and therefore reject LESS — a fail-open.
///
/// This is still only an approximation of NTFS's `$UpCase` table, and NOTHING
/// downstream corrects it: `copy_would_recurse` short-circuits on a layer-1
/// positive, so where this fold and NTFS disagree (U+212A KELVIN SIGN folds to
/// `k` here, not on NTFS) the result is an OVER-REJECTION that layer 2 never
/// gets to see. Deliberate direction — a false refusal is loud and recoverable —
/// but do not read `canonicalize` as a safety net for this. See the module docs.
fn fold_case(s: &str) -> String {
    s.to_lowercase()
}

/// Apply Win32 trailing-`.`/space stripping to one raw component, returning the
/// component's *meaning*.
///
/// Windows strips trailing spaces and periods when parsing a path, but `.` and
/// `..` are special-cased first. So `".. "` means `..`, and `"up."` means `up`.
enum Comp<'a> {
    /// `.` (or anything that strips to nothing) — a no-op.
    Current,
    /// `..` — pop.
    Parent,
    /// An ordinary name, already stripped.
    Name(&'a str),
}

fn classify(part: &str) -> Comp<'_> {
    // Trailing spaces go first, so `".. "` is still recognised as `..` before
    // the trailing-dot strip would turn it into the empty string.
    let t = part.trim_end_matches(' ');
    match t {
        "" | "." => return Comp::Current,
        ".." => return Comp::Parent,
        _ => {}
    }
    let stripped = t.trim_end_matches(['.', ' ']);
    if stripped.is_empty() {
        // An all-dots component (`...`, `....`). Win32 rejects these as invalid
        // names rather than reading them as grandparents, and no legal file can
        // be named this, so we DROP it (same as `.`).
        //
        // Known imprecision, deliberately left: if some Windows path parser did
        // read `...` as "up two levels", dropping it under-resolves the path and
        // this layer would miss a NON-ancestor case such as `d/sub/... -> d`
        // with the link at `d2/link`. That case is covered by layer 2, which
        // asks the OS instead of guessing, and bounded by MAX_COPY_DEPTH if both
        // miss. Modelling a parent reading here would be a guess in the
        // fail-open direction, so we do not.
        Comp::Current
    } else {
        Comp::Name(stripped)
    }
}

/// Normalise a path into rooted-ness plus case-folded, `..`-resolved components.
fn normalize(p: &Path) -> Normalized {
    let raw = p.to_string_lossy();
    let s = strip_verbatim_prefix(raw.as_ref());
    let sb = s.as_bytes();

    // Rootedness is decided UP FRONT, from the raw string — deciding it from the
    // component vector afterwards loses the drive when `..` empties the vector
    // (`C:\..`), which would make it compare unequal to `C:\`.
    let leading_sep = sb.first().is_some_and(|&c| c == b'\\' || c == b'/');
    let has_drive = sb.len() >= 2 && sb[1] == b':' && sb[0].is_ascii_alphabetic() && !leading_sep;
    let rooted = leading_sep || has_drive;
    let root_comps = usize::from(has_drive);

    let mut comps: Vec<String> = Vec::new();
    for part in s.split(['\\', '/']) {
        match classify(part) {
            Comp::Current => {}
            Comp::Parent => {
                if comps.len() > root_comps {
                    comps.pop();
                } else if !rooted {
                    // A relative path CAN go above its own start; keep the `..`
                    // literal so an unresolved prefix never compares equal to a
                    // resolved one.
                    comps.push("..".to_string());
                }
                // Rooted and already at the root: `C:\..` is `C:\`, `/..` is `/`.
            }
            Comp::Name(n) => comps.push(fold_case(n)),
        }
    }

    Normalized {
        rooted,
        root_comps,
        comps,
    }
}

/// Layer 1: does `inner` look like `outer` itself or something underneath it,
/// according to our lexical model of Win32 path folding?
///
/// A `true` here is acted on directly and layer 2 never sees it, so a false
/// positive here is an OVER-REJECTION that nothing downstream can correct (see
/// the module docs — this fold is an approximation of NTFS's `$UpCase` table).
/// A `false` is not conclusive either: [`copy_would_recurse`] then asks the
/// filesystem. Neither layer subsumes the other.
pub(crate) fn path_contains(outer: &Path, inner: &Path) -> bool {
    let o = normalize(outer);
    let i = normalize(inner);

    if o.rooted != i.rooted || o.root_comps != i.root_comps {
        return false;
    }
    if o.comps.len() > i.comps.len() {
        return false;
    }
    o.comps.iter().zip(i.comps.iter()).all(|(a, b)| a == b)
}

// ---------------------------------------------------------------------------
// Layer 2: ask the filesystem instead of modelling it.
// ---------------------------------------------------------------------------

/// Resolve a manifest symlink `target` (relative to the LINK's parent) to a path
/// under `dest` — exactly what the Windows materializer copies FROM.
///
/// `target` uses `/` separators (backslashes are rejected by
/// `validate_symlink_target_contained`), so it is joined component-by-component
/// rather than as one string, which resolves correctly on Windows too.
///
/// Shared with the tests on purpose: a test that re-derived this join would be
/// asserting against its own copy of the logic instead of the real one.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn resolve_symlink_target(dest: &Path, target: &str, link: &Path) -> std::path::PathBuf {
    let link_parent = link.parent().unwrap_or(dest);
    let mut resolved = link_parent.to_path_buf();
    for comp in target.split('/') {
        resolved.push(comp);
    }
    resolved
}

/// LAYER 2 on its own: does the FILESYSTEM say `link` is `target_dir` or lies
/// beneath it?
///
/// Split out from [`copy_would_recurse`] so it can be tested directly. That is
/// not cosmetic: `copy_would_recurse` short-circuits on a layer-1 positive, so
/// every input that exercises an interesting layer-2 case is one layer 1 already
/// answers — which meant the layer-2 tests silently asserted layer 1 instead, and
/// reverting layer 2 outright left the suite green. Point layer-2 tests HERE.
///
/// `target_dir` must exist (the materializer has just `symlink_metadata`'d it).
/// `link` USUALLY does not (it is about to be created) — but it can: hydrate's
/// duplicate-path check compares paths byte-wise, so a manifest may carry both
/// `File d/X/f.txt` and `Symlink d/x`, and pass 3a creates `d/X` before pass 3c
/// reaches the link. On a case-folding filesystem those are one directory. So
/// canonicalise `link` DIRECTLY when it exists, and only fall back to
/// `canonicalize(parent) + file_name` when it genuinely is not there.
///
/// That fallback is this layer's known weak spot: the final component is then a
/// path we composed rather than one the OS resolved, so an alias that differs
/// only in that component (`d/x` vs `d/X`) is invisible to it. Getting the
/// identity case right matters most, because there `src == dst` and the copy
/// recurses immediately.
///
/// # Errors
///
/// Returns the underlying I/O error if either side cannot be canonicalised. The
/// caller must treat that as a REFUSAL, not as "no recursion": an unresolvable
/// path is exactly the case where we cannot prove the copy is safe.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn canonical_would_recurse(target_dir: &Path, link: &Path) -> std::io::Result<bool> {
    let canon_target = std::fs::canonicalize(target_dir)?;
    let canon_link = match std::fs::canonicalize(link) {
        // The link already exists — the OS resolved every component, including
        // the last, so an alias in the final component cannot hide here.
        Ok(c) => c,
        // The link is genuinely absent: the normal case, since it is about to be
        // created. Compose it from its parent, which does exist.
        //
        // ONLY NotFound falls back. A permission error, an ELOOP, or an I/O
        // failure means we could not resolve the link, and silently degrading to
        // the weaker composition there would trade a fail-closed refusal for a
        // guess. Propagate instead — the caller turns it into a refusal.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let parent = link.parent().ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "symlink path has no parent directory",
                )
            })?;
            let canon_parent = std::fs::canonicalize(parent)?;
            match link.file_name() {
                Some(name) => canon_parent.join(name),
                None => canon_parent,
            }
        }
        Err(e) => return Err(e),
    };

    // `Path::starts_with` is COMPONENT-wise, so `/a/b` does not contain `/a/bc`.
    Ok(canon_link.starts_with(&canon_target))
}

/// Would copying the directory `target_dir` into `link` recurse — i.e. is `link`
/// `target_dir` itself, or somewhere beneath it?
///
/// The union of the two layers: a positive from EITHER is a refusal. See the
/// module docs for why neither subsumes the other.
///
/// # Errors
///
/// As [`canonical_would_recurse`]. The caller must treat an error as a REFUSAL.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn copy_would_recurse(target_dir: &Path, link: &Path) -> std::io::Result<bool> {
    // Layer 1 — cheap, no syscalls. NOTE: this short-circuits, so layer 2 never
    // sees an input layer 1 already called contained. That is why layer-2 tests
    // must call `canonical_would_recurse` directly rather than going through here.
    if path_contains(target_dir, link) {
        return Ok(true);
    }

    // Layer 2 — let the OS decide identity, for everything our model of Win32
    // path folding does not or cannot cover (8.3 aliases, junctions, NTFS's own
    // case table, directory symlinks).
    canonical_would_recurse(target_dir, link)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use tempfile::TempDir;

    // -----------------------------------------------------------------------
    // Layer 1 — the lexical pre-filter table.
    //
    // REACHABILITY (a cold review's sharpest point, and it was right): the
    // manifest validator (`validate_symlink_target_contained`, clw-types) refuses
    // `\`, `:`, absolute targets and empty components, so a hostile TARGET can
    // never itself be spelled `C:\`, `\\?\`, `\\.\` or UNC. Those spellings still
    // reach this predicate through the other end — `dest` is user-supplied and on
    // Windows is an absolute, possibly-UNC, possibly-verbatim path that prefixes
    // BOTH arguments — but because it prefixes both identically, those rows are
    // defence-in-depth rather than live attack surface. The rows that are
    // REACHABLE AND WERE WRONG are the trailing-dot/space and Unicode-case ones;
    // they are marked `[REACHABLE]` below and each has an end-to-end fixture in
    // lib.rs.
    // -----------------------------------------------------------------------
    #[test]
    fn lexical_path_containment_table() {
        // (outer, inner, contained?, why)
        let rows: &[(&str, &str, bool, &str)] = &[
            // ---- [REACHABLE] the M17 case the materializer must reject ------
            (
                "/dest/d/..",
                "/dest/d/up",
                true,
                "[REACHABLE] d/up -> ..: the copy source resolves to the link's ancestor",
            ),
            (
                "/dest/d",
                "/dest/d/up",
                true,
                "[REACHABLE] link nested directly under the copy source",
            ),
            ("/dest/d", "/dest/d", true, "a directory contains itself"),
            (
                "/dest/a/b/../..",
                "/dest/a/b/link",
                true,
                "[REACHABLE] multi-level `..` still resolves to an ancestor",
            ),
            // ---- [REACHABLE] BLOCKER 1: Win32 strips trailing space/dot -----
            (
                "/dest/d/sub/.. ",
                "/dest/d/sub/up",
                true,
                "[REACHABLE] `d/sub/up -> \".. \"`: a trailing SPACE is stripped by Win32, \
                 so `.. ` is `..` — the M17 vuln plus one byte",
            ),
            (
                "/dest/d/sub.",
                "/dest/d/sub/up",
                true,
                "[REACHABLE] trailing DOT on an ordinary component: `sub.` IS `sub`. \
                 The outer sits AT sub deliberately — with `..` in it, the path would \
                 resolve to an ancestor and the row would pass with or without dot \
                 folding",
            ),
            (
                "/dest/d/sub/...",
                "/dest/d/sub/up",
                true,
                "[REACHABLE] an all-dots component is DROPPED (see `classify`), \
                 leaving /dest/d/sub — which contains the link either way. This row \
                 pins the drop, NOT a parent reading",
            ),
            (
                "/dest/d/sub/...",
                "/dest/d2/link",
                false,
                "the flip side of dropping `...`: if Win32 ever read it as a \
                 grandparent this would be a MISS. Layer 2 covers it; MAX_COPY_DEPTH \
                 bounds it if both layers miss",
            ),
            (
                "/dest/d/sub/.. . . ",
                "/dest/d/sub/up",
                true,
                "mixed dots and spaces strip to NOTHING and the component is dropped \
                 (same reading as `...`), leaving /dest/d/sub — an ancestor either \
                 way. Pins the drop, not a parent reading",
            ),
            (
                "/dest/d/sub",
                "/dest/d/sub./up",
                true,
                "[REACHABLE] the trailing dot is on the LINK side — again with the \
                 outer AT sub, so only dot folding can make this true",
            ),
            (
                "/dest/d/sub ",
                "/dest/d/sub/up",
                true,
                "[REACHABLE] a trailing space on an ordinary component folds away too",
            ),
            // ---- [REACHABLE] BLOCKER 2: NTFS folds case with UNICODE --------
            (
                "/dest/ä/sub",
                "/dest/Ä/sub/up",
                true,
                "[REACHABLE] `Ä/sub/up -> \"../../ä/sub\"`: NTFS folds ä/Ä to one directory, \
                 so ASCII-only folding was case-SENSITIVE here and rejected LESS",
            ),
            (
                "/dest/Ä/sub/../..",
                "/dest/ä/sub/link",
                true,
                "[REACHABLE] Unicode case folding survives `..` resolution",
            ),
            (
                "/dest/ÉCOLE",
                "/dest/école/x",
                true,
                "[REACHABLE] a whole non-ASCII component folds case",
            ),
            (
                "/dest/ÉCOLE",
                "/dest/ecole/x",
                false,
                "folding must not STRIP the accent: école is not ecole",
            ),
            // ---- the string-prefix false positive ---------------------------
            (
                "/a/b",
                "/a/bc",
                false,
                "STRING-prefix but not a PATH-prefix: /a/bc is a sibling, not a child",
            ),
            ("/a/b", "/a/bc/d", false, "same, one level deeper"),
            (
                "/dest/d2",
                "/dest/d/up",
                false,
                "unrelated sibling directories",
            ),
            (
                "/dest/d/up",
                "/dest/d",
                false,
                "containment is directional: the child does not contain the parent",
            ),
            // ---- defence-in-depth: spellings reachable only via `dest` -------
            (
                "C:\\x",
                "c:\\X\\link",
                true,
                "drive letter + component case folded",
            ),
            ("c:\\X", "C:\\x", true, "same path, different case"),
            (
                "C:\\..",
                "C:\\x",
                true,
                "`C:\\..` is `C:\\` — `..` cannot pop past a drive root",
            ),
            (
                "C:\\x",
                "D:\\x\\link",
                false,
                "a different drive is never contained",
            ),
            (
                "C:\\x",
                "C:\\xy",
                false,
                "case-folded string prefix is still not a path prefix",
            ),
            (
                "C:/x",
                "C:\\x\\link",
                true,
                "forward and backslash separate alike",
            ),
            (
                "/dest/d\\..",
                "/dest/d/up",
                true,
                "mixed separators in one path",
            ),
            (
                "\\\\?\\C:\\x",
                "C:\\x\\link",
                true,
                "\\\\?\\ verbatim prefix folds away",
            ),
            (
                "C:\\x",
                "\\\\?\\c:\\X\\link",
                true,
                "verbatim on the inner side too",
            ),
            (
                "\\\\.\\C:\\x",
                "C:\\x\\link",
                true,
                "\\\\.\\ device prefix folds away",
            ),
            (
                "\\\\?\\C:\\x",
                "\\\\?\\C:\\xy",
                false,
                "verbatim does not defeat component-wise compare",
            ),
            (
                "\\\\server\\share",
                "\\\\server\\share\\d\\link",
                true,
                "UNC share contains a path beneath it",
            ),
            (
                "\\\\SERVER\\Share",
                "\\\\server\\share\\link",
                true,
                "UNC host/share are case-insensitive",
            ),
            (
                "\\\\server\\share",
                "\\\\server\\share2\\link",
                false,
                "a different share is a string prefix, not a path prefix",
            ),
            (
                "\\\\other\\share",
                "\\\\server\\share\\link",
                false,
                "a different host is never contained",
            ),
            (
                "\\\\?\\UNC\\server\\share",
                "\\\\server\\share\\link",
                true,
                "\\\\?\\UNC\\ folds to the plain \\\\server\\share spelling",
            ),
            // ---- rooted vs relative -----------------------------------------
            (
                "/a/b",
                "a/b/c",
                false,
                "a rooted path never contains a relative one",
            ),
            ("a/b", "/a/b/c", false, "and the reverse"),
            ("a/b", "a/b/c", true, "two relative paths still compare"),
            (
                "C:\\x",
                "\\x\\link",
                false,
                "a drive-rooted path does not contain a merely separator-rooted one",
            ),
            // ---- roots --------------------------------------------------------
            (
                "/",
                "/anything/at/all",
                true,
                "the root contains every rooted path",
            ),
        ];

        // Collect EVERY failing row rather than panicking on the first: when a
        // regression (or a mutation run) breaks the predicate, the report then
        // names exactly which rows are load-bearing for which behaviour.
        let mut failures = Vec::new();
        for (outer, inner, expected, why) in rows {
            let got = path_contains(Path::new(outer), Path::new(inner));
            if got != *expected {
                failures.push(format!(
                    "  path_contains({outer:?}, {inner:?}) = {got}, expected {expected} — {why}"
                ));
            }
        }
        assert!(
            failures.is_empty(),
            "{} of {} containment rows failed:\n{}",
            failures.len(),
            rows.len(),
            failures.join("\n")
        );
    }

    /// A non-ASCII path must not PANIC the verbatim-prefix strip. `rest[..3]`
    /// byte-slices a `str` and dies with "byte index 3 is not a char boundary"
    /// when byte 3 is mid-codepoint.
    #[test]
    fn lexical_non_ascii_path_does_not_panic() {
        for p in [
            "\\\\?\\Ä",
            "\\\\?\\ä\\x",
            "\\\\?\\日本語\\x",
            "\\\\.\\é",
            "\\\\?\\UN\u{00c4}\\x",
        ] {
            let _ = path_contains(Path::new(p), Path::new("/x"));
            let _ = path_contains(Path::new("/x"), Path::new(p));
        }
    }

    /// The pre-filter must never claim containment for the ordinary,
    /// NON-self-referential symlink shapes that hydrate materialises every day
    /// on Windows — otherwise the guard would break working manifests.
    #[test]
    fn lexical_ordinary_symlink_copies_are_not_rejected() {
        // `mylink -> sub/inner.txt`: source under dest/sub, link at dest/mylink.
        assert!(!path_contains(
            Path::new("/dest/sub/inner.txt"),
            Path::new("/dest/mylink")
        ));
        // `bin/x -> ../src/x`: a legitimate intra-workspace `..`.
        assert!(!path_contains(
            Path::new("/dest/bin/../src/x"),
            Path::new("/dest/bin/x")
        ));
    }

    // -----------------------------------------------------------------------
    // Layer 2 — the canonicalising check, exercised against a REAL
    // filesystem. Cross-platform, so unix runs the whole mechanism including
    // the fail-closed path.
    // -----------------------------------------------------------------------

    /// The M17 shape, end to end on a real tree: `d/sub/up -> ..`.
    #[test]
    fn canonical_guard_catches_the_self_reference() {
        let td = TempDir::new().unwrap();
        let dest = td.path();
        std::fs::create_dir_all(dest.join("d/sub")).unwrap();

        // Exactly what the materializer builds: link_parent + the raw target.
        let link = dest.join("d/sub/up");
        let resolved = dest.join("d/sub").join("..");

        assert!(canonical_would_recurse(&resolved, &link).unwrap());
    }

    /// The value of stage 2 in one test: a directory SYMLINK on the target side
    /// makes the two paths lexically unrelated, so layer 1 says "no" — and the
    /// canonical comparison still catches the recursion. This is the same shape
    /// as the Windows-only tricks (trailing dot/space, Unicode case, 8.3 names)
    /// that the lexical model cannot be trusted to enumerate, in a form unix can
    /// actually execute.
    #[test]
    fn canonical_guard_sees_through_a_directory_symlink() {
        let td = TempDir::new().unwrap();
        let dest = td.path();
        std::fs::create_dir_all(dest.join("d/sub")).unwrap();
        // `alias` -> `d`, so dest/alias and dest/d are ONE directory.
        #[cfg(unix)]
        std::os::unix::fs::symlink("d", dest.join("alias")).unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_dir("d", dest.join("alias")).unwrap();

        let resolved = dest.join("alias");
        let link = dest.join("d/sub/up");

        assert!(
            !path_contains(&resolved, &link),
            "precondition: the LEXICAL filter cannot see through the symlink"
        );
        assert!(
            canonical_would_recurse(&resolved, &link).unwrap(),
            "the canonical comparison must still catch it"
        );
    }

    /// Does the filesystem under `dir` fold case, the way NTFS does?
    ///
    /// PANICS on an I/O error rather than returning `false`: a probe that turns a
    /// broken temp directory into "case-sensitive, skip the test" is a probe that
    /// fails open, and the whole point of these two tests is that they must not
    /// pass without doing anything.
    fn fs_is_case_insensitive(dir: &Path) -> bool {
        let probe = dir.join("Ä-CaseProbe");
        std::fs::create_dir(&probe)
            .unwrap_or_else(|e| panic!("case-fold probe could not create {probe:?}: {e}"));
        dir.join("ä-caseprobe").is_dir()
    }

    /// LAYER 2 ALONE defeats the Unicode-case attack — the BLOCKER-2 shape — on a
    /// filesystem that folds case, which is the same class NTFS is in.
    ///
    /// # THIS TEST NEVER RUNS IN CI — it is a dev-box probe
    ///
    /// It needs a case-INSENSITIVE filesystem. Every lane in this repo is
    /// `runs-on: corelink`, which is Linux, which is case-sensitive, so on CI
    /// this returns early and reports `ok` without asserting anything. Do not
    /// read a green CI run as evidence that this passed — it did not execute.
    /// It is kept because it *does* run on a macOS/APFS dev box, where it is a
    /// genuine check.
    ///
    /// **The BLOCKER-2 protection does NOT depend on this test.** What guards it
    /// in CI is `path_containment_table`'s two Unicode rows and
    /// `materializer_decision_refuses_every_self_referential_shape`; both go red
    /// on a case-sensitive filesystem if the Unicode folding is reverted.
    ///
    /// What this adds on a folding FS: it bypasses the lexical layer entirely and
    /// asserts the canonical comparison catches the shape by itself — the actual
    /// claim behind delegating identity to the OS.
    #[test]
    fn canonical_guard_alone_catches_unicode_case_on_a_folding_fs() {
        let td = TempDir::new().unwrap();
        let dest = td.path();
        if !fs_is_case_insensitive(dest) {
            eprintln!("skipped: filesystem is case-sensitive, nothing to fold");
            return;
        }
        std::fs::create_dir_all(dest.join("Ä/sub")).unwrap();

        // `Ä/sub/up -> ../../ä/sub` — validated as a legitimate manifest entry.
        let link = dest.join("Ä/sub/up");
        let resolved = resolve_symlink_target(dest, "../../ä/sub", &link);

        // Layer 1 answers this input too, so calling `copy_would_recurse` here
        // would assert layer 1 and never reach layer 2 — the defect that let an
        // outright revert of layer 2 leave this test green. Call layer 2 DIRECTLY.
        assert!(
            canonical_would_recurse(&resolved, &link).unwrap(),
            "layer 2 alone must catch the case-folded self-reference"
        );
    }

    /// The IDENTITY case, where `src == dst` and the copy recurses immediately.
    ///
    /// A manifest may legitimately carry both `File d/X/f.txt` and `Symlink d/x`
    /// — hydrate's duplicate-path check is byte-wise, so `d/X` and `d/x` are two
    /// distinct entries — and pass 3a creates `d/X` before pass 3c reaches the
    /// link. On a case-folding filesystem they are ONE directory, so the link
    /// path already exists when the guard runs.
    ///
    /// This is the case the `canonicalize(parent) + file_name` fallback cannot
    /// see: the alias is in the FINAL component, which that composition never
    /// asks the OS about. Canonicalising `link` itself fixes it.
    ///
    /// Skipped on a case-sensitive filesystem, where `d/x` and `d/X` genuinely
    /// are two directories and there is nothing to alias. Like its sibling probe
    /// test, that means it does NOT run on `corelink` — see
    /// `canonical_guard_alone_catches_unicode_case_on_a_folding_fs`.
    #[test]
    fn canonical_guard_catches_the_identity_case_on_a_folding_fs() {
        let td = TempDir::new().unwrap();
        let dest = td.path();
        if !fs_is_case_insensitive(dest) {
            eprintln!("skipped: filesystem is case-sensitive, nothing to alias");
            return;
        }
        std::fs::create_dir_all(dest.join("d/X")).unwrap();

        // `Symlink d/x -> "X"`, with `d/X` already materialised by pass 3a.
        let link = dest.join("d/x");
        let resolved = resolve_symlink_target(dest, "X", &link);

        assert!(
            link.exists(),
            "precondition: on a folding FS the link path already exists as d/X"
        );
        // LAYER 2 DIRECTLY — this is the test that pins `canonicalize(link)`.
        // Through `copy_would_recurse` it would be answered by layer 1 (`d/X` and
        // `d/x` fold to identical components) and would stay green even with the
        // whole of layer 2 reverted.
        assert!(
            canonical_would_recurse(&resolved, &link).unwrap(),
            "src == dst: layer 2 alone must see that copying d/X into d/x recurses"
        );
    }

    /// The same `canonicalize(link)` behaviour as the test above, but pinned on
    /// EVERY filesystem — including the case-sensitive Linux CI runs.
    ///
    /// The aliasing final component is a directory SYMLINK rather than a case
    /// variant, so no case folding is needed to make `d/x` and `d/X` one
    /// directory. That matters: the folding-FS test above is skipped on
    /// `corelink`, which would leave the `canonicalize(link)` fix pinned only on
    /// a developer's machine. This test closes that gap.
    ///
    /// It is the fallback's blind spot exactly: `canonicalize(parent) +
    /// file_name` never resolves the last component, so it yields `…/d/alias`
    /// and misses that the OS considers it `…/d/real`.
    #[test]
    fn canonical_guard_resolves_an_aliased_final_component() {
        let td = TempDir::new().unwrap();
        let dest = td.path();
        std::fs::create_dir_all(dest.join("d/real")).unwrap();
        // `d/alias` is a directory symlink to `d/real` — one directory, two
        // names, on ANY filesystem.
        //
        // The two names differ by more than case ON PURPOSE. An earlier draft
        // used `d/X` and `d/x`, which cannot both exist on a case-FOLDING
        // filesystem: `create_dir(d/X)` already occupies `d/x`, so the symlink
        // failed with AlreadyExists. That draft passed on a case-sensitive
        // volume and panicked on the dev box — the same "verified on one
        // filesystem" error this test exists to compensate for.
        #[cfg(unix)]
        std::os::unix::fs::symlink("real", dest.join("d/alias")).unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_dir("real", dest.join("d/alias")).unwrap();

        let link = dest.join("d/alias");
        let resolved = dest.join("d/real");

        assert!(
            canonical_would_recurse(&resolved, &link).unwrap(),
            "layer 2 must resolve the final component and see d/alias IS d/real"
        );
    }

    /// The ordinary case still copies: an unrelated sibling directory.
    #[test]
    fn canonical_guard_allows_an_ordinary_directory_copy() {
        let td = TempDir::new().unwrap();
        let dest = td.path();
        std::fs::create_dir_all(dest.join("src/x")).unwrap();
        std::fs::create_dir_all(dest.join("bin")).unwrap();

        let resolved = dest.join("bin").join("..").join("src");
        let link = dest.join("bin/x");

        assert!(!canonical_would_recurse(&resolved, &link).unwrap());
    }

    /// FAIL-CLOSED: an unresolvable path is an ERROR, never a silent "safe".
    /// The materializer turns this into a refusal.
    #[test]
    fn canonical_guard_fails_closed_when_a_side_cannot_be_resolved() {
        let td = TempDir::new().unwrap();
        let dest = td.path();
        std::fs::create_dir_all(dest.join("d")).unwrap();

        // Target does not exist.
        assert!(canonical_would_recurse(&dest.join("nope"), &dest.join("d/link")).is_err());
        // The link's PARENT does not exist.
        assert!(canonical_would_recurse(&dest.join("d"), &dest.join("nope/link")).is_err());
    }

    /// Component-wise on the canonical side too: `/a/b` must not contain
    /// `/a/bc`, even once both are real directories.
    #[test]
    fn canonical_guard_is_component_wise_not_string_prefix() {
        let td = TempDir::new().unwrap();
        let dest = td.path();
        std::fs::create_dir_all(dest.join("b")).unwrap();
        std::fs::create_dir_all(dest.join("bc")).unwrap();

        assert!(!canonical_would_recurse(&dest.join("b"), &dest.join("bc/link")).unwrap());
    }
}
