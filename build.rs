fn main() {
    linker_be_nice();
    // make sure linkall.x is the last linker script (otherwise might cause problems with flip-link)
    println!("cargo:rustc-link-arg=-Tlinkall.x");

    embed_git_version();
    generate_station_ids_ts();

    if std::env::var("CARGO_FEATURE_USE_ENV").is_ok() {
        load_env_file();
    }
}

/// Embed git tag and commit (e.g. v1.0.1-abe59403-dirty) into the firmware.
fn embed_git_version() {
    let output = std::process::Command::new("git")
        .args(["describe", "--always", "--dirty", "--tags"])
        .output();
    let version = match output {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).trim().to_string(),
        _ => "unknown".to_string(),
    };
    println!("cargo:rustc-env=GIT_VERSION={}", version);
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/index");
}

fn load_env_file() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let env_file = std::path::Path::new(&manifest_dir).join(".env");

    if !env_file.exists() {
        panic!(
            "use-env feature enabled but .env not found. Copy .env.dist to .env and set SSID and PASSWORD."
        );
    }

    let contents = std::fs::read_to_string(&env_file)
        .unwrap_or_else(|e| panic!("use-env: failed to read .env: {}", e));

    let mut ssid = None;
    let mut password = None;

    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            let key = key.trim();
            let value = value.trim().trim_matches('"').to_string();
            match key {
                "SSID" => ssid = Some(value),
                "PASSWORD" => password = Some(value),
                _ => {}
            }
        }
    }

    let ssid = ssid.expect("use-env: SSID not found in .env");
    let password = password.expect("use-env: PASSWORD not found in .env");

    println!("cargo:rustc-env=SSID={}", ssid);
    println!("cargo:rustc-env=PASSWORD={}", password);
    println!("cargo:rerun-if-changed=.env");
}

/// Parses `src/stations.rs` and emits `web/src/generated/station-ids.ts`.
///
/// The generated file exports a `STATION_IDS` map from StationIdx enum variant
/// name to GBFS station UUID, so the web UI can look up station names without
/// duplicating the UUID list.
fn generate_station_ids_ts() {
    println!("cargo:rerun-if-changed=src/stations.rs");

    let src = std::fs::read_to_string("src/stations.rs")
        .expect("build.rs: could not read src/stations.rs");

    // Locate the slice literal that follows TARGET_STATIONS: find `= &[` then
    // the matching `]` and extract (uuid, StationIdx::Variant) tuples.
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
                if depth == 0 {
                    array_end = i;
                    break;
                }
            }
            _ => {}
        }
    }
    let array_body = &array_src[..array_end];

    // Extract (uuid, variant_name) from each tuple ("uuid", StationIdx::Variant).
    let mut entries: Vec<(String, String)> = Vec::new();
    let mut i = 0;
    let bytes = array_body.as_bytes();
    while i < bytes.len() {
        // Look for "uuid" string
        if bytes[i] == b'"' {
            let start = i + 1;
            let mut end = start;
            while end < bytes.len() && bytes[end] != b'"' {
                end += 1;
            }
            let uuid = &array_body[start..end];
            if uuid.len() > 8 && (uuid.contains('-') || uuid.chars().all(|c| c.is_ascii_digit())) {
                // Look for StationIdx::VariantName after this uuid (skip to next tuple element)
                let rest = &array_body[end + 1..];
                if let Some(idx) = rest.find("StationIdx::") {
                    let variant_start = idx + "StationIdx::".len();
                    let variant_rest = &rest[variant_start..];
                    let variant_end = variant_rest
                        .find(|c: char| !c.is_alphanumeric() && c != '_')
                        .unwrap_or(variant_rest.len());
                    let variant = &variant_rest[..variant_end];
                    entries.push((variant.to_string(), uuid.to_string()));
                }
            }
            i = end + 1;
        } else {
            i += 1;
        }
    }

    let out_dir = "web/src/generated";
    std::fs::create_dir_all(out_dir).expect("build.rs: could not create web/src/generated");

    let mut ts = String::new();
    ts.push_str("// Generated from src/stations.rs by build.rs — do not edit manually.\n");
    ts.push_str("// Map from StationIdx ordinal to GBFS station UUID.\n");
    ts.push_str("// StationIdx::None and StationIdx::Unknown have no entries.\n");
    ts.push_str("export const STATION_IDS: Record<number, string> = {\n");
    for (i, (_, uuid)) in entries.iter().enumerate() {
        ts.push_str(&format!("  {i}: \"{uuid}\",\n"));
    }
    ts.push_str("};\n");

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
