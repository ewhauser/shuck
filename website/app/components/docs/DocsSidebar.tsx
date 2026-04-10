"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
import { useState } from "react";
import { ChevronDown, Menu, X } from "lucide-react";
import { docsNavigation, type NavItem } from "@/app/lib/docs-navigation";

function NavSection({ item }: { item: NavItem }) {
  const pathname = usePathname();
  const isActive = item.items?.some((child) => child.href === pathname);
  const [open, setOpen] = useState(true);

  return (
    <div className="mb-4">
      <button
        onClick={() => setOpen(!open)}
        className="flex w-full items-center justify-between text-xs font-semibold uppercase tracking-wider text-fg-dim hover:text-fg-secondary transition-colors mb-1.5 px-2"
      >
        <span className={isActive ? "text-accent" : ""}>{item.title}</span>
        <ChevronDown
          className={`h-3.5 w-3.5 transition-transform ${open ? "" : "-rotate-90"}`}
        />
      </button>
      {open && item.items && (
        <ul className="space-y-0.5">
          {item.items.map((child) => (
            <li key={child.href}>
              <Link
                href={child.href!}
                className={`block rounded-md px-2 py-1.5 text-sm transition-colors ${
                  pathname === child.href
                    ? "bg-accent/10 text-accent font-medium"
                    : "text-fg-secondary hover:text-fg-primary hover:bg-bg-card"
                }`}
              >
                {child.title}
              </Link>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

export default function DocsSidebar() {
  const [mobileOpen, setMobileOpen] = useState(false);

  return (
    <>
      {/* Mobile toggle */}
      <button
        onClick={() => setMobileOpen(!mobileOpen)}
        className="fixed bottom-4 right-4 z-50 flex h-12 w-12 items-center justify-center rounded-full bg-accent text-bg-primary shadow-lg lg:hidden"
        aria-label="Toggle navigation"
      >
        {mobileOpen ? <X className="h-5 w-5" /> : <Menu className="h-5 w-5" />}
      </button>

      {/* Mobile overlay */}
      {mobileOpen && (
        <div
          className="fixed inset-0 z-40 bg-black/60 lg:hidden"
          onClick={() => setMobileOpen(false)}
        />
      )}

      {/* Sidebar */}
      <aside
        className={`fixed top-14 left-0 z-40 h-[calc(100vh-3.5rem)] w-64 overflow-y-auto border-r border-fg-dim/20 bg-bg-primary p-4 transition-transform lg:sticky lg:translate-x-0 lg:block ${
          mobileOpen ? "translate-x-0" : "-translate-x-full"
        }`}
      >
        <nav>
          {docsNavigation.map((section) => (
            <NavSection key={section.title} item={section} />
          ))}
        </nav>
      </aside>
    </>
  );
}
