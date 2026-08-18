use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DownloadPolicy {
    pub redirect_hosts: Vec<String>,
    pub connect_deadline_ms: u64,
    pub read_deadline_ms: u64,
    pub overall_deadline_ms: u64,
    pub maximum_retries: u8,
    pub proxy_aware: bool,
}

impl DownloadPolicy {
    /// Validates bounded deadlines and retries before constructing a transport.
    ///
    /// # Errors
    ///
    /// Returns an error for zero/unbounded deadlines or excessive retries.
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.connect_deadline_ms == 0
            || self.read_deadline_ms == 0
            || self.overall_deadline_ms == 0
            || self.connect_deadline_ms > self.overall_deadline_ms
            || self.read_deadline_ms > self.overall_deadline_ms
        {
            return Err("download deadlines must be non-zero and bounded by the overall deadline");
        }
        if self.maximum_retries > 5 {
            return Err("download retries must not exceed five");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResumeMetadata {
    pub trusted_total_bytes: u64,
    pub received_bytes: u64,
    pub etag: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResumeResponse {
    pub status: u16,
    pub content_range_start: Option<u64>,
    pub complete_length: Option<u64>,
    pub etag: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResumeDecision {
    Append,
    Restart,
}

pub fn validate_resume_response(
    metadata: &ResumeMetadata,
    response: &ResumeResponse,
) -> ResumeDecision {
    if metadata.received_bytes == 0 {
        return ResumeDecision::Restart;
    }
    let matches = response.status == 206
        && response.content_range_start == Some(metadata.received_bytes)
        && response.complete_length == Some(metadata.trusted_total_bytes)
        && response.etag.as_deref() == Some(metadata.etag.as_str())
        && !metadata.etag.trim().is_empty()
        && metadata.received_bytes < metadata.trusted_total_bytes;
    if matches {
        ResumeDecision::Append
    } else {
        ResumeDecision::Restart
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resume_requires_partial_content_exact_range_length_and_etag() {
        let metadata = ResumeMetadata {
            trusted_total_bytes: 100,
            received_bytes: 40,
            etag: "immutable".to_owned(),
        };
        let valid = ResumeResponse {
            status: 206,
            content_range_start: Some(40),
            complete_length: Some(100),
            etag: Some("immutable".to_owned()),
        };
        assert_eq!(
            validate_resume_response(&metadata, &valid),
            ResumeDecision::Append
        );
        for invalid in [
            ResumeResponse {
                status: 200,
                ..valid.clone()
            },
            ResumeResponse {
                content_range_start: Some(0),
                ..valid.clone()
            },
            ResumeResponse {
                complete_length: Some(101),
                ..valid.clone()
            },
            ResumeResponse {
                etag: Some("changed".to_owned()),
                ..valid.clone()
            },
        ] {
            assert_eq!(
                validate_resume_response(&metadata, &invalid),
                ResumeDecision::Restart
            );
        }
    }
}
