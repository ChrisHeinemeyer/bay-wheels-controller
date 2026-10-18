"""Split a `ppk2.py measure --csv` recording into the sleep floor, the fetch
cycles and the awake bursts between them, with each burst assigned to a task
by the period it recurs at (500 ms input poll, 2 s serial status, 30 s
battery).

    python3 tools/ppk2/breakdown.py run.csv [skip_s]   # skip_s: boot window, default 40

The fetch-cycle and floor numbers are solid (the parts sum to within ~1% of the
measured average). The per-task split is approximate: the task timers line up
on a 500 ms grid, so coinciding wakes land in one burst. For an exact split,
multiply the telemetry's `task_polls_total` (sleep-trace builds) by the
measured charge per wake.
"""
import csv
import statistics
import sys

path = sys.argv[1]
skip_s = float(sys.argv[2]) if len(sys.argv) > 2 else 40
with open(path) as f:
    r = csv.reader(f)
    next(r)
    ma = [float(row[1]) for row in r]
ma = ma[int(skip_s * 1000):]
n = len(ma)
total_s = n / 1000
avg = sum(ma) / n

AWAKE = 25.0
RADIO = 80.0
floor = statistics.median(b for b in ma if b < AWAKE)

# Radio windows: from 3 s before the first radio bin to 3 s after the last,
# merging radio bins closer than 5 s. Everything inside counts as "fetch cycle".
# Only sustained radio activity seeds a window: >= 20 radio bins in 100 ms.
# Single 1 ms spikes from wake transients would otherwise chain windows.
is_radio = [1 if b >= RADIO else 0 for b in ma]
radio = []
run = sum(is_radio[:100])
for i in range(n - 100):
    if run >= 20 and is_radio[i]:
        radio.append(i)
    run += is_radio[i + 100] - is_radio[i]
spikes = sum(is_radio) - len(radio)
print(f"isolated >= {RADIO:.0f} mA bins outside sustained radio: {spikes}")
windows = []
for i in radio:
    if windows and i - windows[-1][1] < 5000:
        windows[-1][1] = i
    else:
        windows.append([i, i])
windows = [(max(0, a - 3000), min(n, b + 3000)) for a, b in windows]
in_window = bytearray(n)
for a, b in windows:
    for i in range(a, b):
        in_window[i] = 1

# Awake bursts outside radio windows: (start ms, length ms, excess charge mA*ms).
bursts = []
i = 0
while i < n:
    if ma[i] >= AWAKE and not in_window[i]:
        j = i
        while j < n and ma[j] >= AWAKE * 0.6 and not in_window[j]:
            j += 1
        bursts.append((i, j - i, sum(ma[i:j]) - floor * (j - i)))
        i = j
    else:
        i += 1

starts = [b[0] for b in bursts]
idx = {s: k for k, s in enumerate(starts)}
import bisect


def has_neighbor(t, period, tol=8):
    for d in (period, -period):
        k = bisect.bisect_left(starts, t + d - tol)
        if k < len(starts) and abs(starts[k] - (t + d)) <= tol:
            return True
    return False


TOL = 4


def best_period(nominal):
    """The spacing near `nominal` (within 8%) at which most bursts recur."""
    best, score = nominal, -1
    for p in range(int(nominal * 0.92), int(nominal * 1.08) + 1):
        s = 0
        for t in starts:
            k = bisect.bisect_left(starts, t + p - TOL)
            if k < len(starts) and starts[k] <= t + p + TOL:
                s += 1
        if s > score:
            best, score = p, s
    return best, score


# Assign each burst to the first task period it repeats at, busiest task first
# so rarer periods don't claim coincidental matches.
PERIODS = [("input poll (500 ms)", 500), ("serial status (2 s)", 2000), ("battery (30 s)", 30000)]
assigned = {}
for name, nominal in PERIODS:
    p, score = best_period(nominal)
    print(f"{name}: period {p} ms ({score} bursts recur at it)")
    for t, length, q in bursts:
        if t not in assigned and has_neighbor(t, p, tol=TOL):
            assigned[t] = name

idle_s = (n - sum(in_window)) / 1000
rows = {}
for t, length, q in bursts:
    name = assigned.get(t, "other / unmatched")
    c, ms, qq = rows.get(name, (0, 0, 0.0))
    rows[name] = (c + 1, ms + length, qq + q)

cycle_charge = sum(sum(ma[a:b]) - floor * (b - a) for a, b in windows)

print(f"{total_s:.0f} s analysed, average {avg:.2f} mA, sleep floor {floor:.2f} mA")
print(f"{len(windows)} fetch cycles, {idle_s:.0f} s between them, {len(bursts)} awake bursts there")
print(f"{'part':24} {'per s':>7} {'ms/wake':>8} {'mA*ms/wake':>11} {'avg mA':>7}")
print(f"{'sleep floor (board+chip)':24} {'':>7} {'':>8} {'':>11} {floor:7.2f}")
for name, (c, ms, q) in sorted(rows.items(), key=lambda kv: -kv[1][2]):
    print(f"{name:24} {c / idle_s:7.2f} {ms / c:8.1f} {q / c:11.1f} {q / total_s / 1000:7.2f}")
print(f"{'fetch cycle (radio etc.)':24} {len(windows) / total_s:7.4f} {'':>8} {cycle_charge / len(windows):11.0f} {cycle_charge / total_s / 1000:7.2f}")
explained = floor + sum(q for _, _, q in rows.values()) / total_s / 1000 + cycle_charge / total_s / 1000
print(f"sum {explained:.2f} mA vs measured {avg:.2f} mA")
