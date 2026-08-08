import { chromium, expect, test as base, type Browser, type Page } from "@playwright/test";
import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";

const CDP_ENDPOINT = "http://127.0.0.1:9222";
const START_TIMEOUT_MS = 180_000;

type TestMode = "cdp";

type TauriFixtures = {
  mode: TestMode;
  tauriPage: Page;
};

async function sleep(ms: number): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, ms));
}

async function connectWithRetry(endpoint: string, timeoutMs: number): Promise<Browser> {
  const deadline = Date.now() + timeoutMs;
  let lastError: unknown = null;

  while (Date.now() < deadline) {
    try {
      return await chromium.connectOverCDP(endpoint);
    } catch (error) {
      lastError = error;
      await sleep(500);
    }
  }

  throw new Error(`Failed to connect to CDP endpoint ${endpoint} within ${timeoutMs}ms. Last error: ${String(lastError)}`);
}

function startTauriProcess(cwd: string): ChildProcessWithoutNullStreams {
  const command = process.platform === "win32" ? "cmd.exe" : "npm";
  const args = process.platform === "win32"
    ? ["/d", "/s", "/c", "npm run tauri:dev:e2e"]
    : ["run", "tauri:dev:e2e"];

  const child = spawn(command, args, {
    cwd,
    env: {
      ...process.env,
      WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: "--remote-debugging-port=9222",
    },
    stdio: "pipe",
  });

  child.stdout.on("data", () => {
    // stdout is intentionally consumed to avoid child process backpressure.
  });
  child.stderr.on("data", () => {
    // stderr is intentionally consumed to avoid child process backpressure.
  });

  return child;
}

async function stopTauriProcess(child: ChildProcessWithoutNullStreams | null): Promise<void> {
  if (!child || child.pid === undefined) return;

  await new Promise<void>((resolve) => {
    const killer = spawn("taskkill", ["/PID", String(child.pid), "/T", "/F"], {
      stdio: "ignore",
    });

    killer.on("exit", () => resolve());
    killer.on("error", () => resolve());
  });
}

export const test = base.extend<TauriFixtures>({
  mode: ["cdp", { option: true }],
  tauriPage: async ({ mode }, use) => {
    if (mode !== "cdp") {
      throw new Error(`Unsupported test mode: ${mode}`);
    }

    const tauriProcess = startTauriProcess(process.cwd());
    let browser: Browser | null = null;

    try {
      browser = await connectWithRetry(CDP_ENDPOINT, START_TIMEOUT_MS);
      const context = browser.contexts()[0] ?? await browser.newContext();
      const page = context.pages()[0] ?? await context.newPage();
      await page.waitForLoadState("domcontentloaded");

      await use(page);
    } finally {
      if (browser) {
        await browser.close().catch(() => undefined);
      }
      await stopTauriProcess(tauriProcess);
    }
  },
});

export { expect };
