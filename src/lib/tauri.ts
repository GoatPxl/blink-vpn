import { invoke } from "@tauri-apps/api/core";

type TauriWindow = Window & {
  __TAURI_INTERNALS__?: unknown;
};

export async function invokeHello(name: string): Promise<string> {
  if (typeof window === "undefined") {
    return "UI is rendering on the server.";
  }

  const tauriWindow = window as TauriWindow;
  if (!tauriWindow.__TAURI_INTERNALS__) {
    return "Running in browser mode. Start with `npm run tauri:dev` to reach Rust.";
  }

  return invoke<string>("hello", { name });
}