"""Package the unsigned CI test executable with its exact source and provenance."""
import hashlib
import io
import json
import os
from pathlib import Path
import subprocess
import tempfile
import zipfile

root = Path(__file__).resolve().parents[1]
def require_clean_checkout():
    status = subprocess.check_output(
        ["git", "--no-optional-locks", "status", "--porcelain", "--untracked-files=normal"], cwd=root)
    if status:
        raise RuntimeError("Refusing to package a dirty checkout")

require_clean_checkout()
exe = root / "src-tauri/target/release/slopbar.exe"
mcp = exe.with_name("slopbar-mcp.exe")
head = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=root, text=True).strip()
base = "60a7c8e0835b4cd320dba3e0b1352d7a8161c9e1"
repository = os.environ.get("GITHUB_REPOSITORY", "layibabalola/slopbar")
run = os.environ.get("GITHUB_RUN_ID")
ci = f"https://github.com/{repository}/actions/runs/{run}" if run else None
with tempfile.TemporaryDirectory() as temporary:
    info_path = Path(temporary) / "build-info.json"
    subprocess.run([str(exe), "--build-info-to", str(info_path)], check=True, timeout=15)
    binary_build = json.loads(info_path.read_text())
if (binary_build["commit"] != head or binary_build["dirty"]
        or binary_build["tauri_dev"] is not False or binary_build["upstream_updates"]
        or binary_build["source_tree"] != subprocess.check_output(["git", "rev-parse", "HEAD^{tree}"], cwd=root, text=True).strip()):
    raise RuntimeError("Executable provenance or release configuration does not match this checkout")
manifest = {
    "commit": head,
    "source_tree": subprocess.check_output(["git", "rev-parse", "HEAD^{tree}"], cwd=root, text=True).strip(),
    "base": base,
    "executable_sha256": hashlib.sha256(exe.read_bytes()).hexdigest(),
    "mcp_executable_sha256": hashlib.sha256(mcp.read_bytes()).hexdigest(),
    "target": "x86_64-pc-windows-msvc",
    "signed": False,
    "upstream_updates": False,
    "tauri_custom_protocol": not binary_build["tauri_dev"],
    "cargo_features": ["desktop", "tauri/custom-protocol"],
    "ci": ci,
    "pr": "https://github.com/thelioo/slopbar/pull/6",
    "binary_build": binary_build,
}
instructions = f"""SlopBar independent monitoring — unsigned Windows test build
Source commit: {head}
Build and checks: {ci or 'see BUILD-MANIFEST.json'}

1. Quit any running SlopBar.
2. Extract the whole ZIP to a folder, then run slopbar.exe.
3. Open Settings and choose Connect monitoring for each account. Sign in to the
   same account and organization using the official CLI. This creates a separate
   monitoring login. No model prompts are sent.
4. Test idle renewal. Export a report from Settings > General > Diagnostics >
   Export diagnostics and upload the generated SlopBar-Diagnostics-*.json for review.
   The report includes anonymous account numbers, usage percentages/timestamps,
   refresh/backoff outcomes, login stages, and build metadata. It excludes tokens,
   emails, aliases, account/organization IDs, paths, URLs, and provider response bodies.
   Collection is local; nothing is uploaded automatically. The rolling encrypted
   journal keeps changes plus hourly heartbeats, at most 7 days / 10,000 events.
   Reports show the actual retained period. Keep the exported report private
   until you choose to share it. Prior independent monitoring logins are reused.

slopbar-mcp.exe is an optional MCP server so agents can use SlopBar's settings
(see source/README.md). It only talks to the running SlopBar on this machine.

This build uses manual updates. Install a newer reviewed build to get fixes.
It uses Tauri's release custom-protocol configuration with embedded assets.
It reads your existing settings/accounts from the usual SlopBar data folder.
Monitoring credentials use current-user Windows DPAPI. The older account-switching
vault is still plaintext. These are full OAuth credentials, not usage-only keys.

CI tests are synthetic and do not prove live provider acceptance, WSL process
termination, or main-CLI validity over 24 hours. Claude Credential Manager import
is unsupported and fails safely. Real GUI/login/renewal checks are still needed.
Read source/SECURITY.md for the remaining limits. This is not an official release.
"""
source = subprocess.check_output(["git", "archive", "--format=zip", "--prefix=source/", head], cwd=root)
patch = subprocess.check_output(["git", "diff", "--binary", base, head], cwd=root)
require_clean_checkout()
output = root / "SlopBar-Independent-Monitoring.zip"
with zipfile.ZipFile(output, "w", zipfile.ZIP_DEFLATED, compresslevel=6) as bundle:
    bundle.write(exe, "slopbar.exe")
    bundle.write(mcp, "slopbar-mcp.exe")
    loader = exe.with_name("WebView2Loader.dll")
    if loader.exists():
        bundle.write(loader, loader.name)
    bundle.writestr("START-HERE.txt", instructions)
    bundle.writestr("BUILD-MANIFEST.json", json.dumps(manifest, indent=2) + "\n")
    bundle.writestr("changes.patch", patch)
    bundle.write(root / "LICENSE", "LICENSE")
    with zipfile.ZipFile(io.BytesIO(source)) as archive:
        for entry in archive.infolist():
            bundle.writestr(entry, archive.read(entry))
print(f"Packaged {output.name} at {head}")
print(f"ZIP SHA256: {hashlib.sha256(output.read_bytes()).hexdigest()}")
