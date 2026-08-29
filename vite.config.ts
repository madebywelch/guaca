import { execFileSync } from "node:child_process";

import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

// Tauri drives the dev server on a fixed port and expects a hard failure
// rather than a silent port bump, otherwise the webview loads nothing.
const host = process.env.TAURI_DEV_HOST;

/**
 * The commit this bundle is being built from.
 *
 * The number in `package.json` has not moved since the first commit and is not
 * going to: what ships here is a commit rather than a release, so a version
 * read off that file tells an operator nothing and tells a bug report less.
 * Read here because the built app has no repository to ask, and at build time
 * because that is when the answer stops changing: a dev server keeps the commit
 * it started on, which is the commit the bundle behind it was built from.
 */
function builtOn(): string {
  const git = (...args: string[]) =>
    execFileSync("git", args, { encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] }).trim();
  try {
    const head = git("rev-parse", "--short=7", "HEAD");
    // A tree with edits on top of that commit did not produce this build, and
    // an unqualified hash says it did. The difference is somebody checking out
    // that commit and hunting for a defect that was never in it.
    return git("status", "--porcelain") ? `${head}-dirty` : head;
  } catch {
    // No git, no repository, or a source tarball. About draws a dash.
    return "";
  }
}

export default defineConfig({
  plugins: [react(), tailwindcss()],
  clearScreen: false,
  define: { __COMMIT__: JSON.stringify(builtOn()) },
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: "ws", host, port: 1421 } : undefined,
    watch: { ignored: ["**/src-tauri/**"] },
  },
  envPrefix: ["VITE_", "TAURI_ENV_*"],
  build: {
    target: "safari15",
    sourcemap: !!process.env.TAURI_ENV_DEBUG,
    minify: process.env.TAURI_ENV_DEBUG ? false : "esbuild",
  },
  test: {
    environment: "jsdom",
    globals: true,
    setupFiles: ["./src/test-setup.ts"],
    include: ["src/**/*.test.{ts,tsx}"],
  },
});
