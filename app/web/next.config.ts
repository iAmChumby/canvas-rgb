import type { NextConfig } from "next";

// Static export: the Tauri shell loads app/web/out/ from disk. No server runtime.
const nextConfig: NextConfig = {
  output: "export",
  images: { unoptimized: true },
  trailingSlash: true,
  reactStrictMode: true,
};

export default nextConfig;
