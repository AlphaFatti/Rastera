#!/usr/bin/env python3
"""Separate CPU callback gaps from Metal's reported presentedTime gaps.

Sort presentation timestamps offline (callbacks may arrive out of order), reject
zero/nonfinite/unavailable timestamps, and never substitute callback arrival time.
"""
import argparse
import json
import math
from pathlib import Path


def parse_log(path):
    rows = []
    for line in path.read_text().splitlines():
        parts = line.split()
        if not parts:
            continue
        row = dict(part.split("=", 1) for part in parts[1:] if "=" in part)
        row["record_type"] = parts[0]
        rows.append(row)
    return rows


def max_gap(values):
    values = sorted(values)
    return max((b-a for a, b in zip(values, values[1:])), default=None)


def ms(value):
    return None if value is None else round(value*1000, 6)


def summarize(path):
    rows = parse_log(path)
    events = [json.loads(line) for line in path.with_suffix(".events.jsonl").read_text().splitlines()]
    anchor = next(r for r in rows if r["record_type"] == "clock_anchor")
    mono_epoch = int(anchor["unix_us"])-int(anchor["mono_us"])
    begin = next(e["unix_us"] for e in events if e["event"] == "measure_start")
    end = next(e["unix_us"] for e in events if e["event"] == "measure_end")
    if path.stem.endswith("-egui"):
        # Leave the original driver unchanged; cycles 2/3 occur after layout.
        begin = max(begin, mono_epoch+8_000_000)
    result = dict(case=path.stem, measured_seconds=round((end-begin)/1e6, 3))
    view_rows = [r for r in rows if r["record_type"] == "viewport" and
                 begin <= mono_epoch+int(r["max_gap_start_us"]) and
                 mono_epoch+int(r["max_gap_start_us"])+int(r["max_gap_us"]) <= end]
    result["ui_gap_ms"] = {name: max((int(r["max_gap_us"])/1000 for r in view_rows if r["name"] == name), default=None)
                           for name in ["operator", "left", "right"]}
    native = next((r for r in rows if r["record_type"] == "native_clock_anchor"), None)
    if not native:
        result["presentation_gap_ms"] = None
        result["presentation_status"] = "not instrumented in baseline"
        return result
    host_epoch = float(native["host_s"])-int(native["unix_us"])/1e6
    lo, hi = begin/1e6+host_epoch, end/1e6+host_epoch
    records = [r for r in rows if r["record_type"] == "native_event"]
    measured = lambda r, key: lo <= float(r[key]) <= hi
    scheduled = [float(r["callback_host_s"]) for r in records if r["kind"] == "schedule" and measured(r, "callback_host_s")]
    complete = [float(r["callback_host_s"]) for r in records if r["kind"] == "complete" and measured(r, "callback_host_s")]
    gpu_end = [float(r["gpu_end_host_s"]) for r in records if r["kind"] == "complete" and
               float(r["gpu_end_host_s"]) > 0 and measured(r, "gpu_end_host_s")]
    handlers = [r for r in records if r["kind"] == "presented_handler"]
    valid = sorted((float(r["presented_host_s"]) for r in handlers
                    if r["timestamp_valid"] == "1" and math.isfinite(float(r["presented_host_s"])) and float(r["presented_host_s"]) > 0))
    presented = [t for t in valid if lo <= t <= hi]
    arrivals = [float(r["callback_host_s"]) for r in handlers if measured(r, "callback_host_s")]
    zeros = sum(float(r["presented_host_s"]) == 0.0 for r in handlers if measured(r, "callback_host_s"))
    result.update(schedule_gap_ms=ms(max_gap(scheduled)), complete_callback_gap_ms=ms(max_gap(complete)),
                  gpu_completion_gap_ms=ms(max_gap(gpu_end)), presented_callback_gap_ms=ms(max_gap(arrivals)),
                  presentation_gap_ms=ms(max_gap(presented)), valid_presentation_samples=len(presented),
                  zero_timestamp_callbacks_in_interval=zeros,
                  presentation_status="Metal-reported timestamps; no physical scanout measurement" if presented else "unavailable")
    for kind in ["commit", "present_call"]:
        calls = [float(r["call_us"]) for r in records if r["kind"] == kind and measured(r, "return_host_s")]
        result[kind+"_max_us"] = max(calls, default=None)
    intervals = [r for r in rows if r["record_type"] == "native_interval"]
    result["max_inflight"] = max(int(r["max_inflight"]) for r in intervals)
    result["lost_events"] = max(int(r["lost_events"]) for r in intervals)
    result["errors"] = max(int(r["errors"]) for r in intervals)
    result["total_zero_timestamps"] = max(int(r["zero_time"]) for r in intervals)
    result["total_unavailable_timestamps"] = max(int(r["unavailable_time"]) for r in intervals)
    result["backpressure"] = max(int(r["backpressure"]) for r in intervals)
    if result["lost_events"]:
        result["presentation_status"] = "incomplete trace: events lost"
    result["operator_pauses"] = []
    for r in view_rows:
        if r["name"] != "operator" or int(r["max_gap_us"]) < 100_000:
            continue
        start = (mono_epoch+int(r["max_gap_start_us"]))/1e6+host_epoch
        finish = start+int(r["max_gap_us"])/1e6
        surrounding = [t for t in valid if start-0.1 <= t <= finish+0.1]
        result["operator_pauses"].append(dict(ui_start_us=int(r["max_gap_start_us"]), ui_gap_ms=int(r["max_gap_us"])/1000,
                                             presentation_samples_during_gap=sum(start <= t <= finish for t in valid),
                                             native_schedule_callbacks_during_gap=sum(start <= float(e["callback_host_s"]) <= finish for e in records if e["kind"] == "schedule"),
                                             native_completion_callbacks_during_gap=sum(start <= float(e["callback_host_s"]) <= finish for e in records if e["kind"] == "complete"),
                                             zero_presentation_callbacks_during_gap=sum(start <= float(e["callback_host_s"]) <= finish and float(e["presented_host_s"]) == 0 for e in handlers),
                                             surrounding_presentation_gap_ms=ms(max_gap(surrounding))))
    result["content_accounting"] = content_accounting(rows)
    result["native_stop"] = [r for r in rows if r["record_type"] == "native_stop"]
    return result


def content_accounting(rows):
    """Whole trace joins retain missing callbacks and unknown presentation outcomes."""
    links = [r for r in rows if r["record_type"] == "native_content_link"]
    stages = {}
    for r in links:
        stages.setdefault(r["kind"], {})[r["seq"]] = r
    encoded = stages.get("content", {})
    committed = stages.get("commit", {})
    handlers = stages.get("presented", {})
    valid = {r["seq"] for r in rows if r["record_type"] == "native_event" and
             r.get("kind") == "presented_handler" and r.get("timestamp_valid") == "1"}
    mismatch = [r for r in links if r["seq"] in encoded and
                (r["content_id"],r["drawable_id"]) !=
                (encoded[r["seq"]]["content_id"],encoded[r["seq"]]["drawable_id"])]
    return dict(encoded=len(encoded), committed=len(committed),
                missing_completion=sorted(set(committed)-set(stages.get("complete", {})), key=int),
                missing_present_call=sorted(set(committed)-set(stages.get("present_call", {})), key=int),
                missing_presented_handler=sorted(set(committed)-set(handlers), key=int),
                without_valid_presentation=sorted(set(committed)-valid, key=int),
                content_link_mismatches=len(mismatch),
                note="Whole-process records; missing/invalid presentation is unknown, not proof of display or GPU deadlock")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    args = parser.parse_args()
    for path in sorted(args.directory.glob("*.log")):
        try:
            print(json.dumps(summarize(path), sort_keys=True))
        except (StopIteration, KeyError) as error:
            print(json.dumps(dict(case=path.stem, error="incomplete run", detail=str(error))))


if __name__ == "__main__":
    main()
