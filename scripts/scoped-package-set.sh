#!/usr/bin/env bash
# scoped-package-set.sh — expand a changed scoped crate to its workspace consumers.
#
# A `cargo test -p changed` follows dependencies DOWNWARD only.  That is not enough
# for a low-level crate: its dependents are the callers most likely to expose an API
# or behavioural regression.  `scoped_package_set` therefore asks Cargo for the
# resolved graph, walks the reverse edges to a fixed point, and unions that result
# with the historical end-to-end sinks.  Only workspace members participate in the
# walk: registry dependencies are compiled by Cargo as dependencies, but are never
# meaningful `-p` targets in this workspace.
#
# This script is sourced by gates.sh.  It deliberately fails closed: a missing tool,
# failed metadata command, malformed JSON, incomplete resolve graph, or ambiguous
# crates/<directory> -> workspace-package mapping means the caller must fail rather
# than run a deceptively narrow package set.

set -euo pipefail

# These were the scoped lane's consumers before reverse-dependency closure existed.
# Keep them as named sinks: their tests exercise cross-crate flows that a dependency
# graph cannot express (for example, process-level integration fixtures).
SCOPED_HISTORICAL_SINKS=(clw-cli clw-integration-tests)

# scoped_package_set <space-separated crate-directory names>
#
# Prints one Cargo package name per line, sorted and de-duplicated.  The classifier
# supplies directory names, not package names, so map through Cargo metadata instead
# of assuming that crates/foo is always package `foo`.
scoped_package_set() {
  local scope="$1" metadata package_lines

  command -v cargo >/dev/null 2>&1 || {
    echo "GATE FAILED: cargo is unavailable; cannot expand scoped reverse dependencies" >&2
    return 1
  }
  command -v jq >/dev/null 2>&1 || {
    echo "GATE FAILED: jq is unavailable; cannot expand scoped reverse dependencies" >&2
    return 1
  }

  # --all-features makes this match the scoped clippy invocation; --locked prevents
  # metadata from silently changing Cargo.lock while deciding what the gate tests.
  if ! metadata="$(cargo metadata --format-version 1 --all-features --locked)"; then
    echo "GATE FAILED: cargo metadata failed; refusing a scoped package set" >&2
    return 1
  fi

  # jq owns both validation and graph traversal so an absent/malformed field cannot
  # degrade into an empty loop.  A reverse walk needs at most N rounds for N workspace
  # members; every round can only add a member, so N rounds reach the fixed point.
  if ! package_lines="$(jq -er '
    if ((.workspace_members | type) != "array" or (.workspace_members | length) == 0
          or (.packages | type) != "array"
          or (.resolve | type) != "object"
          or (.resolve.nodes | type) != "array")
    then error("invalid cargo metadata shape") else . end
    | .workspace_members as $members
    | if ([ $members[] | select(type != "string") ] | length) != 0
         or (($members | unique | length) != ($members | length))
      then error("workspace_members is malformed or non-unique") else . end
    | ([.packages[] | select(.id as $id | $members | index($id))
        | select((.id | type) == "string"
                 and (.name | type) == "string"
                 and (.name | test("^[A-Za-z0-9_-]+$"))
                 and (.manifest_path | type) == "string"
                 and (.manifest_path | endswith("/Cargo.toml"))
                 and (.dependencies | type) == "array")]) as $workspace_packages
    | if ($workspace_packages | length) != ($members | length)
      then error("workspace member package mapping is incomplete") else . end
    | if (($workspace_packages | map(.id) | unique | length) != ($members | length))
      then error("workspace member IDs are not unique") else . end
    | if (($workspace_packages | map(.name) | unique | length) != ($members | length))
      then error("workspace member names are not unique") else . end
    | (.resolve.nodes) as $nodes
    | if ([ $nodes[]
           | select((.id | type) != "string"
                    or (.dependencies | type) != "array"
                    or ([.dependencies[]? | select(type != "string")] | length) != 0
                    or ((.dependencies | unique | length) != (.dependencies | length))
                    or (.deps | type) != "array"
                    or ([.deps[]?
                         | select((type != "object")
                                  or ((.pkg | type) != "string")
                                  or ((.dep_kinds | type) != "array"))
                       ] | length) != 0
                    or ((.dependencies | sort)
                        != ([.deps[]?.pkg] | sort | unique)))
         ] | length) != 0
      then error("resolve graph contains malformed or inconsistent nodes") else . end
    | if (($nodes | map(.id) | unique | length) != ($nodes | length))
      then error("resolve node IDs are not unique") else . end
    | if ([ $members[]
           | select(. as $id | ($nodes | map(.id) | index($id)) | not)
         ] | length) != 0
      then error("workspace member is missing from the resolve graph") else . end
    | if ([ $workspace_packages[]
           | .dependencies[]?
           | select((type != "object")
                    or ((.path != null) and ((.path | type) != "string")))
         ] | length) != 0
      then error("workspace package dependency declarations are malformed") else . end
    | if ([ $workspace_packages[] as $package
           | ([ $package.dependencies[]?
                | select(.path != null)
                | .path as $path
                | [ $workspace_packages[]
                    | select((.manifest_path | rtrimstr("/Cargo.toml")) == $path)
                  ] as $found
                | if ($found | length) != 1
                  then error("workspace path dependency does not map uniquely: " + $path)
                  else $found[0].id
                  end ] | sort | unique) as $declared_workspace_edges
           | ([ $nodes[]
                | select(.id == $package.id)
                | .deps[]?.pkg
                | select(. as $id | $members | index($id))
              ] | sort | unique) as $resolved_workspace_edges
           | select($declared_workspace_edges != $resolved_workspace_edges)
         ] | length) != 0
      then error("resolved workspace edges disagree with declared path dependencies") else . end
    | ($scope | split(" ") | map(select(length > 0))) as $requested
    | if ($requested | length) == 0
      then error("empty scoped crate set") else . end
    | if ([ $requested[] | select(test("^[A-Za-z0-9._-]+$") | not) ] | length) != 0
      then error("scoped crate directory is malformed") else . end
    | ([ $requested[] as $directory
         | [$workspace_packages[]
            | select((.manifest_path | split("/") | .[-2]) == $directory)] as $found
         | if ($found | length) != 1
           then error("scoped crate directory maps to zero or multiple workspace packages: " + $directory)
           else $found[0].id
           end ] | unique) as $changed_ids
    | (reduce range(0; ($members | length)) as $_ ($changed_ids | unique;
         . as $seen
         | reduce $nodes[] as $node
             (.;
              if (($members | index($node.id)) != null
                  and ([ $node.deps[]?.pkg
                         | select(. as $dependency | $seen | index($dependency))
                     ] | length) > 0)
              then . + [$node.id] | unique
              else .
              end)
       )) as $closure
    | ([ $ARGS.positional[] as $sink
         | [$workspace_packages[] | select(.name == $sink)] as $found
         | if ($found | length) != 1
           then error("historical sink does not map uniquely to a workspace package: " + $sink)
           else $found[0].id
           end ]) as $sink_ids
    | ($closure + $sink_ids | unique) as $selected_ids
    | [$selected_ids[] as $id
       | $workspace_packages[] | select(.id == $id) | .name]
    | sort | unique
    | if length == 0 then error("scoped package set is empty") else .[] end
  ' --arg scope "$scope" --args "${SCOPED_HISTORICAL_SINKS[@]}" <<<"$metadata")"; then
    echo "GATE FAILED: cargo metadata/jq could not derive a complete scoped package set" >&2
    return 1
  fi

  [ -n "$package_lines" ] || {
    echo "GATE FAILED: reverse-dependency expansion returned no packages" >&2
    return 1
  }
  printf '%s\n' "$package_lines"
}
