# SpeakEasy local model notice

SpeakEasy provisions one of two ASR packs for the live streaming display,
depending on whether this machine qualifies for GPU acceleration. Both are
`nemotron-3.5-asr-streaming-0.6b`:

## CPU pack (`nemotron-3.5-streaming-en-cpu`)

- upstream: `nvidia/nemotron-3.5-asr-streaming-0.6b`;
- revision: `f3d333391852ba876df169dcc9ba902d25b6ab0b`;
- distributed asset: sherpa-onnx's own int8 ONNX export,
  `sherpa-onnx-nemotron-3.5-asr-streaming-0.6b-560ms-int8-2026-06-11.tar.bz2`;
- archive SHA-256:
  `c6bf5e0df765f9d5b43bc9e0536d4b4b3e7d40bdf5ecf13e45f134c51c05ae3a`;
- model license: Open Model Weights License 1.1 (OpenMDW-1.1); runtime
  (sherpa-onnx) license: Apache-2.0;
- conversion: none; official sherpa-onnx quantized ONNX bytes;
- capability: English, punctuation, streaming transcription.

## GPU pack (`nemotron-3.5-streaming-en-cuda`)

- upstream: `nvidia/nemotron-3.5-asr-streaming-0.6b`;
- revision: `f3d333391852ba876df169dcc9ba902d25b6ab0b`;
- conversion: re-exported to float32 ONNX by
  `scripts/model-export/export_onnx.py`, a derived work of the upstream
  checkpoint. Upstream publishes this model as int8 only; float is required
  to run on the CUDA execution provider;
- archive SHA-256:
  `5f6a28fe4d33b038f5062dc064bc55260d35278e64c5f600d4908b5c524f1717`;
- hosted archive: [SpeakEasy Hugging Face model repository](https://huggingface.co/orangeblue39/nemotron-3.5-streaming-en-cuda), pinned at revision
  `bae0a819fa4f4bc0878f535509886455037f8f63`;
- model license: Open Model Weights License 1.1 (OpenMDW-1.1); runtime
  (sherpa-onnx) license: Apache-2.0;
- the archive is a derived SpeakEasy export, not an official NVIDIA release;
  the OpenMDW-1.1 text is retained at `packaging/licenses/OpenMDW-1.1.txt`;
- capability: English, punctuation, streaming transcription.

Exact file lengths and SHA-256 values are in `models/trusted-manifest.json`
and are verified before activation.

## Delivered-transcript pack: Granite Speech 4.1 2B (`granite-speech-4.1-2b-q4_k_m-cpu`)

Layered on top of the streaming pack above: Nemotron drives the live HUD
while the user talks, and on hotkey release Granite re-transcribes the
retained audio for the text that is actually delivered. Not bundled with
this installer -- fetched on demand by `scripts/Get-Granite.ps1` and
verified against the pin below before every use.

- upstream: `ibm-granite/granite-speech-4.1-2b-GGUF`;
- upstream revision: `8267dad2adc84209b0efd2702ec68a98356125eb`;
- distributed assets: `granite-speech-4.1-2b-Q4_K_M.gguf` (the language model)
  and `mmproj-model-f16.gguf` (the shared speech encoder/projector);
- archive SHA-256: `d18e3e79826c4f0fa6734eb05d2db3f06baccbcd5791a83653f946b3178b35d8`
  (model) and `0d3615076cbe1d35c3f60c43a60a4047b3e2eeee1b2c233580be60186faab5c5`
  (projector);
- model license: Apache License 2.0 (IBM Corporation); runtime (llama.cpp,
  a cherry-pick fork) license: MIT -- see `THIRD-PARTY-NOTICES.txt`;
- conversion: none; IBM's own published GGUF conversion bytes, verified
  against Hugging Face's own file-tree API and redistribution headers;
- capability: English, punctuation, grammar/casing cleanup, offline
  transcription of the retained utterance.

A recorded alternative quantization, `granite-speech-4.1-2b-q8_0-cpu`, is in
the manifest but not install-eligible. Q4_K_M replaced it as the shipped
quantization on 2026-08-04, on measurement rather than by decision: ~21%
faster on a 120 s utterance with an identical transcript, and byte-identical
on the pinned fixture (see `docs/handoff/granite-final-pass.md`, Phase 8 and
Phase 9).
