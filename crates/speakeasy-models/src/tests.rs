use crate::{
    Architecture, CompatibilityContext, CompatibilityIssue, ExactPackRequest, ExecutionProvider,
    ManifestError, ManifestStatus, PackRole, Platform, RoleSelectionError, RuntimeEvidence,
    RuntimeState, SelectionError, Task, TrustedManifest, bundled_manifest,
};
use semver::Version;
use serde_json::{Value, json};

#[test]
fn runtime_ready_requires_production_smoke_transition() {
    let smoke = RuntimeState::VerifiedOnDisk
        .transition(RuntimeState::RuntimeSmokeTesting)
        .expect("begin runtime smoke");
    let ready = smoke
        .transition(RuntimeState::Ready(RuntimeEvidence {
            artifact_id: "synthetic-asr-runtime-0.0.69".to_owned(),
            runtime_abi: "0.0.69".to_owned(),
            provider: "onnxruntime-cpu".to_owned(),
            inference_sample_count: 16_000,
        }))
        .expect("production smoke can admit runtime");
    assert!(matches!(ready, RuntimeState::Ready(_)));
    assert!(
        RuntimeState::VerifiedOnDisk
            .transition(RuntimeState::Ready(RuntimeEvidence {
                artifact_id: "fake".to_owned(),
                runtime_abi: "fake".to_owned(),
                provider: "fake".to_owned(),
                inference_sample_count: 0,
            }))
            .is_err()
    );
}

fn valid_catalog() -> Value {
    json!({
        "schema_version": 3,
        "manifest_status": "admitted-catalog",
        "generated_utc": "2026-07-19T20:00:00Z",
        "install_eligible": true,
        "artifacts": [],
        "packs": [{
            "id": "synthetic-streaming-en",
            "revision": "pack-r1",
            "display_name": "Synthetic Streaming English",
            "role": "streaming-asr",
            "streaming": "true-online",
            "install_eligible": true,
            "source": {
                "upstream_repository": "https://example.test/upstream",
                "upstream_revision": "a".repeat(40),
                "conversion": {
                    "repository": "https://example.test/conversion",
                    "revision": "b".repeat(40),
                    "command": "convert --exact-options",
                    "tool_versions": ["converter=1.2.3", "onnx=1.2.3"],
                    "provenance": "Synthetic deterministic test fixture"
                }
            },
            "archive": {
                "url": "https://example.test/synthetic-streaming-en-pack-r1.tar.zst",
                "bytes": 900,
                "sha256": "c".repeat(64)
            },
            "installed_bytes": 1200,
            "required_files": [{
                "path": "model/encoder.onnx",
                "bytes": 1000,
                "sha256": "d".repeat(64)
            }],
            "runtime": {
                "name": "synthetic-runtime",
                "version": "1.2.3",
                "abi": "synthetic-abi-1",
                "provider": "cpu",
                "platform": "windows",
                "architecture": "x86-64",
                "decoder": "modified-beam-search",
                "sample_rate_hz": 16000
            },
            "memory_evidence": [{
                "evidence_id": "synthetic-run-1",
                "private_working_set_min_bytes": 1_000_000,
                "private_working_set_max_bytes": 2_000_000,
                "vram_min_bytes": null,
                "vram_max_bytes": null
            }],
            "capabilities": [
                {
                    "locale": "en-US",
                    "task": "transcribe",
                    "target_locale": null,
                    "features": ["punctuation", "timestamps", "known-locale"]
                },
                {
                    "locale": "en-GB",
                    "task": "transcribe",
                    "target_locale": null,
                    "features": ["punctuation", "known-locale"]
                }
            ],
            "licenses": [{
                "component": "synthetic-model",
                "spdx_id": "MIT",
                "name": "MIT License",
                "text_url": "https://example.test/license",
                "attribution": "Synthetic fixture authors",
                "modification_notice": "No modifications",
                "redistribution": "allowed"
            }],
            "compatibility": {
                "minimum_application_version": "0.1.0",
                "maximum_application_version": "0.2.0",
                "minimum_worker_version": "0.1.0",
                "maximum_worker_version": "0.2.0"
            },
            "variant_group": "synthetic-streaming-en",
            "chunk_size_ms": 560
        }],
        "limitations": ["Synthetic test catalog only"]
    })
}

fn parse(value: &Value) -> Result<TrustedManifest, ManifestError> {
    TrustedManifest::parse_bundled(value.to_string().as_bytes())
}

fn compatible_context() -> CompatibilityContext {
    CompatibilityContext {
        application_version: Version::new(0, 1, 0),
        worker_version: Version::new(0, 1, 0),
        platform: Platform::Windows,
        architecture: Architecture::X86_64,
        provider: ExecutionProvider::Cpu,
    }
}

#[test]
fn bundled_packs_declare_no_version_ceiling() {
    // Every shipped pack used to cap `maximum_application_version` and
    // `maximum_worker_version` at the then-current product version, which turned
    // an ordinary version bump into a silent, total failure: `select_exact`
    // returns `SelectionError::Incompatible` for an out-of-range pack, so a build
    // one patch above the ceiling installed cleanly, started, and then could not
    // select any ASR pack. Nothing failed at build time — the ceiling was data,
    // and the only check on it lived in a packaging script.
    //
    // A ceiling is still expressible for a pack genuinely known to break above
    // some version. It must never again be the default, so assert the absence
    // here rather than trusting whoever next authors a pack to remember why.
    let manifest = bundled_manifest().expect("bundled manifest must validate");
    let ceilinged: Vec<&str> = manifest
        .packs()
        .iter()
        .filter(|pack| {
            pack.compatibility().maximum_application_version().is_some()
                || pack.compatibility().maximum_worker_version().is_some()
        })
        .map(super::manifest::Pack::id)
        .collect();
    assert_eq!(
        ceilinged,
        Vec::<&str>::new(),
        "a shipped pack declaring a version ceiling breaks the next version bump"
    );

    // The floors stay: they encode a real lower bound, and nothing about
    // bumping the version can violate one.
    for pack in manifest.packs() {
        assert!(
            *pack.compatibility().minimum_application_version() <= Version::new(1, 0, 0),
            "{} declares an application floor above 1.0.0",
            pack.id()
        );
    }
}

#[test]
fn bundled_proof_manifest_is_embedded_valid_and_fail_closed() {
    let manifest = bundled_manifest().expect("bundled manifest must validate");

    assert_eq!(manifest.schema_version(), 3);
    assert_eq!(manifest.status(), ManifestStatus::AdmittedCatalog);
    assert!(manifest.is_install_eligible());
    assert_eq!(manifest.proof_artifacts().len(), 12);
    assert_eq!(manifest.packs().len(), 4);
    assert!(!manifest.capability_view().is_empty());
    assert!(!manifest.license_notice_view().is_empty());

    let selection = manifest
        .select_exact(
            ExactPackRequest {
                id: "nemotron-3.5-streaming-en-cpu",
                revision: "560ms-int8-2026-06-11",
            },
            &compatible_context(),
        )
        .expect("the serving pack must be selectable for installation");
    assert_eq!(selection.pack().id(), "nemotron-3.5-streaming-en-cpu");
}

#[test]
fn the_authored_nemotron_packs_are_admitted_and_the_manifest_holds_no_moonshine() {
    // With both Nemotron packs eligible, streaming-asr is unambiguous per
    // provider: float on CUDA, int8 on CPU. Moonshine is gone from the
    // manifest entirely, not merely retired, so there is no third entry left
    // to make that ambiguous.
    let manifest = bundled_manifest().expect("bundled manifest must validate");
    let pack = |id: &str| {
        manifest
            .packs()
            .iter()
            .find(|pack| pack.id() == id)
            .expect("pack is in the catalog")
    };

    let cpu = pack("nemotron-3.5-streaming-en-cpu");
    let cuda = pack("nemotron-3.5-streaming-en-cuda");

    for pack in [cpu, cuda] {
        assert!(pack.is_install_eligible(), "{} is not admitted", pack.id());
        // The digest is the trust anchor whether or not there is a URL.
        assert_eq!(
            pack.archive()
                .expect("both nemotron packs are archive-based")
                .sha256()
                .len(),
            64
        );
    }
    assert!(
        !manifest
            .packs()
            .iter()
            .any(|pack| pack.id().to_lowercase().contains("moonshine")),
        "Moonshine must not reappear as a pack"
    );

    // The CPU pack comes from sherpa-onnx's own release; the CUDA pack is a
    // derived export hosted at an immutable Hugging Face revision.
    assert!(cpu.is_downloadable());
    assert!(cuda.is_downloadable());
    assert_eq!(
        cuda.archive()
            .expect("the CUDA pack still carries an archive record")
            .url(),
        Some(
            "https://huggingface.co/orangeblue39/nemotron-3.5-streaming-en-cuda/resolve/bae0a819fa4f4bc0878f535509886455037f8f63/nemotron-3.5-streaming-en-cuda-320ms-fp32.tar.gz"
        )
    );

    assert_eq!(cpu.runtime().provider(), ExecutionProvider::Cpu);
    assert_eq!(cuda.runtime().provider(), ExecutionProvider::Cuda);
    assert_eq!(cpu.chunk_size_ms(), Some(560));
    assert_eq!(cuda.chunk_size_ms(), Some(320));

    assert_eq!(
        manifest
            .select_sole_install_eligible(PackRole::StreamingAsr, ExecutionProvider::Cpu)
            .expect("the CPU Nemotron pack is admitted")
            .id(),
        "nemotron-3.5-streaming-en-cpu"
    );
    assert_eq!(
        manifest
            .select_sole_install_eligible(PackRole::StreamingAsr, ExecutionProvider::Cuda)
            .expect("the CUDA Nemotron pack is admitted")
            .id(),
        "nemotron-3.5-streaming-en-cuda"
    );
}

#[test]
fn the_authored_granite_packs_are_archive_less_and_q4_k_m_alone_is_admitted() {
    // Phase 5 of docs/handoff/granite-final-pass.md: Hugging Face serves
    // Granite's GGUFs as loose files, not one archive, so both packs are the
    // schema-v3 archive-less shape — `archive` absent, each required file
    // carrying its own URL.
    let manifest = bundled_manifest().expect("bundled manifest must validate");
    let pack = |id: &str| {
        manifest
            .packs()
            .iter()
            .find(|pack| pack.id() == id)
            .expect("pack is in the catalog")
    };

    let q8_0 = pack("granite-speech-4.1-2b-q8_0-cpu");
    let q4_k_m = pack("granite-speech-4.1-2b-q4_k_m-cpu");

    for pack in [q8_0, q4_k_m] {
        assert_eq!(pack.role(), PackRole::FinalAsr);
        assert_eq!(pack.runtime().provider(), ExecutionProvider::Cpu);
        assert!(
            pack.archive().is_none(),
            "{} should be the archive-less, loose-file shape",
            pack.id()
        );
        assert!(
            pack.is_downloadable(),
            "{} carries a URL on every required file",
            pack.id()
        );
        for file in pack.required_files() {
            assert!(file.url().is_some(), "{}: {}", pack.id(), file.path());
        }
    }

    // Q4_K_M is the shipped quantization since Phase 9, on Phase 8's
    // measurement (~21% faster on a 120 s utterance, identical transcript);
    // Q8_0 is now the recorded, non-selectable alternative. Exactly one of
    // the two is admitted, which is what keeps `select_sole_install_eligible`
    // unambiguous -- flipping both on would make it return `Ambiguous` and
    // take Granite out of every install at once.
    assert!(q4_k_m.is_install_eligible());
    assert!(!q8_0.is_install_eligible());

    assert_eq!(
        manifest
            .select_sole_install_eligible(PackRole::FinalAsr, ExecutionProvider::Cpu)
            .expect("the Q4_K_M Granite pack is admitted")
            .id(),
        "granite-speech-4.1-2b-q4_k_m-cpu"
    );
}

fn native_runtime_artifact(source_commit: Option<&str>) -> Value {
    let mut artifact = json!({
        "id": "synthetic-native-runtime",
        "kind": "native-runtime",
        "version": "1.2.3",
        "url": "https://example.test/runtime.zip",
        "archive_bytes": 900,
        "archive_sha256": "c".repeat(64),
        "extracted_bytes": 1200,
        "licenses": ["MIT"],
        "proof_files": [{ "path": "bin/runtime.dll", "bytes": 1000, "sha256": "d".repeat(64) }],
        "proof_status": "hash-verified"
    });
    if let Some(commit) = source_commit {
        artifact["source_commit"] = json!(commit);
    }
    let mut catalog = valid_catalog();
    catalog["artifacts"]
        .as_array_mut()
        .expect("artifacts is an array")
        .push(artifact);
    catalog
}

#[test]
fn a_native_runtime_may_omit_a_source_commit_it_does_not_have() {
    // NVIDIA ships versioned redistributable archives built from no public
    // tree. Requiring the field would have meant writing the archive's own
    // digest into it, which validates while claiming something untrue about
    // where the bytes came from.
    let manifest = parse(&native_runtime_artifact(None)).expect("must validate without a commit");

    assert_eq!(manifest.proof_artifacts().len(), 1);
}

#[test]
fn a_source_commit_that_is_present_is_still_held_to_being_a_revision() {
    // Optional must not mean unchecked: sherpa does have a git SHA, and a
    // malformed one there is still a manifest defect.
    let error = parse(&native_runtime_artifact(Some("not-a-revision")))
        .expect_err("a malformed source commit must still be refused");

    assert!(
        matches!(&error, ManifestError::Invariant { path, .. } if path.ends_with("source_commit")),
        "unexpected error: {error}"
    );
}

/// A second `streaming-asr` pack, identical to the fixture's but for its id, so
/// the only thing distinguishing the two is where they sit in the array.
fn with_second_streaming_pack(id: &str, install_eligible: bool) -> Value {
    let mut catalog = valid_catalog();
    let mut second = catalog["packs"][0].clone();
    second["id"] = json!(id);
    second["variant_group"] = json!(id);
    second["install_eligible"] = json!(install_eligible);
    catalog["packs"]
        .as_array_mut()
        .expect("packs is an array")
        .push(second);
    catalog
}

/// A second `streaming-asr` pack differing from the fixture's only in id and
/// execution provider — the shape the migration actually ships, where CUDA and
/// CPU packs are alternatives for different machines rather than rivals.
fn with_streaming_pack_on_provider(id: &str, provider: &str) -> Value {
    let mut catalog = valid_catalog();
    let mut second = catalog["packs"][0].clone();
    second["id"] = json!(id);
    second["variant_group"] = json!(id);
    second["runtime"]["provider"] = json!(provider);
    catalog["packs"]
        .as_array_mut()
        .expect("packs is an array")
        .push(second);
    catalog
}

#[test]
fn the_admitted_streaming_pack_is_selected_by_role_not_by_a_written_out_id() {
    let manifest = bundled_manifest().expect("bundled manifest must validate");

    let pack = manifest
        .select_sole_install_eligible(PackRole::StreamingAsr, ExecutionProvider::Cpu)
        .expect("the admitted streaming pack must resolve by role");

    assert_eq!(pack.id(), "nemotron-3.5-streaming-en-cpu");
    assert_eq!(pack.role(), PackRole::StreamingAsr);
}

#[test]
fn two_packs_in_one_role_are_refused_rather_than_resolved_by_array_order() {
    let manifest = parse(&with_second_streaming_pack(
        "synthetic-streaming-en-b",
        true,
    ))
    .expect("must validate");

    let error = manifest
        .select_sole_install_eligible(PackRole::StreamingAsr, ExecutionProvider::Cpu)
        .expect_err("an under-specified role must not silently resolve");

    // The ids matter as much as the refusal: a caller reading this in a log has
    // to be able to open the manifest and see which two packs collided.
    assert_eq!(
        error,
        RoleSelectionError::Ambiguous {
            role: PackRole::StreamingAsr,
            provider: ExecutionProvider::Cpu,
            ids: vec![
                "synthetic-streaming-en".to_owned(),
                "synthetic-streaming-en-b".to_owned(),
            ],
        }
    );
}

#[test]
fn one_role_holds_a_pack_per_provider_without_them_colliding() {
    // The whole reason provider joined the key. These two packs fill the same
    // role and are both install-eligible, but they are answers for different
    // machines. Keyed on role alone they would be `Ambiguous` and the app would
    // refuse to select anything at all.
    let manifest = parse(&with_streaming_pack_on_provider(
        "synthetic-streaming-en-cuda",
        "cuda",
    ))
    .expect("must validate");

    let cpu = manifest
        .select_sole_install_eligible(PackRole::StreamingAsr, ExecutionProvider::Cpu)
        .expect("the CPU pack must still resolve");
    let cuda = manifest
        .select_sole_install_eligible(PackRole::StreamingAsr, ExecutionProvider::Cuda)
        .expect("the CUDA pack must resolve independently");

    assert_eq!(cpu.id(), "synthetic-streaming-en");
    assert_eq!(cuda.id(), "synthetic-streaming-en-cuda");
}

#[test]
fn ambiguity_is_still_an_error_within_a_single_provider() {
    // Scoping ambiguity by provider must not weaken it into "first CUDA pack
    // wins". Two packs on the same provider is an under-specified manifest for
    // exactly the reason it always was.
    let mut catalog = with_streaming_pack_on_provider("synthetic-streaming-en-cuda", "cuda");
    let mut third = catalog["packs"][0].clone();
    third["id"] = json!("synthetic-streaming-en-cuda-b");
    third["variant_group"] = json!("synthetic-streaming-en-cuda-b");
    third["runtime"]["provider"] = json!("cuda");
    catalog["packs"]
        .as_array_mut()
        .expect("packs is an array")
        .push(third);

    let error = parse(&catalog)
        .expect("must validate")
        .select_sole_install_eligible(PackRole::StreamingAsr, ExecutionProvider::Cuda)
        .expect_err("two CUDA packs in one role must not silently resolve");

    assert_eq!(
        error,
        RoleSelectionError::Ambiguous {
            role: PackRole::StreamingAsr,
            provider: ExecutionProvider::Cuda,
            ids: vec![
                "synthetic-streaming-en-cuda".to_owned(),
                "synthetic-streaming-en-cuda-b".to_owned(),
            ],
        }
    );
}

#[test]
fn a_missing_provider_does_not_fall_back_to_another_one() {
    // A GPU-preferred caller asking for CUDA on a machine that only has the CPU
    // pack must be told so, not handed the CPU pack. Which engine a user lands
    // on is disclosed to them, so it cannot be decided inside a lookup.
    let manifest = parse(&valid_catalog()).expect("synthetic catalog must validate");

    let error = manifest
        .select_sole_install_eligible(PackRole::StreamingAsr, ExecutionProvider::Cuda)
        .expect_err("no CUDA pack is admitted in this catalog");

    assert_eq!(
        error,
        RoleSelectionError::NoneAdmitted {
            role: PackRole::StreamingAsr,
            provider: ExecutionProvider::Cuda,
        }
    );
    assert!(
        manifest
            .select_sole_install_eligible(PackRole::StreamingAsr, ExecutionProvider::Cpu)
            .is_ok(),
        "and the CPU pack it did not fall back to is genuinely there"
    );
}

#[test]
fn a_pack_that_is_not_install_eligible_does_not_make_a_role_ambiguous() {
    // The migration leaves evaluated-but-rejected packs in the manifest as
    // history. Those must stay invisible to selection, or every retired
    // candidate would break the role it was once a candidate for.
    let manifest = parse(&with_second_streaming_pack(
        "synthetic-streaming-en-retired",
        false,
    ))
    .expect("must validate");

    let pack = manifest
        .select_sole_install_eligible(PackRole::StreamingAsr, ExecutionProvider::Cpu)
        .expect("a retired pack must not count toward the role");

    assert_eq!(pack.id(), "synthetic-streaming-en");
}

#[test]
fn a_role_no_pack_fills_is_refused_by_name() {
    let manifest = parse(&valid_catalog()).expect("synthetic catalog must validate");

    let error = manifest
        .select_sole_install_eligible(PackRole::FinalAsr, ExecutionProvider::Cpu)
        .expect_err("no pack fills final-asr in this catalog");

    assert_eq!(
        error,
        RoleSelectionError::NoneAdmitted {
            role: PackRole::FinalAsr,
            provider: ExecutionProvider::Cpu,
        }
    );
    // The message is read beside the manifest, so it spells both the role and
    // the provider the way the JSON does. The provider matters here: with two
    // packs per role, "no final-asr pack" and "no final-asr pack *on CUDA*" are
    // different problems with different fixes.
    assert_eq!(
        error.to_string(),
        "no install-eligible final-asr pack is admitted for the cpu provider"
    );
}

#[test]
fn capability_and_license_views_expose_auditable_pack_limits() {
    let manifest = parse(&valid_catalog()).expect("synthetic catalog must validate");
    let capabilities = manifest.capability_view();
    let licenses = manifest.license_notice_view();

    assert_eq!(capabilities.len(), 2);
    assert_eq!(capabilities[0].pack_id, "synthetic-streaming-en");
    assert_eq!(capabilities[0].revision, "pack-r1");
    assert_eq!(capabilities[0].task, Task::Transcribe);
    assert_eq!(capabilities[0].chunk_size_ms, Some(560));
    assert_eq!(licenses.len(), 1);
    assert_eq!(licenses[0].spdx_id, Some("MIT"));
    assert_eq!(licenses[0].installed_bytes, 1200);
    assert_eq!(licenses[0].source_revision, "a".repeat(40));
}

#[test]
fn resolver_reports_every_relevant_mismatch_without_recommending() {
    let manifest = parse(&valid_catalog()).expect("synthetic catalog must validate");
    let context = CompatibilityContext {
        application_version: Version::new(0, 3, 0),
        worker_version: Version::new(0, 0, 9),
        platform: Platform::Linux,
        architecture: Architecture::Aarch64,
        provider: ExecutionProvider::Cuda,
    };

    let resolutions = manifest.resolve_compatibility(&context);
    let issues = resolutions[0].issues();
    assert_eq!(issues.len(), 5);
    assert!(
        issues
            .iter()
            .any(|issue| matches!(issue, CompatibilityIssue::ApplicationTooNew { .. }))
    );
    assert!(
        issues
            .iter()
            .any(|issue| matches!(issue, CompatibilityIssue::WorkerTooOld { .. }))
    );
    assert!(
        issues
            .iter()
            .any(|issue| matches!(issue, CompatibilityIssue::PlatformMismatch { .. }))
    );
    assert!(
        issues
            .iter()
            .any(|issue| matches!(issue, CompatibilityIssue::ArchitectureMismatch { .. }))
    );
    assert!(
        issues
            .iter()
            .any(|issue| matches!(issue, CompatibilityIssue::ProviderMismatch { .. }))
    );
}

#[test]
fn exact_selection_never_falls_back_to_another_revision() {
    let manifest = parse(&valid_catalog()).expect("synthetic catalog must validate");
    let selection = manifest
        .select_exact(
            ExactPackRequest {
                id: "synthetic-streaming-en",
                revision: "pack-r1",
            },
            &compatible_context(),
        )
        .expect("the exact compatible admitted pack must select");
    assert_eq!(selection.pack().revision(), "pack-r1");

    let error = manifest
        .select_exact(
            ExactPackRequest {
                id: "synthetic-streaming-en",
                revision: "pack-r2",
            },
            &compatible_context(),
        )
        .expect_err("a missing revision must not fall back");
    assert_eq!(
        error,
        SelectionError::PackNotFound {
            id: "synthetic-streaming-en".to_owned(),
            revision: "pack-r2".to_owned(),
        }
    );
}

#[test]
fn caller_supplied_valid_manifest_cannot_become_a_selection_root() {
    let bytes = valid_catalog().to_string();
    let manifest = TrustedManifest::parse(bytes.as_bytes())
        .expect("caller-supplied metadata may be validated for inspection");

    assert_eq!(
        manifest
            .select_exact(
                ExactPackRequest {
                    id: "synthetic-streaming-en",
                    revision: "pack-r1",
                },
                &compatible_context(),
            )
            .expect_err("validation is not authentication"),
        SelectionError::CallerSuppliedManifest
    );
}

#[test]
fn selection_fails_closed_for_catalog_pack_license_and_compatibility() {
    let mut catalog = valid_catalog();
    catalog["install_eligible"] = json!(false);
    let manifest = parse(&catalog).expect("ineligible catalog remains valid metadata");
    assert_eq!(
        manifest
            .select_exact(
                ExactPackRequest {
                    id: "synthetic-streaming-en",
                    revision: "pack-r1",
                },
                &compatible_context(),
            )
            .expect_err("catalog must fail closed"),
        SelectionError::ManifestNotInstallEligible
    );

    let mut catalog = valid_catalog();
    catalog["packs"][0]["licenses"][0]["redistribution"] = json!("review-required");
    let manifest = parse(&catalog).expect("pending license remains visible metadata");
    assert_eq!(
        manifest
            .select_exact(
                ExactPackRequest {
                    id: "synthetic-streaming-en",
                    revision: "pack-r1",
                },
                &compatible_context(),
            )
            .expect_err("pending redistribution must block selection"),
        SelectionError::RedistributionNotAllowed {
            components: vec!["synthetic-model".to_owned()]
        }
    );

    let mut incompatible = compatible_context();
    incompatible.worker_version = Version::new(0, 3, 0);
    let manifest = parse(&valid_catalog()).expect("synthetic catalog must validate");
    assert!(matches!(
        manifest.select_exact(
            ExactPackRequest {
                id: "synthetic-streaming-en",
                revision: "pack-r1",
            },
            &incompatible,
        ),
        Err(SelectionError::Incompatible { .. })
    ));
}

#[test]
fn validation_rejects_schema_drift_and_broken_trust_fields() {
    let mut catalog = valid_catalog();
    catalog["unexpected"] = json!(true);
    assert!(matches!(parse(&catalog), Err(ManifestError::Json(_))));

    let mut catalog = valid_catalog();
    catalog["packs"][0]["archive"]["sha256"] = json!("not-a-digest");
    assert!(matches!(
        parse(&catalog),
        Err(ManifestError::Invariant { path, .. }) if path == "packs[0].archive.sha256"
    ));

    let mut catalog = valid_catalog();
    catalog["manifest_status"] = json!("phase-0b-proof-only");
    catalog["install_eligible"] = json!(true);
    assert!(matches!(
        parse(&catalog),
        Err(ManifestError::Invariant { path, .. }) if path == "install_eligible"
    ));
}

/// A schema-v3, archive-less pack: no `archive` field at all, and every
/// required file carries its own `url` instead — the shape Granite's
/// loose Hugging Face GGUFs need.
fn archive_less_catalog() -> Value {
    let mut catalog = valid_catalog();
    let pack = &mut catalog["packs"][0];
    pack.as_object_mut()
        .expect("pack is an object")
        .remove("archive");
    pack["required_files"][0]["url"] = json!("https://example.test/model/encoder.onnx");
    catalog
}

#[test]
fn an_archive_less_pack_with_a_url_on_every_file_validates_and_is_downloadable() {
    let manifest = parse(&archive_less_catalog()).expect("archive-less pack must validate");
    let pack = &manifest.packs()[0];

    assert!(pack.archive().is_none());
    assert!(pack.is_downloadable());
    assert_eq!(
        pack.required_files()[0].url(),
        Some("https://example.test/model/encoder.onnx")
    );
}
