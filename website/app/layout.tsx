import type { Metadata, Viewport } from "next";
import { IBM_Plex_Mono } from "next/font/google";
import "./globals.css";

const ibmPlexMono = IBM_Plex_Mono({
  subsets: ["latin"],
  weight: ["400", "500", "600"],
  variable: "--font-ibm-plex-mono",
  display: "swap",
});

const basePath = process.env.NEXT_PUBLIC_BASE_PATH ?? "";

const metadataBase = new URL(
  process.env.NEXT_PUBLIC_SITE_URL ??
    (process.env.VERCEL_PROJECT_PRODUCTION_URL
      ? `https://${process.env.VERCEL_PROJECT_PRODUCTION_URL}`
      : process.env.VERCEL_URL
        ? `https://${process.env.VERCEL_URL}`
        : "http://localhost:3000")
);

export const metadata: Metadata = {
  metadataBase,
  icons: {
    icon: {
      url: `${basePath}/icon`,
      type: "image/png",
      sizes: "32x32",
    },
  },
  title: {
    default: "shuck - A fast shell script linter and formatter",
    template: "%s | shuck",
  },
  description:
    "A fast shell script linter and formatter, built in Rust. Checks for correctness, portability, and style issues in shell scripts.",
  openGraph: {
    title: "shuck - A fast shell script linter and formatter",
    description:
      "A fast shell script linter and formatter, built in Rust. Checks for correctness, portability, and style issues in shell scripts.",
    type: "website",
  },
  twitter: {
    card: "summary_large_image",
    title: "shuck",
    description:
      "A fast shell script linter and formatter, built in Rust.",
  },
};

export const viewport: Viewport = {
  width: "device-width",
  initialScale: 1,
  viewportFit: "cover",
};

export default function RootLayout({
  children,
}: Readonly<{
  children: React.ReactNode;
}>) {
  return (
    <html lang="en" className="dark">
      <body className={`${ibmPlexMono.variable} antialiased`}>
        {children}
      </body>
    </html>
  );
}
