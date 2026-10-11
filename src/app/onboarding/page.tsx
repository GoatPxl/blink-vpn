import { OnboardingWizard } from "@/components/onboarding/onboarding-wizard";

export default function OnboardingPage() {
  return (
    <div className="scroll-area min-h-0 flex-1 overflow-y-auto">
      <OnboardingWizard />
    </div>
  );
}
