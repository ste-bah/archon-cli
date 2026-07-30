"""Measure Marker's real GPU memory footprint via the sidecar's run_marker.
Usage: python mem_probe.py <pdf> <cuda|mps>
Reports torch allocator peak (the footprint the placement planner cares about) +,
on CUDA, the nvidia-smi process view (which includes the CUDA context overhead)."""
import os
import sys

import torch

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from archon_marker_sidecar import run_marker  # noqa: E402

pdf, device = sys.argv[1], sys.argv[2]

if device == "cuda":
    torch.cuda.init()
    torch.cuda.reset_peak_memory_stats()
    total = torch.cuda.get_device_properties(0).total_memory / 1048576
    _ = run_marker(pdf, "cuda")
    print("device:", torch.cuda.get_device_name(0))
    print("total_vram_mb:", round(total))
    print("peak_alloc_mb:", round(torch.cuda.max_memory_allocated() / 1048576))
    print("peak_reserved_mb:", round(torch.cuda.max_memory_reserved() / 1048576))
    import subprocess

    pid = os.getpid()
    out = subprocess.run(
        ["nvidia-smi", "--query-compute-apps=pid,used_memory",
         "--format=csv,noheader,nounits"],
        capture_output=True, text=True,
    )
    mine = [l for l in out.stdout.splitlines() if l.strip().startswith(str(pid))]
    print("nvidia_smi_this_proc_mb:", mine[0].split(",")[-1].strip() if mine else "n/a")
else:  # mps
    _ = run_marker(pdf, "mps")
    print("device: mps (Apple unified)")
    print("mps_current_alloc_mb:", round(torch.mps.current_allocated_memory() / 1048576))
    try:
        print("mps_driver_alloc_mb:", round(torch.mps.driver_allocated_memory() / 1048576))
    except Exception as e:
        print("mps_driver_alloc_mb: n/a", e)
