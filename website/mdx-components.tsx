import Link from "next/link";
import type { ComponentPropsWithoutRef } from "react";
import type { MDXComponents } from "mdx/types";

function MDXAnchor({ href = "", ...props }: ComponentPropsWithoutRef<"a">) {
  if (href.startsWith("/")) {
    return <Link href={href} {...props} />;
  }
  return <a href={href} {...props} />;
}

export function useMDXComponents(components: MDXComponents): MDXComponents {
  return {
    a: MDXAnchor,
    ...components,
  };
}
