/// <reference types="vitest/config" />
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// The Tauri shell loads http://localhost:5173 in development and ../app/dist in release builds.
export default defineConfig({
  plugins: [react()],
  base: "./",
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true
  },
  build: {
    outDir: "dist",
    target: "es2022"
  },
  test: {
    include: ["src/**/*.test.ts"],
    environment: "node"
  }
});
