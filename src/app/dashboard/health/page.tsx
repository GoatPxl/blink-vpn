"use client";

import { HealthView } from "@/components/dashboard/health-view";
import { useDashboard } from "@/components/dashboard/dashboard-provider";
import type { LogEntry } from "@/components/dashboard/shared";

export default function DashboardHealthPage() {
  const { busyAction, diagnostics, health, recovery, refreshHealth, selectedNode, selectedTunnelId } = useDashboard();

  const diagnosticsLogs: LogEntry[] = diagnostics
    ? [
        { time: diagnostics.generated_at.slice(11, 19), level: "INFO", source: "NetGuard VPN Engine", message: "Diagnostics bundle generated", tone: "success" },
        { time: "14:02:11", level: "INFO", source: "Backend", message: `Nodes captured: ${diagnostics.nodes.length}`, tone: "info" },
        { time: "14:02:12", level: "WG", source: "WireGuard", message: `Tunnels captured: ${diagnostics.tunnels.length}`, tone: "neutral" },
        { time: "14:02:13", level: "WARN", source: "Recovery", message: recovery ? recovery.nextAction : "No recovery issue active", tone: recovery ? "warning" : "neutral" },
      ]
    : [
        { time: "14:02:11", level: "INFO", source: "NetGuard VPN Engine", message: "Initializing diagnostics view...", tone: "info" },
        { time: "14:02:12", level: "INFO", source: "Backend", message: "Awaiting bundle data...", tone: "neutral" },
      ];

  return (
    <HealthView
      diagnosticsLogs={diagnosticsLogs}
      health={health}
      selectedNode={selectedNode}
      selectedTunnelId={selectedTunnelId}
      busyAction={busyAction}
      onRefreshHealth={refreshHealth}
    />
  );
}
