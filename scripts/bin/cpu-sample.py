#!/usr/bin/env python3
"""cpu-sample.py - 1s-resolution CPU sampler for ONE process (P0 perf work).

Acceptance metric: the process's utime+stime PEAK over consecutive
1-second buckets ("max1s") must stay < 1% of ONE core; the window avg
and a per-thread expansion come from the same run.

    scripts/bin/cpu-sample.py --pid PID --seconds N
        [--interval 0.1] [--json] [--top-threads 5]
        [--thread-max1s SPEC]

--thread-max1s tracks ONE thread for the same window and reports its own
peak 1-second bucket. SPEC is a tid (all digits) or a comm; a comm that
matches several threads is an error (pass the tid). The app's MAIN thread
is tid == pid, so `--thread-max1s PID` isolates app-side work from the
GPU driver's rasterizer threads - on a software-rendering host (llvmpipe
under Xvfb) the process-wide number is dominated by them.

Data source: /proc/<pid>/stat fields 14 (utime) + 15 (stime) in clock
ticks (os.sysconf('SC_CLK_TCK')); threads from /proc/<pid>/task/*/stat
(comm = field 2, the kernel truncates it to 15 chars).

Exit codes: 0 ok - also when the process exits mid-sample (partial
results are printed plus a note on stderr); 2 pid not found or
unreadable; 130 SIGINT (partial results printed).

stdlib only: os, sys, time, json, argparse, signal. No classes.
"""

import argparse
import json
import os
import signal
import sys
import time

# Set by the SIGINT handler; the sampling loop stops on the next tick.
_INTERRUPTED = False
# Buckets covering less wall time than this are dropped from max1s:
# a 20ms sliver at the window edge is not a 1-second observation.
MIN_BUCKET_COVERAGE = 0.5


def clock_ticks_per_second():
    """CLK_TCK (100 on every mainstream kernel); 100 as a fallback."""
    try:
        return int(os.sysconf("SC_CLK_TCK"))
    except (ValueError, OSError):
        return 100


def parse_stat(text):
    """(comm, utime, stime) from one /proc/<pid>/stat line, or None.

    comm is parenthesised and may contain spaces and parens -> cut at
    the LAST ')'. The remainder starts at field 3, so utime/stime
    (fields 14/15) are indices 11/12 after splitting.
    """
    try:
        comm = text[text.index("(") + 1:text.rindex(")")]
        rest = text[text.rindex(")") + 2:].split()
        return comm, int(rest[11]), int(rest[12])
    except (ValueError, IndexError):
        return None


def read_ticks(pid):
    """Total cpu ticks (utime + stime) for pid, or None when it is gone."""
    try:
        with open("/proc/%d/stat" % pid, "rb") as fh:
            parsed = parse_stat(fh.read().decode("utf-8", "replace"))
    except OSError:
        return None
    return None if parsed is None else parsed[1] + parsed[2]


def read_tid_ticks(pid, tid):
    """Total cpu ticks for ONE thread (`tid == pid` is the main thread).

    NOTE: /proc/<pid>/stat is PROCESS-wide (the sum over threads); the
    per-thread value only lives at /proc/<pid>/task/<tid>/stat, so the
    main thread must be read through the task path even when tid == pid.
    """
    try:
        with open("/proc/%d/task/%d/stat" % (pid, tid), "rb") as fh:
            parsed = parse_stat(fh.read().decode("utf-8", "replace"))
    except OSError:
        return None
    return None if parsed is None else parsed[1] + parsed[2]


def read_threads(pid):
    """{tid: (comm, total ticks)} across /proc/<pid>/task/*/stat.

    Threads that vanish between the two reads simply drop out.
    """
    out = {}
    try:
        tids = os.listdir("/proc/%d/task" % pid)
    except OSError:
        return out
    for tid in tids:
        try:
            with open("/proc/%d/task/%s/stat" % (pid, tid), "rb") as fh:
                parsed = parse_stat(fh.read().decode("utf-8", "replace"))
        except OSError:
            continue
        if parsed:
            out[int(tid)] = (parsed[0], parsed[1] + parsed[2])
    return out


def bucket_percents(samples, hz):
    """[(bucket_idx, pct, coverage_s)] from [(t_seconds, ticks)] pairs.

    Consecutive samples bound intervals; each interval's cpu seconds are
    distributed proportionally over the integer wall-second buckets it
    spans, so a bucket's pct = bucket_cpu_seconds / covered_wall_seconds
    * 100 = percent of ONE core (a saturated core reads 100.0).
    """
    wall, cpu = {}, {}
    for (t0, c0), (t1, c1) in zip(samples, samples[1:]):
        dt, dtc = t1 - t0, c1 - c0
        if dt <= 0.0 or dtc < 0:
            continue  # non-monotonic clock or a tick counter reset
        b = int(t0)
        while t0 < t1 - 1e-9:
            end = min(t1, b + 1.0)
            span = end - t0
            wall[b] = wall.get(b, 0.0) + span
            cpu[b] = cpu.get(b, 0.0) + dtc / hz * (span / dt)
            t0, b = end, b + 1
    return [(b, cpu[b] / wall[b] * 100.0, wall[b])
            for b in sorted(wall) if wall[b] > 0.0]


def collect(pid, seconds, interval, tid=None):
    """(samples, thread_samples, exited): one sample per interval.

    thread_samples is [] without `tid`; otherwise it follows that one
    thread over the same instants as the process series.
    """
    global _INTERRUPTED
    t0 = time.monotonic()
    first = read_ticks(pid)
    exited = first is None
    samples = [(t0, first or 0)]
    thread = [(t0, read_tid_ticks(pid, tid) or 0)] if tid is not None else []
    deadline = t0 + seconds
    k = 1
    while not exited and not _INTERRUPTED:
        due = t0 + k * interval
        if due > deadline:
            break
        now = time.monotonic()
        if due > now:
            time.sleep(due - now)
        ticks = read_ticks(pid)
        if ticks is None:
            exited = True
            break
        now = time.monotonic()
        samples.append((now, ticks))
        if tid is not None:
            thread.append((now, read_tid_ticks(pid, tid) or 0))
        k += 1
    return samples, thread, exited


def pct_series(samples, hz):
    """(avg_pct, max1s_pct) for one [(t, ticks)] series."""
    elapsed = max(samples[-1][0] - samples[0][0], 1e-9)
    ticks = max(samples[-1][1] - samples[0][1], 0)
    buckets = bucket_percents(samples, hz)
    full = [pct for _b, pct, cov in buckets if cov >= MIN_BUCKET_COVERAGE]
    if not full:
        full = [pct for _b, pct, _cov in buckets]
    avg = round(ticks / hz / elapsed * 100.0, 3)
    peak = round(max(full) if full else 0.0, 3)
    return avg, peak


def summarize(pid, samples, start_threads, end_threads, hz,
              thread_tid=None, thread_samples=None):
    """The JSON-shaped result dict (threads sorted by pct desc)."""
    avg_pct, max1s_pct = pct_series(samples, hz)
    elapsed = max(samples[-1][0] - samples[0][0], 1e-9)
    ticks = max(samples[-1][1] - samples[0][1], 0)
    threads = []
    for tid, (comm, end) in end_threads.items():
        start = start_threads.get(tid, (comm, 0))[1]
        delta = max(end - start, 0)
        threads.append({"tid": tid, "comm": comm,
                        "pct": round(delta / hz / elapsed * 100.0, 3)})
    threads.sort(key=lambda t: (-t["pct"], t["tid"]))
    out = {
        "pid": pid,
        "seconds": round(elapsed, 3),
        "avg_pct": avg_pct,
        "max1s_pct": max1s_pct,
        "ticks_total": ticks,
        "threads": threads,
    }
    if thread_tid is not None and thread_samples:
        tavg, tpeak = pct_series(thread_samples, hz)
        comm = end_threads.get(thread_tid, ("?", 0))[0]
        out["thread_max1s"] = {
            "tid": thread_tid,
            "comm": comm,
            "avg_pct": tavg,
            "max1s_pct": tpeak,
        }
    return out


def print_human(p, top, hz):
    print("pid %d: %d ticks total (%.3fs cpu) over %.3fs"
          % (p["pid"], p["ticks_total"], p["ticks_total"] / hz, p["seconds"]))
    print("avg:   %.1f%% of one core" % p["avg_pct"])
    print("max1s: %.1f%% of one core (peak 1-second bucket)" % p["max1s_pct"])
    thread = p.get("thread_max1s")
    if thread:
        print("thread %d (%s): avg %.2f%%, max1s %.2f%%"
              % (thread["tid"], thread["comm"],
                 thread["avg_pct"], thread["max1s_pct"]))
    threads = p["threads"][:top] if top > 0 else []
    if threads:
        print("top %d threads (task/*/stat comm over the same window):"
              % len(threads))
        for t in threads:
            print("  tid %-7d %-16s %6.2f%%" % (t["tid"], t["comm"], t["pct"]))
    else:
        print("threads: none readable (process gone?)")


def resolve_thread(pid, spec):
    """(tid, comm) for `spec` (digits = tid, else a unique comm), or None."""
    if spec.isdigit():
        tid = int(spec)
        comm = read_threads(pid).get(tid, ("?", 0))[0]
        return tid, comm
    hits = [(tid, comm) for tid, (comm, _t) in read_threads(pid).items()
            if comm == spec]
    if len(hits) != 1:
        return hits
    return hits[0]


def _on_sigint(_signum, _frame):
    global _INTERRUPTED
    _INTERRUPTED = True


def main(argv=None):
    ap = argparse.ArgumentParser(
        description="Sample one process's utime+stime (1s-bucket peak).")
    ap.add_argument("--pid", type=int, required=True)
    ap.add_argument("--seconds", type=float, required=True)
    ap.add_argument("--interval", type=float, default=0.1)
    ap.add_argument("--json", action="store_true",
                    help="print one JSON object on stdout")
    ap.add_argument("--top-threads", type=int, default=5)
    ap.add_argument("--thread-max1s", metavar="SPEC",
                    help="follow ONE thread (tid, or a unique comm) for "
                         "its own avg/max1s; use the pid for the main thread")
    args = ap.parse_args(argv)
    if args.seconds <= 0 or args.interval <= 0 or args.interval > args.seconds:
        ap.error("need 0 < --interval <= --seconds")

    signal.signal(signal.SIGINT, _on_sigint)
    hz = clock_ticks_per_second()
    if read_ticks(args.pid) is None:
        print("cpu-sample: pid %d not found (or /proc/%d unreadable)"
              % (args.pid, args.pid), file=sys.stderr)
        return 2

    tid = None
    if args.thread_max1s:
        got = resolve_thread(args.pid, args.thread_max1s)
        if isinstance(got, list):
            tids = ", ".join("%d (%s)" % (t, c) for t, c in got) or "none"
            print("cpu-sample: --thread-max1s '%s' matches %d threads: %s - "
                  "pass a tid" % (args.thread_max1s, len(got), tids),
                  file=sys.stderr)
            return 2
        tid = got[0]
    start_threads = read_threads(args.pid)
    samples, thread_samples, exited = collect(
        args.pid, args.seconds, args.interval, tid)
    end_threads = read_threads(args.pid)
    summary = summarize(args.pid, samples, start_threads, end_threads, hz,
                        tid, thread_samples)
    if exited:
        print("note: pid %d exited mid-sample; partial results below"
              % args.pid, file=sys.stderr)
    if _INTERRUPTED:
        print("note: interrupted (SIGINT); partial results below",
              file=sys.stderr)
    if args.json:
        print(json.dumps(summary))
    else:
        print_human(summary, args.top_threads, hz)
    return 130 if _INTERRUPTED else 0


if __name__ == "__main__":
    sys.exit(main())
