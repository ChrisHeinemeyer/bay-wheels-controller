fn main() {
    linker_be_nice();
    // make sure linkall.x is the last linker script (otherwise might cause problems with flip-link)
    println!("cargo:rustc-link-arg=-Tlinkall.x");

    generate_station_ids_ts();
}

/// Parses `src/stations.rs` and emits `web/src/generated/station-ids.ts`.
///
/// The generated file exports a `STATION_IDS` array where each index matches
/// the corresponding `StationIdx` ordinal so that the web UI can look up the
/// GBFS station name without duplicating the UUID list.
fn generate_station_ids_ts() {
    println!("cargo:rerun-if-changed=src/stations.rs");

    let src = std::fs::read_to_string("src/stations.rs")
        .expect("build.rs: could not read src/stations.rs");

    // Locate the slice literal that follows TARGET_STATIONS: find `= &[` then
    // the matching `]` and extract all quoted string literals within that range.
    let array_start = src
        .find("TARGET_STATIONS")
        .and_then(|p| src[p..].find("= &[").map(|q| p + q + 4))
        .expect("build.rs: could not find TARGET_STATIONS array in src/stations.rs");

    let array_src = &src[array_start..];

    // Walk forward to find the matching `]` (depth-aware, string-aware).
    let mut depth = 1usize;
    let mut array_end = array_src.len();
    let mut in_str = false;
    for (i, c) in array_src.char_indices() {
        match c {
            '"' => in_str = !in_str,
            '[' if !in_str => depth += 1,
            ']' if !in_str => {
                depth -= 1;
                if depth == 0 { array_end = i; break; }
            }
            _ => {}
        }
    }
    let array_body = &array_src[..array_end];

    // Extract every quoted string literal from the array body in order.
    // Each tuple entry is ("uuid", StationIdx::Variant); we accept anything
    // that looks like a UUID (contains '-') or a pure-numeric GBFS station ID.
    let mut ids: Vec<String> = Vec::new();
    let mut i = 0;
    let bytes = array_body.as_bytes();
    while i < bytes.len() {
        if bytes[i] == b'"' {
            let start = i + 1;
            let mut end = start;
            while end < bytes.len() && bytes[end] != b'"' { end += 1; }
            let s = &array_body[start..end];
            if s.len() > 8 && (s.contains('-') || s.chars().all(|c| c.is_ascii_digit())) {
                ids.push(s.to_string());
            }
            i = end + 1;
        } else {
            i += 1;
        }
    }

    let out_dir = "web/src/generated";
    std::fs::create_dir_all(out_dir)
        .expect("build.rs: could not create web/src/generated");

    let mut ts = String::new();
    ts.push_str("// Generated from src/stations.rs by build.rs — do not edit manually.\n");
    ts.push_str("// Each array index matches the corresponding StationIdx ordinal.\n");
    ts.push_str("// StationIdx::None = 255 has no entry; check for that before indexing.\n");
    ts.push_str("export const STATION_IDS: string[] = [\n");
    for id in &ids {
        ts.push_str(&format!("  \"{id}\",\n"));
    }
    ts.push_str("];\n");

    let out_path = format!("{out_dir}/station-ids.ts");
    // Only write if content changed to avoid spurious Vite rebuilds.
    let existing = std::fs::read_to_string(&out_path).unwrap_or_default();
    if existing != ts {
        std::fs::write(&out_path, ts)
            .expect("build.rs: could not write web/src/generated/station-ids.ts");
    }
}

fn linker_be_nice() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 {
        let kind = &args[1];
        let what = &args[2];

        match kind.as_str() {
            "undefined-symbol" => match what.as_str() {
                what if what.starts_with("_defmt_") => {
                    eprintln!();
                    eprintln!(
                        "💡 `defmt` not found - make sure `defmt.x` is added as a linker script and you have included `use defmt_rtt as _;`"
                    );
                    eprintln!();
                }
                "_stack_start" => {
                    eprintln!();
                    eprintln!("💡 Is the linker script `linkall.x` missing?");
                    eprintln!();
                }
                what if what.starts_with("esp_rtos_") => {
                    eprintln!();
                    eprintln!(
                        "💡 `esp-radio` has no scheduler enabled. Make sure you have initialized `esp-rtos` or provided an external scheduler."
                    );
                    eprintln!();
                }
                "embedded_test_linker_file_not_added_to_rustflags" => {
                    eprintln!();
                    eprintln!(
                        "💡 `embedded-test` not found - make sure `embedded-test.x` is added as a linker script for tests"
                    );
                    eprintln!();
                }
                "free"
                | "malloc"
                | "calloc"
                | "get_free_internal_heap_size"
                | "malloc_internal"
                | "realloc_internal"
                | "calloc_internal"
                | "free_internal" => {
                    eprintln!();
                    eprintln!(
                        "💡 Did you forget the `esp-alloc` dependency or didn't enable the `compat` feature on it?"
                    );
                    eprintln!();
                }
                _ => (),
            },
            // we don't have anything helpful for "missing-lib" yet
            _ => {
                std::process::exit(1);
            }
        }

        std::process::exit(0);
    }

    // Only add error-handling-script for RISC-V targets (xtensa-esp-elf-gcc doesn't support it)
    let target = std::env::var("TARGET").unwrap_or_default();
    if target.starts_with("riscv") {
        println!(
            "cargo:rustc-link-arg=--error-handling-script={}",
            std::env::current_exe().unwrap().display()
        );
    }
}
