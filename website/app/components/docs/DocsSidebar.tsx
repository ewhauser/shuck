"use client";

import Link from "next/link";
import { usePathname, useSearchParams } from "next/navigation";
import { useEffect, useState } from "react";
import { ChevronDown, Menu, X } from "lucide-react";
import { docsNavigation, type NavItem } from "@/app/lib/docs-navigation";

function hrefIsActive(href: string, pathname: string, currentHref: string) {
  if (href.includes("?")) {
    return currentHref === href;
  }
  if (pathname === href) {
    return currentHref === href;
  }
  return pathname.startsWith(`${href}/`);
}

function navItemIsActive(item: NavItem, pathname: string, currentHref: string): boolean {
  if (item.href && hrefIsActive(item.href, pathname, currentHref)) {
    return true;
  }
  return item.items?.some((child) => navItemIsActive(child, pathname, currentHref)) ?? false;
}

function NavListItem({
  item,
  pathname,
  currentHref,
  depth = 0,
}: {
  item: NavItem;
  pathname: string;
  currentHref: string;
  depth?: number;
}) {
  const hasChildren = Boolean(item.items?.length);
  const isActive = navItemIsActive(item, pathname, currentHref);
  const [open, setOpen] = useState(isActive);

  useEffect(() => {
    if (isActive) setOpen(true);
  }, [isActive]);

  if (hasChildren) {
    return (
      <li>
        <button
          onClick={() => setOpen(!open)}
          aria-expanded={open}
          className={`flex w-full items-center justify-between rounded-md py-1.5 pr-2 text-sm transition-colors ${
            depth > 0 ? "pl-4" : "pl-2"
          } ${
            isActive
              ? "text-accent font-medium"
              : "text-fg-secondary hover:text-fg-primary hover:bg-bg-card"
          }`}
        >
          <span>{item.title}</span>
          <ChevronDown
            className={`h-3.5 w-3.5 transition-transform ${open ? "" : "-rotate-90"}`}
          />
        </button>
        {open && (
          <ul className="ml-3 mt-0.5 space-y-0.5 border-l border-fg-dim/20 pl-2">
            {item.items!.map((child) => (
              <NavListItem
                key={`${child.title}-${child.href ?? "group"}`}
                item={child}
                pathname={pathname}
                currentHref={currentHref}
                depth={depth + 1}
              />
            ))}
          </ul>
        )}
      </li>
    );
  }

  if (!item.href) {
    return null;
  }

  const isLinkActive = hrefIsActive(item.href, pathname, currentHref);

  return (
    <li>
      <Link
        href={item.href}
        className={`block rounded-md py-1.5 pr-2 text-sm transition-colors ${
          depth > 0 ? "pl-4" : "pl-2"
        } ${
          isLinkActive
            ? "bg-accent/10 text-accent font-medium"
            : "text-fg-secondary hover:text-fg-primary hover:bg-bg-card"
        }`}
      >
        {item.title}
      </Link>
    </li>
  );
}

function NavSection({ item }: { item: NavItem }) {
  const pathname = usePathname();
  const searchParams = useSearchParams();
  const query = searchParams.toString();
  const currentHref = query ? `${pathname}?${query}` : pathname;
  const isActive = navItemIsActive(item, pathname, currentHref);
  const [open, setOpen] = useState(true);

  useEffect(() => {
    if (isActive) setOpen(true);
  }, [isActive]);

  return (
    <div className="mb-4">
      <button
        onClick={() => setOpen(!open)}
        aria-expanded={open}
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
            <NavListItem
              key={`${child.title}-${child.href ?? "group"}`}
              item={child}
              pathname={pathname}
              currentHref={currentHref}
            />
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
