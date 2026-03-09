import { defineConfig } from "vite";

export default defineConfig({
  // Use relative paths for local dev; override with /bay-wheels-controller/ for GitHub Pages
  base: process.env.GITHUB_PAGES ? "/bay-wheels-controller/" : "./",
});
