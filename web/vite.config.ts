import { defineConfig } from "vite";

const FIRMWARE_URL =
  "https://github.com/ChrisHeinemeyer/bay-wheels-controller/releases/latest/download/firmware-bay-wheels-controller.bin";

export default defineConfig({
  // Use relative paths for local dev; override with /bay-wheels-controller/ for GitHub Pages
  base: process.env.GITHUB_PAGES ? "/bay-wheels-controller/" : "./",
  plugins: [
    {
      name: "firmware-proxy",
      configureServer(server) {
        server.middlewares.use("/firmware-bay-wheels-controller.bin", async (_req, res, next) => {
          try {
            const r = await fetch(FIRMWARE_URL, { redirect: "follow" });
            if (!r.ok) throw new Error(`Firmware fetch failed: ${r.status}`);
            const buf = await r.arrayBuffer();
            res.setHeader("Content-Type", "application/octet-stream");
            res.end(Buffer.from(buf));
          } catch (e) {
            next(e);
          }
        });
      },
    },
  ],
});
