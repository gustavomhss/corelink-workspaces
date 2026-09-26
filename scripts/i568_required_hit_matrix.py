#!/usr/bin/env python3
"""Provenance checks and bounded fixture control for the issue-568 hosted Action matrix."""

from __future__ import annotations

import argparse
import base64
import binascii
from http.server import BaseHTTPRequestHandler, HTTPServer
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import signal
import subprocess
import sys
import time
import urllib.error
import urllib.request

WORKSPACES_REPOSITORY = "HuGR-dev/corelink-workspaces"
WORKSPACES_REPOSITORY_ID = 1259579816
RUNNERS_REPOSITORY = "HuGR-dev/corelink-runners"
RUNNERS_REPOSITORY_ID = 1266754321
WORKFLOW_PATH = ".github/workflows/issue-568-required-hit.yml"
CAPABILITY = "required-hit/v1"
PACKAGE_VERSION = "0.1.12"
TARGET = "x86_64-unknown-linux-gnu"
PRINCIPAL = "corelink-release-candidate"
NAMESPACE = "corelink-clw-candidate"
FINGERPRINT = "SHA256:2qTotApppWZfJL8DCuy4qKud7ItKRTJGdaoqicQl3dg"
ALLOWED_SIGNER = (
    "corelink-release-candidate ssh-ed25519 "
    "AAAAC3NzaC1lZDI1NTE5AAAAIBf1KSyJEvR257OTrOydiPDkA+bDd3I2z03qOeVBJj4/\n"
)
MANIFEST_FIELDS = {
    "schema_version", "repository", "repository_id", "source_sha", "package_version",
    "capability", "binary", "target", "binary_sha256", "workflow_run_id",
    "workflow_run_attempt",
}
SHA_RE = re.compile(r"^[0-9a-f]{40}$")
HEX_RE = re.compile(r"^[0-9a-f]{64}$")
STORE_ROUTE = re.compile(r"^/v1/(ac|cas)/issue-568-test/([0-9a-f]{64})$")
TOKEN = "Bearer issue-568-local-only"
EXPECTED_CASES = {
    "optional-seed-hit": ("success", 1),
    "required-verified-hit": ("success", 1),
    "malformed-ac": ("failure", 1),
    "incomplete-ac": ("failure", 1),
    "missing-stdout-blob": ("failure", 1),
    "missing-stderr-blob": ("failure", 1),
    "corrupt-stdout-blob": ("failure", 1),
    "corrupt-stderr-blob": ("failure", 1),
    "backend-unavailable": ("failure", 1),
    "required-hit-preserves-exit-7": ("failure", 1),
    "required-miss": ("failure", 0),
    "missing-proof": ("failure", 0),
    "invalid-signature": ("failure", 0),
    "tampered-capability-manifest": ("failure", 0),
    "binary-digest-mismatch": ("failure", 0),
    "optional-not-started-fallback-once": ("success", 1),
}
EXPECTED_CASE_ORDER = tuple(EXPECTED_CASES)


def fail(message: str) -> None:
    raise ValueError(message)


def env(name: str) -> str:
    value = os.environ.get(name, "")
    if not value:
        fail(f"required input {name} is empty")
    return value


def positive_int(name: str) -> int:
    value = env(name)
    if not value.isdecimal() or int(value) < 1:
        fail(f"{name} must be a positive decimal integer")
    return int(value)


def github_json(path: str) -> dict[str, object]:
    request = urllib.request.Request(
        f"https://api.github.com{path}",
        headers={
            "Accept": "application/vnd.github+json",
            "Authorization": f"Bearer {env('GH_TOKEN')}",
            "X-GitHub-Api-Version": "2022-11-28",
        },
    )
    try:
        with urllib.request.urlopen(request, timeout=20) as response:
            value = json.loads(response.read())
    except (urllib.error.HTTPError, urllib.error.URLError, TimeoutError) as error:
        fail(f"read-only GitHub API request failed for {path}: {error}")
    if not isinstance(value, dict):
        fail(f"GitHub API response for {path} is not an object")
    return value


def validate_source() -> dict[str, object]:
    if env("GH_REPOSITORY") != WORKSPACES_REPOSITORY or env("GH_REF") != "refs/heads/main":
        fail("trusted matrix must run from canonical Workspaces main")
    workflow_sha = env("GH_SHA")
    source_sha = env("CANDIDATE_SOURCE_SHA")
    if not SHA_RE.fullmatch(workflow_sha) or not SHA_RE.fullmatch(source_sha):
        fail("workflow/source commit inputs must be exact 40-hex SHAs")

    run_id = positive_int("CANDIDATE_RUN_ID")
    attempt = positive_int("CANDIDATE_RUN_ATTEMPT")
    artifact_id = positive_int("CANDIDATE_ARTIFACT_ID")
    artifact_name = env("CANDIDATE_ARTIFACT_NAME")
    if artifact_name != f"clw-required-hit-{source_sha}":
        fail("artifact name is not bound to the supplied source SHA")

    repository = github_json(f"/repos/{WORKSPACES_REPOSITORY}")
    if repository.get("id") != WORKSPACES_REPOSITORY_ID:
        fail("canonical Workspaces repository ID mismatch")
    run = github_json(f"/repos/{WORKSPACES_REPOSITORY}/actions/runs/{run_id}/attempts/{attempt}")
    head_repository, run_repository = run.get("head_repository"), run.get("repository")
    if not isinstance(head_repository, dict) or not isinstance(run_repository, dict):
        fail("candidate source run does not report repository identity")
    if (
        run.get("id") != run_id
        or run.get("run_attempt") != attempt
        or run.get("event") != "pull_request"
        or run.get("status") != "completed"
        or run.get("conclusion") != "success"
        or run.get("head_sha") != source_sha
        or run.get("path", "").split("@", 1)[0] != WORKFLOW_PATH
        or head_repository.get("id") != WORKSPACES_REPOSITORY_ID
        or run_repository.get("id") != WORKSPACES_REPOSITORY_ID
    ):
        fail("candidate run identity/event/workflow/conclusion/source/repository mismatch")

    artifacts = github_json(
        f"/repos/{WORKSPACES_REPOSITORY}/actions/runs/{run_id}/artifacts"
    ).get("artifacts")
    if not isinstance(artifacts, list):
        fail("candidate artifact listing is malformed")
    found = [item for item in artifacts if isinstance(item, dict) and item.get("id") == artifact_id]
    if len(found) != 1:
        fail("artifact ID is not uniquely present in the exact candidate source run")
    artifact = found[0]
    workflow_run = artifact.get("workflow_run")
    if not isinstance(workflow_run, dict) or (
        artifact.get("name") != artifact_name
        or artifact.get("expired") is not False
        or workflow_run.get("id") != run_id
        or workflow_run.get("repository_id") != WORKSPACES_REPOSITORY_ID
        or workflow_run.get("head_repository_id") != WORKSPACES_REPOSITORY_ID
        or workflow_run.get("head_sha") != source_sha
    ):
        fail("artifact name/expiry/repository/run/source binding mismatch")
    return {
        "source_sha": source_sha,
        "source_run_id": run_id,
        "source_run_attempt": attempt,
        "artifact_id": artifact_id,
        "artifact_name": artifact_name,
    }


def canonical_repository(remote: str) -> str | None:
    match = re.fullmatch(
        r"(?:https://github\.com/|ssh://git@github\.com/|git@github\.com:)"
        r"([A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+?)(?:\.git)?/?",
        remote,
    )
    return match.group(1).lower() if match else None


def verify_runner_checkout() -> tuple[Path, str]:
    requested = env("RUNNERS_SHA")
    if not SHA_RE.fullmatch(requested):
        fail("Runners Action SHA must be a full 40-hex commit")
    directory = Path(env("RUNNERS_DIRECTORY"))
    actual = subprocess.check_output(["git", "-C", str(directory), "rev-parse", "HEAD"], text=True).strip()
    remote = subprocess.check_output(["git", "-C", str(directory), "remote", "get-url", "origin"], text=True).strip()
    if actual != requested or canonical_repository(remote) != RUNNERS_REPOSITORY.lower():
        fail("Runners Action checkout remote or exact HEAD mismatch")
    repository = github_json(f"/repos/{RUNNERS_REPOSITORY}")
    commit = github_json(f"/repos/{RUNNERS_REPOSITORY}/commits/{requested}")
    if repository.get("id") != RUNNERS_REPOSITORY_ID or commit.get("sha") != requested:
        fail("Runners repository ID or exact Action commit is unverified")
    return directory / "actions" / "corelink-memoize", requested


def reject_duplicate_keys(pairs: list[tuple[str, object]]) -> dict[str, object]:
    result: dict[str, object] = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f"duplicate JSON field {key!r}")
        result[key] = value
    return result


def decode_manifest(raw: bytes) -> dict[str, object]:
    try:
        value = json.loads(raw, object_pairs_hook=reject_duplicate_keys)
    except (json.JSONDecodeError, UnicodeDecodeError) as error:
        fail(f"candidate manifest is malformed: {error}")
    if not isinstance(value, dict) or set(value) != MANIFEST_FIELDS:
        fail("candidate manifest has an unknown or incomplete schema")
    return value


def verify_signed_candidate() -> tuple[Path, Path, dict[str, object], Path]:
    candidate = Path(env("RUNNER_TEMP")) / "clw-required-hit-candidate"
    manifest_path, binary_path = candidate / "candidate-manifest.json", candidate / "clw"
    try:
        manifest_bytes = manifest_path.read_bytes()
    except OSError as error:
        fail(f"candidate manifest cannot be read: {error}")

    try:
        signature = base64.b64decode(env("CANDIDATE_SIGNATURE_BASE64"), validate=True)
    except (binascii.Error, ValueError) as error:
        fail(f"candidate signature is not valid base64: {error}")
    if not signature.startswith(b"-----BEGIN SSH SIGNATURE-----"):
        fail("candidate signature is not an OpenSSH detached signature")

    temp = Path(env("RUNNER_TEMP"))
    signature_path = temp / "clw-required-hit-candidate.sig"
    allowed_signers = temp / "clw-required-hit-allowed-signers"
    public_key = temp / "clw-required-hit-public-key.pub"
    signature_path.write_bytes(signature)
    allowed_signers.write_text(ALLOWED_SIGNER, encoding="ascii")
    public_key.write_text(" ".join(ALLOWED_SIGNER.strip().split()[1:]) + "\n", encoding="ascii")
    for path in (signature_path, allowed_signers, public_key):
        path.chmod(0o600)
    fingerprint = subprocess.check_output(
        ["ssh-keygen", "-lf", str(public_key), "-E", "sha256"], text=True
    ).split()[1]
    if fingerprint != FINGERPRINT:
        fail("pinned central signing key fingerprint mismatch")
    check = subprocess.run(
        ["ssh-keygen", "-Y", "verify", "-f", str(allowed_signers), "-I", PRINCIPAL,
         "-n", NAMESPACE, "-s", str(signature_path)],
        input=manifest_bytes, stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=False,
    )
    if check.returncode != 0:
        fail("detached SSH signature over exact candidate manifest bytes is invalid")

    manifest = decode_manifest(manifest_bytes)
    source_sha, run_id, attempt = env("CANDIDATE_SOURCE_SHA"), env("CANDIDATE_RUN_ID"), env("CANDIDATE_RUN_ATTEMPT")
    if (
        manifest.get("schema_version") != 1
        or isinstance(manifest.get("schema_version"), bool)
        or manifest.get("repository") != WORKSPACES_REPOSITORY
        or manifest.get("repository_id") != WORKSPACES_REPOSITORY_ID
        or manifest.get("source_sha") != source_sha
        or manifest.get("package_version") != PACKAGE_VERSION
        or manifest.get("capability") != CAPABILITY
        or manifest.get("binary") != "clw"
        or manifest.get("target") != TARGET
        or manifest.get("workflow_run_id") != run_id
        or manifest.get("workflow_run_attempt") != attempt
    ):
        fail("signed candidate repository/source/run/version/capability/target binding mismatch")
    digest = manifest.get("binary_sha256")
    if not isinstance(digest, str) or not HEX_RE.fullmatch(digest):
        fail("signed candidate binary digest is malformed")
    if not binary_path.is_file() or binary_path.is_symlink():
        fail("candidate clw is absent or not a regular file")
    if hashlib.sha256(binary_path.read_bytes()).hexdigest() != digest:
        fail("candidate clw binary does not match signed SHA256")
    if platform.system() != "Linux" or platform.machine() not in {"x86_64", "amd64"}:
        fail("hosted runner does not match the signed candidate target")
    binary_path.chmod(0o755)
    version = subprocess.run([str(binary_path), "--version"], capture_output=True, text=True, timeout=15, check=False)
    if version.returncode != 0 or version.stdout.strip() != f"clw {manifest['package_version']}":
        fail("actual candidate binary version does not match signed package version")
    return manifest_path, binary_path, manifest, signature_path


def state_path() -> Path:
    return Path(env("I568_STATE_FILE"))


def load_state(path: Path | None = None) -> dict[str, object]:
    source = path or state_path()
    try:
        value = json.loads(source.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        value = {}
    if not isinstance(value, dict):
        value = {}
    value.setdefault("records", {})
    value.setdefault("blobs", {})
    value.setdefault("unavailable", False)
    value.setdefault("snapshot", None)
    value.setdefault("last_ac_path", None)
    value.setdefault("receipt", [])
    return value


def save_state(value: dict[str, object], path: Path | None = None) -> None:
    destination = path or state_path()
    temp = destination.with_suffix(".tmp")
    temp.write_text(json.dumps(value, sort_keys=True), encoding="utf-8")
    os.replace(temp, destination)


class LoopbackHandler(BaseHTTPRequestHandler):
    server_version = "issue-568-loopback/1"

    def response(self, code: int, body: bytes = b"") -> None:
        self.send_response(code)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        if body:
            self.wfile.write(body)

    def route(self) -> re.Match[str] | None:
        if self.client_address[0] != "127.0.0.1" or self.headers.get("Authorization") != TOKEN:
            return None
        return STORE_ROUTE.fullmatch(self.path)

    def do_GET(self) -> None:
        match = self.route()
        if not match:
            self.response(403)
            return
        state = load_state(Path(self.server.state_file))
        if state["unavailable"]:
            self.response(503)
            return
        values = state["records"] if match.group(1) == "ac" else state["blobs"]
        raw = values.get(self.path)
        self.response(404) if raw is None else self.response(200, base64.b64decode(raw))

    def do_PUT(self) -> None:
        match = self.route()
        if not match:
            self.response(403)
            return
        length = int(self.headers.get("Content-Length", "0"))
        if length < 0 or length > 5 * 1024 * 1024:
            self.response(413)
            return
        body = self.rfile.read(length)
        state = load_state(Path(self.server.state_file))
        values = state["records"] if match.group(1) == "ac" else state["blobs"]
        values[self.path] = base64.b64encode(body).decode("ascii")
        if match.group(1) == "ac":
            state["last_ac_path"] = self.path
        save_state(state, Path(self.server.state_file))
        self.response(201)

    def log_message(self, _format: str, *_args: object) -> None:
        return


def serve() -> None:
    path = Path(env("I568_STATE_FILE"))
    server = HTTPServer(("127.0.0.1", 0), LoopbackHandler)
    server.state_file = str(path)
    port_file = Path(env("I568_PORT_FILE"))
    pid_file = Path(env("I568_PID_FILE"))
    port_file.write_text(str(server.server_address[1]), encoding="ascii")
    pid_file.write_text(str(os.getpid()), encoding="ascii")
    server.serve_forever(poll_interval=0.05)


def start_server() -> None:
    manifest_path, binary_path, manifest, signature_path = verify_signed_candidate()
    action_path, runners_sha = verify_runner_checkout()
    workspace = Path(env("WORKSPACES_DIRECTORY"))
    if not (workspace / "Cargo.toml").is_file():
        fail("trusted Workspaces checkout is missing Cargo.toml")
    if subprocess.check_output(["git", "-C", str(workspace), "rev-parse", "HEAD"], text=True).strip() != env("GH_SHA"):
        fail("trusted Workspaces helper checkout does not match workflow-definition SHA")
    workspace_remote = subprocess.check_output(
        ["git", "-C", str(workspace), "remote", "get-url", "origin"], text=True
    ).strip()
    if canonical_repository(workspace_remote) != WORKSPACES_REPOSITORY.lower():
        fail("trusted Workspaces harness remote is not canonical")

    root = Path(env("RUNNER_TEMP"))
    state = root / "i568-loopback-state.json"
    port_file, pid_file = root / "i568-loopback.port", root / "i568-loopback.pid"
    save_state({"records": {}, "blobs": {}, "unavailable": False, "snapshot": None, "receipt": []}, state)
    child_env = dict(os.environ)
    child_env.pop("GH_TOKEN", None)
    child_env.pop("GITHUB_TOKEN", None)
    child_env["I568_STATE_FILE"] = str(state)
    child_env["I568_PORT_FILE"] = str(port_file)
    child_env["I568_PID_FILE"] = str(pid_file)
    process = subprocess.Popen(
        [sys.executable, __file__, "serve"], env=child_env,
        stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
        start_new_session=True,
    )
    for _ in range(100):
        if port_file.is_file() and port_file.read_text(encoding="ascii").strip().isdecimal():
            break
        if process.poll() is not None:
            fail("loopback fixture server exited during startup")
        time.sleep(0.05)
    if not port_file.is_file():
        process.terminate()
        fail("loopback fixture server did not publish a port")
    port = port_file.read_text(encoding="ascii").strip()
    with Path(env("GITHUB_ENV")).open("a", encoding="utf-8") as stream:
        stream.write(f"CLW_ENDPOINT=http://127.0.0.1:{port}\n")
        stream.write("CLW_TENANT=issue-568-test\n")
        stream.write("CLW_TOKEN=issue-568-local-only\n")
        stream.write(f"CLW_CACHE_DIR={root / 'i568-clw-cache'}\n")
        stream.write(f"I568_STATE_FILE={state}\n")
        stream.write(f"I568_PORT_FILE={port_file}\n")
        stream.write(f"I568_PID_FILE={pid_file}\n")
        stream.write(f"I568_BINARY={binary_path}\n")
        stream.write(f"I568_ACTION_PATH={action_path}\n")
        stream.write(f"I568_RUNNERS_SHA={runners_sha}\n")
        stream.write(f"I568_MANIFEST_PATH={manifest_path}\n")
        stream.write(f"I568_SIGNATURE_PATH={signature_path}\n")
        stream.write("I568_CASE_MANIFEST=" + str(manifest_path) + "\n")
        stream.write("I568_CASE_SIGNATURE=" + str(signature_path) + "\n")
        stream.write(f"I568_EFFECT_LOG={root / 'i568-hit-effect-count'}\n")
        stream.write(f"I568_MISS_EFFECT={root / 'i568-miss-effect-count'}\n")
        stream.write(f"I568_PROOF_EFFECT={root / 'i568-proof-effect-count'}\n")
        stream.write(f"I568_OPTIONAL_EFFECT={root / 'i568-optional-effect-count'}\n")
        stream.write(f"I568_RECEIPT_PATH={root / 'i568-matrix-receipt.json'}\n")
    with Path(env("GITHUB_PATH")).open("a", encoding="utf-8") as stream:
        stream.write(f"{binary_path.parent}\n")
    print(json.dumps({"status": "candidate-verified-loopback-ready", "manifest": manifest,
                      "workspaces_source_sha": env("CANDIDATE_SOURCE_SHA"),
                      "candidate_run_id": env("CANDIDATE_RUN_ID"),
                      "candidate_attempt": env("CANDIDATE_RUN_ATTEMPT"),
                      "candidate_artifact_id": env("CANDIDATE_ARTIFACT_ID"),
                      "runners_action_sha": runners_sha}, sort_keys=True))


def count_effect(path: str) -> int:
    try:
        return len(Path(path).read_bytes())
    except OSError:
        return 0


def snapshot_seed() -> None:
    outcome = env("SEED_OUTCOME")
    effects = count_effect(env("I568_EFFECT_LOG"))
    expected_outcome = env("EXPECTED_SEED_OUTCOME")
    expected_effects = int(env("EXPECTED_SEED_EFFECTS"))
    if outcome != expected_outcome or effects != expected_effects:
        fail(f"optional seed expected outcome/effects {expected_outcome}/{expected_effects}, got {outcome}/{effects}")
    state = load_state()
    if not state["records"] or not state["blobs"] or not state["last_ac_path"]:
        fail("real optional Action seed did not populate loopback AC and CAS")
    state["snapshot"] = {
        "records": state["records"], "blobs": state["blobs"], "last_ac_path": state["last_ac_path"]
    }
    state["receipt"].append({"case": env("CASE_NAME"), "outcome": outcome, "child_effect_bytes": effects})
    save_state(state)


def set_fixture() -> None:
    name = env("FIXTURE_NAME")
    state = load_state()
    snapshot = state.get("snapshot")
    if name == "empty":
        state["records"] = {}
        state["blobs"] = {}
        state["last_ac_path"] = None
    elif name == "restore":
        if not isinstance(snapshot, dict):
            fail("restore requested before a real optional Action seed")
        state["records"] = dict(snapshot["records"])
        state["blobs"] = dict(snapshot["blobs"])
        state["last_ac_path"] = snapshot["last_ac_path"]
    elif name == "cached-exit-7":
        if not isinstance(snapshot, dict):
            fail("exit-code fixture requested before a real optional Action seed")
        state["records"] = dict(snapshot["records"])
        state["blobs"] = dict(snapshot["blobs"])
        state["last_ac_path"] = snapshot["last_ac_path"]
        ac_path = state["last_ac_path"]
        if not isinstance(ac_path, str) or ac_path not in state["records"]:
            fail("successful seed did not leave an action-cache record")
        record = json.loads(
            base64.b64decode(state["records"][ac_path]),
            object_pairs_hook=reject_duplicate_keys,
        )
        if not isinstance(record, dict) or set(record) != {"exit_code", "stdout", "stderr"}:
            fail("successful seed action-cache record has an unexpected schema")
        if record.get("exit_code") != 0:
            fail("successful seed record is not exit 0 before fixture mutation")
        for field in ("stdout", "stderr"):
            digest = record.get(field)
            blob_path = f"/v1/cas/issue-568-test/{digest}"
            if not isinstance(digest, str) or not HEX_RE.fullmatch(digest):
                fail(f"successful seed {field} blob digest is malformed")
            encoded_blob = state["blobs"].get(blob_path)
            if encoded_blob is None:
                fail(f"successful seed {field} blob is missing")
            blob = base64.b64decode(encoded_blob, validate=True)
            if not blob:
                fail(f"successful seed {field} blob is empty")
        record["exit_code"] = 7
        state["records"][ac_path] = base64.b64encode(
            json.dumps(record, sort_keys=True, separators=(",", ":")).encode("utf-8")
        ).decode("ascii")
        state["fixture_record_exit_code"] = record["exit_code"]
    elif name not in {"unavailable", "optional-unavailable-no-proof", "missing-proof", "invalid-signature", "tampered-capability-manifest", "binary-digest-mismatch"}:
        if not isinstance(snapshot, dict):
            fail("required cache fixture requested before an optional Action seed")
        state["records"] = dict(snapshot["records"])
        state["blobs"] = dict(snapshot["blobs"])
        state["last_ac_path"] = snapshot["last_ac_path"]
    state["unavailable"] = name in {"unavailable", "optional-unavailable-no-proof"}

    if name == "malformed-ac":
        state["records"][state["last_ac_path"]] = base64.b64encode(b"{").decode("ascii")
    elif name == "incomplete-ac":
        state["records"][state["last_ac_path"]] = base64.b64encode(b'{"exit_code":0}').decode("ascii")
    elif name in {"missing-stdout", "missing-stderr", "corrupt-stdout", "corrupt-stderr"}:
        record_bytes = base64.b64decode(state["records"][state["last_ac_path"]])
        record = json.loads(record_bytes, object_pairs_hook=reject_duplicate_keys)
        field = "stdout" if name.endswith("stdout") else "stderr"
        blob_path = f"/v1/cas/issue-568-test/{record[field]}"
        if name.startswith("missing"):
            state["blobs"].pop(blob_path, None)
        else:
            corruption = b"corrupt " + field.encode("ascii")
            state["blobs"][blob_path] = base64.b64encode(corruption).decode("ascii")

    env_file = Path(env("GITHUB_ENV"))
    manifest_path, signature_path = env("I568_MANIFEST_PATH"), env("I568_SIGNATURE_PATH")
    proof_effect = Path(env("I568_PROOF_EFFECT"))
    proof_effect.unlink(missing_ok=True)
    if name in {"missing-proof", "optional-unavailable-no-proof", "invalid-signature", "tampered-capability-manifest", "binary-digest-mismatch"}:
        temp = Path(env("RUNNER_TEMP"))
        manifest_path, signature_path = "", ""
        if name not in {"missing-proof", "optional-unavailable-no-proof"}:
            original_manifest = Path(env("I568_MANIFEST_PATH")).read_bytes()
            original_signature = Path(env("I568_SIGNATURE_PATH")).read_bytes()
            case_manifest = temp / f"{name}.json"
            case_signature = temp / f"{name}.sig"
            if name == "invalid-signature":
                case_manifest.write_bytes(original_manifest)
                case_signature.write_bytes(b"not an OpenSSH signature\n")
            elif name == "tampered-capability-manifest":
                altered = json.loads(original_manifest, object_pairs_hook=reject_duplicate_keys)
                altered["capability"] = "required-hit/v999"
                case_manifest.write_text(json.dumps(altered, sort_keys=True) + "\n", encoding="utf-8")
                case_signature.write_bytes(original_signature)
            else:
                wrong_dir = temp / "wrong-candidate-bin"
                wrong_dir.mkdir(exist_ok=True)
                wrong_binary = wrong_dir / "clw"
                shutil.copy2(env("I568_BINARY"), wrong_binary)
                with wrong_binary.open("ab") as stream:
                    stream.write(b"\0")
                with Path(env("GITHUB_PATH")).open("a", encoding="utf-8") as stream:
                    stream.write(f"{wrong_dir}\n")
                case_manifest.write_bytes(original_manifest)
                case_signature.write_bytes(original_signature)
            manifest_path, signature_path = str(case_manifest), str(case_signature)
        with env_file.open("a", encoding="utf-8") as stream:
            stream.write(f"I568_CASE_MANIFEST={manifest_path}\n")
            stream.write(f"I568_CASE_SIGNATURE={signature_path}\n")
    else:
        with env_file.open("a", encoding="utf-8") as stream:
            stream.write(f"I568_CASE_MANIFEST={manifest_path}\n")
            stream.write(f"I568_CASE_SIGNATURE={signature_path}\n")
    if name in {"unavailable", "optional-unavailable-no-proof"}:
        with Path(env("GITHUB_PATH")).open("a", encoding="utf-8") as stream:
            stream.write(f"{Path(env('I568_BINARY')).parent}\n")
    save_state(state)


def assert_case() -> None:
    name, actual, expected = env("CASE_NAME"), env("ACTION_OUTCOME"), env("EXPECTED_OUTCOME")
    effect_path = env("EFFECT_PATH")
    expected_effects = int(env("EXPECTED_EFFECTS"))
    effects = count_effect(effect_path)
    if actual != expected or effects != expected_effects:
        fail(f"{name}: expected Action outcome/effects {expected}/{expected_effects}, got {actual}/{effects}")
    state = load_state()
    entry = {"case": name, "outcome": actual, "child_effect_bytes": effects}
    if name == "required-hit-preserves-exit-7":
        if state.get("fixture_record_exit_code") != 7:
            fail("exit-code fixture did not preserve the valid AC record exit_code=7")
        if state.get("cli_probe_effects_before") != 1 or state.get("cli_probe_effects_after") != 1:
            fail("candidate CLI exit-7 probe did not preserve the one-child effect count")
        entry["action_outcome"] = actual
        entry["record_exit_code"] = 7
        entry["cli_probe_exit"] = state.get("cli_probe_exit")
        entry["cli_probe_effects_before"] = state["cli_probe_effects_before"]
        entry["cli_probe_effects_after"] = state["cli_probe_effects_after"]
        if entry["cli_probe_exit"] != 7:
            fail("hosted CLI probe did not verify the cached exit code 7")
    state["receipt"].append(entry)
    save_state(state)


def probe_cached_exit_7() -> None:
    state = load_state()
    if state.get("fixture_record_exit_code") != 7:
        fail("exact-exit probe requires a verified valid AC record with exit_code=7")
    effect_path = env("I568_EFFECT_LOG")
    before = count_effect(effect_path)
    command = (
        "printf 'x' >> \"$I568_EFFECT_LOG\"\n"
        "printf 'cached stdout\\n'\n"
        "printf 'cached stderr\\n' >&2\n"
        "exit 0\n"
    )
    child_env = dict(os.environ)
    child_env.pop("GH_TOKEN", None)
    child_env.pop("GITHUB_TOKEN", None)
    if child_env.get("I568_CASE_KEY") != "issue-568-hit-seed":
        fail("cached exit-7 probe must use the exact seed environment value")
    child_env["CLW_REF_DOMAIN"] = "runner"
    result = subprocess.run(
        [env("I568_BINARY"), "run", "--require-hit", "--input", "Cargo.toml",
         "--env", "I568_CASE_KEY", "--", "bash", "-c", command],
        cwd=env("WORKSPACES_DIRECTORY"), env=child_env,
        stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        timeout=30, check=False,
    )
    after = count_effect(effect_path)
    if result.returncode != 7 or after != before or before != 1:
        fail(
            f"candidate CLI did not replay cached exit 7 without child effects: "
            f"exit={result.returncode}, effects={before}->{after}"
        )
    state["cli_probe_exit"] = result.returncode
    state["cli_probe_effects_before"] = before
    state["cli_probe_effects_after"] = after
    save_state(state)
    print(json.dumps({"status": "cached-exit-7-proven", "exit_code": result.returncode,
                      "child_effect_bytes_before": before, "child_effect_bytes_after": after}, sort_keys=True))


def verify_restored_binary_path() -> None:
    expected = Path(env("I568_BINARY")).resolve()
    actual = shutil.which("clw")
    if not actual or Path(actual).resolve() != expected:
        fail("verified candidate clw is not first on PATH after the negative digest fixture")
    manifest = decode_manifest(Path(env("I568_MANIFEST_PATH")).read_bytes())
    actual_digest = hashlib.sha256(expected.read_bytes()).hexdigest()
    if actual_digest != manifest["binary_sha256"]:
        fail("restored clw path does not match the signed candidate digest")
    print(json.dumps({"status": "verified-original-candidate-restored", "path": str(expected),
                      "binary_sha256": actual_digest}, sort_keys=True))


def write_receipt() -> None:
    state = load_state()
    cases = state["receipt"]
    names = [item.get("case") for item in cases if isinstance(item, dict)]
    failures: list[str] = []
    if tuple(names) != EXPECTED_CASE_ORDER:
        failures.append("required case set/order is incomplete or duplicated")
    for item in cases:
        if not isinstance(item, dict):
            failures.append("receipt includes a malformed case")
            continue
        expected = EXPECTED_CASES.get(str(item.get("case")))
        actual = (item.get("outcome"), item.get("child_effect_bytes"))
        if expected is None or actual != expected:
            failures.append(f"case {item.get('case')!r} has unexpected outcome/effects {actual!r}")
        if item.get("case") == "required-hit-preserves-exit-7":
            if (
                item.get("action_outcome") != "failure"
                or item.get("record_exit_code") != 7
                or item.get("cli_probe_exit") != 7
                or item.get("cli_probe_effects_before") != 1
                or item.get("cli_probe_effects_after") != 1
            ):
                failures.append("cached exit-7 Action and exact CLI proof are incomplete")
    status = "passed" if not failures else "incomplete-or-failed"
    receipt = {
        "schema_version": 1,
        "status": status,
        "repository": WORKSPACES_REPOSITORY,
        "repository_id": WORKSPACES_REPOSITORY_ID,
        "trusted_workspaces_harness_sha": env("GH_SHA"),
        "matrix_run_id": env("GITHUB_RUN_ID"),
        "matrix_run_attempt": env("GITHUB_RUN_ATTEMPT"),
        "matrix_run_url": f"{env('GITHUB_SERVER_URL')}/{WORKSPACES_REPOSITORY}/actions/runs/{env('GITHUB_RUN_ID')}",
        "workspaces_source_sha": env("CANDIDATE_SOURCE_SHA"),
        "source_run_id": env("CANDIDATE_RUN_ID"),
        "source_run_attempt": env("CANDIDATE_RUN_ATTEMPT"),
        "candidate_artifact_id": env("CANDIDATE_ARTIFACT_ID"),
        "candidate_artifact_name": env("CANDIDATE_ARTIFACT_NAME"),
        "runners_action_sha": env("RUNNERS_SHA"),
        "runner_image_label": "ubuntu-24.04",
        "runner_image_os": os.environ.get("ImageOS", ""),
        "runner_image_version": os.environ.get("ImageVersion", ""),
        "workflow_action_pins": {
            "actions/checkout": "11bd71901bbe5b1630ceea73d27597364c9af683",
            "actions/download-artifact": "d3f86a106a0bac45b974a628896c90dbdf5c8093",
            "actions/upload-artifact": "ea165f8d65b6e75b540449e92b4886f43607fa02",
        },
        "candidate_binary_sha256": hashlib.sha256(Path(env("I568_BINARY")).read_bytes()).hexdigest(),
        "candidate_signature_sha256": hashlib.sha256(Path(env("I568_SIGNATURE_PATH")).read_bytes()).hexdigest(),
        "capability": CAPABILITY,
        "cases": cases,
        "failures": failures,
    }
    Path(env("I568_RECEIPT_PATH")).write_text(json.dumps(receipt, sort_keys=True, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(receipt, sort_keys=True))
    if failures:
        fail("final hosted matrix receipt is incomplete or failed: " + "; ".join(failures))


def stop_server() -> None:
    try:
        pid = int(Path(env("I568_PID_FILE")).read_text(encoding="ascii"))
        os.kill(pid, signal.SIGTERM)
    except (OSError, ValueError):
        return


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("mode", choices=("validate-source", "prepare", "serve", "snapshot-seed", "fixture", "assert-case", "probe-exit7", "verify-path", "receipt", "stop"))
    args = parser.parse_args()
    try:
        if args.mode == "validate-source":
            print(json.dumps({"status": "source-run-and-artifact-valid", **validate_source()}, sort_keys=True))
        elif args.mode == "prepare":
            validate_source()
            start_server()
        elif args.mode == "serve":
            serve()
        elif args.mode == "snapshot-seed":
            snapshot_seed()
        elif args.mode == "fixture":
            set_fixture()
        elif args.mode == "assert-case":
            assert_case()
        elif args.mode == "probe-exit7":
            probe_cached_exit_7()
        elif args.mode == "verify-path":
            verify_restored_binary_path()
        elif args.mode == "receipt":
            write_receipt()
        else:
            stop_server()
    except (OSError, subprocess.SubprocessError, ValueError, TypeError, KeyError) as error:
        print(f"issue-568 matrix: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
