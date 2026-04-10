import type { NextConfig } from "next";
import createMDX from "@next/mdx";

const isDev = process.env.NODE_ENV !== "production";
const isStaticExport = process.env.SHUCK_WEBSITE_EXPORT === "1";
const basePath = process.env.SHUCK_WEBSITE_BASE_PATH ?? "";

const cspHeader = `
  default-src 'self';
  script-src 'self' 'unsafe-inline' ${isDev ? "'unsafe-eval'" : ""} https://vercel.live https://va.vercel-scripts.com;
  style-src 'self' 'unsafe-inline' https://vercel.live;
  img-src 'self' data: blob: https://vercel.live https://vercel.com https://*.vercel.com;
  font-src 'self' https://vercel.live https://assets.vercel.com;
  connect-src 'self' https://vercel.live wss://*.pusher.com https://va.vercel-scripts.com;
  frame-src 'self' https://vercel.live;
  object-src 'none';
  base-uri 'self';
  form-action 'self';
`
  .replace(/\n/g, " ")
  .trim();

const withMDX = createMDX({
  options: {
    // Turbopack requires loader options to stay serializable.
    remarkPlugins: ["remark-gfm"],
    rehypePlugins: [
      [
        "rehype-pretty-code",
        {
          theme: "github-dark-default",
          keepBackground: false,
        },
      ],
      "rehype-slug",
    ],
  },
});

const nextConfig: NextConfig = {
  basePath: basePath || undefined,
  env: {
    NEXT_PUBLIC_BASE_PATH: basePath,
  },
  output: isStaticExport ? "export" : undefined,
  pageExtensions: ["js", "jsx", "md", "mdx", "ts", "tsx"],
  trailingSlash: isStaticExport,
  headers: isStaticExport
    ? undefined
    : async () => [
        {
          source: "/(.*)",
          headers: [{ key: "Content-Security-Policy", value: cspHeader }],
        },
      ],
};

export default withMDX(nextConfig);
