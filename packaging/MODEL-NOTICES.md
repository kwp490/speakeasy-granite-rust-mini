# SpeakEasy Mini local model notice

SpeakEasy Mini provisions exactly one speech model. There is no streaming pack
and no second engine: IBM Granite Speech runs once over the retained recording
after the user stops it, and that single pass produces the transcript, its
punctuation and its casing together.

This notice used to describe two NVIDIA Nemotron packs — a CPU pack and a
re-exported CUDA pack — that drove a live transcription display. Both left with
the streaming engine when this fork was created, and neither appears in
`models/trusted-manifest.json` any more.

## Speech model: Granite Speech 4.1 2B (`granite-speech-4.1-2b-q4_k_m-cpu`)

Not bundled with this installer — fetched on demand and verified against the
pins below before every use. No model bytes ship inside the installer.

- upstream: `ibm-granite/granite-speech-4.1-2b-GGUF`;
- upstream revision: `8267dad2adc84209b0efd2702ec68a98356125eb`;
- distributed assets: `granite-speech-4.1-2b-Q4_K_M.gguf` (the language model)
  and `mmproj-model-f16.gguf` (the speech encoder/projector);
- SHA-256: `d18e3e79826c4f0fa6734eb05d2db3f06baccbcd5791a83653f946b3178b35d8`
  (model) and `0d3615076cbe1d35c3f60c43a60a4047b3e2eeee1b2c233580be60186faab5c5`
  (projector);
- model license: Apache License 2.0 (IBM Corporation); runtime (llama.cpp,
  a cherry-pick fork) license: MIT — see `THIRD-PARTY-NOTICES.txt`;
- conversion: none; IBM's own published GGUF conversion bytes, verified
  against Hugging Face's own file-tree API and redistribution headers;
- capability: English, punctuation, casing, offline transcription of the
  retained utterance.

A recorded alternative quantization, `granite-speech-4.1-2b-q8_0-cpu`, is in
the manifest but is deliberately not install-eligible. Q4_K_M replaced it as
the shipped quantization on 2026-08-04, on measurement rather than by decision:
~21% faster on a 120 s utterance with an identical transcript, and
byte-identical on the pinned fixture.

Exact file lengths and SHA-256 values are in `models/trusted-manifest.json` and
are verified before activation.
