import { HelloPanel } from "@/components/hello-panel";

export default function Home() {
  return (
    <main className="flex min-h-screen items-center justify-center bg-slate-100 p-6">
      <section className="w-full max-w-xl rounded-2xl border border-slate-200 bg-white p-6 shadow-sm">
        <p className="text-xs font-semibold uppercase tracking-[0.18em] text-slate-500">
          Phase 1 Scaffold
        </p>
        <h1 className="mt-2 text-2xl font-semibold text-slate-900">
          GoatStare VPN Manager
        </h1>
        <p className="mt-2 text-sm text-slate-600">
          Windows + DigitalOcean + WireGuard using Tauri, Next.js, and Rust.
        </p>
        <div className="mt-6">
          <HelloPanel />
        </div>
      </section>
    </main>
  );
}
