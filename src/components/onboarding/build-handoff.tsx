"use client";

import { useRouter, useSearchParams } from "next/navigation";
import { Suspense, useEffect } from "react";

import { useDashboard } from "@/components/dashboard/dashboard-provider";
import { VIEW_ROUTE } from "@/components/dashboard/shared";

type BuildRequest = { providerId: string; region: string; planKey: string; place: string };

/** Builds already started from a handoff URL, so a remount — React's dev double-effect — cannot build twice. */
const consumed = new Set<string>();

/**
 * The URL onboarding sends the user to: Home, carrying the build it should start.
 *
 * Onboarding runs outside the dashboard, and the dashboard owns the build screen and the
 * auto-connect. Handing the request across in the URL keeps one build path instead of two.
 */
export function buildHandoffUrl(request: BuildRequest): string {
  return `${VIEW_ROUTE.home}?build=${encodeURIComponent(JSON.stringify(request))}`;
}

function parse(raw: string | null): BuildRequest | null {
  if (!raw) return null;
  try {
    const value = JSON.parse(raw) as Partial<BuildRequest>;
    if (typeof value.providerId === "string" && typeof value.region === "string" && typeof value.planKey === "string" && typeof value.place === "string") {
      return value as BuildRequest;
    }
  } catch {
    // A malformed request is dropped, not guessed at.
  }
  return null;
}

function Handoff() {
  const router = useRouter();
  const params = useSearchParams();
  const { moveTo } = useDashboard();
  const raw = params.get("build");

  useEffect(() => {
    const request = parse(raw);
    if (!raw || !request || consumed.has(raw)) return;
    consumed.add(raw);
    // Drop the parameter first, so a reload or Back cannot start the build again.
    router.replace(VIEW_ROUTE.home);
    void moveTo({ ...request, nodeName: request.place, keepOld: false });
  }, [moveTo, raw, router]);

  return null;
}

/** Mounted once in the dashboard layout. */
export function BuildHandoff() {
  return (
    <Suspense fallback={null}>
      <Handoff />
    </Suspense>
  );
}
