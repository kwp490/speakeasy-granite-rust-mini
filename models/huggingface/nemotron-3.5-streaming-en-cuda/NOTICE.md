# Notice

The archive in this repository is a derived float32 ONNX export of NVIDIA's
`nemotron-3.5-asr-streaming-0.6b` checkpoint. The upstream checkpoint is
attributed to NVIDIA Corporation and is available under OpenMDW-1.1.

SpeakEasy created the export with:

- exporter: `scripts/model-export/export_onnx.py --chunk-size-ms 320`;
- NeMo revision: `23de54bb6d5c1c4abd97c8d51b8c20e91447dd32`;
- locked tool versions: `nemo-toolkit=3.1.0+23de54bb6`, `torch=2.9.0+cu129`,
  `onnx=1.22.0`;
- runtime target: sherpa-onnx 1.13.4, CUDA execution provider, Windows x86-64.

This repository is maintained by SpeakEasy's publisher account and is not an
official NVIDIA repository. The application verifies the archive length and
SHA-256 before using it.

The model license text is included in `LICENSE.OpenMDW-1.1`. The sherpa-onnx
runtime is separately licensed under Apache-2.0 by its authors.
