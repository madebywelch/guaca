import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { createServer } from "node:http";
import { test } from "node:test";
import { promisify } from "node:util";

const execute = promisify(execFile);
const failure = { id: 9, name: "build", status: "completed", conclusion: "failure" };
const communication = {
  annotation_level: "failure",
  path: ".github",
  title: "",
  message: "The self-hosted runner lost communication with the server. Verify the machine is running.",
};
const shutdown = { ...communication, message: "The runner has received a shutdown signal." };
const assertion = { ...communication, message: "Process completed with exit code 1." };
const aggregate = {
  ...failure,
  id: 11,
  name: "gate",
  steps: [{ name: "Every job passed", conclusion: "failure" }],
};

async function replay(t, { attempt = 1, annotations = { 9: [communication] }, jobs = [failure], stale = false } = {}) {
  let reads = 0;
  const posts = [];
  const server = createServer((req, res) => {
    if (req.method === "POST") {
      posts.push(req.url);
      res.writeHead(201).end("{}");
      return;
    }
    const url = new URL(req.url, "http://127.0.0.1");
    const match = url.pathname.match(/check-runs\/(\d+)\/annotations$/);
    const result = match
      ? annotations[match[1]] ?? []
      : url.pathname.endsWith("/jobs")
        ? { jobs }
        : {
            path: ".github/workflows/ci.yml",
            status: "completed",
            conclusion: "failure",
            run_attempt: stale && reads++ > 0 ? attempt + 1 : attempt,
          };
    res.end(JSON.stringify(result));
  });
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  t.after(() => server.close());
  const { stdout } = await execute(process.execPath, ["scripts/recover-lost-runner-jobs.mjs"], {
    env: {
      GITHUB_API_URL: `http://127.0.0.1:${server.address().port}`,
      GITHUB_REPOSITORY: "owner/repo",
      GITHUB_TOKEN: "test",
      TARGET_RUN_ID: "42",
      TARGET_RUN_ATTEMPT: String(attempt),
    },
  });
  return { posts, stdout };
}

for (const [attempt, annotation] of [[1, communication], [2, shutdown]]) {
  test(`requests the next attempt after runner loss on attempt ${attempt}`, async (t) => {
    const { posts, stdout } = await replay(t, { attempt, annotations: { 9: [annotation] } });
    assert.match(stdout, /Requested rerun of failed jobs/);
    assert.deepEqual(posts, ["/repos/owner/repo/actions/runs/42/rerun-failed-jobs"]);
  });
}

test("does not request a fourth attempt", async (t) => {
  assert.deepEqual((await replay(t, { attempt: 3 })).posts, []);
});

test("does not rerun a real test failure", async (t) => {
  assert.deepEqual((await replay(t, { annotations: { 9: [assertion] } })).posts, []);
  const jobs = [{ ...failure, steps: [{ name: "Run tests", conclusion: "failure" }] }];
  assert.deepEqual((await replay(t, { jobs })).posts, []);
});

test("does not rerun when a test failed alongside runner loss", async (t) => {
  const jobs = [failure, { ...failure, id: 10, name: "test" }];
  assert.deepEqual((await replay(t, { jobs, annotations: { 9: [communication], 10: [assertion] } })).posts, []);
  assert.deepEqual((await replay(t, { annotations: { 9: [communication, assertion] } })).posts, []);
});

test("does not rerun a newer attempt on a stale event", async (t) => {
  assert.deepEqual((await replay(t, { stale: true })).posts, []);
});

test("reruns the aggregate gate only with its lost dependency", async (t) => {
  const { posts } = await replay(t, { jobs: [failure, aggregate] });
  assert.equal(posts.length, 1);
  assert.deepEqual((await replay(t, { jobs: [aggregate] })).posts, []);
});
