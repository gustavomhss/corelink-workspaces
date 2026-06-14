//! Telemetry policy + consent gate (M5.3).
//!
//! # What this module is — and what it is NOT
//!
//! This is the **policy and consent scaffolding** for an opt-in, GDPR-consent-ready
//! telemetry pipeline. There is **NO network backend** here: [`record`] currently
//! routes events into a documented **no-op sink**. The point of this work package is
//! to land the *gate* — the off-by-default policy, the kill-switches, the first-run
//! consent concept, and a redaction-by-construction event type — so that a future
//! backend is **compliant by construction** the day it is plugged in.
//!
//! # Default: OFF
//!
//! Telemetry NEVER activates unless the user *explicitly opts in*. The decision is
//! resolved by [`Decision::resolve`] in this strict precedence:
//!
//! 1. **Kill-switches win, always.** `DO_NOT_TRACK=1` (the cross-vendor standard,
//!    <https://consoledonottrack.com>) or `CLW_TELEMETRY=0` force telemetry OFF and
//!    cannot be overridden by anything else.
//! 2. **Explicit env opt-in.** `CLW_TELEMETRY=1` turns it ON.
//! 3. **Recorded consent.** `telemetry = "on"|"off"` in `~/.clw/config.toml`.
//! 4. **Default.** OFF.
//!
//! In **non-interactive / CI / non-TTY** contexts the default stays OFF and we NEVER
//! prompt — a telemetry consent prompt must never block a script or a pipeline.
//!
//! # Redaction contract (enforced by construction)
//!
//! [`record`] does not accept free-form strings, paths, or tokens. It accepts only an
//! [`Event`] — a small **fixed enum of event kinds** carrying only non-PII fields
//! (an enum-coded subcommand name, a coarse outcome, a small duration bucket). It is
//! *type-impossible* to pass a PAT, a filesystem path, or file contents through this
//! API. See [`Event`] for the per-field justification.
//!
//! # Pluggable backend (TODO)
//!
//! When a real sink lands it should implement a `Sink` trait and be selected behind a
//! cargo feature; [`emit`] is the single seam where the no-op is replaced. Until then
//! every recorded event is dropped after the enabled-gate, by design.

use std::sync::OnceLock;

use crate::config::{clw_config_path, parse_config_lines};

// ---------------------------------------------------------------------------
// Resolved decision
// ---------------------------------------------------------------------------

/// The resolved telemetry decision for this process. Default is [`Decision::Off`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Telemetry is active (the user explicitly opted in).
    On,
    /// Telemetry is inactive (the default, or an explicit opt-out / kill-switch).
    Off,
}

impl Decision {
    #[must_use]
    fn enabled(self) -> bool {
        matches!(self, Decision::On)
    }

    /// Resolve the telemetry decision from env + config, honoring kill-switches.
    ///
    /// Pure over its inputs (no global state, no I/O beyond the supplied closures)
    /// so it is fully unit-testable:
    /// - `env` looks up an environment variable by name,
    /// - `config_value` returns the recorded `telemetry = ...` config value, if any.
    ///
    /// Precedence (first match wins):
    /// 1. `DO_NOT_TRACK=1` or `CLW_TELEMETRY=0` → **Off** (kill-switch, unoverridable).
    /// 2. `CLW_TELEMETRY=1` → **On** (explicit env opt-in).
    /// 3. recorded config `telemetry = "on"|"off"` → that choice.
    /// 4. otherwise → **Off** (off by default).
    fn resolve<E, C>(env: E, config_value: C) -> Decision
    where
        E: Fn(&str) -> Option<String>,
        C: FnOnce() -> Option<String>,
    {
        // (1) Kill-switches always win. DO_NOT_TRACK=1 is the cross-vendor standard.
        if env("DO_NOT_TRACK").as_deref() == Some("1") {
            return Decision::Off;
        }
        match env("CLW_TELEMETRY").as_deref() {
            // (1) explicit kill-switch.
            Some("0") => return Decision::Off,
            // (2) explicit env opt-in.
            Some("1") => return Decision::On,
            _ => {}
        }
        // (3) recorded consent in ~/.clw/config.toml.
        match config_value().as_deref() {
            Some("on") => Decision::On,
            Some("off") => Decision::Off,
            // (4) off by default.
            _ => Decision::Off,
        }
    }
}

// ---------------------------------------------------------------------------
// Process-global gate
// ---------------------------------------------------------------------------

/// Process-wide resolved decision, set once by [`init_from_env_and_config`].
/// Defaults to OFF if never initialized, so [`record`] is a no-op out of the box.
static DECISION: OnceLock<Decision> = OnceLock::new();

/// Initialize the telemetry gate from the environment and `~/.clw/config.toml`.
///
/// Call this **once**, early in `main()`. It is **no-op-safe**: it never errors,
/// never prompts, never blocks, and on any ambiguity defaults to OFF. Subsequent
/// calls are ignored (the first decision wins for the process lifetime).
///
/// This does NOT run the first-run consent prompt — see [`maybe_prompt_consent`].
/// Resolution is silent so it is safe to call unconditionally in CI and scripts.
pub fn init_from_env_and_config() {
    let decision = Decision::resolve(env_lookup, read_config_telemetry);
    let _ = DECISION.set(decision);
}

/// Whether telemetry is currently enabled for this process.
#[must_use]
pub fn is_enabled() -> bool {
    DECISION.get().copied().unwrap_or(Decision::Off).enabled()
}

/// Standard env lookup used in production (returns `None` for unset/non-UTF-8).
fn env_lookup(key: &str) -> Option<String> {
    std::env::var(key).ok()
}

/// Read the recorded `telemetry = "on"|"off"` value from `~/.clw/config.toml`.
/// Returns `None` if the file or key is absent (the off-by-default path).
fn read_config_telemetry() -> Option<String> {
    let path = clw_config_path()?;
    let content = std::fs::read_to_string(&path).ok()?;
    let mut map = std::collections::HashMap::new();
    parse_config_lines(&content, &mut map);
    map.get("telemetry").cloned()
}

// ---------------------------------------------------------------------------
// Event type — redaction by construction
// ---------------------------------------------------------------------------

/// Which clw subcommand produced an event. A closed enum, NOT a free string, so a
/// caller can never smuggle a path, a workspace name, or a token in here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Subcommand {
    Init,
    Snapshot,
    Hydrate,
    Status,
    Run,
    Ls,
    Doctor,
    Auth,
}

/// Coarse outcome of a command. Deliberately low-cardinality (no error strings,
/// no messages — those could contain paths or tenant data).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Outcome {
    Ok,
    Error,
}

/// A telemetry event. **This type is the redaction contract.**
///
/// Every field is a fixed enum or a coarse numeric bucket. There is deliberately
/// NO `String`, `PathBuf`, `&Path`, or byte field anywhere in this type, so by
/// construction an event can NEVER carry:
/// - the personal access token (PAT),
/// - a filesystem path or workspace name,
/// - file contents or any free-form user text.
///
/// If a future field is needed it must be added as another low-cardinality enum or
/// bucket — never a free string — to preserve this guarantee.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Event {
    /// Which subcommand ran (enum-coded).
    pub subcommand: Subcommand,
    /// Coarse success/failure outcome (enum-coded).
    pub outcome: Outcome,
    /// Coarse duration bucket — NOT a raw timestamp or exact duration.
    pub duration: DurationBucket,
}

/// A coarse, low-cardinality duration bucket. Avoids exact timings (which can be
/// fingerprintable) and is the only "measurement" an event may carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum DurationBucket {
    /// < 1s
    SubSecond,
    /// 1s – 10s
    Seconds,
    /// 10s – 60s
    TensOfSeconds,
    /// > 60s
    Minutes,
}

impl DurationBucket {
    /// Bucket a wall-clock duration into a coarse range (no exact value retained).
    #[must_use]
    pub fn from_secs(secs: u64) -> Self {
        match secs {
            0 => Self::SubSecond,
            1..=9 => Self::Seconds,
            10..=59 => Self::TensOfSeconds,
            _ => Self::Minutes,
        }
    }
}

impl Event {
    /// Construct an event. Only enum-coded, non-PII inputs are accepted — there is
    /// no constructor that takes a string, path, or token.
    #[must_use]
    pub fn new(subcommand: Subcommand, outcome: Outcome, duration: DurationBucket) -> Self {
        Self {
            subcommand,
            outcome,
            duration,
        }
    }
}

// ---------------------------------------------------------------------------
// record / emit
// ---------------------------------------------------------------------------

/// Record a telemetry [`Event`].
///
/// **This is a NO-OP unless telemetry is enabled** (see [`is_enabled`]). When
/// disabled — which is the default, and the only state in CI/tests — the event is
/// dropped immediately and nothing is computed or sent.
///
/// The [`Event`] type guarantees by construction that no PAT, path, or file
/// content can be carried (see its docs). This function therefore cannot leak PII
/// regardless of the enabled state.
pub fn record(event: Event) {
    if !is_enabled() {
        return;
    }
    emit(event);
}

/// The single pluggable-backend seam.
///
/// TODO(M5.x — telemetry backend): replace this no-op with a real `Sink`
/// (e.g. selected behind a `telemetry-backend` cargo feature) that serializes the
/// event and ships it over the network. Until that lands every event is dropped
/// here AFTER the enabled-gate, so behavior is identical to telemetry being off.
fn emit(_event: Event) {
    // No-op sink. Intentionally empty: no network backend is built in this WP.
}

// ---------------------------------------------------------------------------
// First-run consent (interactive only)
// ---------------------------------------------------------------------------

/// First-run consent concept (documented; interactive-only).
///
/// In a future interactive build this would, **only when stdin/stdout are a TTY**
/// and no decision has yet been recorded, prompt the user once for opt-in and
/// persist the choice as `telemetry = "on"|"off"` in `~/.clw/config.toml` via the
/// shared 0600 writer (so the consent record is owner-only, like the PAT).
///
/// It MUST never prompt in tests, CI, or any non-TTY context — a consent prompt may
/// never block a script. This function is the documented hook for that flow; the
/// actual prompt is deferred until a backend exists (there is nothing to consent to
/// while the sink is a no-op). Returns the recorded decision if one already exists,
/// else `None` (meaning: would prompt, in an interactive build).
///
/// # Persisting a decision
///
/// To record a choice, callers use [`persist_decision`], which writes through the
/// shared `~/.clw/config.toml` 0600 writer — never a bespoke file write.
#[must_use]
pub fn maybe_prompt_consent() -> Option<Decision> {
    // Never prompt automatically. If a decision was already recorded, surface it;
    // otherwise return None (an interactive build would prompt here).
    match read_config_telemetry().as_deref() {
        Some("on") => Some(Decision::On),
        Some("off") => Some(Decision::Off),
        _ => None,
    }
}

/// Persist a telemetry consent decision to `~/.clw/config.toml` as
/// `telemetry = "on"|"off"`, reusing the shared 0600 writer/parser (the same
/// owner-only file that holds the PAT). Existing keys are preserved.
///
/// This is the ONLY way a decision is recorded; it is never called automatically
/// from [`init_from_env_and_config`] (which must stay silent and non-blocking).
pub fn persist_decision(decision: Decision) -> std::io::Result<()> {
    use crate::config::{
        ensure_config_parent_0700, read_config_pairs, render_config, write_config_file_0600,
    };

    let path = clw_config_path().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "cannot determine home directory for config file",
        )
    })?;
    ensure_config_parent_0700(&path)?;

    let value = match decision {
        Decision::On => "on",
        Decision::Off => "off",
    };

    // Preserve existing keys; overwrite/insert `telemetry`.
    let mut pairs = read_config_pairs(&path);
    pairs.retain(|(k, _)| k != "telemetry");
    pairs.push(("telemetry".to_string(), value.to_string()));
    pairs.sort_by(|a, b| a.0.cmp(&b.0));

    write_config_file_0600(&path, &render_config(&pairs))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Tiny env-map helper so `resolve` tests are hermetic (no real env mutation).
    /// Owns its data so the returned closure has no borrow lifetime to thread.
    fn env_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let owned: Vec<(String, String)> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect();
        move |k: &str| {
            owned
                .iter()
                .find(|(key, _)| key == k)
                .map(|(_, v)| v.clone())
        }
    }

    // -----------------------------------------------------------------------
    // Off by default
    // -----------------------------------------------------------------------

    #[test]
    fn off_by_default_no_env_no_config() {
        let d = Decision::resolve(env_of(&[]), || None);
        assert_eq!(d, Decision::Off, "telemetry must be OFF by default");
    }

    #[test]
    fn is_enabled_defaults_false_when_uninitialized() {
        // DECISION is a process-global OnceLock; without init it must read as OFF.
        // (Other tests never set it to On, so this stays a no-op gate.)
        assert!(!is_enabled());
    }

    // -----------------------------------------------------------------------
    // Kill-switches always win
    // -----------------------------------------------------------------------

    #[test]
    fn do_not_track_forces_off_even_over_opt_in() {
        // DO_NOT_TRACK=1 beats CLW_TELEMETRY=1 and a recorded `on`.
        let d = Decision::resolve(
            env_of(&[("DO_NOT_TRACK", "1"), ("CLW_TELEMETRY", "1")]),
            || Some("on".to_string()),
        );
        assert_eq!(d, Decision::Off, "DO_NOT_TRACK=1 must force OFF");
    }

    #[test]
    fn clw_telemetry_zero_forces_off_over_config_on() {
        let d = Decision::resolve(env_of(&[("CLW_TELEMETRY", "0")]), || Some("on".to_string()));
        assert_eq!(d, Decision::Off, "CLW_TELEMETRY=0 must force OFF");
    }

    #[test]
    fn do_not_track_only_triggers_on_value_one() {
        // A stray DO_NOT_TRACK that is not "1" does not flip anything; default OFF.
        let d = Decision::resolve(env_of(&[("DO_NOT_TRACK", "0")]), || None);
        assert_eq!(d, Decision::Off);
    }

    // -----------------------------------------------------------------------
    // Explicit opt-in
    // -----------------------------------------------------------------------

    #[test]
    fn clw_telemetry_one_opts_in() {
        let d = Decision::resolve(env_of(&[("CLW_TELEMETRY", "1")]), || None);
        assert_eq!(d, Decision::On, "CLW_TELEMETRY=1 must opt in");
    }

    #[test]
    fn config_on_opts_in_when_env_silent() {
        let d = Decision::resolve(env_of(&[]), || Some("on".to_string()));
        assert_eq!(d, Decision::On, "recorded `telemetry = on` opts in");
    }

    #[test]
    fn config_off_stays_off() {
        let d = Decision::resolve(env_of(&[]), || Some("off".to_string()));
        assert_eq!(d, Decision::Off);
    }

    #[test]
    fn env_opt_in_beats_config_off() {
        // Explicit env opt-in (2) outranks recorded config (3).
        let d = Decision::resolve(env_of(&[("CLW_TELEMETRY", "1")]), || {
            Some("off".to_string())
        });
        assert_eq!(d, Decision::On);
    }

    #[test]
    fn garbage_config_value_is_off() {
        let d = Decision::resolve(env_of(&[]), || Some("maybe".to_string()));
        assert_eq!(d, Decision::Off, "unknown config value must default OFF");
    }

    // -----------------------------------------------------------------------
    // record() is a no-op when disabled
    // -----------------------------------------------------------------------

    #[test]
    fn record_is_noop_when_disabled() {
        // With telemetry disabled (the default), record() must not panic and must
        // do nothing observable. The no-op sink makes this a pure gate test.
        assert!(!is_enabled());
        record(Event::new(
            Subcommand::Snapshot,
            Outcome::Ok,
            DurationBucket::Seconds,
        ));
        // Still disabled, still nothing emitted — reaching here proves no-op.
        assert!(!is_enabled());
    }

    // -----------------------------------------------------------------------
    // Redaction by type: an Event cannot carry a path or token
    // -----------------------------------------------------------------------

    #[test]
    fn event_is_copy_and_carries_no_owned_data() {
        // Redaction-by-construction: `Event: Copy` is only possible because every
        // field is a Copy enum/bucket. A String/PathBuf/token field would make
        // `Event` non-Copy, so this static bound is a compile-time guard that no
        // PII-carrying field was ever added.
        fn assert_copy<T: Copy>() {}
        assert_copy::<Event>();
        assert_copy::<Subcommand>();
        assert_copy::<Outcome>();
        assert_copy::<DurationBucket>();

        // And the whole event fits in a handful of bytes — it physically cannot
        // hold a path/token/file-content (those need heap-backed String/PathBuf).
        assert!(
            std::mem::size_of::<Event>() <= 8,
            "Event must stay a tiny POD value; a path/token field would blow this up"
        );
    }

    #[test]
    fn duration_bucketing_is_coarse() {
        assert_eq!(DurationBucket::from_secs(0), DurationBucket::SubSecond);
        assert_eq!(DurationBucket::from_secs(5), DurationBucket::Seconds);
        assert_eq!(DurationBucket::from_secs(30), DurationBucket::TensOfSeconds);
        assert_eq!(DurationBucket::from_secs(120), DurationBucket::Minutes);
    }

    // -----------------------------------------------------------------------
    // Consent never prompts automatically
    // -----------------------------------------------------------------------

    #[test]
    fn maybe_prompt_consent_never_blocks() {
        // In a test (non-TTY) context this must return without prompting. It may
        // return a recorded decision or None, but must never hang/panic.
        let _ = maybe_prompt_consent();
    }
}
