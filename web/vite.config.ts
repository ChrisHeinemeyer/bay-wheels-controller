import { defineConfig } from "vite";

const FIRMWARE_URL =
  "https://github.com/ChrisHeinemeyer/bay-wheels-controller/releases/latest/download/firmware-bay-wheels-controller.bin";

const GBFS_STATION_INFO_URL =
  "https://gbfs.lyft.com/gbfs/2.3/bay/en/station_information.json";

export default defineConfig({
  // Use relative paths for local dev; override with /bay-wheels-controller/ for GitHub Pages
  base: process.env.GITHUB_PAGES ? "/bay-wheels-controller/" : "./",
  plugins: [
    {
      name: "dev-proxies",
      configureServer(server) {
        // Proxy the firmware binary.
        server.middlewares.use(
          "/firmware-bay-wheels-controller.bin",
          async (_req, res, next) => {
            try {
              const r = await fetch(FIRMWARE_URL, { redirect: "follow" });
              if (!r.ok) throw new Error(`Firmware fetch failed: ${r.status}`);
              const buf = await r.arrayBuffer();
              res.setHeader("Content-Type", "application/octet-stream");
              res.end(Buffer.from(buf));
            } catch (e) {
              next(e);
            }
          },
        );

        // Proxy GBFS station_information.json so local dev mirrors the
        // bundled file path used in production (/gbfs/station_information.json).
        server.middlewares.use(
          "/gbfs/station_information.json",
          async (_req, res, next) => {
            try {
              const r = await fetch(GBFS_STATION_INFO_URL);
              if (!r.ok) throw new Error(`GBFS fetch failed: ${r.status}`);
              const buf = await r.arrayBuffer();
              res.setHeader("Content-Type", "application/json");
              res.setHeader("Cache-Control", "max-age=60");
              res.end(Buffer.from(buf));
            } catch (e) {
              next(e);
            }
          },
        );
      },
    },
  ],
});
