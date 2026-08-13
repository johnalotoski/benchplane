# ADR 0005: explicit fixed llama.cpp execution target

## Status

Accepted.

## Decision

Represent llama.cpp CPU versus NVIDIA/CUDA intent with the closed public `llamaCpp.target` value. `cpu` remains the default and is omitted from deterministic serialization so existing CPU plan identities remain stable. `nvidiaCuda` is executable only from the `x86_64-linux` package containing Benchplane's separate fixed CUDA helper/backend.

Keep device and offload policy outside experiment control. The NVIDIA helper selects logical CUDA device 0, supplies only that device to llama.cpp, disables split mode, requests every model layer on GPU, and requires observed complete offload before success. Successful attempt provenance records bounded NVIDIA device/driver/CUDA/offload facts; it excludes UUID, serial, and PCI identity. CPU and NVIDIA targets are comparison-incompatible, while environment differences between otherwise matching NVIDIA runs remain context.

## Consequences

One resolved experiment can no longer silently change between CPU and CUDA according to the installed package or host. Unsupported package/platform combinations reject before run allocation; a missing, malformed, older-than-575.57.08, or otherwise unusable host driver/device fails through the allocated runtime lifecycle. The helper checks that conservative native CUDA 12.9 Update 1 driver floor before backend/CUDA initialization. The host NVIDIA driver/device interface remains a mutable dependency outside the Nix closure.

The design deliberately provides no arbitrary GPU index, multi-GPU split, backend selector, offload control, accelerator abstraction, GPU telemetry, or CPU-versus-GPU speedup analysis. Ordinary CI can prove schema, package, no-fallback, provenance/verifier/comparison, and NixOS policy behavior without claiming real CUDA execution; hardware acceptance remains an explicit host procedure.
