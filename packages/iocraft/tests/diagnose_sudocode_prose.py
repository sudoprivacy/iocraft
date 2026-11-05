"""Run the original failing native CLI journeys with this diagnostic renderer.

This is a temporary source reduction, not a signed SudoCode qualification.
Only Cargo's renderer source resolution changes; retain its lock diff and every
original test result, with and without the optional application-output trace.
"""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time
import zipfile

import psutil


def main():
    source, renderer, output, terminal = [Path(p).resolve() for p in sys.argv[1:]]
    output.mkdir(parents=True, exist_ok=False)
    expected = "dd1efe4ad41afa1bad4a052b6b46b6648c7e268e"
    actual = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=source, text=True).strip()
    assert actual == expected
    assert not subprocess.check_output(["git", "status", "--porcelain"], cwd=source)
    original_lock = (source / "rust/Cargo.lock").read_bytes()
    patch = 'patch."https://github.com/sudoprivacy/iocraft.git".iocraft.path=' + json.dumps(
        str(renderer / "packages/iocraft").replace("\\", "/")
    )
    command = ["cargo", "--config", patch, "test", "-p", "rusty-sudocode-cli",
               "--test", "pty_prose_preview", "--no-run", "--message-format=json"]
    with (output / "build.jsonl").open("wb") as stdout, (output / "build.stderr.log").open("wb") as stderr:
        build = subprocess.run(command, cwd=source / "rust", stdout=stdout, stderr=stderr)
    report = {"diagnostic_only": True, "source": actual,
              "renderer": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=renderer, text=True).strip(),
              "rustc": subprocess.check_output(["rustc", "--version", "--verbose"], text=True),
              "original_lock_sha256": hashlib.sha256(original_lock).hexdigest(),
              "build_command": command, "build_exit": build.returncode, "cases": []}
    (output / "cargo-lock.diff").write_bytes(subprocess.check_output(
        ["git", "diff", "--", "rust/Cargo.lock"], cwd=source))
    (output / "result.json").write_text(json.dumps(report, indent=2) + "\n")
    assert build.returncode == 0, "Diagnostic build failed; inspect the retained compiler logs"
    metadata = json.loads(subprocess.check_output(
        ["cargo", "--config", patch, "metadata", "--locked", "--format-version=1"], cwd=source / "rust"))
    resolved = [package for package in metadata["packages"] if package["name"] == "iocraft"]
    assert len(resolved) == 1 and resolved[0]["source"] is None
    assert Path(resolved[0]["manifest_path"]).resolve() == renderer / "packages/iocraft/Cargo.toml"
    report["renderer_resolution"] = resolved[0]
    artifacts = [json.loads(line) for line in (output / "build.jsonl").read_text(encoding="utf-8").splitlines()]
    tests = [entry["executable"] for entry in artifacts
             if entry.get("reason") == "compiler-artifact"
             and entry["target"]["name"] == "pty_prose_preview" and entry.get("executable")]
    assert len(tests) == 1
    test_binary = Path(tests[0])
    harnesses = output.parent / (output.name + "-harnesses")
    harnesses.mkdir(exist_ok=False)
    spec = importlib.util.spec_from_file_location("terminal_installer", source / "e2e/terminal-resize/setup_windows.py")
    installer = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(installer)
    installer.verify_archive(terminal / "host.zip")
    report["host_archive_sha256"] = installer.SHA256
    report["host_distribution_commit"] = installer.COMMIT
    with zipfile.ZipFile(terminal / "host.zip") as package:
        members = set(package.namelist())
        hosts = [name for name in members if name.endswith("/conpty.dll")
                 and str(Path(name).parent / "OpenConsole.exe").replace("\\", "/") in members]
        assert len(hosts) == 1, hosts
        host_members = [hosts[0], str(Path(hosts[0]).parent / "OpenConsole.exe").replace("\\", "/")]
        sidecars = {Path(name).name: package.read(name) for name in host_members}
    report["sidecars"] = {name: hashlib.sha256(data).hexdigest() for name, data in sidecars.items()}
    report["test_executable_sha256"] = hashlib.sha256(test_binary.read_bytes()).hexdigest()
    programs = {}
    for backend in ["system", "bundled"]:
        directory = harnesses / backend
        directory.mkdir()
        programs[backend] = directory / test_binary.name
        shutil.copyfile(test_binary, programs[backend])
        assert hashlib.sha256(programs[backend].read_bytes()).hexdigest() == report["test_executable_sha256"]
        if backend == "bundled":
            for name, data in sidecars.items():
                (directory / name).write_bytes(data)
    try:
        for case in ["ambiguous_grapheme_width_uses_complete_block_fallback",
                     "live_prose_is_visible_and_input_remains_usable"]:
            for traced in [False, True]:
                for backend in ["system", "bundled"]:
                    label = f"{case}-{'traced' if traced else 'untraced'}"
                    label += f"-{backend}"
                    environment = {**os.environ, "SCODE_TEST_BACKEND": "mock"}
                    environment.pop("IOCRAFT_DIAG_DELAYED_WRAP", None)
                    environment.pop("IOCRAFT_FRAME_WRITE_TRACE", None)
                    if traced:
                        environment["IOCRAFT_FRAME_WRITE_TRACE"] = str(output / f"{label}-application.log")
                    invocation = [str(programs[backend]), case, "--exact", "--nocapture", "--test-threads=1"]
                    loaded = []
                    observer_error = None
                    with (output / f"{label}.stdout.log").open("wb") as stdout, \
                            (output / f"{label}.stderr.log").open("wb") as stderr:
                        process = subprocess.Popen(invocation, cwd=source / "rust", env=environment,
                                                   stdout=stdout, stderr=stderr)
                        if backend == "bundled":
                            while process.poll() is None and not loaded:
                                try:
                                    loaded = [mapping.path for mapping in psutil.Process(process.pid).memory_maps()
                                              if Path(mapping.path).name.lower() == "conpty.dll"]
                                except psutil.NoSuchProcess:
                                    break
                                except psutil.Error as error:
                                    observer_error = str(error)
                                    break
                                time.sleep(0.05)
                        code = process.wait()
                    entry = {"case": case, "traced": traced, "backend": backend,
                             "command": invocation, "exit": code, "loaded_conpty_modules": loaded,
                             "observer_error": observer_error}
                    if backend == "bundled":
                        entry["pinned_host_observed"] = bool(loaded) and all(
                            Path(p).resolve() == programs[backend].parent / "conpty.dll" for p in loaded)
                    if traced:
                        trace = output / f"{label}-application.log"
                        entry["application_trace_bytes"] = trace.stat().st_size if trace.exists() else 0
                    report["cases"].append(entry)
                    print(json.dumps(entry), flush=True)
    finally:
        changed = subprocess.check_output(["git", "diff", "--name-only"], cwd=source, text=True).splitlines()
        report["changed_source_paths"] = changed
        bundled = [entry for entry in report["cases"] if entry["backend"] == "bundled"]
        report["pinned_host_cases_success"] = len(bundled) == 4 and all(
            entry["exit"] == 0 and entry["pinned_host_observed"] for entry in bundled)
        (output / "result.json").write_text(json.dumps(report, indent=2) + "\n")
        assert set(changed) <= {"rust/Cargo.lock"}, "Do not alter the original product or assertions"
    assert len(report["cases"]) == 8 and all(entry["exit"] == 0 for entry in report["cases"]), \
        "Original CLI journeys failed; retain and inspect their application/wire traces"
    assert all(entry.get("application_trace_bytes", 1) > 0 for entry in report["cases"]), \
        "A traced case did not produce application bytes; no diagnostic conclusion is valid"


if __name__ == "__main__":
    main()
