"use client";

import { ActivitySection } from "./activity-section";
import { HealthSection } from "./health-section";

/** Health and Activity, stacked until the tabbed Shield view replaces both. */
export default function ShieldPage() {
  return (
    <div className="flex flex-col gap-5">
      <HealthSection />
      <ActivitySection />
    </div>
  );
}
