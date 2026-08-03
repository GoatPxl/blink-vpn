import { defineConfig, devices } from "@playwright/test";

const port = Number(process.env.E2E_PORT ?? 4173);
const baseURL = `http://127.0.0.1:${port}`;

export default defineConfig({
  testDir: "./tests/e2e",
  fullyParallel: false,
  workers: 1,
  retries: 0,
  reporter: [["list"], ["html", { open: "never" }]],
  use: {
    baseURL,
    trace: "on-first-retry",
  },
  webServer: {
    command: `npm run dev -- --hostname 127.0.0.1 --port ${port}`,
    url: `${baseURL}/onboarding`,
    reuseExistingServer: true,
    timeout: 120000,
  },
  projects: [
    {
      name: "frontend",
      testIgnore: /dashboard-docker-real-tauri\.spec\.ts/,
      use: { ...devices["Desktop Chrome"] },
    },
    {
      name: "tauri-real",
      testMatch: /dashboard-docker-real-tauri\.spec\.ts/,
    },
  ],
});
