import type { Metadata } from "next";
import { headers } from "next/headers";
import "./globals.css";

export async function generateMetadata(): Promise<Metadata> {
  const requestHeaders = await headers();
  const host =
    requestHeaders.get("x-forwarded-host") ??
    requestHeaders.get("host") ??
    "localhost:3000";
  const protocol =
    requestHeaders.get("x-forwarded-proto") ??
    (host.startsWith("localhost") ? "http" : "https");
  const metadataBase = new URL(`${protocol}://${host}`);
  const socialImage = new URL("/og.png", metadataBase).toString();

  return {
    metadataBase,
    title: "Spick — Voice typing for macOS",
    description:
      "Speak into any text field on your Mac. Spick transcribes locally, cleans up the pauses, and puts the words back at your cursor.",
    icons: { icon: "/spick-mark.png", shortcut: "/spick-mark.png" },
    openGraph: {
      title: "Keep your hands on the work.",
      description: "Spick — voice typing for macOS",
      type: "website",
      images: [
        {
          url: socialImage,
          width: 1200,
          height: 630,
          alt: "Spick voice typing for macOS",
        },
      ],
    },
    twitter: {
      card: "summary_large_image",
      title: "Keep your hands on the work.",
      description: "Spick — voice typing for macOS",
      images: [socialImage],
    },
  };
}

export default function RootLayout({
  children,
}: Readonly<{ children: React.ReactNode }>) {
  return (
    <html lang="en">
      <body>{children}</body>
    </html>
  );
}
