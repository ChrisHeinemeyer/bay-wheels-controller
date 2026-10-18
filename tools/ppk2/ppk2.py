#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = ["click>=8.1", "ppk2-api>=0.9.2"]
# ///
"""Power the board from a Nordic PPK2 and measure its current.

The PPK2 runs in source-meter mode: it supplies the board in place of the AA
pack (batteries removed) and samples the current at 100 kS/s. Samples are
averaged into 1 ms bins as they arrive, which is enough to separate the
light-sleep naps from the awake and radio-on stretches.

    uv run tools/ppk2/ppk2.py on                 # power on at 4.5 V
    uv run tools/ppk2/ppk2.py off
    uv run tools/ppk2/ppk2.py cycle              # off 2 s, then on (a hard reset)
    uv run tools/ppk2/ppk2.py measure -s 300     # 5 min summary
    uv run tools/ppk2/ppk2.py measure -s 300 --csv run.csv --json run.json

The power state is left as it is on exit, so the board keeps running between
commands. The summary splits time into asleep / awake / radio on by 1 ms bin
current (thresholds in mA, see --sleep-max and --radio-min).
"""

import csv
import json
import statistics
import sys
import time

import click
from ppk2_api.ppk2_api import PPK2_API, PPK2_MP

BIN_SAMPLES = 100  # 100 kS/s -> 1 ms bins


def find_port():
    ports = PPK2_MP.list_devices()
    if not ports:
        sys.exit("No PPK2 found")
    return ports[0][0] if isinstance(ports[0], tuple) else ports[0]


def open_ppk(port, mv):
    ppk = PPK2_MP(port or find_port(), timeout=1, write_timeout=1, exclusive=True)
    # A run killed mid-measurement leaves the PPK2 streaming samples, which
    # then garble the metadata read. Stop it and drain before asking.
    for attempt in range(5):
        PPK2_API.stop_measuring(ppk)
        # Drain until the port has been quiet for 300 ms: samples already in
        # flight keep arriving for a while after the stop command.
        quiet_since = time.monotonic()
        while time.monotonic() - quiet_since < 0.3:
            if ppk.ser.in_waiting:
                ppk.ser.reset_input_buffer()
                quiet_since = time.monotonic()
            time.sleep(0.02)
        try:
            ppk.get_modifiers()
            break
        except (UnicodeDecodeError, ValueError):
            if attempt == 4:
                raise
    ppk.use_source_meter()
    ppk.set_source_voltage(mv)
    return ppk


@click.group()
@click.option("--port", default=None, help="PPK2 serial port (default: autodetect)")
@click.option("--mv", default=4500, show_default=True, help="Supply voltage in mV")
@click.pass_context
def cli(ctx, port, mv):
    ctx.obj = {"port": port, "mv": mv}


@cli.command()
@click.pass_obj
def on(o):
    """Power the board."""
    open_ppk(o["port"], o["mv"]).toggle_DUT_power("ON")
    click.echo(f"DUT on at {o['mv']} mV")


@cli.command()
@click.pass_obj
def off(o):
    """Cut power to the board."""
    open_ppk(o["port"], o["mv"]).toggle_DUT_power("OFF")
    click.echo("DUT off")


@cli.command()
@click.option("--off-s", default=2.0, show_default=True, help="Seconds unpowered")
@click.pass_obj
def cycle(o, off_s):
    """Power-cycle the board (hard reset, starts the 20 s no-sleep boot window)."""
    ppk = open_ppk(o["port"], o["mv"])
    ppk.toggle_DUT_power("OFF")
    time.sleep(off_s)
    ppk.toggle_DUT_power("ON")
    click.echo(f"DUT power-cycled ({off_s}s off), on at {o['mv']} mV")


def collect(ppk, seconds, progress=None):
    """Stream for `seconds` and return 1 ms bin means in mA. Every 10 s, appends
    "<seconds> <last-10s mA>" to the `progress` file if given."""
    bins, acc, n = [], 0.0, 0
    ppk.start_measuring()
    end = time.monotonic() + seconds
    next_report = time.monotonic() + 10
    try:
        while time.monotonic() < end:
            data = ppk.get_data()
            if not data:
                time.sleep(0.01)
                continue
            samples, _ = ppk.get_samples(data)
            for ua in samples:
                acc += ua
                n += 1
                if n == BIN_SAMPLES:
                    bins.append(acc / BIN_SAMPLES / 1000)
                    acc, n = 0.0, 0
            if time.monotonic() >= next_report:
                next_report += 10
                recent = bins[-10000:]
                click.echo(
                    f"  {len(bins) / 1000:6.0f}s  last 10s avg {statistics.fmean(recent):6.2f} mA",
                    err=True,
                )
                if progress:
                    progress.write(f"{len(bins) / 1000:.0f} {statistics.fmean(recent):.3f}\n")
                    progress.flush()
    finally:
        ppk.stop_measuring()
    return bins


def summarize(bins, sleep_max, radio_min):
    if not bins:
        sys.exit("No samples received")
    ordered = sorted(bins)
    classes = {
        "asleep": [b for b in bins if b < sleep_max],
        "awake": [b for b in bins if sleep_max <= b < radio_min],
        "radio": [b for b in bins if b >= radio_min],
    }
    return {
        "seconds": len(bins) / 1000,
        "avg_ma": statistics.fmean(bins),
        "p05_ma": ordered[len(ordered) // 20],
        "median_ma": ordered[len(ordered) // 2],
        "max_ma": ordered[-1],
        "thresholds_ma": {"sleep_max": sleep_max, "radio_min": radio_min},
        "split": {
            name: {
                "share": len(v) / len(bins),
                "avg_ma": statistics.fmean(v) if v else None,
                "contrib_ma": sum(v) / len(bins),
            }
            for name, v in classes.items()
        },
    }


@cli.command()
@click.option("-s", "--seconds", default=60.0, show_default=True)
@click.option("--sleep-max", default=25.0, show_default=True, help="mA; below = asleep")
@click.option("--radio-min", default=80.0, show_default=True, help="mA; at/above = radio on")
@click.option("--power-on/--no-power-on", default=True, show_default=True,
              help="Make sure the DUT is powered before measuring")
@click.option("--csv", "csv_path", type=click.Path(dir_okay=False), help="Write 1 ms bins")
@click.option("--json", "json_path", type=click.Path(dir_okay=False), help="Write summary")
@click.option("--label", default="", help="Stored in the JSON summary")
@click.option("--progress", "progress_path", type=click.Path(dir_okay=False),
              help="Append a line every 10 s (for watching long runs)")
@click.pass_obj
def measure(o, seconds, sleep_max, radio_min, power_on, csv_path, json_path, label,
            progress_path):
    """Measure current for a while and print a summary."""
    ppk = open_ppk(o["port"], o["mv"])
    if power_on:
        ppk.toggle_DUT_power("ON")
    progress = open(progress_path, "a") if progress_path else None
    bins = collect(ppk, seconds, progress)
    s = summarize(bins, sleep_max, radio_min)
    s.update(label=label, supply_mv=o["mv"], started_at=time.time() - s["seconds"])

    click.echo(f"{s['seconds']:.0f}s at {o['mv']} mV: avg {s['avg_ma']:.2f} mA, "
               f"p5 {s['p05_ma']:.2f}, median {s['median_ma']:.2f}, max {s['max_ma']:.0f}")
    for name, c in s["split"].items():
        avg = f"{c['avg_ma']:7.2f} mA" if c["avg_ma"] is not None else "      —   "
        click.echo(f"  {name:6} {c['share'] * 100:5.1f}%  avg {avg}  -> {c['contrib_ma']:6.2f} mA")

    if csv_path:
        with open(csv_path, "w", newline="") as f:
            w = csv.writer(f)
            w.writerow(["ms", "ma"])
            w.writerows((i, f"{b:.4f}") for i, b in enumerate(bins))
    if json_path:
        with open(json_path, "w") as f:
            json.dump(s, f, indent=2)


@cli.command()
@click.argument("csv_path", type=click.Path(exists=True, dir_okay=False))
@click.option("-w", "--window", required=True, type=float, help="Window length, seconds")
@click.option("--offset", default=0.0, show_default=True,
              help="Seconds from the start of the CSV to board uptime 0")
@click.option("--trim", default=15.0, show_default=True,
              help="Seconds dropped at each window edge")
@click.option("--levels", default=4, show_default=True,
              help="Windows repeat with this period (SLEEP_SWEEP_S levels)")
@click.option("--sleep-max", default=25.0, show_default=True)
@click.option("--radio-min", default=80.0, show_default=True)
def windows(csv_path, window, offset, trim, levels, sleep_max, radio_min):
    """Summarize a `measure --csv` recording per fixed-length window.

    For a SLEEP_SWEEP_S build, use the same window: window n is level n % levels.
    """
    with open(csv_path) as f:
        r = csv.reader(f)
        next(r)
        bins = [float(row[1]) for row in r]
    w_ms, o_ms, t_ms = int(window * 1000), int(offset * 1000), int(trim * 1000)
    n = 0
    click.echo("win level   secs   avg mA  asleep% (mA)    awake% (mA)    radio% (mA)")
    while o_ms + n * w_ms + t_ms < len(bins):
        lo = o_ms + n * w_ms + t_ms
        hi = min(o_ms + (n + 1) * w_ms - t_ms, len(bins))
        if hi - lo < 10_000:
            break
        s = summarize(bins[lo:hi], sleep_max, radio_min)
        cols = []
        for name in ("asleep", "awake", "radio"):
            c = s["split"][name]
            avg = f"{c['avg_ma']:5.1f}" if c["avg_ma"] is not None else "  —  "
            cols.append(f"{c['share'] * 100:5.1f}% ({avg})")
        click.echo(f"{n:3} {n % levels:5} {s['seconds']:6.0f} {s['avg_ma']:8.2f}  " + "  ".join(cols))
        n += 1


if __name__ == "__main__":
    cli()
