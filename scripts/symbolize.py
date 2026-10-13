#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = ["click>=8.1", "pyelftools>=0.31"]
# ///
"""Symbolize panic backtraces reported to the log server.

After a panic reset the firmware sends raw return addresses (`backtrace` in a
telemetry report). This finds the ELF that scripts/flash.sh archived for the
report's `fw` version and turns each address into function + file:line.

Function names come from the Xtensa addr2line. File:line comes from scanning
the DWARF line tables with pyelftools: in these fat-LTO builds the address
index (.debug_aranges) is wrong, so addr2line and llvm-dwarfdump --lookup
both return `??` for lines that are in fact in the line table.

    uv run scripts/symbolize.py              # latest report with a backtrace
    uv run scripts/symbolize.py --all        # every report with a backtrace
    uv run scripts/symbolize.py --elf X.elf  # use a specific ELF
"""

import bisect
import glob
import json
import os
import subprocess
import urllib.request
from datetime import datetime

import click
from elftools.elf.elffile import ELFFile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def find_addr2line():
    hits = sorted(glob.glob(os.path.expanduser(
        "~/.rustup/toolchains/esp/xtensa-esp-elf/*/xtensa-esp-elf/bin/xtensa-esp32s3-elf-addr2line")))
    if not hits:
        raise click.ClickException("xtensa-esp32s3-elf-addr2line not found (install the esp toolchain with espup)")
    return hits[-1]


class LineTable:
    """Every (address -> file:line) row from all line programs, sorted."""

    def __init__(self, elf_path):
        self.addrs, self.locs = [], []
        rows = []
        with open(elf_path, "rb") as f:
            dwarf = ELFFile(f).get_dwarf_info()
            for cu in dwarf.iter_CUs():
                lp = dwarf.line_program_for_CU(cu)
                if lp is None:
                    continue
                files = lp["file_entry"]
                dirs = lp["include_directory"]
                v5 = lp.header.version >= 5
                for entry in lp.get_entries():
                    st = entry.state
                    if st is None:
                        continue
                    if st.end_sequence:
                        rows.append((st.address, None))
                        continue
                    idx = st.file if v5 else st.file - 1
                    if not 0 <= idx < len(files):
                        continue
                    fe = files[idx]
                    name = fe.name.decode(errors="replace")
                    d = fe.dir_index if v5 else fe.dir_index - 1
                    if 0 <= d < len(dirs) and not name.startswith("/"):
                        name = f"{dirs[d].decode(errors='replace')}/{name}"
                    rows.append((st.address, (name, st.line)))
        rows.sort(key=lambda r: r[0])
        self.addrs = [a for a, _ in rows]
        self.locs = [l for _, l in rows]

    def lookup(self, addr):
        i = bisect.bisect_right(self.addrs, addr) - 1
        return self.locs[i] if i >= 0 else None


def short_path(path):
    for marker in ("/bay-wheels-controller/", "/index.crates.io-", "/rustlib/src/rust/"):
        if marker in path:
            tail = path.split(marker, 1)[1]
            return tail.split("/", 1)[1] if marker.startswith("/index.") else tail
    return path


def function_names(tool, elf, addrs):
    out = subprocess.run([tool, "-fC", "-e", elf, *addrs], capture_output=True, text=True, check=True).stdout
    lines = out.splitlines()
    return lines[0::2]  # -f prints "function\nfile:line" per address


@click.command(help=__doc__)
@click.option("--server", default="http://10.0.0.107:8099", show_default=True, help="Log server URL.")
@click.option("--all", "show_all", is_flag=True, help="Every report with a backtrace, not just the latest.")
@click.option("--elf", type=click.Path(exists=True, dir_okay=False), help="ELF to use instead of firmware-archive/<fw>.elf.")
def main(server, show_all, elf):
    with urllib.request.urlopen(f"{server}/api/reports?limit=5000", timeout=10) as resp:
        reports = [r for r in json.load(resp) if r.get("backtrace")]
    if not reports:
        raise click.ClickException("no reports with a backtrace yet")
    if not show_all:
        reports = reports[-1:]

    tool = find_addr2line()
    tables = {}
    for r in reports:
        fw = r.get("fw", "unknown")
        when = datetime.fromtimestamp(r["received_at"]).strftime("%Y-%m-%d %H:%M:%S")
        click.secho(f"=== {when}  boot #{r.get('boot')}  fw {fw}", bold=True)
        click.echo(f"    {r.get('reset', '')}")
        path = elf or os.path.join(ROOT, "firmware-archive", f"{fw}.elf")
        if not os.path.exists(path):
            click.secho(f"    no ELF for {fw} at {path}; pass --elf", fg="yellow")
            click.echo(f"    raw addresses: {' '.join(r['backtrace'])}\n")
            continue
        if path not in tables:
            tables[path] = LineTable(path)
        names = function_names(tool, path, r["backtrace"])
        for i, (addr, name) in enumerate(zip(r["backtrace"], names)):
            loc = tables[path].lookup(int(addr, 16))
            where = f"{short_path(loc[0])}:{loc[1]}" if loc else "??"
            click.echo(f"  #{i:<2} {addr}  {name}")
            click.secho(f"          at {where}", dim=True)
        click.echo()


if __name__ == "__main__":
    main()
