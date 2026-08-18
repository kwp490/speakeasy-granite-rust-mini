use crate::{
    CapabilityFeature, PackRole, RedistributionDecision, StreamingClassification, Task,
    TrustedManifest,
};

#[derive(Clone, Copy, Debug)]
pub struct CapabilityView<'a> {
    pub pack_id: &'a str,
    pub revision: &'a str,
    pub display_name: &'a str,
    pub role: PackRole,
    pub streaming: StreamingClassification,
    pub locale: &'a str,
    pub task: Task,
    pub target_locale: Option<&'a str>,
    pub features: &'a [CapabilityFeature],
    pub chunk_size_ms: Option<u32>,
}

#[derive(Clone, Copy, Debug)]
pub struct LicenseNoticeView<'a> {
    pub pack_id: &'a str,
    pub revision: &'a str,
    pub display_name: &'a str,
    pub source_repository: &'a str,
    pub source_revision: &'a str,
    pub installed_bytes: u64,
    pub component: &'a str,
    pub spdx_id: Option<&'a str>,
    pub license_name: &'a str,
    pub license_text_url: &'a str,
    pub attribution: &'a str,
    pub modification_notice: &'a str,
    pub redistribution: RedistributionDecision,
}

impl TrustedManifest {
    pub fn capability_view(&self) -> Vec<CapabilityView<'_>> {
        self.packs()
            .iter()
            .flat_map(|pack| {
                pack.capabilities()
                    .iter()
                    .map(move |capability| CapabilityView {
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
            })
            .collect()
    }

    pub fn license_notice_view(&self) -> Vec<LicenseNoticeView<'_>> {
        self.packs()
            .iter()
            .flat_map(|pack| {
                pack.licenses()
                    .iter()
                    .map(move |license| LicenseNoticeView {
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
            })
            .collect()
    }
}
