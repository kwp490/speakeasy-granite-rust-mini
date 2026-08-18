---
language:
- en
license: openmdw-1.1
library_name: onnxruntime
tags:
- automatic-speech-recognition
- streaming-asr
- onnx
- cuda
- sherpa-onnx
- nemotron
---

# SpeakEasy Nemotron 3.5 Streaming English — CUDA

This repository contains the SpeakEasy-derived float32 ONNX pack used by the
Windows CUDA streaming-ASR path. It is an application-specific export, not an
official NVIDIA ONNX release and is not endorsed by NVIDIA.

The upstream checkpoint is NVIDIA's
[`nemotron-3.5-asr-streaming-0.6b`](https://huggingface.co/nvidia/nemotron-3.5-asr-streaming-0.6b),
pinned to revision `f3d333391852ba876df169dcc9ba902d25b6ab0b`. SpeakEasy exported
the 320 ms float32 ONNX files with the pinned environment in
[`scripts/model-export`](https://github.com/kwp490/speakeasy-granite-rust/tree/transcriber-record-button/scripts/model-export)
using NeMo revision `23de54bb6d5c1c4abd97c8d51b8c20e91447dd32`.

## Downloaded pack

The application downloads this archive and verifies its length and SHA-256
before extraction or activation:

| File | Bytes | SHA-256 |
|---|---:|---|
| `nemotron-3.5-streaming-en-cuda-320ms-fp32.tar.gz` | 2406690010 | `5f6a28fe4d33b038f5062dc064bc55260d35278e64c5f600d4908b5c524f1717` |

The archive contains:

- `encoder.onnx` — `42247579` bytes — `288271e199371ea7fbd0ba7e246f243d4e7685a7670618194698c19e030967a1`
- `encoder.data` — `2454405120` bytes — `7584f85df76bc9ae6fbdfa53aa8d97b07a842525d1c501d536d77fd9e4f57ac7`
- `decoder.onnx` — `59764943` bytes — `ab1934b40bffacfc4fd6795fb1b5c6531cd4403cbf434e0144f69fc8aba3f8c2`
- `joiner.onnx` — `37824290` bytes — `02dd5450d1ca9c541ceb037b94b09a9b262b4a4c22c240a97537f59ac250d338`
- `tokens.txt` — `144528` bytes — `32be3ebfabfff475d64d7829b435f1c7856a1c497907def5c41d54ca9f1eccfd`

The pack targets sherpa-onnx 1.13.4, the CUDA execution provider, Windows
x86-64, 16 kHz English transcription, and true-online transducer decoding.

## License and notices

The model materials are provided under [OpenMDW-1.1](./LICENSE.OpenMDW-1.1).
The associated sherpa-onnx runtime is Apache-2.0. See
[`NOTICE.md`](./NOTICE.md) for attribution and derivation details.

This export is distributed without user audio or application source code.
