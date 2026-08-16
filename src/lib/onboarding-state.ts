export type OnboardingStatus = "not_started" | "in_progress" | "completed";

export interface OnboardingState {
  status: OnboardingStatus;
  currentStep: number;
  providerId: string;
  providerName: string;
  region: string;
  plan: string;
  nodeId: string;
  nodeName: string;
  tokenSaved: boolean;
  tokenValidated: boolean;
  nodeReady: boolean;
  lastNodePublicIp: string | null;
  lastWireGuardPassed: boolean;
}

const ONBOARDING_STORAGE_KEY = "goatstare.onboarding.v1";

export const DEFAULT_ONBOARDING_STATE: OnboardingState = {
  status: "not_started",
  currentStep: 1,
  providerId: "provider-1",
  providerName: "DigitalOcean",
  region: "",
  plan: "",
  nodeId: "node-1",
  nodeName: "Primary Node",
  tokenSaved: false,
  tokenValidated: false,
  nodeReady: false,
  lastNodePublicIp: null,
  lastWireGuardPassed: false,
};

export function loadOnboardingState(): OnboardingState {
  if (typeof window === "undefined") {
    return DEFAULT_ONBOARDING_STATE;
  }

  try {
    const raw = window.localStorage.getItem(ONBOARDING_STORAGE_KEY);
    if (!raw) return DEFAULT_ONBOARDING_STATE;

    const parsed = JSON.parse(raw) as Partial<OnboardingState>;
    return {
      ...DEFAULT_ONBOARDING_STATE,
      ...parsed,
    };
  } catch {
    return DEFAULT_ONBOARDING_STATE;
  }
}

export function saveOnboardingState(state: OnboardingState): void {
  if (typeof window === "undefined") return;
  window.localStorage.setItem(ONBOARDING_STORAGE_KEY, JSON.stringify(state));
}

