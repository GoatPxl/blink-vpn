"use client";

import { useRouter } from "next/navigation";
import { useEffect } from "react";

/**
 * Sends an old route to its new home.
 *
 * The app is a static export, so there is no server to answer with a redirect; the old pages stay
 * as stubs that replace themselves in history, so Back does not bounce the user into a loop.
 */
export function RouteRedirect({ to }: { to: string }) {
  const router = useRouter();

  useEffect(() => {
    router.replace(to);
  }, [router, to]);

  return null;
}
