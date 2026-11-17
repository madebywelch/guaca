import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

root = Path(__file__).resolve().parent.parent
gate = (root / "scripts" / "ci.sh").read_text()
workflow = (root / ".github" / "workflows" / "ci.yml").read_text()


def check(script: str, jobs: str) -> subprocess.CompletedProcess:
    with tempfile.TemporaryDirectory() as temp:
        (Path(temp) / "ci.sh").write_text(script)
        (Path(temp) / "ci.yml").write_text(jobs)
        return subprocess.run(
            [sys.executable, root / "scripts" / "check-ci-parts.py", Path(temp) / "ci.sh", Path(temp) / "ci.yml"],
            capture_output=True,
            text=True,
        )


class CiParts(unittest.TestCase):
    def test_the_workflow_runs_every_part(self):
        result = check(gate, workflow)
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_a_job_removed_from_the_workflow_fails(self):
        result = check(gate, workflow.replace("./scripts/ci.sh daemon", "true"))
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("./scripts/ci.sh daemon", result.stderr)

    def test_a_part_added_to_the_gate_alone_fails(self):
        script = gate.replace("    run_connector\n", "    run_connector\n    run_probe\n").replace(
            "  connector) run_connector ;;\n", "  connector) run_connector ;;\n  probe) run_probe ;;\n"
        )
        self.assertIn("run_probe", script)
        result = check(script, workflow)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("./scripts/ci.sh probe", result.stderr)

    def test_running_rust_does_not_stand_in_for_desktop(self):
        result = check(gate, workflow.replace("./scripts/ci.sh desktop", "./scripts/ci.sh rust"))
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("desktop", result.stderr)


if __name__ == "__main__":
    unittest.main()
