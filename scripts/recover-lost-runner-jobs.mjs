#!/usr/bin/env node
import { execFileSync } from "node:child_process";
import { appendFileSync } from "node:fs";
import { recoveryDecision } from "./lib/lost-runner-jobs.mjs";

const args = process.argv.slice(2);
const arg = (name) => args[args.indexOf(name) + 1];
const dryRun = args.includes("--dry-run");
const runId = args.includes("--run-id")
  ? arg("--run-id")
  : process.env.TARGET_RUN_ID;
const attempt = Number(
  args.includes("--attempt")
    ? arg("--attempt")
    : process.env.TARGET_RUN_ATTEMPT,
);
const repository = process.env.GITHUB_REPOSITORY;
const token = process.env.GITHUB_TOKEN;
const useGh = dryRun && process.env.RECOVERY_USE_GH === "1";

async function request(path, method = "GET") {
  if (useGh)
    return JSON.parse(
      execFileSync("gh", ["api", `repos/${repository}${path}`], {
        encoding: "utf8",
      }),
    );
  const response = await fetch(
    `${process.env.GITHUB_API_URL || "https://api.github.com"}/repos/${repository}${path}`,
    {
      method,
      headers: {
        Authorization: `Bearer ${token}`,
        Accept: "application/vnd.github+json",
        "X-GitHub-Api-Version": "2022-11-28",
        "User-Agent": "lost-runner-recovery",
      },
      signal: AbortSignal.timeout(30_000),
    },
  );
  if (!response.ok)
    throw new Error(`${method} ${path} returned HTTP ${response.status}`);
  return response.status === 204 ? null : response.json();
}

async function paginate(path, field) {
  const results = [];
  for (let page = 1; page <= 100; page += 1) {
    const response = await request(
      `${path}${path.includes("?") ? "&" : "?"}per_page=100&page=${page}`,
    );
    const batch = field ? response[field] : response;
    if (!Array.isArray(batch))
      throw new Error(`Invalid paginated response: ${path}`);
    results.push(...batch);
    if (batch.length < 100) return results;
  }
  throw new Error(`Pagination limit exceeded: ${path}`);
}

async function main() {
  if (
    !/^\d+$/.test(runId || "") ||
    !Number.isInteger(attempt) ||
    attempt < 1 ||
    !repository ||
    (!token && !useGh)
  ) {
    throw new Error(
      "Run id, attempt, repository and GitHub token are required",
    );
  }
  const run = await request(
    `/actions/runs/${runId}${dryRun ? `/attempts/${attempt}` : ""}`,
  );
  if (
    run.status !== "completed" ||
    run.conclusion !== "failure" ||
    Number(run.run_attempt) !== attempt
  ) {
    console.log(
      `run ${runId} attempt ${attempt}: no rerun (run is not the matching completed failure)`,
    );
    return;
  }
  if (attempt > 2) {
    console.log(
      `run ${runId} attempt ${attempt}: no rerun (attempt is outside the automatic recovery limit)`,
    );
    return;
  }
  const jobs = await paginate(
    `/actions/runs/${runId}/attempts/${attempt}/jobs`,
    "jobs",
  );
  for (const job of jobs) {
    if (job.conclusion !== "failure") continue;
    job.annotations = await paginate(`/check-runs/${job.id}/annotations`);
  }
  const decision = recoveryDecision({ jobs, workflowPath: run.path?.split("@")[0] });
  console.log(
    `run ${runId} attempt ${attempt}: ${decision.rerun ? "rerun failed jobs" : "no rerun"} (${decision.reason})${dryRun ? " [dry run]" : ""}`,
  );
  if (!dryRun && decision.rerun) {
    const latest = await request(`/actions/runs/${runId}`);
    if (
      latest.status !== "completed" ||
      latest.conclusion !== "failure" ||
      Number(latest.run_attempt) !== attempt
    ) {
      console.log("Run changed before recovery; no rerun requested");
      return;
    }
    await request(`/actions/runs/${runId}/rerun-failed-jobs`, "POST");
    console.log("Requested rerun of failed jobs");
    if (process.env.GITHUB_STEP_SUMMARY) {
      const runUrl = `${process.env.GITHUB_SERVER_URL}/${repository}/actions/runs/${runId}`;
      appendFileSync(process.env.GITHUB_STEP_SUMMARY,
        `Requested attempt ${attempt + 1}: [attempt ${attempt}](${runUrl}/attempts/${attempt}) had only runner loss (${decision.lost.map((id) => `[job ${id}](${runUrl}/job/${id})`).join(", ")}).\n`,
      );
    }
  }
}

main().catch((error) => {
  console.error(error.message);
  process.exitCode = 1;
});
