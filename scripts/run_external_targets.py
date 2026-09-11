#!/usr/bin/env python3
"""Resume-safe orchestrator for the Syntheseus/SynPlanner target runners.

RENKIN and AiZynthFinder retain their existing comparison runner because their
container/native resource instrumentation is already established there. This
orchestrator gives the two Python planners the same frozen-target loop,
one-process-per-target boundary, output identity, and resume semantics.
"""

from __future__ import annotations

import argparse
import os
import re
import subprocess
import sys
import threading
import time
from pathlib import Path

from four_tool_record import FourToolRecord, load_records
from four_tool_resources import apply_resource_limits, resource_environment


_DOCKER_MEM_RE = re.compile(r"^\s*([0-9.]+)\s*([kmgt]i?b|b)\s*/", re.IGNORECASE)
_MEMORY_UNITS = {
    "b": 1,
    "kb": 1000,
    "kib": 1024,
    "mb": 1000**2,
    "mib": 1024**2,
    "gb": 1000**3,
    "gib": 1024**3,
    "tb": 1000**4,
    "tib": 1024**4,
}


def parse_docker_memory_usage(text: str) -> int | None:
    """Parse the used-memory part of Docker's ``MemUsage`` output."""
    match = _DOCKER_MEM_RE.search(text)
    if not match:
        return None
    return int(float(match.group(1)) * _MEMORY_UNITS[match.group(2).lower()])


def _poll_docker_peak_rss(container_name: str, stop: threading.Event, result: dict[str, int]) -> None:
    while not stop.is_set():
        try:
            completed = subprocess.run(
                ["docker", "stats", "--no-stream", "--format", "{{.MemUsage}}", container_name],
                capture_output=True, text=True, timeout=2, check=False,
            )
            if completed.returncode == 0:
                usage = parse_docker_memory_usage(completed.stdout)
                if usage is not None:
                    result["peak_rss_bytes"] = max(result.get("peak_rss_bytes", 0), usage)
        except (OSError, subprocess.TimeoutExpired):
            pass
        stop.wait(0.25)


def _cleanup_container(container_name: str, timeout_s: float = 10.0) -> None:
    """Best-effort bounded cleanup for a timed-out named container."""
    try:
        subprocess.run(["docker", "kill", container_name], capture_output=True,
                       text=True, timeout=timeout_s, check=False)
    except (OSError, subprocess.TimeoutExpired):
        pass
    try:
        subprocess.run(["docker", "rm", "-f", container_name], capture_output=True,
                       text=True, timeout=timeout_s, check=False)
    except (OSError, subprocess.TimeoutExpired):
        pass


def command_for(args: argparse.Namespace, target: dict, output: Path) -> list[str]:
    common = [
        "--stock", args.stock, "--renkin", args.renkin,
        "--target-id", target["target_id"], "--target-smiles", target["canonical_smiles"],
        "--sample-rank", str(target["sample_rank"]), "--output", str(output),
    ]
    if args.tool == "synplanner":
        return [sys.executable, str(Path(__file__).with_name("run_synplanner_target.py")),
                "--synplan", args.synplan, "--config", args.config,
                "--reaction-rules", args.reaction_rules, "--building-blocks", args.building_blocks,
                "--policy-network", args.policy_network, "--value-network", args.value_network,
                *common, "--timeout-s", str(args.timeout_s), "--grace-s", str(args.grace_s)]
    return [args.python, str(Path(__file__).with_name("run_syntheseus_target.py")),
            "--model-dir", args.model_dir, *common, "--timeout-s", str(args.timeout_s)]


def container_command(
    args: argparse.Namespace, command: list[str], row_path: Path, container_name: str | None = None
) -> list[str]:
    """Translate a host runner command into a bounded, network-isolated container."""
    repo = Path(args.repo_root).resolve()
    artifacts = Path(args.artifact_dir).resolve()

    def mapped(value: str) -> str:
        path = Path(value)
        try:
            return "/repo/" + str(path.resolve().relative_to(repo))
        except ValueError:
            try:
                return "/artifacts/" + str(path.resolve().relative_to(artifacts))
            except ValueError:
                return value

    translated = ["python" if index == 0 else mapped(value) for index, value in enumerate(command)]
    translated = ["/repo/scripts/" + Path(command[1]).name if index == 1 else value
                  for index, value in enumerate(translated)]
    docker_command = [
        "docker", "run", "--rm", "--network", "none", "--cpus", str(args.cpus),
        "--memory", args.memory, "--memory-swap", args.memory,
        "-v", f"{repo}:/repo:ro", "-v", f"{artifacts}:/artifacts:rw",
        # The benchmark images use `python` as their ENTRYPOINT; omit the
        # host interpreter token from the command passed after the image.
        args.container_image, *translated[1:],
    ]
    if container_name:
        docker_command[3:3] = ["--name", container_name]
    return docker_command


def failure(target: dict, tool: str, arm_id: str, status: str, reason: str) -> FourToolRecord:
    return FourToolRecord(
        target_id=target["target_id"], target_smiles=target["canonical_smiles"],
        sample_rank=target["sample_rank"], tool=tool, arm_id=arm_id,
        run_status=status, failure_reason=reason,
    )


def run(args: argparse.Namespace) -> int:
    targets = []
    with open(args.target_manifest, encoding="utf-8") as handle:
        import json
        targets = [json.loads(line) for line in handle if line.strip()][:args.sample_size]
    existing = {r.target_id for r in load_records(args.output)} if Path(args.output).exists() else set()
    artifact_dir = Path(args.artifact_dir)
    artifact_dir.mkdir(parents=True, exist_ok=True)
    output = Path(args.output)
    output.parent.mkdir(parents=True, exist_ok=True)
    with output.open("a", encoding="utf-8") as sink:
        for target in targets:
            if target["target_id"] in existing:
                continue
            row_path = artifact_dir / f"{target['sample_rank']:06d}.row.jsonl"
            started = time.perf_counter()
            container_name = f"renkin-{args.tool}-{os.getpid()}-{target['sample_rank']}"
            peak_result: dict[str, int] = {}
            stop_stats = threading.Event()
            stats_thread = None
            container_timed_out = False
            try:
                command = (container_command(args, command_for(args, target, row_path), row_path, container_name)
                           if args.container_image else command_for(args, target, row_path))
                if args.container_image:
                    process = subprocess.Popen(
                        command, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
                        env={**os.environ, **resource_environment(args.cpus)},
                    )
                    stats_thread = threading.Thread(
                        target=_poll_docker_peak_rss,
                        args=(container_name, stop_stats, peak_result), daemon=True,
                    )
                    stats_thread.start()
                    try:
                        stdout, stderr = process.communicate(
                            timeout=args.timeout_s + args.grace_s + args.runner_overhead_s
                        )
                    except subprocess.TimeoutExpired:
                        container_timed_out = True
                        process.kill()
                        stdout, stderr = process.communicate()
                        _cleanup_container(container_name)
                    completed = subprocess.CompletedProcess(command, process.returncode, stdout, stderr)
                else:
                    completed = subprocess.run(
                        command, check=False, capture_output=True, text=True,
                        timeout=args.timeout_s + args.grace_s + args.runner_overhead_s,
                        env={**os.environ, **resource_environment(args.cpus)},
                        preexec_fn=lambda: apply_resource_limits(
                            args.memory_bytes, int(args.timeout_s + args.grace_s)
                        ),
                    )
            except subprocess.TimeoutExpired as exc:
                record = failure(target, args.tool, args.arm_id, "timeout", str(exc))
            else:
                if container_timed_out:
                    record = failure(
                        target, args.tool, args.arm_id, "timeout",
                        f"container exceeded {args.timeout_s + args.grace_s + args.runner_overhead_s}s",
                    )
                elif row_path.exists():
                    lines = [line for line in row_path.read_text(encoding="utf-8").splitlines() if line.strip()]
                    if len(lines) != 1:
                        record = failure(target, args.tool, args.arm_id, "parse_error",
                                         f"runner emitted {len(lines)} rows")
                    else:
                        import json
                        record = FourToolRecord(**json.loads(lines[0]))
                        record.total_elapsed_ms = (time.perf_counter() - started) * 1000
                else:
                    reason = completed.stderr.strip() or completed.stdout.strip() or "runner emitted no row"
                    record = failure(target, args.tool, args.arm_id, "crashed", reason[-2000:])
            finally:
                stop_stats.set()
                if stats_thread:
                    stats_thread.join(timeout=2)

            if peak_result.get("peak_rss_bytes") is not None:
                record.peak_rss_bytes = peak_result["peak_rss_bytes"]
                record.rss_measurement_method = "docker_stats_sampled"
            
            sink.write(record.to_json_line() + "\n")
            sink.flush()
            existing.add(record.target_id)
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tool", choices=["syntheseus", "synplanner"], required=True)
    parser.add_argument("--target-manifest", required=True)
    parser.add_argument("--sample-size", type=int, required=True)
    parser.add_argument("--output", required=True)
    parser.add_argument("--artifact-dir", required=True)
    parser.add_argument("--stock", required=True)
    parser.add_argument("--renkin", required=True)
    parser.add_argument("--arm-id", required=True)
    parser.add_argument("--timeout-s", type=float, default=150)
    parser.add_argument("--grace-s", type=float, default=10)
    parser.add_argument("--runner-overhead-s", type=float, default=5)
    parser.add_argument("--cpus", type=int, default=8)
    parser.add_argument("--memory-bytes", type=int, default=6 * 1024**3)
    parser.add_argument("--memory", default="6g")
    parser.add_argument("--repo-root", default=".")
    parser.add_argument("--container-image")
    parser.add_argument("--python", default=sys.executable)
    parser.add_argument("--model-dir", default="")
    parser.add_argument("--synplan", default="")
    parser.add_argument("--config", default="")
    parser.add_argument("--reaction-rules", default="", dest="reaction_rules")
    parser.add_argument("--building-blocks", default="", dest="building_blocks")
    parser.add_argument("--policy-network", default="", dest="policy_network")
    parser.add_argument("--value-network", default="", dest="value_network")
    args = parser.parse_args()
    return run(args)


if __name__ == "__main__":
    raise SystemExit(main())
