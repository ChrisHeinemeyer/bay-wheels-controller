import { defineConfig } from "vite";

export default defineConfig({
  server: {
    proxy: {
      "/gbfs-proxy": {
        target: "https://gbfs.lyft.com",
        changeOrigin: true,
        rewrite: (path) => path.replace(/^\/gbfs-proxy/, ""),
      },
    },
  },
});
