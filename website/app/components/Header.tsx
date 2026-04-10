"use client";

import Link from "next/link";
import { Github, ExternalLink } from "lucide-react";

export default function Header() {
  return (
    <header className="sticky top-0 z-50 border-b border-fg-dim/30 bg-bg-primary/80 backdrop-blur-md">
      <div className="mx-auto flex h-14 max-w-6xl items-center justify-between px-4 sm:px-6">
        <Link href="/" className="font-heading text-lg font-bold tracking-tight text-fg-primary hover:text-accent transition-colors">
          shuck
        </Link>

        <nav className="flex items-center gap-1">
          <Link
            href="/docs/getting-started"
            className="rounded-md px-3 py-1.5 text-sm font-medium text-fg-secondary hover:text-fg-primary hover:bg-bg-card transition-colors"
          >
            Docs
          </Link>
          <a
            href="https://github.com/ewhauser/shuck"
            target="_blank"
            rel="noopener noreferrer"
            className="flex items-center gap-1.5 rounded-md px-3 py-1.5 text-sm font-medium text-fg-secondary hover:text-fg-primary hover:bg-bg-card transition-colors"
          >
            <Github className="h-4 w-4" />
            <span className="hidden sm:inline">GitHub</span>
          </a>
          <a
            href="https://github.com/ewhauser/shuck/releases"
            target="_blank"
            rel="noopener noreferrer"
            className="ml-2 flex items-center gap-1.5 rounded-md bg-accent/10 px-3.5 py-1.5 text-sm font-semibold text-accent border border-accent/20 hover:bg-accent/20 hover:border-accent/40 transition-all"
          >
            Download
            <ExternalLink className="h-3.5 w-3.5" />
          </a>
        </nav>
      </div>
    </header>
  );
}
