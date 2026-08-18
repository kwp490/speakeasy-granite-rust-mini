use crate::{
    Archive, CapabilityView, ExactPackSelection, HardwareSnapshot, LicenseNoticeView, PackRole,
};

#[derive(Clone, Debug)]
pub struct ConfirmationDisclosure<'a> {
    pub pack_id: &'a str,
    pub revision: &'a str,
    pub display_name: &'a str,
    pub download_bytes: u64,
    pub installed_bytes: u64,
    pub role: PackRole,
    pub provider: &'static str,
    pub privacy: &'static str,
    pub capabilities: Vec<CapabilityView<'a>>,
    pub licenses: Vec<LicenseNoticeView<'a>>,
    pub hardware: &'a HardwareSnapshot,
    pub requires_explicit_confirmation: bool,
}

pub fn recommendation_disclosure<'a>(
    selection: &'a ExactPackSelection<'a>,
    hardware: &'a HardwareSnapshot,
) -> ConfirmationDisclosure<'a> {
    let pack = selection.pack();
    ConfirmationDisclosure {
        pack_id: pack.id(),
        revision: pack.revision(),
        display_name: pack.display_name(),
        download_bytes: pack.archive().map_or(0, Archive::bytes),
        installed_bytes: pack.installed_bytes(),
        role: pack.role(),
        provider: match pack.runtime().provider() {
            crate::ExecutionProvider::Cpu => "cpu",
            crate::ExecutionProvider::DirectMl => "direct-ml",
            crate::ExecutionProvider::Cuda => "cuda",
        },
        privacy: "local model; provisioning contacts only the disclosed artifact host",
        capabilities: pack
            .capabilities()
            .iter()
            .map(|capability| CapabilityView {
                pack_id: pack.id(),
                revision: pack.revision(),
                display_name: pack.display_name(),
                role: pack.role(),
                streaming: pack.streaming(),
                locale: capability.locale(),
                task: capability.task(),
                target_locale: capability.target_locale(),
                features: capability.features(),
                chunk_size_ms: pack.chunk_size_ms(),
            })
            .collect(),
        licenses: pack
            .licenses()
            .iter()
            .map(|license| LicenseNoticeView {
                pack_id: pack.id(),
                revision: pack.revision(),
                display_name: pack.display_name(),
                source_repository: pack.source().upstream_repository(),
                source_revision: pack.source().upstream_revision(),
                installed_bytes: pack.installed_bytes(),
                component: license.component(),
                spdx_id: license.spdx_id(),
                license_name: license.name(),
                license_text_url: license.text_url(),
                attribution: license.attribution(),
                modification_notice: license.modification_notice(),
                redistribution: license.redistribution(),
            })
            .collect(),
        hardware,
        requires_explicit_confirmation: true,
    }
}
