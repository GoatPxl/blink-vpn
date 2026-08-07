import { createTauriTest } from "../../../node_modules/@srsholmes/tauri-playwright/dist/index.js";

export const { test, expect } = createTauriTest({
  devUrl: "http://localhost:3000",
  tauriCommand: "cmd /c scripts\\tauri-dev-e2e-wrapper.cmd",
  tauriCwd: process.cwd(),
  startTimeout: 180,
});
