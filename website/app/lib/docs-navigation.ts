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
