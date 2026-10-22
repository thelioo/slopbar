"""Exercise the real build script and packaging guard using a disposable std-only crate."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

source = Path(__file__).resolve().parents[1]
with tempfile.TemporaryDirectory(prefix="slopbar-provenance-") as temporary:
    root = Path(temporary)
    crate = root / "src-tauri"
    (crate / "src").mkdir(parents=True)
    for path in [root / "src", root / "scripts", root / ".github", crate / "icons"]:
        path.mkdir()
    shutil.copyfile(source / "src-tauri/build.rs", crate / "build.rs")
    (crate / "Cargo.toml").write_text('[package]\nname="provenance-probe"\nversion="0.1.0"\nedition="2021"\n[features]\ndesktop=[]\n')
    (crate / "src/main.rs").write_text('fn main() { println!("{}", env!("SLOPBAR_BUILD_DIRTY")); }\n')
    (crate / "tauri.conf.json").write_text('{}\n')
    (root / "README.md").write_text('probe\n')
    (root / ".gitignore").write_text('src-tauri/target/\n')
    subprocess.run(["git", "init", "-q"], cwd=root, check=True)
    subprocess.run(["git", "config", "user.name", "Synthetic test"], cwd=root, check=True)
    subprocess.run(["git", "config", "user.email", "test@example.invalid"], cwd=root, check=True)
    subprocess.run(["cargo", "generate-lockfile", "--offline"], cwd=crate, check=True, capture_output=True)
    subprocess.run(["git", "add", "."], cwd=root, check=True)
    subprocess.run(["git", "commit", "-qm", "synthetic"], cwd=root, check=True)

    def build(expected):
        result = subprocess.run(["cargo", "build", "--offline", "--verbose"], cwd=crate, check=True, capture_output=True, text=True)
        executable = crate / "target/debug" / ("provenance-probe.exe" if os.name == "nt" else "provenance-probe")
        actual = subprocess.check_output([str(executable)], text=True).strip()
        assert actual == str(expected).lower(), (expected, actual, result.stderr)
        return any("Running" in line and "build-script-build" in line for line in result.stderr.splitlines())

    build(False)
    for _ in range(3):
        reran = build(False)
    assert not reran, "build script never reaches a stable clean build"
    # No Git command between changing a file and rebuilding.
    for relative in ["src-tauri/src/main.rs", "src-tauri/Cargo.toml", "src-tauri/tauri.conf.json", "README.md"]:
        path = root / relative
        original = path.read_bytes()
        suffix = b" " if path.suffix == ".json" else b"\n"  # legal edits in all probe formats
        path.write_bytes(original + suffix)
        build(True)
        path.write_bytes(original)
        build(False)
    untracked = crate / "src/untracked.txt"
    untracked.write_text("synthetic\n")
    build(True)
    untracked.unlink()
    build(False)

    # Execute the actual packaging guard, without requiring or running an exe.
    script = (source / "scripts/package-test-build.py").read_text()
    guard = script[script.index("def require_clean_checkout"):script.index("\nrequire_clean_checkout()")]
    namespace = {"root": root, "subprocess": subprocess}
    exec(guard, namespace)
    namespace["require_clean_checkout"]()
    (crate / "src/main.rs").write_bytes((crate / "src/main.rs").read_bytes() + b"// changed\n")
    try:
        namespace["require_clean_checkout"]()
    except RuntimeError:
        pass
    else:
        raise AssertionError("packaging accepted dirty sources")
print(json.dumps({"steady_build": "pass", "edit_and_revert": "pass", "untracked_files": "pass", "packaging_guard": "pass"}))
