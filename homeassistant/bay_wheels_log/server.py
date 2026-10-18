"""Telemetry receiver + status page for the bay-wheels-controller firmware.

The firmware (built with the `log-server` feature) POSTs one JSON report per
fetch cycle (~every 2 minutes) to /report. This stores them as JSON lines in
/data (persistent across add-on restarts) and serves a status page at /.

Standard library only, so the add-on image is just Alpine + python3.
"""

import html
import json
import os
import socket
import threading
import time
import urllib.request
from collections import deque
from datetime import datetime, timezone
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlparse

PORT = int(os.environ.get("PORT", "8099"))
DATA_DIR = os.environ.get("DATA_DIR", "/data")
LOG_PATH = os.path.join(DATA_DIR, "reports.jsonl")
MAX_FILE_BYTES = 5 * 1024 * 1024  # rotate: keep the newest KEEP lines
KEEP = 5000
MAX_BODY = 8192
# The firmware reports every ~120s; call it offline after a few missed cycles.
OFFLINE_AFTER_S = 6 * 60

# Battery ADC sits behind a 100k/22k divider (see src/tasks/battery.rs).
DIVIDER = (100 + 22) / 22

lock = threading.Lock()
reports = deque(maxlen=KEEP)

# GBFS mirror (firmware `gbfs-mirror` feature): keep a copy of Lyft's
# station_status feed and serve only the stations a board asks for, over plain
# HTTP. That spares the board DNS, the TLS handshake and ~270 KB of download
# per cycle. The feed regenerates every 60 s (its `ttl`); polling every 30 s
# keeps the copy at most ~90 s behind Lyft.
GBFS_URL = os.environ.get(
    "GBFS_URL", "https://gbfs.lyft.com/gbfs/2.3/bay/en/station_status.json"
)
MIRROR_EVERY_S = 30
# Older than this and the mirror answers 503, so the board fetches from Lyft.
MIRROR_STALE_S = 5 * 60
mirror = {"fetched_at": 0.0, "last_updated": 0, "ttl": 60, "by_id": {}, "error": None, "served": 0}


def mirror_loop():
    while True:
        try:
            req = urllib.request.Request(GBFS_URL, headers={"User-Agent": "bay-wheels-log"})
            with urllib.request.urlopen(req, timeout=20) as r:
                feed = json.load(r)
            by_id = {s["station_id"]: s for s in feed["data"]["stations"]}
            with lock:
                mirror.update(
                    fetched_at=time.time(),
                    last_updated=feed.get("last_updated", 0),
                    ttl=feed.get("ttl", 60),
                    by_id=by_id,
                    error=None,
                )
        except Exception as e:  # keep serving the last good copy until it goes stale
            with lock:
                mirror["error"] = f"{type(e).__name__}: {e}"
            print(f"GBFS mirror fetch failed: {e}", flush=True)
        time.sleep(MIRROR_EVERY_S)


def mirror_response(query):
    """(status, body) for GET /gbfs/station_status.json?ids=a,b,c[&fields=x,y].

    `fields`, if given, keeps only those keys of each station (the firmware
    only reads a few), which shrinks the response several-fold."""
    q = parse_qs(query)
    ids = [i for i in q.get("ids", [""])[0].split(",") if i]
    fields = [f for f in q.get("fields", [""])[0].split(",") if f]
    with lock:
        age = time.time() - mirror["fetched_at"]
        if not mirror["by_id"] or age > MIRROR_STALE_S:
            return 503, "mirror not ready or stale\n"
        stations = [mirror["by_id"][i] for i in ids if i in mirror["by_id"]]
        if fields:
            stations = [{k: s[k] for k in fields if k in s} for s in stations]
        mirror["served"] += 1
        body = {
            "last_updated": mirror["last_updated"],
            "ttl": mirror["ttl"],
            "data": {"stations": stations},
        }
    return 200, json.dumps(body)


# Connect-time UDP probes from the firmware ("BWP" + cycle + seq, see
# src/tasks/mirror.rs): logged with their arrival time and echoed back, to see
# when a freshly associated board's uplink starts passing frames.
probes = deque(maxlen=5000)


def probe_loop():
    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    sock.bind(("0.0.0.0", PORT))
    while True:
        data, addr = sock.recvfrom(64)
        if len(data) < 7 or data[:3] != b"BWP":
            continue
        sock.sendto(data, addr)
        with lock:
            probes.append({
                "t": time.time(),
                "remote": addr[0],
                "cycle": int.from_bytes(data[3:5], "big"),
                "seq": int.from_bytes(data[5:7], "big"),
            })


def load():
    if not os.path.exists(LOG_PATH):
        return
    with open(LOG_PATH, encoding="utf-8") as f:
        for line in f:
            try:
                reports.append(json.loads(line))
            except ValueError:
                pass


def append(report):
    with lock:
        reports.append(report)
        os.makedirs(DATA_DIR, exist_ok=True)
        with open(LOG_PATH, "a", encoding="utf-8") as f:
            f.write(json.dumps(report, separators=(",", ":")) + "\n")
        if os.path.getsize(LOG_PATH) > MAX_FILE_BYTES:
            tmp = LOG_PATH + ".tmp"
            with open(tmp, "w", encoding="utf-8") as f:
                for r in reports:
                    f.write(json.dumps(r, separators=(",", ":")) + "\n")
            os.replace(tmp, LOG_PATH)


def pack_mv(r):
    mv = r.get("battery_adc_mv")
    return round(mv * DIVIDER) if isinstance(mv, (int, float)) and mv else None


def fmt_age(seconds):
    if seconds is None:
        return "—"
    seconds = int(seconds)
    if seconds < 120:
        return f"{seconds}s"
    if seconds < 7200:
        return f"{seconds // 60}m"
    if seconds < 172800:
        return f"{seconds // 3600}h {seconds % 3600 // 60}m"
    return f"{seconds // 86400}d {seconds % 86400 // 3600}h"


def fmt_time(ts):
    return datetime.fromtimestamp(ts).strftime("%m-%d %H:%M:%S")


def sparkline(values, width=320, height=56, color="var(--accent)"):
    pts = [(i, v) for i, v in enumerate(values) if v is not None]
    if len(pts) < 2:
        return '<div class="muted">not enough data yet</div>'
    lo = min(v for _, v in pts)
    hi = max(v for _, v in pts)
    span = (hi - lo) or 1
    n = len(values) - 1 or 1
    coords = " ".join(
        f"{i / n * width:.1f},{height - 4 - (v - lo) / span * (height - 8):.1f}"
        for i, v in pts
    )
    return (
        f'<svg viewBox="0 0 {width} {height}" preserveAspectRatio="none" '
        f'class="spark"><polyline fill="none" stroke="{color}" stroke-width="2" '
        f'points="{coords}"/></svg>'
        f'<div class="spark-range muted">{lo:g} – {hi:g}</div>'
    )


def card(label, value, sub=""):
    return (
        f'<div class="card"><div class="label">{html.escape(label)}</div>'
        f'<div class="value">{value}</div>'
        f'<div class="sub muted">{sub}</div></div>'
    )


def mirror_card(now):
    with lock:
        fetched_at, n, served, error = (
            mirror["fetched_at"], len(mirror["by_id"]), mirror["served"], mirror["error"]
        )
    if not fetched_at:
        return card("GBFS mirror", "—", html.escape(error or "first fetch pending"))
    sub = f"{n} stations · served {served}×"
    if error:
        sub += f" · last error: {html.escape(error)}"
    return card("GBFS mirror", fmt_age(now - fetched_at) + " old", sub)


def render_page():
    with lock:
        recent = list(reports)
    now = time.time()
    if not recent:
        body = (
            '<p class="muted">No reports yet. Build the firmware with '
            "<code>--features log-server</code> and set <code>LOG_SERVER</code> "
            f"in <code>.env</code> to this host, port {PORT}.</p>"
        )
        return page(body)

    last = recent[-1]
    age = now - last["received_at"]
    online = age < OFFLINE_AFTER_S
    status = (
        f'<span class="pill ok">online</span>'
        if online
        else f'<span class="pill bad">no report for {fmt_age(age)}</span>'
    )

    mv = pack_mv(last)
    cards = "".join(
        [
            card(
                "Battery",
                f'{last.get("battery_pct", "—")}%',
                f"{mv / 1000:.2f} V pack" if mv else "",
            ),
            card("RSSI", f'{last.get("rssi", "—")} dBm', f'TX {last.get("tx_power_qdbm", 0) / 4:g} dBm'),
            card(
                "Data age",
                fmt_age(last.get("fetch_age_s")),
                "last fetch " + ("ok" if last.get("fetch_ok") else "FAILED"),
            ),
            card(
                "Uptime",
                fmt_age(last.get("uptime_s")),
                f'boot #{last.get("boot", "—")} since power-on',
            ),
            card(
                "Radio on",
                f'{last.get("prev_radio_on_ms", 0) / 1000:.1f}s',
                f'connect {last.get("connect_ms", 0) / 1000:.1f}s · '
                f'DHCP {last.get("dhcp_ms", 0) / 1000:.1f}s · '
                f'fetch {last.get("fetch_ms", 0) / 1000:.1f}s',
            ),
            card(
                "Fetches",
                f'{last.get("fetch_ok_total", 0)} ok',
                f'{last.get("fetch_fail_total", 0)} failed this boot',
            ),
            mirror_card(now),
        ]
    )

    window = recent[-360:]  # ~12h at one report per 2 min
    charts = (
        '<div class="charts">'
        f'<div class="chart"><div class="label">Battery pack (mV), last {len(window)} reports</div>'
        f"{sparkline([pack_mv(r) for r in window])}</div>"
        f'<div class="chart"><div class="label">Radio-on per cycle (s)</div>'
        f'{sparkline([(r.get("prev_radio_on_ms") or 0) / 1000 or None for r in window])}</div>'
        f'<div class="chart"><div class="label">Free heap at cycle start (bytes)</div>'
        f'{sparkline([r.get("heap_cycle_start") for r in window])}</div>'
        f'<div class="chart"><div class="label">RSSI (dBm)</div>'
        f'{sparkline([r.get("rssi") for r in window])}</div>'
        "</div>"
    )

    resets = [r for r in recent if r.get("reset")][-20:]
    def reset_cell(r):
        text = html.escape(str(r["reset"]))
        bt = r.get("backtrace")
        if bt:
            text += (
                '<br><span class="muted">backtrace (symbolize with '
                "<code>uv run scripts/symbolize.py</code>): "
                + " ".join(html.escape(a) for a in bt)
                + "</span>"
            )
        return text

    reset_rows = "".join(
        f'<tr><td>{fmt_time(r["received_at"])}</td><td>#{r.get("boot", "")}</td>'
        f'<td class="mono">{reset_cell(r)}</td></tr>'
        for r in reversed(resets)
    )
    reset_table = (
        "<h2>Resets</h2>"
        + (
            f'<table><tr><th>reported</th><th>boot</th><th>reason</th></tr>{reset_rows}</table>'
            if resets
            else '<p class="muted">none recorded</p>'
        )
    )

    rows = []
    for r in reversed(recent[-100:]):
        mv = pack_mv(r)
        cls = ' class="flag"' if r.get("reset") or not r.get("fetch_ok") else ""
        rows.append(
            f"<tr{cls}><td>{fmt_time(r['received_at'])}</td>"
            f"<td>{r.get('boot', '')}</td>"
            f"<td>{fmt_age(r.get('uptime_s'))}</td>"
            f"<td>{r.get('battery_pct', '')}%</td>"
            f"<td>{mv if mv else ''}</td>"
            f"<td>{r.get('rssi', '')}</td>"
            f"<td>{'ok' if r.get('fetch_ok') else 'FAIL'}</td>"
            f"<td>{fmt_age(r.get('fetch_age_s'))}</td>"
            f"<td>{(r.get('prev_radio_on_ms') or 0) / 1000:.1f}</td>"
            f"<td>{(r.get('dhcp_ms') or 0) / 1000:.1f}</td>"
            f"<td>{r.get('heap_cycle_start', '')}</td>"
            f"<td>{r.get('heap_free', '')}</td></tr>"
        )
    history = (
        "<h2>Recent reports</h2><div class=\"scroll\"><table>"
        "<tr><th>received</th><th>boot</th><th>uptime</th><th>batt</th><th>pack mV</th>"
        "<th>RSSI</th><th>fetch</th><th>data age</th><th>radio s</th><th>DHCP s</th>"
        "<th>heap idle</th><th>heap now</th></tr>" + "".join(rows) + "</table></div>"
    )

    head = (
        f'<div class="status">{status} '
        f'<span class="muted">last report {fmt_age(age)} ago · '
        f'{html.escape(str(last.get("board", "")))} · fw {html.escape(str(last.get("fw", "")))}</span></div>'
    )
    return page(head + f'<div class="cards">{cards}</div>' + charts + reset_table + history)


def page(body):
    return f"""<!doctype html>
<html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta http-equiv="refresh" content="30">
<title>Bay Wheels Log</title>
<style>
:root {{ --bg:#f7f7f5; --fg:#1d1d1b; --muted:#6b6b66; --card:#fff; --line:#e3e3de;
        --accent:#2f6fdf; --ok:#1f8a4c; --bad:#c2372f; --flag:#fff3f0; }}
@media (prefers-color-scheme: dark) {{
  :root {{ --bg:#151514; --fg:#ececea; --muted:#9a9a94; --card:#1f1f1d; --line:#33332f;
          --accent:#6f9cf0; --ok:#4cc27e; --bad:#ef6a61; --flag:#3a2120; }}
}}
body {{ margin:0; padding:16px; background:var(--bg); color:var(--fg);
       font:14px/1.45 system-ui, -apple-system, sans-serif; }}
h1 {{ font-size:20px; margin:0 0 8px; }} h2 {{ font-size:15px; margin:24px 0 8px; }}
.muted {{ color:var(--muted); }} .mono {{ font-family:ui-monospace, monospace; font-size:12px; }}
.status {{ margin-bottom:12px; }}
.pill {{ display:inline-block; padding:2px 10px; border-radius:999px; color:#fff; font-weight:600; }}
.pill.ok {{ background:var(--ok); }} .pill.bad {{ background:var(--bad); }}
.cards {{ display:grid; grid-template-columns:repeat(auto-fill, minmax(160px, 1fr)); gap:10px; }}
.card, .chart {{ background:var(--card); border:1px solid var(--line); border-radius:8px; padding:10px 12px; }}
.label {{ font-size:12px; color:var(--muted); }} .value {{ font-size:22px; font-weight:600; }}
.sub {{ font-size:12px; }}
.charts {{ display:grid; grid-template-columns:repeat(auto-fill, minmax(260px, 1fr)); gap:10px; margin-top:10px; }}
.spark {{ width:100%; height:56px; display:block; }} .spark-range {{ font-size:11px; }}
.scroll {{ overflow-x:auto; }}
table {{ border-collapse:collapse; width:100%; background:var(--card); }}
th, td {{ text-align:left; padding:4px 8px; border-bottom:1px solid var(--line); white-space:nowrap; }}
th {{ font-weight:600; font-size:12px; color:var(--muted); }}
tr.flag td {{ background:var(--flag); }}
td.mono {{ white-space:normal; min-width:280px; }}
</style></head>
<body><h1>Bay Wheels Log</h1>{body}</body></html>"""


class Handler(BaseHTTPRequestHandler):
    server_version = "BayWheelsLog/1.0"

    def log_message(self, fmt, *args):
        pass  # one line per report below is enough

    def _send(self, code, body, ctype="text/plain; charset=utf-8"):
        data = body.encode() if isinstance(body, str) else body
        self.send_response(code)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(data)))
        self.send_header("Connection", "close")
        self.end_headers()
        self.wfile.write(data)

    def do_POST(self):
        if urlparse(self.path).path != "/report":
            return self._send(404, "not found\n")
        length = int(self.headers.get("Content-Length") or 0)
        if not 0 < length <= MAX_BODY:
            return self._send(400, "bad length\n")
        try:
            report = json.loads(self.rfile.read(length))
            if not isinstance(report, dict):
                raise ValueError("not an object")
        except ValueError as e:
            return self._send(400, f"bad json: {e}\n")
        report["received_at"] = time.time()
        report["remote"] = self.client_address[0]
        append(report)
        extra = f' RESET: {report["reset"]}' if report.get("reset") else ""
        print(
            f'{datetime.now(timezone.utc).isoformat(timespec="seconds")} '
            f'{report["remote"]} boot#{report.get("boot")} '
            f'batt={report.get("battery_pct")}% rssi={report.get("rssi")} '
            f'fetch_ok={report.get("fetch_ok")}{extra}',
            flush=True,
        )
        self._send(200, "ok\n")

    def do_GET(self):
        url = urlparse(self.path)
        if url.path == "/":
            return self._send(200, render_page(), "text/html; charset=utf-8")
        if url.path == "/api/reports":
            limit = int(parse_qs(url.query).get("limit", ["200"])[0])
            with lock:
                data = list(reports)[-max(1, min(limit, KEEP)) :]
            return self._send(200, json.dumps(data), "application/json")
        if url.path == "/api/probes":
            with lock:
                data = list(probes)
            return self._send(200, json.dumps(data), "application/json")
        if url.path == "/healthz":
            return self._send(200, "ok\n")
        if url.path == "/gbfs/station_status.json":
            code, body = mirror_response(url.query)
            return self._send(code, body, "application/json" if code == 200 else "text/plain")
        return self._send(404, "not found\n")


if __name__ == "__main__":
    load()
    threading.Thread(target=mirror_loop, daemon=True).start()
    threading.Thread(target=probe_loop, daemon=True).start()
    print(f"Bay Wheels Log listening on :{PORT}, {len(reports)} reports loaded from {LOG_PATH}", flush=True)
    ThreadingHTTPServer(("0.0.0.0", PORT), Handler).serve_forever()
