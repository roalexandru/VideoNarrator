import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import { resolve } from "path";
import { readFileSync } from "fs";
import { execSync } from "child_process";

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;
const pkg = JSON.parse(readFileSync(resolve(__dirname, "package.json"), "utf-8"));

function gitSha(): string {
  try {
    const sha = execSync("git rev-parse --short HEAD", { cwd: __dirname, stdio: ["ignore", "pipe", "ignore"] }).toString().trim();
    const dirty = execSync("git status --porcelain", { cwd: __dirname, stdio: ["ignore", "pipe", "ignore"] }).toString().trim().length > 0;
    return dirty ? `${sha}-dirty` : sha;
  } catch {
    return "unknown";
  }
}

// https://vite.dev/config/
export default defineConfig(async () => ({
  plugins: [react(), tailwindcss()],

  define: {
    __APP_VERSION__: JSON.stringify(pkg.version),
    __APP_BUILD_TIME__: JSON.stringify(new Date().toISOString()),
    __APP_GIT_SHA__: JSON.stringify(gitSha()),
  },

  build: {
    rollupOptions: {
      input: {
        main: resolve(__dirname, "index.html"),
        "recorder-overlay": resolve(__dirname, "recorder-overlay.html"),
      },
    },
  },

  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
}));
