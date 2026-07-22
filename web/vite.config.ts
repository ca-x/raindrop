/// <reference types="vitest/config" />

import react from "@vitejs/plugin-react"
import { readFileSync } from "node:fs"
import { defineConfig } from "vite"

const packageVersion = (JSON.parse(
  readFileSync(new URL("./package.json", import.meta.url), "utf8"),
) as { version: string }).version

export default defineConfig({
  define: {
    __RAINDROP_VERSION__: JSON.stringify(packageVersion),
  },
  plugins: [react()],
  server: {
    proxy: {
      "/api": {
        target: "http://127.0.0.1:8080",
        changeOrigin: false,
      },
      "/reader-assets": {
        target: "http://127.0.0.1:8080",
        changeOrigin: false,
      },
    },
  },
  test: {
    environment: "jsdom",
    include: ["src/**/*.test.{ts,tsx}"],
    setupFiles: ["./src/test/setup.ts"],
    css: true,
    restoreMocks: true,
  },
})
