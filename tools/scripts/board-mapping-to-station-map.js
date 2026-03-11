#!/usr/bin/env node
/**
 * Converts a board-mapping YAML (from the UI) into a BOARD_STATION_MAP entry.
 * Uses TARGET_STATIONS from src/stations.rs to map station_id -> StationIdx.
 *
 * Usage: node tools/scripts/board-mapping-to-station-map.js <yaml-file> [--board-id BoardN]
 *
 * Example: node tools/scripts/board-mapping-to-station-map.js ~/Downloads/board-mapping-1773211837841.yaml --board-id Board4
 */

import fs from "fs";
import path from "path";
import { fileURLToPath } from "url";
import yaml from "js-yaml";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const PROJECT_ROOT = path.resolve(__dirname, "../..");
const STATIONS_RS = path.join(PROJECT_ROOT, "src/stations.rs");

function parseTargetStations(stationsRs) {
  const map = new Map();
  // Match both multiline (,\n    ) and single-line (,) formats before closing paren
  const pairRe = /\(\s*"([^"]+)"\s*,\s*StationIdx::([A-Za-z0-9_]+)[\s,]*\)/g;
  const targetSection = stationsRs.includes("TARGET_STATIONS")
    ? stationsRs.slice(stationsRs.indexOf("TARGET_STATIONS"))
    : stationsRs;
  let m;
  while ((m = pairRe.exec(targetSection)) !== null) {
    const [, stationId, stationIdx] = m;
    if (stationId && stationIdx) {
      map.set(stationId, stationIdx);
    }
  }
  return map;
}

function main() {
  const args = process.argv.slice(2);
  const boardIdIdx = args.indexOf("--board-id");
  const boardId =
    boardIdIdx >= 0 && args[boardIdIdx + 1] ? args[boardIdIdx + 1] : "Board4";
  const yamlPath = args.find(
    (a) => !a.startsWith("--") && a !== args[boardIdIdx + 1],
  );
  if (!yamlPath) {
    console.error(
      "Usage: node board-mapping-to-station-map.js <yaml-file> [--board-id BoardN]",
    );
    process.exit(1);
  }

  const yamlContent = fs.readFileSync(yamlPath, "utf-8");
  const stationsRs = fs.readFileSync(STATIONS_RS, "utf-8");

  const data = yaml.load(yamlContent);
  if (!data?.mappings?.length) {
    console.error("No mappings found in YAML");
    process.exit(1);
  }

  const targetStations = parseTargetStations(stationsRs);
  const entries = [];

  for (const m of data.mappings) {
    const stationIdx = targetStations.get(String(m.station_id)) ?? "Unknown";
    entries.push(
      `            ((Row(${m.row}), Column(${m.column})), StationIdx::${stationIdx}),`,
    );
  }

  console.log(
    `    (\n        BoardId::${boardId},\n        &[\n${entries.join("\n")}\n        ],\n    ),`,
  );
}

main();
