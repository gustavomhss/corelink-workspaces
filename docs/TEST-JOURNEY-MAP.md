# clw — Journey, assertion and execution map

Source reviewed: **`e46e90b9a3308724da446d8dc3887cfc5c39a882`**, 2026-09-18.
This map records assertion coverage at that source, not a new product-test run.
The [previous map][old] is preserved in Git. Its total test counts, blanket
coverage labels and proposed future live suite are superseded by this inventory.
The runtime reference is [GUIDE.md](GUIDE.md); lane selection is in [TESTING.md](TESTING.md).

## Reading the map

**A** means a relevant assertion was located and inspected for the stated slice.
**P** means partial coverage: the assertion is narrower than the original journey.
**G** means an explicit remaining gap, unverified platform or unsupported guarantee.
These are not grades of the entire feature. A function name or a successful build
alone never establishes the assertion's adequacy.

**Execution status for every inventory row below: not executed by this mapping
change.** A refers to source evidence, not a new PASS. Even libtest's `ok` may
represent an early return; the non-UTF-8 and live-skip cases below make that visible.
Use an exact source/build, selector, environment, terminal result and the relevant
assertion when attaching execution evidence. Historical receipts retain their own
SHAs. Pending PRs, particularly #332's replay/GC tests, are not part of this baseline.

Layers: **unit** is a production helper/builder; **library** drives the public API
with an in-process store; **HTTP** uses the actual client with a local HTTP fixture;
**CLI** spawns the actual binary, normally against that fixture; **live** depends on
an explicitly authorized remote environment. None is a substitute for the others.
Most CLI integration scenarios below live in the Unix-gated `e2e` target. A library
test without `cfg(unix)` can still depend on tools such as `true`; absence of a cfg
attribute does not demonstrate Windows runtime portability.

## Six previously misclassified cases

| Original gap | Assertion inspected | Actual scope and limit |
|---|---|---|
| Command argument changes | [command_arg_change_yields_different_key][R] | A/unit: varies only a command argument, requires a different key and a stable repeated key. Not a CLI execution/replay test. |
| HIT replays stderr | [hit_replays_stderr_byte_identical][R] | A/library, Unix shell: MISS and HIT preserve distinct expected stdout/stderr bytes. No CLI stream-delivery guarantee. |
| Ignored input stays invisible | [gitignored_file_is_invisible_to_the_key][R] | A/library: changing only an ignored file preserves key and HIT. Requires the fixture command to exist. |
| Undeclared input produces stale replay | [undeclared_file_change_is_a_stale_hit][R] | A/limitation witness: the child initially reads an undeclared file; after mutation, HIT returns the old bytes. This deliberately documents non-hermeticity, not a correctness fix for arbitrary commands. |
| Empty snapshot | [snapshot_empty_tree_succeeds_with_minimal_manifest][S] | A/library: zero counters, empty stored manifest, matching local digest and a ref to that root. |
| Empty hydrate | [empty_snapshot_hydrates_to_empty_dest][H] | A/library: creates an empty destination, checks root and all zero counters. Empty content legitimately needs no materialization bytes. |

## Journey inventory by feature

Every item from the old feature roll-up is assigned below; compound rows retain
their separate limits. Test identifiers link to the pinned source file, not an
unqualified moving branch. The old ranked backlog has a separate crosswalk below.

### Snapshot, walking and manifest production

| ID / original journey | Assertion source | Disposition / boundary |
|---|---|---|
| S01 First snapshot; counters and ref | [full_pipeline_basic][S], [json_report][E], [roundtrip_bit_identical][E] | A/library + CLI: known fixture counters, root/ref and restored bytes. JSON hex checks use `is_ascii_hexdigit`, not a lowercase-only oracle. |
| S02 Identical no-op; changed content and lineage | [second_snapshot_unchanged_no_uploads][S], [third_snapshot_changed_file][S], [resnapshot_identical_is_noop_and_force_replaces][E] | A: unchanged outcome, changed-root/dedup and parent association. Parent is not retention. The CLI force case alone does not prove atomic replacement. |
| S03 Differing conflict; force; corrupt-ref recovery | [divergent_resnapshot_without_force_refuses_without_uploading][S], [force_recovers_a_present_but_corrupt_ref][S], [resnapshot_existing_name_409_is_actionable_and_rm_then_retry_works][E] | A/P: local refusal and occupied-ref recovery; an error represented as HTTP 409 is not proof the server returned it. Atomic refs and missing-root repair remain #292/#294; wording remains #309. |
| S04 Lost ACK / idempotent conflict reconciliation | [idempotent_409_identical_ref_is_success][S], [idempotent_409_differing_ref_errors][S] | P: the controlled transport supplies 409 and a fetched occupant; the same-content case checks another GET. Not an actual lost-response HTTP fault or distributed atomicity proof. |
| S05 Root/nested `.git` exclusion | [walk_git_dir_skipped][M], [walk_nested_git_dir_skipped][M] | A/library walk: excluded names plus retained neighboring content. No production invocation is claimed. |
| S06 Ignore sources | [walk_gitignore_honored][M], [walk_clwignore_honored][M], [walk_honors_dot_ignore_nested_gitignore_and_git_exclude][M] | A for root rules, `.ignore`, nested `.gitignore` and `.git/info/exclude`. G for a dedicated `core.excludesFile`/global-config precedence oracle in this reviewed set. |
| S07 Dotfiles; non-UTF-8 names | [walk_hidden_files_included][M], [walk_non_utf8_filename_is_invalid_manifest][M] | A for hidden files. P for invalid UTF-8: the test returns before `walk` when the filesystem cannot create the name; a nominal pass on such a volume is not rejection evidence. |
| S08 Internal/external links; CLI skip | [walk_intra_workspace_symlink_still_recorded][M], [snapshot_skips_external_symlink_and_reports_it][S], [snapshot_external_symlink_skipped_through_cli][E] | A for these fixtures: CLI checks named warning, fresh-cache restored regular bytes and absent skipped link. Not complete combined-link confinement (#296). |
| S09 Secret-name warnings collected and printed | [snapshot_collects_secret_warnings][S], [snapshot_warns_about_secret_looking_files_on_stderr][E] | A: collection and CLI warning/name/advice. Heuristic names, not content scanning, blocked upload or proof of every diagnostic failure path. |
| S10 Concurrency and deduplication | [per_run_dedup_uploads_repeated_chunk_once][S], [manifest_digest_is_stable_across_concurrent_upload_schedule][S], [concurrent_snapshot_processes_shared_cache_no_corruption][E] | A for per-operation dedup, stable root and parallel fixtures. The first case counts exactly one content PUT plus root PUT in a fake transport; not global exactly-once delivery. |
| S11 Size edges; empty tree; changing source size | [manifest_blob_size_check_rejects_oversized][S], [file_entry_size_derives_from_chunked_bytes_not_walk_metadata][S], [small_path_rejects_file_grown_past_threshold][S], empty case above | A/P: actual chunk-derived metadata and post-read rejection. G for allocation prevention until #291; a rejected oversized root can follow chunk uploads. |
| S12 Failures; ref domain; Windows | [hard_failure_mid_snapshot_then_clean_rerun][ADV], [auth_401_mid_operation_clean_error_no_retry_storm][ADV], [snapshot_ref_domain_flag_uses_runner_keyspace][E] | A/HTTP/CLI for scoped failures and key domain. G/native Windows acceptance; domain separation is not authorization. |

### Hydrate, materialization and recovery

| ID / original journey | Assertion source | Disposition / boundary |
|---|---|---|
| H01 By name and supplied digest | [full_hydrate_two_files_multichunk_symlink_empty_dir][H], [hydrate_manifest_digest_roundtrips_byte_identical][E] | A: expected files/root and digest entry point. |
| H02 Missing ref, malformed/substituted manifest, missing chunk | [missing_ref_returns_not_found][H], [corrupt_manifest_bytes_returns_json_error][H], [substituted_manifest_returns_integrity_error][H], [mid_file_failure_leaves_no_tmp_stray][H] | A/library: distinct error classes and failed-file cleanup; not all concurrent-writer termination (#300). |
| H03 Nonempty destination and read I/O | [hydrate_into_nonempty_dest_refuses_and_preserves_existing][E], [unreadable_dest_surfaces_io_error_not_not_empty][H] | A/P: refusal preserves resident content; permission fixture behavior depends on effective host access. |
| H04 Write-side failure | [mid_write_io_failure_leaves_no_tmp_stray_and_surfaces_io][H], [injected_writer_failure_leaves_no_final_or_temp][H] | P: the former fails **rename after writing**, not `write_all`; the latter injects the writer error boundary. Actual disk-full/short-write device behavior is not proved. |
| H05 Empty snapshot | [empty_snapshot_hydrates_to_empty_dest][H] | A/library, as detailed above; no false demand for positive downloaded bytes on an empty tree. |
| H06 Manifest/chunk integrity and length | [corrupt_fetched_chunk_returns_integrity_error][H], [chunk_length_mismatch_returns_integrity_error][H], [hydrate_rejects_corrupt_cas_chunk_without_publishing_or_caching_it][E] | A: wrong bytes/length do not authorize publishing the failed file. No hash forgery or full-tree rollback claim. |
| H07 Cold cache | [test_roundtrip_snapshot_hydrate][HTTP], [roundtrip_bit_identical][E] | A: distinct empty destination cache and byte equality; the HTTP-library case additionally requires `bytes_downloaded > 0`. |
| H08 Warm cache / HTTP boundary | [cache_first_bytes_from_cache_and_downloaded_split][H], [warm_cache_second_hydrate_serves_locally][LIVE] | A for FakeStore byte split and cached miss population; live assertion exists but was not executed here. G for a dedicated offline whole-CLI warm-cache wire-count oracle in this inspected map. |
| H09 Containment; files before links | [materialize_file_re_rejects_hostile_rel_path][H], [regular_files_materialized_before_symlinks][H], [intra_workspace_dotdot_symlink_roundtrips][E] | P: helper recheck, ordering and selected allowed relative links. Literal validation does not close combined-resolution or destination-alias #296/#303. |
| H10 Counts, byte budget and per-file equality | [oversized_entry_count_rejected][H], [oversized_symlink_count_rejected][H], [oversized_chunk_counts_rejected][H], [declared_total_size_disagreeing_with_chunks_is_rejected][H], [hydrate_cli_per_file_size_rejects_before_chunks_and_fs][E], [hydrate_cli_per_file_size_valid_controls_restore_exact_bytes][E] | A for helper count boundaries and real-CLI #306 preflight/healthy controls. Count helpers avoid allocating their production-size maxima. Not a physical disk or all-allocation bound. |
| H11 Per-file publication; temp cleanup | [writer_eof_without_finish_aborts_and_cleans_temp][H], [writer_exclusive_open_preserves_preexisting_temp][H], [sync_failure_aborts_before_publication_and_cleans_owned_temp][H], [writer_temp_is_private_during_writes_and_mode_precedes_publication][H] | A for explicit Finish/abort, owned cleanup and Unix mode ordering. Not atomic publication of the tree or Windows effective-access acceptance. |
| H12 Re-run and crash durability | [hydrate_rerun_heals_a_partial_destination][H] | P: the fixture damages a completed tree, **removes the destination**, then restores it. Not incremental resume, interruption of a live writer, real power loss or proof of sibling quiescence. Power-loss testing remains G. |
| H13 Windows copies | [symlink_materialized_as_copy_on_windows][H], [self_referential_symlinks_rejected_on_windows][H], [path_containment.rs][PC] | P/unexecuted-native: file copy, lexical/canonical guards and self-reference fixtures exist. Full directory/dangling/dependency-order/expansion acceptance remains #298/#299/#317. Wine's selected lexical functions do not run the materializer. |
| H14 Large, stress and concurrent round trips | [large_streaming_roundtrip_byte_identical][ADV], [multichunk_file_fetches_concurrent_but_writes_in_order][H], [stress.rs][STRESS] | A for bounded normal fixtures; ignored scale tests need explicit selection and resource authority. Neither their presence nor compilation is a scale-test result. |

### Run: declared-input memoization

| ID / original journey | Assertion source | Disposition / boundary |
|---|---|---|
| R01 MISS/store/HIT stdout and stderr | [run_memoization_and_exit_codes][E], [miss_executes_and_stores_then_hit_replays][R], [hit_replays_stderr_byte_identical][R] | A for outcomes/bytes. CLI banners alone are not an independent child-execution counter; stderr byte separation is library-level. |
| R02 Content, nested directory, modes, tool and cwd key changes | [changed_input_yields_different_key][R], [directory_input_picks_up_nested_changes][R], [input_file_mode_change_yields_different_key][R], [input_dir_file_mode_change_yields_different_key][R], [tool_identity_change_yields_different_key][R], [different_cwd_yields_different_key][R] | A for these mutations, not universal executable selection or transitive tool identity (#301). |
| R03 Command argument only | [command_arg_change_yields_different_key][R] | A/unit key calculation with repeat control; see six-case table. |
| R04 Declared/unlisted/unset/empty environment | [env_keys_listed_var_affects_key][R], [env_keys_unlisted_var_does_not_affect_key][R], [env_unset_differs_from_empty][R], [run_env_var_value_busts_the_cache_key][E] | A for selected Unicode environment cases; OS-string encoding remains #302. |
| R05 Ignored/undeclared file changes | [gitignored_file_is_invisible_to_the_key][R], [undeclared_file_change_is_a_stale_hit][R] | A/limitation witnesses, not hermetic execution or restored outputs. |
| R06 Nonzero producer; received record | [nonzero_exit_not_cached][R], [run_memoization_and_exit_codes][E] | A for failed-child non-storage. Received nonzero-record eligibility remains #307; its open PR's tests are not part of the pinned main. |
| R07 Internal failure, panic and ordinary 125/137 | [run_dead_endpoint_exits_reserved_125_child_never_ran][E], [run_dispatch_panic_exits_reserved_125_not_101][E], [run_oversized_initial_ac_response_is_not_child_output][E], [run_child_exit_125_and_137_remain_passthrough][E] | A for specific errors and legitimate child values, including integrated #305. Marker/phase assertions matter: a numeric 125 or 137 alone never identifies execution history. |
| R08 Output limits: stdout, stderr, exact/over cap | [child_output_exactly_at_cap_is_captured][R], [child_output_one_over_cap_aborts_as_too_large][R], [oversized_stderr_alone_aborts_with_too_large][R], [dual_stream_flood_aborts_without_deadlock][R], [run_oversized_child_output_exits_137][E] | A/library plus CLI stdout-cap slice. Stderr-only is no longer absent; independent CLI stderr-only delivery still narrower than all output paths. |
| R09 Signals and 255 fallback | [run_signal_killed_child_propagates_128_plus_signal_and_is_not_cached][E], [run_process_group_sigterm_reaches_clw_and_child][E] | A/Unix signal cases. G for a dedicated abnormal/non-Unix 255 oracle in this map; Windows behavior is not inferred. |
| R10 Quota message; JSON hint | [actionable_error_402_is_quota_actionable_not_raw_display][MAIN], [snapshot_surfaces_quota_402_and_tombstone_410_cleanly][E], [run.rs][RUNCLI] | P: the common quota renderer and snapshot CLI have assertions, not a located run-specific quota-dispatch CLI test. The run hint has helper coverage, not a full CLI oracle. `run` has no JSON result envelope. |
| R11 Concurrent stores and storage deadline | [store_phase_is_concurrent_not_serial_after_success][R] | A for concurrent rather than serial stores; G for a dedicated full 30-second timeout/return oracle. Concurrent success is not timeout acceptance. |
| R12 Secret environment and native Windows | [secret_env_is_stripped_from_child][R], [e2e_portable.rs][PORTABLE] | A for omitted listed environment values; not same-user filesystem isolation. G/native Windows run/capture semantics; portable offline commands do not cover them. |

### Lifecycle: status, listing, reference removal, prune and erase

| ID / original journey | Assertion source | Disposition / boundary |
|---|---|---|
| L01 Status clean/dirty; added/removed | [status_clean_dirty][E], [diff_added][M], [diff_removed][M], [diff_changed_mode_only][M] | A for CLI clean/modified exit and helper direction sets. P for complete dirty JSON/add/remove CLI combinations. |
| L02 Status error 2 and JSON | [status_json_shape_and_absent_ref_exits_2][E] | A: clean JSON with empty arrays; separate absent-ref invocation exits 2. Not all failure JSON shapes. |
| L03 Named listing human/JSON | [ls_name_lists_the_single_ref][E], [ls_json_name_single_ref_shape][E] | A: known name/count/bytes/date/parent and hex-shaped root. Lowercase is not guaranteed by an ASCII-hexdigit assertion. |
| L04 All listing, memo/corrupt/foreign records | [ls_all_lists_refs_tolerates_corruption_and_reflects_rm][E], [ls_all_distinguishes_run_cache_from_corrupt_refs_without_flooding][E], [json_contract_v1_top_level_keys][JSON] | A: differentiated results plus top-level builder keys; not every nested value or error combination. |
| L05 Listing pagination/capability | [ac_list_refs_cursor_pagination_flattens_all_pages][C], [ac_list_refs_405_surfaces_unavailable_d7][C], [ac_list_refs_501_surfaces_unavailable_d7][C] | A/HTTP transport; capability JSON is additionally a builder slice, not full CLI schema coverage. |
| L06 rm name/raw key/idempotence/refusal | [rm_deletes_ref_is_idempotent_and_requires_yes][E], [rm_ref_key_deletes_a_dangling_corrupt_ref][E], [rm_name_and_ref_key_together_conflict][E] | A: actual selected AC state, absent `deleted:false`, refusal and parser conflict. Does not remove CAS data. |
| L07 rm interactive decline and response variants | [rm_pty_y_deletes_and_n_refuses_without_delete][E], [rm.rs][RM] | A/Unix PTY for yes/no effects; P for exhaustive JSON shape coverage, distinct from parser tests. |
| L08 Prune dry-run, destructive mode and live/orphan separation | [prune_dry_run_then_yes_removes_only_orphans][E] | A: exact dry-run orphan list, retained live keys and deleted orphan after sweep. Store inspection, not a cold consumer GET; no shared-root/concurrency guarantee (#293/#295). |
| L09 Prune missing root, replica 404 and empty-live floor | [prune_aborts_fail_safe_when_manifest_blob_missing][E], [prune_empty_live_set_over_nonempty_cas_refuses][E], [lagging_replica_404_ref_aborts_prune_and_deletes_nothing][PRUNE] | A for refused bounded fixtures; does not force actual service replication lag. |
| L10 Memo references remain live | [run_records_do_not_abort_prune_and_their_blobs_stay_live][PRUNE], [run_record_blobs_returns_digests_for_run_records_and_none_otherwise][R] | P: marks references and excludes them from candidate orphans in-process. Not actual CLI prune followed by fresh HTTP stream reads; #332 supplies separate pending work. |
| L11 Prune partial failure / `failed_at` / resume | [prune_reports_failed_at_and_does_not_sweep_on_a_mid_sweep_delete_failure][E], [orphan_sweep_drains_mixed_failures_deterministically_for_50_runs][PRUNE] | A for error/digest/failed-orphan preservation and separate scheduler draining. G for a full CLI retry completing a previously partial sweep. The first test does not execute a resume. |
| L12 Prune destructive JSON | Same CLI failure test plus [prune.rs][PRUNE] | P: dry-run and failure fields are asserted; not a golden of every successful/failed destructive variant. |
| L13 Erase default dry-run and confirmed success | [erase_dry_run_default_reports_blast_radius_and_deletes_nothing][E], [erase_yes_confirmed_empties_both_stores][E] | A: human/JSON preview preserves nonempty stores; confirmed success checks both stores empty. Not permanent offboarding or backup erasure (#316). |
| L14 Erase confirmation and wrong-but-matching tenant | [erase_wrong_confirm_tenant_aborts_deletes_nothing][E], [confirm_tenant_flag_must_match_exactly][ERASE] | A for mismatch. A correctly typed but wrongly selected tenant requires operator/identity-scope controls; equality cannot establish the requester's actual intent. G for that operational acceptance, not a promised CLI semantic guard. |
| L15 Erase incomplete and three-pass exhaustion | [erase_fails_loud_and_exits_nonzero_when_data_still_remains][E], [erase_final_duplicate_enumeration_reports_distinct_remaining_counts][ERASE] | A: CLI nonzero/`complete:false`/remaining refs; helper asserts exactly three passes and distinct remaining refs/blobs. Not every CLI field on every failure. |
| L16 Erase partial deletion and JSON inventory | [mid_delete_failure_reports_count_erased_before_failure][ERASE], [ref_failure_drains_started_refs_and_starts_zero_blobs][ERASE] | P: partial counts and phase completion at helper level; real-CLI retry and exhaustive success/partial/refusal JSON remain incomplete. |

### Transport and reliability

| ID / original journey | Assertion source | Disposition / boundary |
|---|---|---|
| T01 Retry 5xx/429, exhaustion and nonretryable 4xx | [retry_500_then_200_succeeds][C], [retry_persistent_500s_returns_http_error][C], [put_exhausts_429_retries_with_error][C], [no_retry_on_404][C], [no_retry_on_401][C] | A/local HTTP: response sequence and failure behavior. Not a guarantee that arbitrary real networks recover. |
| T02 Transport retry and ladder | [transport_error_retried_to_exhaustion_surfaces_error][C] | A/helper: simulated Transport error, exact attempts and original error. Not a real connection-loss sequence; complete timing ladder values remain a narrower unit/inspection claim. |
| T03 Retry-After integer/date/huge values | [retry_after_header_drives_backoff_then_succeeds][C], [retry_after_http_date_form_is_ignored_falls_back_to_ladder][C], [retry_after_delta_secs_to_sleep_ms_maps_and_caps][C], [exhausted_429_huge_retry_after_surfaces_quota_message][C] | A/P: wiring, date fallback timing, pure conversion/clamp and diagnostic branches. Pure conversion does not prove an actual full maximum sleep; no complete malformed-header/RFC oracle. |
| T04 Deadline defaults, bounds, hanging backend and contention | [op_deadline_env_parsing_defaults_and_overrides][C], [op_deadline_clamps_absurd_value_and_never_overflows][C], [op_deadline_bounds_a_hanging_backend][C], [op_deadline_bounds_a_contended_governor_acquire][C], [op_deadline_does_not_perturb_a_fast_success][C] | A for configured helper/request paths. P for every backoff interaction; no whole-command filesystem/child deadline. |
| T05 AIMD state and HTTP wiring | [governor_shrinks_on_throttle_then_recovers][C], [governor_concurrent_throttle_success_acquire_is_consistent][C], [real_429_shrinks_governor_target_end_to_end][C] | A: wiring test requires one 429 then success and target 8→4. Not live workload throughput acceptance. |
| T06 GET integrity, absent/lying length | [cas_get_integrity_violation][C], [cas_get_oversized_body_rejected_before_oom][C], [cas_get_streaming_body_without_content_length_rejected][C] | A for wrong digest, honest oversized body and chunked response exceeding the running cap. P/G for separately malformed or contradictory Content-Length fixtures; the chunked test is not every kind of lying length. |
| T07 URL, redirect, userinfo and bearer handling | [new_rejects_loopback_prefix_impersonation][C], [new_rejects_https_userinfo_smuggling][C], [redirect_is_not_followed_and_pat_not_leaked][C], [bearer_header_sent_on_cas_get][C] | A for enumerated destinations/requests; transport-policy acceptance is not an allowlist of trusted application hosts. No new security reproduction ran for this map. |
| T08 Enumeration caps and page size | [list_aborts_when_total_entries_exceed_cap][C], [list_aborts_on_always_advancing_empty_pages_at_page_cap][C], [list_page_over_16mib_cap_rejected][C] | A/P: first two use reduced limits of 3 entries/5 pages; third supplies 16 MiB+1 on CAS listing. Not 10-million-entry/1-million-page stress or all AC/stream-framing variants. |
| T09 Quota UX and credential-response cap | [snapshot_surfaces_quota_402_and_tombstone_410_cleanly][E], [actionable_error_402_is_quota_actionable_not_raw_display][MAIN], [redeem_oversize_body_is_size_capped][C] | A for represented responses and redemption cap. Backend billing/quotas are independently verified operational facts, not inferred from wording. |

### Configuration, identity, runner and other CLI surfaces

| ID / original journey | Assertion source | Disposition / boundary |
|---|---|---|
| A01 Precedence and aliases | [config_precedence_flag_beats_env_beats_file][CFG], [resolve_accepts_corelink_aliases_and_clw_wins][CFG], [config_precedence_env_tenant_beats_file][E] | A for flag/env/file and CLW-over-CORELINK with empty HOME. G for a dedicated CORELINK-versus-populated-file precedence oracle in this inspected set. |
| A02 Missing/unreadable/loose/corrupt config; parser | [absent_config_file_is_silent_and_yields_empty_map][CFG], [warn_config_unreadable_seam_emits_advisory][CFG], [perms_are_overbroad_flags_group_world_bits][CFG], [corrupt_config_file_does_not_block_env_configured_op][E], [parse_config_lines_never_panics][CFG] | A/P: pure permission/advisory seams, parser properties and selected command paths. A permission predicate is not effective-access or unreadable-filesystem proof. |
| A03 Strict/lenient, tenant, tilde and HOME | [resolve_lenient_tolerates_missing_tenant_and_token][CFG], [resolve_rejects_path_injection_tenant][CFG], [expand_tilde_with_home][CFG], [home_dir_falls_back_to_userprofile_when_home_absent][CFG], [tenant_resolution_rejects_injection][CFG] | A: specific resolution/helper cases. Control-character and alternate-host behavior remain scoped to these inputs, not all platforms. |
| A04 Auth login/logout/whoami and no-op JSON | [auth_login_persists_0600_config_without_echoing_token][E], [auth_logout_removes_token_keeps_other_keys][E], [auth_whoami_prints_tenant_not_token][E], [logout_json_noop_shapes_omit_file_key][E] | A: selected persistence/removal/report behavior. No-op cases check missing `file`, empty removals and retained tenant; not every byte/value of all auth variants. |
| A05 Rotate | [auth_rotate_directs_to_login][E] | A for guidance, not token issuance/revocation or a server rotation operation. |
| A06 Ticket happy path, scope and missing inputs | [cred_ticket_redeem_happy_path_snapshots][E], [cred_ticket_redeem_happy_path_hydrates_once][E], [cred_ticket_redeem_happy_path_runs_once][E], [cred_runner_ticket_missing_lease_is_fail_closed][CFG], [cred_runner_ticket_missing_fabric_is_fail_closed][CFG], [redeem_rejects_path_injection_lease_id][C] | A for command-gated redemption and input errors; separate credentials and endpoints are not implicitly authorized. |
| A07 Ticket failures, static fallback and live exception | [redeem_401_bad_ticket_is_fail_closed][C], [redeem_404_unknown_lease_is_fail_closed][C], [redeem_transport_error_is_fail_closed][C], [cred_runner_with_full_ticket_is_broker_and_never_reads_static_token][CFG], [redeem_failure_classifier_is_fail_closed_except_exact_410][LIVE] | A/helper/HTTP: distinct failures and static-token closure not called. P for whole-CLI transport failure with an otherwise valid static token; the CLI 410 case removes that token. The exact live 410 soft-skip rule is not a generic success. |
| A08 Runner keyspace and invalid domain | [runner_model_env_only_lifecycle_and_keyspace_isolation][E], [resolve_ref_domain_selector][CFG] | A for keyspace behavior and invalid-value resolution; no exclusive exit-code/no-request CLI oracle for every invalid domain is claimed. Domain names are not ACLs. |
| A09 Init first/repeat/force, mode and preservation | [init_writes_0600_config_and_rejects_reinit][E], [init_force_overwrites_existing_config_keeping_0600][E], [config.rs][CFG] | A for real Unix CLI re-init/force and selected writer assertions. Windows ACL acceptance remains separate. |
| A10 Doctor, uninstall and completions | [doctor_runs_and_redacts_token][E], [doctor_json_redacts_token][E], [uninstall_dry_run_lists_paths_removes_nothing_and_no_token][E], [uninstall_yes_scrubs_config_and_cache_keeps_binary][E], [completions_each_shell_nonempty_and_offline][E], [e2e_portable.rs][PORTABLE] | A for selected offline surfaces and identity fields. Portable doctor checks stdout; the Unix doctor checks both streams. Generated help disclosure #333 is not covered away by those tests. |
| A11 NO_COLOR, RUST_LOG and verbosity | [no_color_env_produces_no_ansi_on_stderr][E], [rust_log_overrides_quiet_emits_debug][E], [verbose_and_quiet_conflict][MAIN] | A/P: RUST_LOG has a quiet-only control; NO_COLOR uses already non-TTY stderr, so G remains for isolation of the environment knob on a TTY. |
| A12 Native Windows execution | [e2e_portable.rs][PORTABLE], [windows.yml][WIN] | G/unverified-native: portable source is available, but the workflow runs explicit Wine smoke and lexical functions, not `cargo test --workspace` or the portable target on native Windows. |

## JSON inventory: coverage is per selector and outcome

The [production-builder golden][JSON] compares sorted top-level keys for
`snapshot` success, `ls --name` success, and `ls --all` success/capability failure.
That is **three selectors and four outcomes**, not every command's schema. Its
second test also checks that an empty snapshot warning list remains present.
It does not establish all nested types/values, reject unknown keys on input, or
create a versioned universal response envelope.

| Command family | Existing assertion slice | Explicit remainder |
|---|---|---|
| snapshot | Golden success keys; CLI fixture counts/root/chunks; unchanged response | Every nested value, all output/reporting failures and all branch combinations are not goldened. |
| ls | Golden keys including `foreign_ac_entries`; named/all CLI cases | Every nested error/reference value and capability failure through the full CLI is not established by the golden. |
| hydrate | CLI/state round trips and #306 failures; library report counters | No entry in the top-level-key golden; do not treat filesystem success as complete JSON schema coverage. |
| status | Real CLI clean arrays plus separate missing-ref exit | Dirty added/removed/changed payload combinations and all failures remain partial. |
| rm | Absent `deleted:false`; selected consent and state cases | Exhaustive success/refusal shape equality is not asserted by every state test. |
| prune | Exact dry-run `would_delete`, failure `failed_at` | Successful destructive and all partial/refusal variants are not fully goldened. |
| erase | Dry-run fields and incomplete `complete:false`/remaining refs | No all-outcome golden; helper counts do not assert every CLI JSON field. |
| init/auth/doctor/uninstall | Production builder tests and selected real-CLI shape/value tests | Scope is each test's actual fields/streams. Guidance-only rotate and instruction-only executable removal are not remote effects. |
| run | Captured child output; helper tests for the JSON-inapplicable hint | No clw JSON result envelope is part of the contract. |
| completions | [json_completions_is_rejected_early_without_stdout][PORTABLE] | Raw shell output; JSON combination exits 2 without script output, not a JSON mode. |

## Disposition of the original ranked gap backlog

These rows preserve the original obligations rather than replacing false absence
claims with blanket coverage. They reference the detailed inventory above.

| Old priority item | Current disposition |
|---|---|
| 1 Erase incomplete reporting | L15: real CLI assertion exists; bounded enumerated scope, not legal-compliance certification. |
| 2 Erase default dry-run | L13: human and JSON fixtures already assert retained store counts. |
| 3 Prune partial failure/resume | L11: failure/digest and helper drain covered; actual full CLI retry still a gap. |
| 4 Erase MAX_PASSES | L15: helper explicitly asserts three passes and remaining-object counts. |
| 5 Snapshot warnings/external skip | S08/S09: both CLI assertions exist; failed-output handling is not implied. |
| 6 Lying/absent content length | T06/T08: chunked cap and real-size page assertions exist; arbitrary malformed framing is not thereby covered. |
| 7 ARG/ignored/undeclared inputs | R03/R05: all exist; distinguish key-only and documented stale-HIT witnesses. |
| 8 AIMD wiring | T05: actual HttpClient + local HTTP 429 updates target; not a deployed workload test. |
| 9 Redemption failures/fallback | A07: 401/404/transport and config-choice assertions exist; combined CLI fallback oracle remains partial. |
| 10 Windows command execution | A12 and Windows section below: source portability and Wine are not native evidence. |
| 11 JSON families | JSON inventory above: named ls, status and logout no-op are no longer absent; erase/all variants remain partial. |
| 12 Status error/init force/invalid domain | L02/A09 have CLI tests; A08 invalid-domain resolution is narrower than complete CLI acceptance. |
| 13 Color, empty tree, warm cache, write failure, ignores | A11/S11/H05/H08/H04/S06: present assertions and their remaining PTY/transport/device/global-ignore limits are explicit. |

Power-loss, real production-scale pagination and concurrent source mutation are
not certified by small helper inputs or fabricated run results. Their acceptance
needs the relevant environment and workload; this map does not authorize it.

## Windows: source, cross-check, Wine and native runtime

`e2e.rs`, HTTP integration and round-trip fixtures carry Unix-specific gates or
semantics. The portable CLI target has no whole-file Unix gate and contains
version/completion/doctor/configuration cases. That is a source selection fact,
not proof those cases ran on Windows.

[windows.yml][WIN] builds a Windows-GNU executable on Ubuntu, invokes `--version`
and `--json doctor` through Wine, and runs only `lexical_` functions from the
hydrate test executable. Its doctor smoke checks process success, not the full
JSON/redaction assertions in the portable target. The ordinary gate's Windows
cross-Clippy type-checks selected targets; neither path proves NTFS permissions,
canonical filesystem resolution, copy materialization or native subprocess exits.
A returned child value 125/137 is not a unique wrapper sentinel on any platform.

Native results require an actual Windows host, exact binary/selector and observed
assertions. The native Authenticode verification job in release is signature
verification, not a substitute runtime suite. No Windows acceptance is created
by this documentation correction.

## Live and evidence tooling already exist; they were not executed here

The old proposed `live_journeys.rs` already exists. Do not create a duplicate suite.
[Live conformance][LC] and [live lifecycle][LL] primarily call library/client APIs;
[live journeys][LIVE] combines library-driven cases with a binary-driven destructive
journey. [clw-e2e-evidence][EV] is a separate real-binary argv/environment/report
runner. The regular CLI E2E target is already another real-binary entry point,
but against a local controlled HTTP service rather than production.

Live cases cover dedup across names, a second warm hydrate, two identities,
pinned-content restoration, concurrency, credential redemption, quota and scoped
deletion. Their existence does not demonstrate execution or current provisioning.
The replica-lag function is an **always-soft-skipped placeholder** pending a server
hook, not an observed replication-fault test. Primary `CLW_CONFORMANCE_REQUIRE`
failures and optional soft skips have different rules. An exact expired/consumed
410 redemption exception must not classify all errors as skips.

The evidence executable separately distinguishes primary and optional personas:
`CLW_E2E_REQUIRE_PROD=1` rejects primary skips; `CLW_E2E_REQUIRE_ALL=1` rejects any
skip. Its dispatch workflow selects both. Inspect skipped counts and raw coverage
conditions even when a process exits 0; feature-disabled tests are not runtime
skips and early-return tests may still be reported `ok` by libtest.

Before any live execution, require an explicit authorized endpoint, test tenant,
credential source, cost/side-effect envelope and cleanup responsibility. Several
helpers fall back to the production endpoint; missing endpoint configuration must
not accidentally authorize a request. Use a dedicated disposable tenant for
prune/erase and verify the actual server scope (#293/#295/#316). This map neither
provisions identities nor schedules jobs. Native, coverage, live and release
acceptance keep their own work packages and receipts.

For persistence assertions, use nonempty known content and cold caches or direct
store/request evidence; do not regenerate lost bytes by rerunning a producer.
For redaction assertions, inspect the pre-scrub leak signal and the exercised raw
output path; a sanitized report proves only the sanitizer. For no-request or
no-child assertions, a phase-specific positive control plus a request/marker
oracle is stronger than a returned error number. See TESTING for the existing
runner and lane details.

## Maintenance and acceptance

Update each affected row with code changes; keep a source SHA, exact selector,
assertion scope and execution receipt separate. A pending runtime fix remains
open when a doc correctly describes it. #312 does not implement every gap, close
#318's execution obligations or certify deployed service health. The source review,
reference checks and final disposition for this update are recorded in
[the validation note](validation/2026-09-18-test-map-reconciliation.md).

[old]: https://github.com/HuGR-Labs/corelink-workspaces/blob/e46e90b9a3308724da446d8dc3887cfc5c39a882/docs/TEST-JOURNEY-MAP.md
[R]: https://github.com/HuGR-Labs/corelink-workspaces/blob/e46e90b9a3308724da446d8dc3887cfc5c39a882/crates/clw-run/src/lib.rs
[S]: https://github.com/HuGR-Labs/corelink-workspaces/blob/e46e90b9a3308724da446d8dc3887cfc5c39a882/crates/clw-snapshot/src/lib.rs
[H]: https://github.com/HuGR-Labs/corelink-workspaces/blob/e46e90b9a3308724da446d8dc3887cfc5c39a882/crates/clw-hydrate/src/lib.rs
[M]: https://github.com/HuGR-Labs/corelink-workspaces/blob/e46e90b9a3308724da446d8dc3887cfc5c39a882/crates/clw-manifest/src/lib.rs
[C]: https://github.com/HuGR-Labs/corelink-workspaces/blob/e46e90b9a3308724da446d8dc3887cfc5c39a882/crates/clw-client/src/lib.rs
[E]: https://github.com/HuGR-Labs/corelink-workspaces/blob/e46e90b9a3308724da446d8dc3887cfc5c39a882/crates/clw-cli/tests/e2e.rs
[CFG]: https://github.com/HuGR-Labs/corelink-workspaces/blob/e46e90b9a3308724da446d8dc3887cfc5c39a882/crates/clw-cli/src/config.rs
[MAIN]: https://github.com/HuGR-Labs/corelink-workspaces/blob/e46e90b9a3308724da446d8dc3887cfc5c39a882/crates/clw-cli/src/main.rs
[JSON]: https://github.com/HuGR-Labs/corelink-workspaces/blob/e46e90b9a3308724da446d8dc3887cfc5c39a882/crates/clw-cli/src/subcmds/json_contract.rs
[PRUNE]: https://github.com/HuGR-Labs/corelink-workspaces/blob/e46e90b9a3308724da446d8dc3887cfc5c39a882/crates/clw-cli/src/subcmds/prune.rs
[ERASE]: https://github.com/HuGR-Labs/corelink-workspaces/blob/e46e90b9a3308724da446d8dc3887cfc5c39a882/crates/clw-cli/src/subcmds/erase.rs
[RM]: https://github.com/HuGR-Labs/corelink-workspaces/blob/e46e90b9a3308724da446d8dc3887cfc5c39a882/crates/clw-cli/src/subcmds/rm.rs
[RUNCLI]: https://github.com/HuGR-Labs/corelink-workspaces/blob/e46e90b9a3308724da446d8dc3887cfc5c39a882/crates/clw-cli/src/subcmds/run.rs
[HTTP]: https://github.com/HuGR-Labs/corelink-workspaces/blob/e46e90b9a3308724da446d8dc3887cfc5c39a882/crates/clw-integration-tests/tests/http_integration.rs
[ADV]: https://github.com/HuGR-Labs/corelink-workspaces/blob/e46e90b9a3308724da446d8dc3887cfc5c39a882/crates/clw-integration-tests/tests/adversarial.rs
[STRESS]: https://github.com/HuGR-Labs/corelink-workspaces/blob/e46e90b9a3308724da446d8dc3887cfc5c39a882/crates/clw-integration-tests/tests/stress.rs
[PC]: https://github.com/HuGR-Labs/corelink-workspaces/blob/e46e90b9a3308724da446d8dc3887cfc5c39a882/crates/clw-hydrate/src/path_containment.rs
[PORTABLE]: https://github.com/HuGR-Labs/corelink-workspaces/blob/e46e90b9a3308724da446d8dc3887cfc5c39a882/crates/clw-cli/tests/e2e_portable.rs
[WIN]: https://github.com/HuGR-Labs/corelink-workspaces/blob/e46e90b9a3308724da446d8dc3887cfc5c39a882/.github/workflows/windows.yml
[LIVE]: https://github.com/HuGR-Labs/corelink-workspaces/blob/e46e90b9a3308724da446d8dc3887cfc5c39a882/crates/clw-conformance/tests/live_journeys.rs
[LC]: https://github.com/HuGR-Labs/corelink-workspaces/blob/e46e90b9a3308724da446d8dc3887cfc5c39a882/crates/clw-conformance/tests/live_conformance.rs
[LL]: https://github.com/HuGR-Labs/corelink-workspaces/blob/e46e90b9a3308724da446d8dc3887cfc5c39a882/crates/clw-conformance/tests/live_lifecycle.rs
[EV]: https://github.com/HuGR-Labs/corelink-workspaces/blob/e46e90b9a3308724da446d8dc3887cfc5c39a882/crates/clw-e2e-evidence/src/lib.rs
