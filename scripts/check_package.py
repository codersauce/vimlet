#!/usr/bin/env python3
"""Build the packaged crate from two independent consumer workspaces."""
import os
from pathlib import Path
import subprocess
import tarfile
import tempfile
import tomllib

root = Path(__file__).resolve().parents[1]
version = tomllib.loads((root / "Cargo.toml").read_text())["package"]["version"]
with tempfile.TemporaryDirectory(prefix="vimlet-package-") as directory:
    work = Path(directory)
    env = dict(os.environ, CARGO_TARGET_DIR=str(work / "target"))
    subprocess.run(["cargo", "package", "--locked", "--allow-dirty"], cwd=root, env=env, check=True)
    with tarfile.open(work / "target/package" / f"vimlet-{version}.crate") as package:
        package.extractall(work, filter="data")
    for feature in ["string", "ropey"]:
        consumer = work / feature
        (consumer / "src").mkdir(parents=True)
        features = '["ropey"]' if feature == "ropey" else "[]"
        manifest = f'''[package]
name = "verify-{feature}"
version = "0.0.0"
edition = "2021"
[dependencies]
vimlet = {{ path = "../vimlet-{version}", default-features = false, features = {features} }}
'''
        (consumer / "Cargo.toml").write_text(manifest)
        (consumer / "src/main.rs").write_text('fn main() { let _ = vimlet::Session::new(); }')
        subprocess.run(["cargo", "fetch"], cwd=consumer, env=env, check=True)
        subprocess.run(["cargo", "run", "--frozen"], cwd=consumer, env=env, check=True)
        graph = subprocess.check_output(
            ["cargo", "tree", "--frozen", "-e", "normal,build", "--prefix", "none"],
            cwd=consumer, env=env, text=True)
        packages = {line.split()[0] for line in graph.splitlines()}
        if ("ropey" in packages) != (feature == "ropey") or packages & {"red", "codex-tui"}:
            raise RuntimeError(graph)
        print(f"{feature} consumer enabled {', '.join(sorted(packages))}")
