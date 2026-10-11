import type { Metadata } from "next";
import { Titlebar } from "@/components/shell/titlebar";
import "./globals.css";

export const metadata: Metadata = {
  title: "Blink",
  description: "Your own WireGuard VPN server, on when you need it and gone when you don't.",
};

export default function RootLayout({
  children,
}: Readonly<{
  children: React.ReactNode;
}>) {
  return (
    <html lang="en" className="h-full antialiased">
      <body className="flex h-full flex-col bg-ink text-bone">
        <Titlebar />
        {/* Every route fills this and scrolls inside it; the titlebar never scrolls away. */}
        <div className="relative flex min-h-0 flex-1 flex-col">{children}</div>
      </body>
    </html>
  );
}
