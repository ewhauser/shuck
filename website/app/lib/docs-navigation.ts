export interface NavItem {
  title: string;
  href?: string;
  items?: NavItem[];
}

export const docsNavigation: NavItem[] = [
  {
    title: "Getting Started",
    items: [
      { title: "Overview", href: "/docs/getting-started" },
      { title: "Configuration", href: "/docs/configuration" },
      { title: "Settings Reference", href: "/docs/settings" },
    ],
  },
  {
    title: "Linting",
    items: [
      { title: "Linting", href: "/docs/linting" },
      { title: "Suppression", href: "/docs/suppression" },
      { title: "Embedded Scripts", href: "/docs/embedded-scripts" },
      { title: "Zsh Support", href: "/docs/zsh-support" },
      { title: "Contracts", href: "/docs/contracts" },
      { title: "ShellCheck Compatibility", href: "/docs/shellcheck-compat" },
      {
        title: "Rules",
        items: [
          { title: "All Rules", href: "/docs/rules" },
          { title: "Correctness (C)", href: "/docs/rules?category=Correctness" },
          { title: "Style (S)", href: "/docs/rules?category=Style" },
          { title: "Portability (X)", href: "/docs/rules?category=Portability" },
          { title: "Performance (P)", href: "/docs/rules?category=Performance" },
          { title: "Security (K)", href: "/docs/rules?category=Security" },
        ],
      },
    ],
  },
  {
    title: "Formatting",
    items: [
      { title: "Formatting", href: "/docs/formatting" },
    ],
  },
  {
    title: "LSP",
    items: [
      { title: "LSP Server", href: "/docs/lsp" },
    ],
  },
  {
    title: "Runtime",
    items: [
      { title: "Managed Runtime", href: "/docs/run" },
    ],
  },
  {
    title: "Performance",
    items: [
      { title: "Benchmarks", href: "/docs/performance/benchmarks" },
    ],
  },
];

export function flattenNav(items: NavItem[]): NavItem[] {
  const flat: NavItem[] = [];
  for (const item of items) {
    if (item.href) flat.push(item);
    if (item.items) flat.push(...flattenNav(item.items));
  }
  return flat;
}
