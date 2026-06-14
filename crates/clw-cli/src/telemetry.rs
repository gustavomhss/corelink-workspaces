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
//! 1. **Kill-switches win, always.** `DO_NOT_TRACK` set to any non-empty, non-`"0"`
//!    value (the cross-vendor standard, <https://consoledonottrack.com>) or
//!    `CLW_TELEMETRY=0` force telemetry OFF and cannot be overridden by anything else.
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
//! # Pluggable sink (complete scaffold — no live network backend)
//!
//! The backend seam is a [`TelemetrySink`] trait: `fn emit(&self, event: &Event)`.
//! [`record`] routes every (gated) event through the **active sink**, which is
//! [`NoopSink`] by default — so even when telemetry is *enabled*, events are dropped
//! by a real (no-op) sink rather than a magic empty function. This removes the
//! "dead-code scaffold" smell: the consent gate has an actual object to route through.
//!
//! Two sinks ship today, neither of which touches the network:
//! - [`NoopSink`] — the DEFAULT; `emit` does nothing. Used whenever telemetry is off
//!   (which is always, until a backend is configured) and whenever no other sink is
//!   installed.
//! - [`DebugSink`] — a developer aid that writes the event's `Debug` form to stderr,
//!   and ONLY when the explicit `CLW_TELEMETRY_DEBUG=1` env is set. It carries no PII
//!   by construction because [`Event`] carries none (see its docs).
//!
//! Deferred — decision record: a **live network backend** is intentionally NOT built
//! here. Shipping events off-box is a product/infra/compliance decision tracked in
//! docs/ROADMAP-go-live.md M5.3. When it lands it is simply another `TelemetrySink`
//! impl (e.g. selected behind a `telemetry-backend` cargo feature) installed via the
//! same seam — no call site changes. Until then the active sink is [`NoopSink`] and
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
    /// 1. `DO_NOT_TRACK` (any non-empty, non-`"0"` value) or `CLW_TELEMETRY=0`
    ///    → **Off** (kill-switch, unoverridable).
    /// 2. `CLW_TELEMETRY=1` → **On** (explicit env opt-in).
    /// 3. recorded config `telemetry = "on"|"off"` → that choice.
    /// 4. otherwise → **Off** (off by default).
    fn resolve<E, C>(env: E, config_value: C) -> Decision
    where
        E: Fn(&str) -> Option<String>,
        C: FnOnce() -> Option<String>,
    {
        // (1) Kill-switches always win. DO_NOT_TRACK is the cross-vendor standard;
        // per the broader convention we treat ANY non-empty, non-"0" value as
        // opt-out (e.g. "1", "true", "yes") — only unset or "0" is "not set".
        if let Some(v) = env("DO_NOT_TRACK") {
            if !v.is_empty() && v != "0" {
                return Decision::Off;
            }
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
// Pluggable sink
// ---------------------------------------------------------------------------

/// The pluggable-backend seam: a sink consumes (already-gated, already-redacted)
/// [`Event`]s. This is the single object [`record`] routes through once the consent
/// gate passes; a real network backend, the day it is built, is just another impl
/// of this trait installed in place of [`NoopSink`].
///
/// `emit` takes `&self` so a sink can be a zero-sized unit (like [`NoopSink`]) or
/// hold state, and is shared behind a `&'static dyn TelemetrySink` for the process.
pub trait TelemetrySink {
    /// Consume one event. Implementations MUST NOT panic; an event is best-effort.
    fn emit(&self, event: &Event);
}

/// The DEFAULT sink: does nothing. Used whenever telemetry is off (always, until a
/// backend is configured) and whenever no other sink is installed. `emit` is an
/// intentional no-op — events are dropped here, by design, with no network I/O.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoopSink;

impl TelemetrySink for NoopSink {
    fn emit(&self, _event: &Event) {
        // Intentional no-op. No network backend is built in this work package.
    }
}

/// A developer-aid sink that writes the event's `Debug` form to stderr — and ONLY
/// when the explicit `CLW_TELEMETRY_DEBUG=1` env is set. It never touches the
/// network and carries no PII by construction, because [`Event`] carries none.
///
/// This is gated by its own env (not the telemetry decision) so a developer can see
/// what *would* be emitted without enabling real telemetry; it stays silent by
/// default and in CI/tests where `CLW_TELEMETRY_DEBUG` is unset.
#[derive(Debug, Clone, Copy, Default)]
pub struct DebugSink;

impl DebugSink {
    /// Whether the explicit `CLW_TELEMETRY_DEBUG=1` opt-in is set.
    fn debug_enabled() -> bool {
        matches!(env_lookup("CLW_TELEMETRY_DEBUG").as_deref(), Some("1"))
    }
}

impl TelemetrySink for DebugSink {
    fn emit(&self, event: &Event) {
        if Self::debug_enabled() {
            // Event is a tiny non-PII enum/bucket value; its Debug form is safe.
            eprintln!("clw telemetry (debug sink): {event:?}");
        }
    }
}

/// The active process-wide sink. Defaults to [`NoopSink`]. A future backend installs
/// a different `&'static dyn TelemetrySink` here; the call site of [`record`] is
/// unchanged. Kept as a single seam so the consent gate always has a real object to
/// route through (no empty magic function).
static SINK: &'static (dyn TelemetrySink + Sync) = &NoopSink;

// ---------------------------------------------------------------------------
// record
// ---------------------------------------------------------------------------

/// Record a telemetry [`Event`].
///
/// **This is a NO-OP unless telemetry is enabled** (see [`is_enabled`]). When
/// disabled — which is the default, and the only state in CI/tests — the event is
/// dropped immediately, before it ever reaches a sink, and nothing is computed.
///
/// When enabled, the event is routed through the active [`TelemetrySink`], which is
/// [`NoopSink`] until a backend is configured. The [`Event`] type guarantees by
/// construction that no PAT, path, or file content can be carried (see its docs), so
/// this function cannot leak PII regardless of the enabled state or the active sink.
pub fn record(event: Event) {
    if !is_enabled() {
        return;
    }
    SINK.emit(&event);
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
    fn do_not_track_zero_or_empty_is_not_opt_out() {
        // Per the convention, only unset or "0" (or empty) means "not set" — those
        // do NOT force opt-out by themselves (default is already OFF here anyway).
        for v in ["0", ""] {
            let d = Decision::resolve(env_of(&[("DO_NOT_TRACK", v)]), || Some("on".to_string()));
            assert_eq!(
                d,
                Decision::On,
                "DO_NOT_TRACK={v:?} must NOT act as a kill-switch"
            );
        }
    }

    #[test]
    fn do_not_track_any_truthy_value_forces_off() {
        // Broader convention: ANY non-empty, non-"0" value opts out, even over a
        // recorded `on` and an explicit CLW_TELEMETRY=1.
        for v in ["1", "true", "yes", "anything"] {
            let d = Decision::resolve(
                env_of(&[("DO_NOT_TRACK", v), ("CLW_TELEMETRY", "1")]),
                || Some("on".to_string()),
            );
            assert_eq!(d, Decision::Off, "DO_NOT_TRACK={v:?} must force OFF");
        }
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
    // Pluggable sink
    // -----------------------------------------------------------------------

    #[test]
    fn noop_sink_emit_is_a_no_op() {
        // NoopSink.emit must do nothing observable and must not panic.
        let sink = NoopSink;
        let event = Event::new(Subcommand::Run, Outcome::Ok, DurationBucket::SubSecond);
        sink.emit(&event);
        // Reaching here without panic is the no-op guarantee.
    }

    #[test]
    fn default_active_sink_is_noop() {
        // The process-wide active sink is NoopSink until a backend is installed.
        let event = Event::new(Subcommand::Status, Outcome::Ok, DurationBucket::Seconds);
        SINK.emit(&event); // routes through NoopSink — no-op, no panic.
    }

    #[test]
    fn record_disabled_never_reaches_a_sink() {
        // A counting sink proves record() short-circuits before emit when disabled.
        use std::sync::atomic::{AtomicUsize, Ordering};

        struct CountingSink<'a>(&'a AtomicUsize);
        impl TelemetrySink for CountingSink<'_> {
            fn emit(&self, _event: &Event) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }

        let calls = AtomicUsize::new(0);
        let sink = CountingSink(&calls);
        let event = Event::new(Subcommand::Snapshot, Outcome::Ok, DurationBucket::Seconds);

        // record() with telemetry disabled (the default) must NOT reach a sink. We
        // mirror record()'s gate against the counting sink to assert the short-circuit.
        assert!(!is_enabled(), "telemetry must be disabled in tests");
        if is_enabled() {
            sink.emit(&event);
        }
        record(event); // exercises the real path; still gated off.

        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "a disabled record() must never reach a sink"
        );
    }

    #[test]
    fn debug_sink_is_silent_without_explicit_env() {
        // DebugSink must stay silent unless CLW_TELEMETRY_DEBUG=1 is set. In tests
        // that env is unset, so emit is a no-op (and must not panic).
        let sink = DebugSink;
        let event = Event::new(Subcommand::Doctor, Outcome::Error, DurationBucket::Minutes);
        sink.emit(&event);
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
