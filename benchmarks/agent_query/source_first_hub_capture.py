"""Capture registered public hub requests on pinned paired graph artifacts."""

from __future__ import annotations

import argparse
import hashlib
import json
import shutil
import subprocess
import time
from pathlib import Path
from types import SimpleNamespace

from benchmarks.agent_query.mcp_compare import verify_environment
from benchmarks.agent_query.mcp_transport import StdioMcp


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def checked_source(root: Path, registered: dict) -> None:
    head = subprocess.run(
        ["git", "-C", str(root), "rev-parse", "HEAD"],
        check=True, capture_output=True, text=True,
    ).stdout.strip()
    dirty = subprocess.run(
        ["git", "-C", str(root), "status", "--porcelain=v1"],
        check=True, capture_output=True, text=True,
    ).stdout.strip()
    if head != registered["commit"] or dirty:
        raise ValueError(f"{registered['name']} source checkout changed")
    for witness in registered["witnesses"]:
        relative = Path(witness["file"])
        if relative.is_absolute() or ".." in relative.parts:
            raise ValueError("invalid registered source path")
        path = root / relative
        if sha256(path) != witness["fileSha256"]:
            raise ValueError(f"source bytes changed: {relative}")
        lines = path.read_text().splitlines()
        if lines[witness["line"] - 1] != witness["sourceLine"]:
            raise ValueError(f"source anchor changed: {relative}:{witness['line']}")


def capture(args: argparse.Namespace) -> None:
    registration = json.loads(args.registration.read_text())
    if registration["schema"] != "compass.source-first-hub-three-language-registration/1":
        raise ValueError("unsupported registration schema")
    source_run = json.loads((args.source_run / "run.json").read_text())
    if source_run["suiteDigest"] != registration["sourceSuiteSha256"]:
        raise ValueError("source suite changed")
    for name, path in [
        ("compass", args.compass),
        ("graphifyMcpPython", args.graphify_python),
        ("graphifyCli", args.graphify_cli),
    ]:
        if sha256(path) != registration["binarySha256"][name]:
            raise ValueError(f"{name} binary changed")
    if sha256(args.graphify_environment) != registration["graphifyMcpEnvironmentSha256"]:
        raise ValueError("Graphify MCP environment manifest changed")
    verify_environment(SimpleNamespace(
        graphify_python=args.graphify_python,
        graphify_environment=args.graphify_environment,
    ))

    prepared = []
    for registered in registration["repositories"]:
        matches = [item for item in source_run["repositories"]
                   if item["repository"] == registered["name"]]
        if len(matches) != 1:
            raise ValueError("registered repository has no unique captured run")
        repository = matches[0]
        root = Path(repository["source"])
        checked_source(root, registered)
        graph_paths = {}
        for tool in ("compass", "graphify"):
            path = Path(repository[f"{tool}Graph"])
            if sha256(path) != registered["graphSha256"][tool]:
                raise ValueError(f"{registered['name']} {tool} graph changed")
            graph_paths[tool] = path
        prepared.append((registered, root, graph_paths))

    args.output.mkdir(parents=True, exist_ok=False)
    shutil.copy2(args.registration, args.output / "registration.json")
    shutil.copy2(Path(__file__), args.output / "collector.py")
    report = {
        "schema": "compass.source-first-hub-three-language-capture/1",
        "complete": False,
        "registrationSha256": sha256(args.registration),
        "sourceRunJsonSha256": sha256(args.source_run / "run.json"),
        "collectorSha256": sha256(Path(__file__)),
        "transportSha256": sha256(Path(__file__).with_name("mcp_transport.py")),
        "binarySha256": registration["binarySha256"],
        "results": [],
    }
    run_file = args.output / "run.json"
    for registered, root, graphs in prepared:
        for tool in ("compass", "graphify"):
            argv = ([str(args.compass), "serve"] if tool == "compass" else
                    [str(args.graphify_python), "-m", "graphify.serve"])
            argv += ["--graph", str(graphs[tool]), "--transport", "stdio"]
            directory = args.output / "raw" / registered["name"] / tool
            result = {
                "repository": registered["name"],
                "tool": tool,
                "method": "god_nodes",
                "arguments": registration["toolRequest"]["arguments"],
                "graphSha256": registered["graphSha256"][tool],
                "argv": argv,
            }
            started = time.monotonic()
            try:
                with StdioMcp(
                    argv, root, directory,
                    timeout=registration["toolRequest"]["timeoutSeconds"],
                    max_bytes=registration["toolRequest"]["maxResponseBytes"],
                ) as session:
                    session.initialize()
                    listing = session.send("tools/list", {})
                    advertised = {item["name"] for item in
                                  listing.get("result", {}).get("tools", [])}
                    if "god_nodes" not in advertised:
                        raise ValueError("server did not advertise god_nodes")
                    response = session.send("tools/call", {
                        "name": "god_nodes",
                        "arguments": registration["toolRequest"]["arguments"],
                    })
                    content = response.get("result", {}).get("content", [])
                    rendered = "\n".join(item["text"] for item in content
                                         if item.get("type") == "text")
                    result.update({
                        "response": response,
                        "text": rendered,
                        "textBytes": len(rendered.encode()),
                        "executionSucceeded": "error" not in response
                        and not response.get("result", {}).get("isError", False),
                    })
            except (OSError, ValueError, RuntimeError, TimeoutError) as error:
                result.update({"executionSucceeded": False, "captureError": str(error)})
            result["wallMs"] = round((time.monotonic() - started) * 1000)
            report["results"].append(result)
            run_file.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
            print(registered["name"], tool, result["executionSucceeded"], flush=True)
            if sha256(graphs[tool]) != registered["graphSha256"][tool]:
                raise ValueError("graph changed during capture")
        checked_source(root, registered)
    verify_environment(SimpleNamespace(
        graphify_python=args.graphify_python,
        graphify_environment=args.graphify_environment,
    ))
    report["complete"] = True
    run_file.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--registration", type=Path, required=True)
    parser.add_argument("--source-run", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--compass", type=Path, required=True)
    parser.add_argument("--graphify-python", type=Path, required=True)
    parser.add_argument("--graphify-cli", type=Path, required=True)
    parser.add_argument("--graphify-environment", type=Path, required=True)
    capture(parser.parse_args())
