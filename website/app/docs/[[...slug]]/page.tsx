import { notFound } from "next/navigation";
import type { Metadata } from "next";
import { flattenNav, docsNavigation } from "@/app/lib/docs-navigation";

// Map of slug paths to their MDX imports
const pages: Record<string, () => Promise<{ default: React.ComponentType; metadata?: { title?: string; description?: string } }>> = {
  "getting-started": () => import("@/content/getting-started/index.mdx"),
  "shellcheck-compat": () => import("@/content/shellcheck-compat/index.mdx"),
  "performance/benchmarks": () => import("@/content/performance/benchmarks"),
};

interface Props {
  params: Promise<{ slug?: string[] }>;
}

export async function generateStaticParams() {
  return Object.keys(pages).map((slug) => ({
    slug: slug.split("/"),
  }));
}

export async function generateMetadata({ params }: Props): Promise<Metadata> {
  const { slug } = await params;
  const key = slug?.join("/") || "getting-started";
  const loader = pages[key];
  if (!loader) return {};

  // Find nav item for title
  const allItems = flattenNav(docsNavigation);
  const navItem = allItems.find((item) => item.href === `/docs/${key}`);

  return {
    title: navItem?.title || key,
  };
}

export default async function DocsPage({ params }: Props) {
  const { slug } = await params;
  const key = slug?.join("/") || "getting-started";
  const loader = pages[key];

  if (!loader) {
    notFound();
  }

  const { default: MDXContent } = await loader();

  return (
    <div className="mdx-content max-w-3xl">
      <MDXContent />
    </div>
  );
}
