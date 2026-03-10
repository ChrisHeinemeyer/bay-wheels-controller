import type { GbfsStation, BoundingBox } from './types';

const GBFS_URL =
  '/gbfs-proxy/gbfs/2.3/bay/en/station_information.json';

export async function fetchStations(): Promise<GbfsStation[]> {
  const response = await fetch(GBFS_URL);
  if (!response.ok) {
    throw new Error(`GBFS fetch failed: ${response.status} ${response.statusText}`);
  }
  const data = (await response.json()) as { data: { stations: GbfsStation[] } };
  return data.data.stations;
}

export function stationsInBbox(
  stations: GbfsStation[],
  bbox: BoundingBox,
): GbfsStation[] {
  return stations.filter(
    (s) =>
      s.lat >= bbox.south &&
      s.lat <= bbox.north &&
      s.lon >= bbox.west &&
      s.lon <= bbox.east,
  );
}

/**
 * Sort stations for physical mapping: north→south, west→east within each
 * ~500 m latitude band so the ordering roughly matches a grid layout.
 */
export function sortStationsForMapping(stations: GbfsStation[]): GbfsStation[] {
  const ROW_DEG = 0.005; // ≈ 550 m per row
  return [...stations].sort((a, b) => {
    const rowA = Math.round(a.lat / ROW_DEG);
    const rowB = Math.round(b.lat / ROW_DEG);
    if (rowA !== rowB) return rowB - rowA; // descending lat (north first)
    return a.lon - b.lon;                  // ascending lon (west first)
  });
}
