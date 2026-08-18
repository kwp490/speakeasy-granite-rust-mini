use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use reqwest::StatusCode;
use reqwest::blocking::{Client, Response};
use reqwest::header::{CONTENT_LENGTH, CONTENT_RANGE, ETAG, IF_RANGE, LOCATION, RANGE};
use sha2::{Digest, Sha256};
use speakeasy_domain::CancelToken;

use crate::{
    DownloadPolicy, ResumeDecision, ResumeMetadata, ResumeResponse, validate_resume_response,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DownloadRequest {
    pub url: String,
    pub destination: PathBuf,
    pub expected_bytes: u64,
    pub expected_sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DownloadResult {
    pub path: PathBuf,
    pub bytes: u64,
    pub etag: Option<String>,
    pub resumed: bool,
}

struct DownloadPaths {
    partial: PathBuf,
    metadata: PathBuf,
}

#[derive(Clone, Copy)]
struct AttemptControl {
    started: Instant,
    overall: Duration,
    allow_test_http: bool,
}

#[derive(Debug)]
pub enum DownloadError {
    InvalidPolicy(&'static str),
    InvalidRequest(&'static str),
    DisallowedRedirect(String),
    RedirectLimit,
    MissingRedirectLocation,
    UnexpectedStatus(u16),
    LengthMismatch { expected: u64, actual: u64 },
    HashMismatch,
    Cancelled,
    DeadlineExceeded,
    Http(reqwest::Error),
    Io(io::Error),
    Metadata(serde_json::Error),
}

impl Display for DownloadError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        write!(formatter, "model download failed: {self:?}")
    }
}

impl Error for DownloadError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Http(error) => Some(error),
            Self::Io(error) => Some(error),
            Self::Metadata(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for DownloadError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<reqwest::Error> for DownloadError {
    fn from(error: reqwest::Error) -> Self {
        Self::Http(error)
    }
}

impl From<serde_json::Error> for DownloadError {
    fn from(error: serde_json::Error) -> Self {
        Self::Metadata(error)
    }
}

/// Downloads an exact trusted artifact with durable, validator-bound resume state.
///
/// # Errors
///
/// Returns a typed error for invalid trust inputs, redirect/status violations,
/// cancellation, deadline expiry, transport failure, or length/hash mismatch.
pub fn download_to_file(
    request: &DownloadRequest,
    policy: &DownloadPolicy,
    cancel: &CancelToken,
) -> Result<DownloadResult, DownloadError> {
    policy.validate().map_err(DownloadError::InvalidPolicy)?;
    validate_request(request)?;
    if request.destination.is_file()
        && request.destination.metadata()?.len() == request.expected_bytes
        && verify_hash(&request.destination, &request.expected_sha256).is_ok()
    {
        return Ok(DownloadResult {
            path: request.destination.clone(),
            bytes: request.expected_bytes,
            etag: None,
            resumed: true,
        });
    }
    remove_if_exists(&request.destination)?;
    let overall = Duration::from_millis(policy.overall_deadline_ms);
    let started = Instant::now();
    let client = client(policy)?;
    download_with_client(request, policy, cancel, &client, started, overall, false)
}

fn download_with_client(
    request: &DownloadRequest,
    policy: &DownloadPolicy,
    cancel: &CancelToken,
    client: &Client,
    started: Instant,
    overall: Duration,
    allow_test_http: bool,
) -> Result<DownloadResult, DownloadError> {
    let paths = DownloadPaths {
        partial: sidecar_path(&request.destination, "part"),
        metadata: sidecar_path(&request.destination, "part.json"),
    };
    let control = AttemptControl {
        started,
        overall,
        allow_test_http,
    };

    let mut attempts = 0_u8;
    loop {
        check_control(cancel, started, overall)?;
        match download_attempt(client, request, policy, cancel, &paths, control) {
            Ok(result) => return Ok(result),
            Err(error) if is_retryable(&error) && attempts < policy.maximum_retries => {
                attempts += 1;
                let backoff = Duration::from_millis(100 * u64::from(attempts));
                if started.elapsed().saturating_add(backoff) >= overall {
                    return Err(DownloadError::DeadlineExceeded);
                }
                thread::sleep(backoff);
            }
            Err(error) => return Err(error),
        }
    }
}

fn download_attempt(
    client: &Client,
    request: &DownloadRequest,
    policy: &DownloadPolicy,
    cancel: &CancelToken,
    paths: &DownloadPaths,
    control: AttemptControl,
) -> Result<DownloadResult, DownloadError> {
    let partial_path = &paths.partial;
    let metadata_path = &paths.metadata;
    let prior = load_resume(partial_path, metadata_path, request.expected_bytes)?;
    let mut response = send(
        client,
        &request.url,
        policy,
        prior.as_ref(),
        control.allow_test_http,
    )?;
    let resumed = prior.as_ref().is_some_and(|metadata| {
        validate_resume_response(metadata, &resume_facts(&response)) == ResumeDecision::Append
    });
    if prior.is_some() && !resumed {
        remove_if_exists(partial_path)?;
        remove_if_exists(metadata_path)?;
        response = send(client, &request.url, policy, None, control.allow_test_http)?;
    }
    if response.status() != StatusCode::OK
        && !(resumed && response.status() == StatusCode::PARTIAL_CONTENT)
    {
        return Err(DownloadError::UnexpectedStatus(response.status().as_u16()));
    }
    validate_response_length(&response, request.expected_bytes, prior.as_ref(), resumed)?;

    if let Some(parent) = partial_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut output = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(!resumed)
        .open(partial_path)?;
    let initial = if resumed {
        output.seek(SeekFrom::End(0))?
    } else {
        0
    };
    let etag = header_string(&response, ETAG);
    let mut received = initial;
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        check_control(cancel, control.started, control.overall)?;
        let count = response.read(&mut buffer).map_err(|error| {
            let wrapped_timeout = error
                .get_ref()
                .and_then(|source| source.downcast_ref::<reqwest::Error>())
                .is_some_and(reqwest::Error::is_timeout);
            if matches!(
                error.kind(),
                io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
            ) || wrapped_timeout
            {
                DownloadError::DeadlineExceeded
            } else {
                DownloadError::Io(error)
            }
        })?;
        if count == 0 {
            break;
        }
        received = received
            .checked_add(count as u64)
            .ok_or(DownloadError::LengthMismatch {
                expected: request.expected_bytes,
                actual: u64::MAX,
            })?;
        if received > request.expected_bytes {
            return Err(DownloadError::LengthMismatch {
                expected: request.expected_bytes,
                actual: received,
            });
        }
        output.write_all(&buffer[..count])?;
        if let Some(etag) = etag.as_ref().filter(|value| !value.trim().is_empty()) {
            persist_resume(metadata_path, request.expected_bytes, received, etag)?;
        }
    }
    output.sync_all()?;
    if received != request.expected_bytes {
        return Err(DownloadError::LengthMismatch {
            expected: request.expected_bytes,
            actual: received,
        });
    }
    verify_hash(partial_path, &request.expected_sha256)?;
    fs::rename(partial_path, &request.destination)?;
    remove_if_exists(metadata_path)?;
    Ok(DownloadResult {
        path: request.destination.clone(),
        bytes: received,
        etag,
        resumed,
    })
}

fn client(policy: &DownloadPolicy) -> Result<Client, DownloadError> {
    let mut builder = Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_millis(policy.connect_deadline_ms))
        .timeout(Duration::from_millis(policy.read_deadline_ms))
        .user_agent(concat!("speakeasy/", env!("CARGO_PKG_VERSION")));
    if !policy.proxy_aware {
        builder = builder.no_proxy();
    }
    Ok(builder.build()?)
}

fn send(
    client: &Client,
    original_url: &str,
    policy: &DownloadPolicy,
    resume: Option<&ResumeMetadata>,
    allow_test_http: bool,
) -> Result<Response, DownloadError> {
    let mut url = reqwest::Url::parse(original_url)
        .map_err(|_| DownloadError::InvalidRequest("download URL must be absolute HTTPS"))?;
    for _ in 0..=5 {
        validate_url(&url, &policy.redirect_hosts, allow_test_http)?;
        let mut request = client.get(url.clone());
        if let Some(metadata) = resume {
            request = request
                .header(RANGE, format!("bytes={}-", metadata.received_bytes))
                .header(IF_RANGE, &metadata.etag);
        }
        let response = request.send()?;
        if !response.status().is_redirection() {
            return Ok(response);
        }
        let location = response
            .headers()
            .get(LOCATION)
            .and_then(|value| value.to_str().ok())
            .ok_or(DownloadError::MissingRedirectLocation)?;
        url = url
            .join(location)
            .map_err(|_| DownloadError::MissingRedirectLocation)?;
    }
    Err(DownloadError::RedirectLimit)
}

fn validate_request(request: &DownloadRequest) -> Result<(), DownloadError> {
    let url = reqwest::Url::parse(&request.url)
        .map_err(|_| DownloadError::InvalidRequest("download URL must be absolute HTTPS"))?;
    if url.scheme() != "https" || url.host_str().is_none() || request.expected_bytes == 0 {
        return Err(DownloadError::InvalidRequest(
            "download requires HTTPS, a host, and a positive trusted length",
        ));
    }
    let hash = request.expected_sha256.as_bytes();
    if hash.len() != 64 || !hash.iter().all(u8::is_ascii_hexdigit) {
        return Err(DownloadError::InvalidRequest(
            "download requires a trusted SHA-256",
        ));
    }
    Ok(())
}

fn validate_url(
    url: &reqwest::Url,
    allowed_hosts: &[String],
    allow_test_http: bool,
) -> Result<(), DownloadError> {
    let host = url.host_str().unwrap_or_default();
    if (url.scheme() != "https" && !(allow_test_http && url.scheme() == "http"))
        || !allowed_hosts
            .iter()
            .any(|allowed| allowed.eq_ignore_ascii_case(host))
    {
        return Err(DownloadError::DisallowedRedirect(url.to_string()));
    }
    Ok(())
}

/// Recover the resume state a previous attempt left behind, if it is usable.
///
/// **A partial file longer than its metadata is the normal outcome of a kill,
/// not corruption**, and treating it as corruption is what made this resume
/// survive only clean stops. The write loop writes the data first and the
/// metadata second, so a process that dies between the two leaves a file up to
/// one buffer ahead of the last durably recorded offset. Measured 2026-08-17 on
/// the first attempt: a bootstrapper killed mid-transfer left 197,424,080 bytes
/// on disk against a recorded 197,407,696 — a gap of exactly one 16 KiB buffer —
/// and the old rule deleted both files and started the 2.3 GB download again
/// from zero. Nothing errored, and the wizard's own copy promised the opposite.
///
/// So the metadata is the authority and the file is trimmed back to it. Bytes
/// past the recorded offset were written but never accounted for, they are at
/// the tail, and a torn final `write_all` lands in exactly the same place — so
/// truncating to `received_bytes` discards precisely the unaccounted region and
/// nothing else.
///
/// The opposite direction is not repairable and is still discarded: metadata
/// claiming more bytes than the file holds means the recorded offset never
/// existed, and resuming from it would append the server's next range on top of
/// a hole. The final SHA-256 would catch it, after re-downloading everything.
fn load_resume(
    partial_path: &Path,
    metadata_path: &Path,
    trusted_total: u64,
) -> Result<Option<ResumeMetadata>, DownloadError> {
    if !partial_path.exists() || !metadata_path.exists() {
        remove_if_exists(partial_path)?;
        remove_if_exists(metadata_path)?;
        return Ok(None);
    }
    let metadata: ResumeMetadata = serde_json::from_slice(&fs::read(metadata_path)?)?;
    let actual = partial_path.metadata()?.len();
    if metadata.trusted_total_bytes != trusted_total
        || metadata.received_bytes > actual
        || metadata.received_bytes == 0
        || metadata.received_bytes >= trusted_total
        || metadata.etag.trim().is_empty()
    {
        remove_if_exists(partial_path)?;
        remove_if_exists(metadata_path)?;
        return Ok(None);
    }
    if actual > metadata.received_bytes {
        // Trimmed before the range request goes out, not after: the writer
        // reopens this file and seeks to its end, so leaving the unaccounted
        // tail in place would splice the resumed range in after bytes the
        // server is about to send again.
        OpenOptions::new()
            .write(true)
            .open(partial_path)?
            .set_len(metadata.received_bytes)?;
    }
    Ok(Some(metadata))
}

fn persist_resume(
    path: &Path,
    trusted_total_bytes: u64,
    received_bytes: u64,
    etag: &str,
) -> Result<(), DownloadError> {
    let temporary = sidecar_path(path, "tmp");
    let bytes = serde_json::to_vec(&ResumeMetadata {
        trusted_total_bytes,
        received_bytes,
        etag: etag.to_owned(),
    })?;
    let mut file = File::create(&temporary)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    fs::rename(temporary, path)?;
    Ok(())
}

fn resume_facts(response: &Response) -> ResumeResponse {
    let (start, total) = response
        .headers()
        .get(CONTENT_RANGE)
        .and_then(|value| value.to_str().ok())
        .and_then(parse_content_range)
        .unwrap_or((None, None));
    ResumeResponse {
        status: response.status().as_u16(),
        content_range_start: start,
        complete_length: total,
        etag: header_string(response, ETAG),
    }
}

fn parse_content_range(value: &str) -> Option<(Option<u64>, Option<u64>)> {
    let value = value.strip_prefix("bytes ")?;
    let (range, total) = value.split_once('/')?;
    let (start, _) = range.split_once('-')?;
    Some((Some(start.parse().ok()?), Some(total.parse().ok()?)))
}

fn validate_response_length(
    response: &Response,
    expected: u64,
    prior: Option<&ResumeMetadata>,
    resumed: bool,
) -> Result<(), DownloadError> {
    let expected_body = if resumed {
        expected - prior.map_or(0, |metadata| metadata.received_bytes)
    } else {
        expected
    };
    if let Some(length) = response
        .headers()
        .get(CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        && length != expected_body
    {
        return Err(DownloadError::LengthMismatch {
            expected: expected_body,
            actual: length,
        });
    }
    Ok(())
}

fn verify_hash(path: &Path, expected: &str) -> Result<(), DownloadError> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    io::copy(&mut file, &mut hasher)?;
    if format!("{:x}", hasher.finalize()).eq_ignore_ascii_case(expected) {
        Ok(())
    } else {
        Err(DownloadError::HashMismatch)
    }
}

fn check_control(
    cancel: &CancelToken,
    started: Instant,
    overall: Duration,
) -> Result<(), DownloadError> {
    if cancel.is_cancelled() {
        return Err(DownloadError::Cancelled);
    }
    if started.elapsed() >= overall {
        return Err(DownloadError::DeadlineExceeded);
    }
    Ok(())
}

fn is_retryable(error: &DownloadError) -> bool {
    matches!(
        error,
        DownloadError::Http(_)
            | DownloadError::Io(_)
            | DownloadError::UnexpectedStatus(408 | 425 | 429 | 500..=599)
    )
}

fn header_string(response: &Response, name: reqwest::header::HeaderName) -> Option<String> {
    response
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

fn sidecar_path(path: &Path, suffix: &str) -> PathBuf {
    let name = path
        .file_name()
        .map_or_else(|| "download".into(), std::ffi::OsStr::to_os_string);
    let mut name = name;
    name.push(format!(".{suffix}"));
    path.with_file_name(name)
}

fn remove_if_exists(path: &Path) -> Result<(), io::Error> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fmt::Write as _;
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    struct TestServer {
        url: String,
        requests: Arc<Mutex<Vec<String>>>,
        thread: thread::JoinHandle<()>,
    }

    fn serve(responses: Vec<(Vec<u8>, Duration)>) -> TestServer {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
        let address = listener.local_addr().expect("test address");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&requests);
        let thread = thread::spawn(move || {
            for (response, delay) in responses {
                let (mut stream, _) = listener.accept().expect("accept request");
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .expect("read timeout");
                let mut request = Vec::new();
                let mut buffer = [0_u8; 1024];
                while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                    let count = stream.read(&mut buffer).expect("read request");
                    if count == 0 {
                        break;
                    }
                    request.extend_from_slice(&buffer[..count]);
                }
                recorded
                    .lock()
                    .expect("record requests")
                    .push(String::from_utf8_lossy(&request).into_owned());
                stream.write_all(&response).expect("write response");
                if !delay.is_zero() {
                    thread::sleep(delay);
                }
            }
        });
        TestServer {
            url: format!("http://{address}/artifact"),
            requests,
            thread,
        }
    }

    fn response(status: &str, headers: &[(&str, &str)], body: &[u8]) -> Vec<u8> {
        let mut response = format!("HTTP/1.1 {status}\r\nConnection: close\r\n");
        for (name, value) in headers {
            write!(response, "{name}: {value}\r\n").expect("format response header");
        }
        response.push_str("\r\n");
        let mut bytes = response.into_bytes();
        bytes.extend_from_slice(body);
        bytes
    }

    fn policy() -> DownloadPolicy {
        DownloadPolicy {
            redirect_hosts: vec!["127.0.0.1".to_owned()],
            connect_deadline_ms: 200,
            read_deadline_ms: 200,
            overall_deadline_ms: 2_000,
            maximum_retries: 0,
            proxy_aware: false,
        }
    }

    fn test_download(
        url: String,
        destination: PathBuf,
        expected_bytes: u64,
        expected_sha256: String,
        policy: &DownloadPolicy,
        client: &Client,
        cancel: &CancelToken,
    ) -> Result<DownloadResult, DownloadError> {
        download_with_client(
            &DownloadRequest {
                url,
                destination,
                expected_bytes,
                expected_sha256,
            },
            policy,
            cancel,
            client,
            Instant::now(),
            Duration::from_millis(policy.overall_deadline_ms),
            true,
        )
    }

    fn test_client(policy: &DownloadPolicy) -> Client {
        client(policy).expect("test client")
    }

    #[test]
    fn content_range_parser_is_exact() {
        assert_eq!(
            parse_content_range("bytes 40-99/100"),
            Some((Some(40), Some(100)))
        );
        assert_eq!(parse_content_range("items 40-99/100"), None);
        assert_eq!(parse_content_range("bytes */100"), None);
    }

    #[test]
    fn redirect_hosts_are_exact_and_https_only() {
        let allowed = vec!["github.com".to_owned()];
        assert!(
            validate_url(
                &reqwest::Url::parse("https://github.com/a").unwrap(),
                &allowed,
                false
            )
            .is_ok()
        );
        assert!(
            validate_url(
                &reqwest::Url::parse("https://evilgithub.com/a").unwrap(),
                &allowed,
                false
            )
            .is_err()
        );
        assert!(
            validate_url(
                &reqwest::Url::parse("http://github.com/a").unwrap(),
                &allowed,
                false
            )
            .is_err()
        );
    }

    #[test]
    fn retryable_status_is_retried_and_verified() {
        let body = b"trusted";
        let server = serve(vec![
            (
                response("500 Internal Server Error", &[("Content-Length", "0")], b""),
                Duration::ZERO,
            ),
            (
                response(
                    "200 OK",
                    &[("Content-Length", "7"), ("ETag", "\"v1\"")],
                    body,
                ),
                Duration::ZERO,
            ),
        ]);
        let temp = tempfile::tempdir().unwrap();
        let mut download_policy = policy();
        download_policy.maximum_retries = 1;
        let result = test_download(
            server.url.clone(),
            temp.path().join("artifact.bin"),
            body.len() as u64,
            digest_bytes(body),
            &download_policy,
            &test_client(&download_policy),
            &CancelToken::default(),
        )
        .expect("retry succeeds");
        assert!(!result.resumed);
        server.thread.join().unwrap();
        assert_eq!(server.requests.lock().unwrap().len(), 2);
    }

    #[test]
    fn interrupted_transfer_resumes_only_with_matching_range_and_etag() {
        let body = b"helloworld";
        let server = serve(vec![
            (
                response(
                    "200 OK",
                    &[("Content-Length", "10"), ("ETag", "\"v1\"")],
                    b"hello",
                ),
                Duration::ZERO,
            ),
            (
                response(
                    "206 Partial Content",
                    &[
                        ("Content-Length", "5"),
                        ("Content-Range", "bytes 5-9/10"),
                        ("ETag", "\"v1\""),
                    ],
                    b"world",
                ),
                Duration::ZERO,
            ),
        ]);
        let temp = tempfile::tempdir().unwrap();
        let mut download_policy = policy();
        download_policy.maximum_retries = 1;
        let destination = temp.path().join("artifact.bin");
        let result = test_download(
            server.url.clone(),
            destination.clone(),
            10,
            digest_bytes(body),
            &download_policy,
            &test_client(&download_policy),
            &CancelToken::default(),
        )
        .expect("resume succeeds");
        assert!(result.resumed);
        assert_eq!(fs::read(destination).unwrap(), body);
        server.thread.join().unwrap();
        let requests = server.requests.lock().unwrap();
        assert!(requests[1].contains("range: bytes=5-"));
        assert!(requests[1].contains("if-range: \"v1\""));
    }

    #[test]
    fn a_partial_left_longer_than_its_metadata_still_resumes() {
        // The state a KILLED download leaves, which is the only state that
        // matters: the write loop writes the data and then the metadata, so a
        // process that dies between the two leaves the file up to one buffer
        // ahead of the recorded offset. Measured on a real 2.3 GB transfer
        // (2026-08-17): 197,424,080 bytes on disk against 197,407,696 recorded.
        //
        // The rule used to be `received_bytes != actual` -> throw both away, so
        // resume survived a clean stop and nothing else, and the first real
        // interruption re-downloaded from zero while the wizard's copy promised
        // it would not. Every test here passed throughout, because they all
        // seeded a partial that agreed with its metadata exactly.
        let body = b"helloworld";
        let server = serve(vec![(
            response(
                "206 Partial Content",
                &[
                    ("Content-Length", "5"),
                    ("Content-Range", "bytes 5-9/10"),
                    ("ETag", "\"v1\""),
                ],
                b"world",
            ),
            Duration::ZERO,
        )]);
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("artifact.bin");
        // Six bytes on disk, five of them accounted for. The sixth is the
        // unaccounted tail a kill leaves behind, and it is wrong — `helloX`
        // rather than `hello` — so a resume that failed to trim it would append
        // the server's range after it and produce `helloXworld`, which is
        // eleven bytes and a hash mismatch.
        fs::write(sidecar_path(&destination, "part"), b"helloX").unwrap();
        persist_resume(&sidecar_path(&destination, "part.json"), 10, 5, "\"v1\"").unwrap();

        let download_policy = policy();
        let result = test_download(
            server.url.clone(),
            destination.clone(),
            10,
            digest_bytes(body),
            &download_policy,
            &test_client(&download_policy),
            &CancelToken::default(),
        )
        .expect("a killed transfer must resume");

        assert!(result.resumed, "the transfer restarted instead of resuming");
        assert_eq!(fs::read(destination).unwrap(), body);
        server.thread.join().unwrap();
        let requests = server.requests.lock().unwrap();
        // One request, and it asked for the tail. Two would mean the bytes
        // already on disk were fetched a second time, which is the defect.
        assert_eq!(requests.len(), 1, "the fetched bytes were requested again");
        assert!(requests[0].contains("range: bytes=5-"));
    }

    #[test]
    fn a_partial_shorter_than_its_metadata_is_discarded() {
        // The unrepairable direction, and the reason the check is not simply
        // relaxed to "any mismatch is fine". Metadata claiming an offset the
        // file never reached would resume by appending the server's range over
        // a hole; the digest would catch it, after paying for the whole
        // transfer a second time to find out.
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("artifact.bin");
        let partial = sidecar_path(&destination, "part");
        let metadata = sidecar_path(&destination, "part.json");
        fs::write(&partial, b"hel").unwrap();
        persist_resume(&metadata, 10, 5, "\"v1\"").unwrap();

        assert!(load_resume(&partial, &metadata, 10).unwrap().is_none());
        assert!(
            !partial.exists(),
            "an unusable partial must not be left behind"
        );
        assert!(!metadata.exists());
    }

    #[test]
    fn mismatched_resume_response_is_discarded_before_full_restart() {
        let body = b"helloworld";
        let server = serve(vec![
            (
                response(
                    "206 Partial Content",
                    &[
                        ("Content-Length", "5"),
                        ("Content-Range", "bytes 5-9/10"),
                        ("ETag", "\"changed\""),
                    ],
                    b"world",
                ),
                Duration::ZERO,
            ),
            (
                response(
                    "200 OK",
                    &[("Content-Length", "10"), ("ETag", "\"changed\"")],
                    body,
                ),
                Duration::ZERO,
            ),
        ]);
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("artifact.bin");
        fs::write(sidecar_path(&destination, "part"), b"hello").unwrap();
        persist_resume(&sidecar_path(&destination, "part.json"), 10, 5, "\"v1\"").unwrap();
        let download_policy = policy();
        let result = test_download(
            server.url.clone(),
            destination.clone(),
            10,
            digest_bytes(body),
            &download_policy,
            &test_client(&download_policy),
            &CancelToken::default(),
        )
        .expect("full restart succeeds");
        assert!(!result.resumed);
        server.thread.join().unwrap();
        let requests = server.requests.lock().unwrap();
        assert!(requests[0].contains("range: bytes=5-"));
        assert!(!requests[1].contains("range:"));
    }

    #[test]
    fn read_deadline_and_hostile_redirect_fail_closed() {
        let slow = serve(vec![(
            response("200 OK", &[("Content-Length", "1")], b""),
            Duration::from_millis(100),
        )]);
        let temp = tempfile::tempdir().unwrap();
        let mut deadline_policy = policy();
        deadline_policy.read_deadline_ms = 20;
        let deadline = test_download(
            slow.url.clone(),
            temp.path().join("slow.bin"),
            1,
            digest_bytes(b"x"),
            &deadline_policy,
            &test_client(&deadline_policy),
            &CancelToken::default(),
        );
        assert!(matches!(deadline, Err(DownloadError::DeadlineExceeded)));
        slow.thread.join().unwrap();

        let redirect = serve(vec![(
            response(
                "302 Found",
                &[
                    ("Location", "http://example.invalid/artifact"),
                    ("Content-Length", "0"),
                ],
                b"",
            ),
            Duration::ZERO,
        )]);
        let redirect_result = test_download(
            redirect.url.clone(),
            temp.path().join("redirect.bin"),
            1,
            digest_bytes(b"x"),
            &policy(),
            &test_client(&policy()),
            &CancelToken::default(),
        );
        assert!(matches!(
            redirect_result,
            Err(DownloadError::DisallowedRedirect(_))
        ));
        redirect.thread.join().unwrap();
    }

    #[test]
    fn explicit_proxy_route_and_integrity_failures_are_enforced() {
        let body = b"proxy";
        let proxy = serve(vec![(
            response("200 OK", &[("Content-Length", "5")], body),
            Duration::ZERO,
        )]);
        let temp = tempfile::tempdir().unwrap();
        let mut proxy_policy = policy();
        proxy_policy.redirect_hosts = vec!["example.test".to_owned()];
        let proxy_client = Client::builder()
            .proxy(reqwest::Proxy::all(&proxy.url).unwrap())
            .build()
            .unwrap();
        test_download(
            "http://example.test/artifact".to_owned(),
            temp.path().join("proxy.bin"),
            5,
            digest_bytes(body),
            &proxy_policy,
            &proxy_client,
            &CancelToken::default(),
        )
        .expect("proxy download");
        proxy.thread.join().unwrap();
        assert!(proxy.requests.lock().unwrap()[0].starts_with("GET http://example.test/artifact "));

        let bad_hash = serve(vec![(
            response("200 OK", &[("Content-Length", "5")], body),
            Duration::ZERO,
        )]);
        let hash_result = test_download(
            bad_hash.url.clone(),
            temp.path().join("bad-hash.bin"),
            5,
            digest_bytes(b"other"),
            &policy(),
            &test_client(&policy()),
            &CancelToken::default(),
        );
        assert!(matches!(hash_result, Err(DownloadError::HashMismatch)));
        bad_hash.thread.join().unwrap();

        let bad_length = serve(vec![(
            response("200 OK", &[("Content-Length", "4")], b"four"),
            Duration::ZERO,
        )]);
        let length_result = test_download(
            bad_length.url.clone(),
            temp.path().join("bad-length.bin"),
            5,
            digest_bytes(body),
            &policy(),
            &test_client(&policy()),
            &CancelToken::default(),
        );
        assert!(matches!(
            length_result,
            Err(DownloadError::LengthMismatch { .. })
        ));
        bad_length.thread.join().unwrap();
    }

    #[test]
    fn cancellation_prevents_transport_attempt() {
        let cancel = CancelToken::default();
        cancel.cancel();
        let download_policy = policy();
        let result = test_download(
            "http://127.0.0.1:1/artifact".to_owned(),
            tempfile::tempdir().unwrap().path().join("cancelled.bin"),
            1,
            digest_bytes(b"x"),
            &download_policy,
            &test_client(&download_policy),
            &cancel,
        );
        assert!(matches!(result, Err(DownloadError::Cancelled)));
    }

    #[test]
    fn a_completed_verified_download_is_reused_without_transport() {
        let temp = tempfile::tempdir().expect("temp root");
        let destination = temp.path().join("complete.bin");
        let bytes = b"already complete";
        fs::write(&destination, bytes).expect("cached download");
        let result = download_to_file(
            &DownloadRequest {
                url: "https://example.test/artifact".to_owned(),
                destination: destination.clone(),
                expected_bytes: bytes.len() as u64,
                expected_sha256: digest_bytes(bytes),
            },
            &DownloadPolicy {
                redirect_hosts: vec!["example.test".to_owned()],
                connect_deadline_ms: 100,
                read_deadline_ms: 100,
                overall_deadline_ms: 200,
                maximum_retries: 0,
                proxy_aware: false,
            },
            &CancelToken::default(),
        )
        .expect("reuse verified download");

        assert_eq!(result.path, destination);
        assert_eq!(result.bytes, bytes.len() as u64);
        assert!(result.resumed);
    }

    #[test]
    fn overall_deadline_prevents_transport_attempt() {
        let download_policy = policy();
        let result = download_with_client(
            &DownloadRequest {
                url: "http://127.0.0.1:1/artifact".to_owned(),
                destination: tempfile::tempdir().unwrap().path().join("deadline.bin"),
                expected_bytes: 1,
                expected_sha256: digest_bytes(b"x"),
            },
            &download_policy,
            &CancelToken::default(),
            &test_client(&download_policy),
            Instant::now()
                .checked_sub(Duration::from_secs(1))
                .expect("past instant"),
            Duration::from_millis(1),
            true,
        );
        assert!(matches!(result, Err(DownloadError::DeadlineExceeded)));
    }

    fn digest_bytes(bytes: &[u8]) -> String {
        format!("{:x}", Sha256::digest(bytes))
    }
}
