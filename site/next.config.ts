import path from "node:path";
import type { NextConfig } from "next";

// GitHub Pages serves this site from https://iamchumby.github.io/canvas-rgb/, so every path
// the page asks for carries the /canvas-rgb prefix.
const basePath = "/canvas-rgb";

const nextConfig: NextConfig = {
  basePath,
  output: "export",
  images: { unoptimized: true },
  trailingSlash: true,
  reactStrictMode: true,
  // The editor's own components live outside this project, in app/web/src. The site imports
  // them read-only, so both webpack and TypeScript have to look outside site/. externalDir is
  // what lets webpack read those files.
  experimental: { externalDir: true },
  // The one place the prefix is written down twice: the page needs it for the images it
  // shows, because next/image does not add it.
  env: { NEXT_PUBLIC_BASE_PATH: basePath },
  webpack: (config) => {
    // The editor has a node_modules of its own. Every bare import has to come from this
    // project's copy, or the editor's files would pull in a second Next and a second React.
    config.resolve.modules = [path.join(__dirname, "node_modules"), "node_modules"];
    return config;
  },
};

export default nextConfig;
