use heapless::String;
use picojson::PullParser;

use crate::stations::StationIdx;

/// Station data we care about
#[derive(Debug, Clone, Copy)]
pub struct StationData {
    pub station_idx: StationIdx,
    pub num_bikes_available: u32,
    pub num_ebikes_available: u32,
}

impl Default for StationData {
    fn default() -> Self {
        Self {
            station_idx: StationIdx::None,
            num_bikes_available: 0,
            num_ebikes_available: 0,
        }
    }
}
#[derive(Debug, PartialEq)]
enum ParserPhase {
    SearchingForStations, // Looking for "stations": [
    InStationsArray,      // Inside the array, extracting objects
    Done,                 // Finished parsing
}

/// Truly incremental streaming parser that processes ONE station at a time
pub struct StreamingStationParser {
    target_stations: &'static [(&'static str, StationIdx)], // (station_id, station_id) pairs
    // Small buffer for incomplete data at chunk boundaries
    remainder: String<8192>, // Can hold one complete station object
    phase: ParserPhase,
    // Counters
    station_count: u32,
    ignored_count: u32,
}

impl StreamingStationParser {
    pub fn new(target_stations: &'static [(&'static str, StationIdx)]) -> Self {
        Self {
            target_stations,
            remainder: String::new(),
            phase: ParserPhase::SearchingForStations,
            station_count: 0,
            ignored_count: 0,
        }
    }

    /// Process a chunk - extracts and handles ONE station at a time
    pub fn process_chunk(&mut self, chunk: &str) -> heapless::Vec<StationData, 8> {
        let mut results = heapless::Vec::new();

        // Prepend remainder from previous chunk
        let mut buffer = String::<12288>::new(); // 8KB remainder + 4KB chunk
        let _ = buffer.push_str(self.remainder.as_str());
        let _ = buffer.push_str(chunk);

        let mut pos = 0;

        match self.phase {
            ParserPhase::SearchingForStations => {
                // Look for "stations": [
                if let Some(stations_pos) = buffer.find("\"stations\"") {
                    // Find the opening [ after "stations":
                    if let Some(bracket_pos) = buffer[stations_pos..].find('[') {
                        pos = stations_pos + bracket_pos + 1;
                        self.phase = ParserPhase::InStationsArray;
                    }
                } else {
                    // Haven't found it yet, save last portion as remainder
                    self.save_remainder(&buffer, buffer.len().saturating_sub(100));
                    return results;
                }
            }
            ParserPhase::InStationsArray => {
                // Already in the array, start from beginning
            }
            ParserPhase::Done => {
                return results;
            }
        }

        // Now extract and parse station objects one at a time
        while pos < buffer.len() && self.phase == ParserPhase::InStationsArray {
            // Skip whitespace and commas
            while pos < buffer.len() {
                let ch = buffer.as_bytes()[pos] as char;
                if ch == ' ' || ch == '\n' || ch == '\r' || ch == '\t' || ch == ',' {
                    pos += 1;
                } else {
                    break;
                }
            }

            if pos >= buffer.len() {
                break;
            }

            let ch = buffer.as_bytes()[pos] as char;

            if ch == ']' {
                // End of stations array
                self.phase = ParserPhase::Done;
                break;
            }

            if ch == '{' {
                // Found start of a station object
                // Find the matching closing }
                if let Some(end_pos) = self.find_matching_brace(&buffer, pos) {
                    // Extract this one station object
                    let station_json = &buffer[pos..=end_pos];

                    // Parse just this one station
                    if let Some(station_data) = self.parse_single_station(station_json) {
                        let _ = results.push(station_data);
                    }

                    // Move past this station (and any trailing comma)
                    pos = end_pos + 1;
                    if pos < buffer.len() && buffer.as_bytes()[pos] as char == ',' {
                        pos += 1;
                    }
                } else {
                    // Incomplete object - save remainder and wait for more data
                    self.save_remainder(&buffer, pos);
                    return results;
                }
            } else {
                // Unexpected character
                pos += 1;
            }
        }

        // Save any remaining data
        if pos < buffer.len() {
            self.save_remainder(&buffer, pos);
        } else {
            self.remainder.clear();
        }

        results
    }

    /// Find the matching closing brace for an opening brace at given position
    fn find_matching_brace(&self, buffer: &str, start: usize) -> Option<usize> {
        let bytes = buffer.as_bytes();
        let mut depth = 0;
        let mut in_string = false;
        let mut escape_next = false;

        for i in start..bytes.len() {
            let ch = bytes[i] as char;

            if escape_next {
                escape_next = false;
                continue;
            }

            if ch == '\\' && in_string {
                escape_next = true;
                continue;
            }

            if ch == '"' {
                in_string = !in_string;
                continue;
            }

            if !in_string {
                if ch == '{' {
                    depth += 1;
                } else if ch == '}' {
                    depth -= 1;
                    if depth == 0 {
                        return Some(i);
                    }
                }
            }
        }

        None // Incomplete object
    }

    /// Parse a single station JSON object
    fn parse_single_station(&mut self, json: &str) -> Option<StationData> {
        self.station_count += 1;

        // Use picojson to parse just this one object
        let mut parser = picojson::SliceParser::new(json);

        let mut station_id: Option<String<64>> = None;
        let mut num_bikes_available: Option<u32> = None;
        let mut num_ebikes_available: Option<u32> = None;
        let mut last_key = String::<32>::new();

        while let Some(event_result) = parser.next() {
            if let Ok(event) = event_result {
                match event {
                    picojson::Event::Key(key) => {
                        last_key.clear();
                        let _ = last_key.push_str(key.as_str());
                    }
                    picojson::Event::String(value) => {
                        if last_key == "station_id" {
                            let mut id = String::new();
                            let _ = id.push_str(value.as_str());
                            station_id = Some(id);
                        }
                    }
                    picojson::Event::Number(num) => {
                        if let Ok(val) = num.as_str().parse::<u32>() {
                            if last_key == "num_bikes_available" {
                                num_bikes_available = Some(val);
                            } else if last_key == "num_ebikes_available" {
                                num_ebikes_available = Some(val);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        // Check if this is a target station
        if let Some(ref id) = station_id {
            // Look up the station in our map
            let target_match = self
                .target_stations
                .iter()
                .find(|&&(target_id, _)| target_id == id.as_str());

            if let Some(&(_, station_idx)) = target_match {
                return Some(StationData {
                    station_idx: station_idx,
                    num_bikes_available: num_bikes_available.unwrap_or(0)
                        - num_ebikes_available.unwrap_or(0),
                    num_ebikes_available: num_ebikes_available.unwrap_or(0),
                });
            } else {
                self.ignored_count += 1;
            }
        }

        None
    }

    fn save_remainder(&mut self, buffer: &str, from_pos: usize) {
        self.remainder.clear();
        if from_pos < buffer.len() {
            let _ = self.remainder.push_str(&buffer[from_pos..]);
        }
    }

    pub fn finish(&mut self) {}
}
