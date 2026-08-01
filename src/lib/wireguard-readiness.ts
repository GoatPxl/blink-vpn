import { checkWireGuardReadiness, type WireGuardReadiness } from "@/lib/tauri";

export async function runWireGuardReadinessCheck(): Promise<WireGuardReadiness> {
  return checkWireGuardReadiness();
}
