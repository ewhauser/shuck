const basePath = process.env.NEXT_PUBLIC_BASE_PATH ?? "";

export function withBasePath(path: string): string {
  if (!path.startsWith("/") || path.startsWith("//")) {
    return path;
  }
  return basePath ? `${basePath}${path}` : path;
}
