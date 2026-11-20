export const LOST_RUNNER_MESSAGE =
  "The self-hosted runner lost communication with the server";

const shutdownMessage = "The runner has received a shutdown signal";

function isLost(job) {
  if (job.conclusion !== "failure") return false;
  if (job.steps?.some((step) => step.conclusion === "failure")) return false;
  const errors = job.annotations?.filter((annotation) => annotation.annotation_level === "failure");
  return errors?.length > 0 && errors.every(({ path, title, message }) =>
    path === ".github" && !title &&
    [LOST_RUNNER_MESSAGE, shutdownMessage].some((prefix) =>
      message === prefix || message?.startsWith(`${prefix}.`),
    ),
  );
}

export function recoveryDecision({ jobs, workflowPath }) {
  if (!Array.isArray(jobs) || jobs.some((job) => job.status !== "completed")) {
    return { rerun: false, reason: "job list is incomplete" };
  }
  const failed = jobs.filter(
    (job) => !["success", "skipped", "neutral"].includes(job.conclusion),
  );
  const lost = failed.filter(isLost);
  if (lost.length === 0) return { rerun: false, reason: "no lost runner jobs" };
  const other = failed.filter((job) => {
    if (lost.includes(job)) return false;
    return !(
      workflowPath === ".github/workflows/ci.yml" &&
      job.name === "gate" &&
      job.conclusion === "failure" &&
      job.steps?.filter((step) => step.conclusion === "failure").length === 1 &&
      job.steps.some((step) => step.name === "Every job passed" && step.conclusion === "failure")
    );
  });
  if (other.length > 0) {
    return {
      rerun: false,
      reason: `non-runner failure: ${other.map((job) => job.name).join(", ")}`,
    };
  }
  return {
    rerun: true,
    reason: `${lost.length} lost runner job(s)`,
    lost: lost.map((job) => job.id),
  };
}
