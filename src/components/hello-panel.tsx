"use client";

import { useState } from "react";
import { invokeHello } from "@/lib/tauri";

export function HelloPanel() {
  const [name, setName] = useState("operator");
  const [message, setMessage] = useState("No response yet.");
  const [loading, setLoading] = useState(false);

  const onSend = async () => {
    setLoading(true);
    try {
      const result = await invokeHello(name);
      setMessage(result);
    } catch (error) {
      const errorMessage = error instanceof Error ? error.message : String(error);
      setMessage(`Command failed: ${errorMessage}`);
    } finally {
      setLoading(false);
    }
  };

  return (
    <div className="space-y-4">
      <label className="block text-sm font-medium text-slate-700" htmlFor="hello-name">
        Name
      </label>
      <input
        id="hello-name"
        value={name}
        onChange={(event) => setName(event.target.value)}
        className="w-full rounded-lg border border-slate-300 px-3 py-2 text-sm outline-none ring-slate-300 focus:ring"
        placeholder="operator"
      />
      <button
        type="button"
        onClick={onSend}
        disabled={loading}
        className="rounded-lg bg-slate-900 px-4 py-2 text-sm font-medium text-white hover:bg-slate-800 disabled:cursor-not-allowed disabled:bg-slate-500"
      >
        {loading ? "Calling Rust..." : "Send hello command"}
      </button>
      <pre className="rounded-lg bg-slate-100 p-3 text-xs text-slate-700">{message}</pre>
    </div>
  );
}