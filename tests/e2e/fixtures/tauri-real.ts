import { createTauriTest } from "../../../node_modules/@srsholmes/tauri-playwright/dist/index.js";

export const { test, expect } = createTauriTest({
  devUrl: "http://localhost:3000",
  tauriCommand:
    "cmd /c \"set GOATSTARE_DEV_DOCKER=1&& set NEXT_PUBLIC_DEV_DOCKER=1&& npm run tauri:dev:e2e\"",
  tauriCwd: process.cwd(),
  startTimeout: 180,
});
