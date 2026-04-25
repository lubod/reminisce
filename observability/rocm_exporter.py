#!/usr/bin/env python3
"""Minimal Prometheus exporter for AMD ROCm GPU metrics via rocm-smi."""
import json
import subprocess
import time
from http.server import BaseHTTPRequestHandler, HTTPServer

PORT = 9400


def rocm_metrics():
    try:
        out = subprocess.check_output(
            ["/opt/rocm/bin/rocm-smi", "--json",
             "--showuse", "--showmemuse", "--showtemp"],
            timeout=10, stderr=subprocess.DEVNULL,
        )
        data = json.loads(out)
    except Exception:
        return []

    lines = []
    for card, info in data.items():
        if not card.startswith("card"):
            continue
        gpu = card.lstrip("card")
        labels = f'gpu="{gpu}"'

        util = info.get("GPU use (%)", info.get("GPU Use (%)", None))
        if util is not None:
            lines.append(f"gpu_utilization_percent{{{labels}}} {float(util)}")

        mem_used = info.get("GTT Memory Used (B)", info.get("VRAM Memory Used (B)", None))
        if mem_used is not None:
            lines.append(f"gpu_memory_used_bytes{{{labels}}} {float(mem_used)}")

        mem_total = info.get("GTT Memory Total (B)", info.get("VRAM Memory Total (B)", None))
        if mem_total is not None:
            lines.append(f"gpu_memory_total_bytes{{{labels}}} {float(mem_total)}")

        temp = info.get("Temperature (Sensor edge) (C)", info.get("Temperature (C)", None))
        if temp is not None:
            lines.append(f"gpu_temperature_celsius{{{labels}}} {float(temp)}")

    return lines


HELP = """\
# HELP gpu_utilization_percent GPU compute utilization percentage
# TYPE gpu_utilization_percent gauge
# HELP gpu_memory_used_bytes GPU memory used in bytes
# TYPE gpu_memory_used_bytes gauge
# HELP gpu_memory_total_bytes GPU memory total in bytes
# TYPE gpu_memory_total_bytes gauge
# HELP gpu_temperature_celsius GPU edge temperature in Celsius
# TYPE gpu_temperature_celsius gauge
"""


class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        if self.path != "/metrics":
            self.send_response(404)
            self.end_headers()
            return
        body = HELP + "\n".join(rocm_metrics()) + "\n"
        self.send_response(200)
        self.send_header("Content-Type", "text/plain; version=0.0.4")
        self.end_headers()
        self.wfile.write(body.encode())

    def log_message(self, *_):
        pass


if __name__ == "__main__":
    print(f"ROCm exporter listening on :{PORT}/metrics", flush=True)
    HTTPServer(("", PORT), Handler).serve_forever()
