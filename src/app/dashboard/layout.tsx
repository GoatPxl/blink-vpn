import type { ReactNode } from "react";
import { DashboardProvider } from "@/components/dashboard/dashboard-provider";
import { DashboardShell } from "@/components/dashboard/dashboard-shell";
import { BuildHandoff } from "@/components/onboarding/build-handoff";

export default function DashboardLayout({ children }: { children: ReactNode }) {
  return (
    <DashboardProvider>
      <DashboardShell>{children}</DashboardShell>
      <BuildHandoff />
    </DashboardProvider>
  );
}