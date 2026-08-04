//! Finding GGUF files on Hugging Face that osstat could download.
//!
//! This is the second of osstat's two verification tiers, and the difference
//! between them is the whole point of the module.
//!
//! The seven pinned models in `osstat-llm`'s registry carry a SHA256 that was
//! reviewed in a pull request against this repository. Verifying against it
//! proves the bytes are the ones somebody looked at. A searched result carries
//! the digest Hugging Face reports beside the file — the LFS oid, which for a
//! `.gguf` in an LFS-backed repository *is* the file's SHA256. Checking it
//! proves the transfer was not corrupted. It cannot prove the upload was not
//! replaced, because the digest and the file come from the same origin.
//!
//! That is a real weakening and it is why [`SearchResult`] exists as its own
//! type rather than as a [`crate::models::ModelKey`] with a different source:
//! nothing downstream can confuse the two, and the UI has to say which it is.
//! ADR-012's reasoning is scoped by this, not overturned — the runtime binary
//! osstat *executes* still gets a hash pinned in the repository.
//!
//! # What is deliberately not derived here
//!
//! A result carries its file size and nothing that resembles a verdict. The fit
//! matrix prices a model from its architecture — layer count, head counts,
//! context length — which lives in the registry for the pinned seven and in the
//! GGUF header for everything else, and the header only exists once the file is
//! on disk. Deriving a parameter count from size and quantization bits would
//! produce a figure that looks like the calculator's output and is a guess.
//! [`SearchResult::quant_hint`] is the one concession, and it is read off the
//! file name for display; nothing computes with it.
//!
//! # What is dropped, and why each would be worse than showing nothing
//!
//! - A file with no LFS oid has no hash, so it could only be downloaded
//!   unverified. Small files are stored directly rather than in LFS.
//! - A file that is not a `.gguf` produces a download the runtime cannot load.
//! - One shard of a split model (`-00001-of-00002.gguf`) fails its own hash
//!   check as a whole model, for a reason nobody could diagnose. Multi-part
//!   files are out of scope, so they are excluded rather than half-supported.
//! - A multimodal projector (`mmproj-*.gguf`) is a GGUF with a hash and a size
//!   and so passes every test above, but it is not a model: downloading one
//!   yields several hundred megabytes no chat can load. It is excluded as a
//!   result and attached to the models it belongs to instead — see
//!   [`SearchResult::projector`].
//!
//! A response that will not parse yields no results rather than an error.
//! Search is a convenience beside the curated matrix; a bad body should read as
//! "nothing found", not take the page down.

use serde::Deserialize;

use crate::error::AcquireError;

/// The origin every search is made against.
///
/// A constant for the same reason [`crate::models`] keeps its host constant: a
/// configurable search origin would be a second place a download URL could come
/// from, widened somewhere nobody reviews for hostnames.
const HUGGING_FACE: &str = "https://huggingface.co";

/// The quantization tags a file name may carry, as whole tokens.
///
/// Compared against tokens split on `-` and `.` rather than searched for as
/// substrings, so `Q4_K_M` cannot be reported for a file that merely happens to
/// contain those characters. Purely a display hint — see the module docs.
const QUANT_TAGS: &[&str] = &[
    "IQ1_S", "IQ1_M", "IQ2_XXS", "IQ2_XS", "IQ2_S", "IQ2_M", "IQ3_XXS", "IQ3_XS", "IQ3_S", "IQ3_M",
    "IQ4_XS", "IQ4_NL", "Q2_K", "Q3_K_S", "Q3_K_M", "Q3_K_L", "Q3_K", "Q4_0", "Q4_1", "Q4_K_S",
    "Q4_K_M", "Q4_K", "Q5_0", "Q5_1", "Q5_K_S", "Q5_K_M", "Q5_K", "Q6_K", "Q8_0", "BF16", "F16",
    "F32",
];

/// How long a hex SHA256 is.
const SHA256_HEX_LEN: usize = 64;

/// One downloadable `.gguf` file a search turned up.
///
/// Everything needed to fetch and verify it, and nothing that could be mistaken
/// for a fit verdict. See the module docs for why `size_bytes` is the only
/// figure here.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, Deserialize)]
#[cfg_attr(feature = "ts-bindings", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-bindings", ts(export))]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    /// The Hugging Face repository, as `owner/name`.
    pub repo: String,
    /// The repository's owner, shown so a result names who uploaded it.
    pub publisher: String,
    /// The path of the file within the repository.
    pub file: String,
    /// What the file weighs, as the API reports it.
    #[cfg_attr(feature = "ts-bindings", ts(type = "number"))]
    pub size_bytes: u64,
    /// The LFS oid, which for these files is the SHA256 the download is checked
    /// against. Same origin as the file — see the module docs.
    pub sha256: String,
    /// The quantization tag read off the file name, if it carries a known one.
    ///
    /// A label, never an input to any calculation.
    pub quant_hint: Option<String>,
    /// The multimodal projector this repository ships, on a vision model.
    ///
    /// Not a verdict and not an estimate — the same three facts every other
    /// downloadable file here carries, read off the same tree listing. A
    /// vision GGUF holds the language model only; the weights that turn an
    /// image into tokens it can attend to live in a second file passed as
    /// `--mmproj`, so a result downloaded without this one produces a model
    /// that loads, answers text, and silently ignores every image.
    ///
    /// `None` is a text-only repository and is what nearly every result is.
    pub projector: Option<SearchProjector>,
}

/// The multimodal projector a searched repository ships beside its weights.
///
/// Deliberately not a second [`SearchResult`]: a projector is never offered as
/// a model of its own, carries no quantization cell, and has no publisher
/// distinct from the repository it came from. Reusing the richer type would
/// invite exactly the mistake this module now excludes — listing a projector
/// among the models, where downloading it yields a file no chat can load.
///
/// It carries no `repo` either. The repository is by construction the parent
/// result's, and a second one would be a second place a download URL could be
/// assembled from — the thing ADR-012 rests on there not being.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, Deserialize)]
#[cfg_attr(feature = "ts-bindings", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-bindings", ts(export))]
#[serde(rename_all = "camelCase")]
pub struct SearchProjector {
    /// The path of the projector within the repository.
    pub file: String,
    /// The LFS oid, checked exactly as the weights' is. Same origin as the
    /// file, so it detects a corrupted transfer and not a replaced upload.
    pub sha256: String,
    /// What the projector weighs, as the API reports it.
    #[cfg_attr(feature = "ts-bindings", ts(type = "number"))]
    pub size_bytes: u64,
}

impl SearchProjector {
    /// Whether this is a projector [`search`] could have produced.
    ///
    /// Asked for the same reason [`SearchResult::is_well_formed`] is, and it is
    /// not a lesser reason: this file name is interpolated into a download URL
    /// too, so a compromised webview handing back a projector of its choosing
    /// would be the URL-taking command ADR-012 rests on there not being.
    ///
    /// Includes [`is_projector`] rather than only the download predicates, so
    /// the check accepts exactly what the search emits and not a superset.
    #[must_use]
    fn is_well_formed(&self) -> bool {
        is_safe_relative_path(&self.file)
            && is_gguf(&self.file)
            && !is_multi_part(&self.file)
            && is_projector(&self.file)
            && is_sha256(&self.sha256)
            && self.size_bytes > 0
    }

    /// The name the projector would be saved under, without any folders above
    /// it.
    ///
    /// The same flattening [`SearchResult::file_name`] does, for the same
    /// reason: a repository may hold `Q8_0/mmproj-model.gguf`, and the local
    /// library is one directory. It cannot collide with the weights it
    /// accompanies, because a name that is a projector's is by definition not a
    /// model's.
    #[must_use]
    pub fn file_name(&self) -> &str {
        self.file.rsplit('/').next().unwrap_or(&self.file)
    }
}

impl SearchResult {
    /// Whether this is a result [`search`] could have produced.
    ///
    /// Asked again at the point of download, because by then the value has been
    /// through the webview. ADR-012 rests on there being no command that takes a
    /// URL, and a result whose `repo` and `file` are interpolated into one is
    /// exactly as good as a URL unless both are checked. A compromised webview
    /// must not be able to name a path, an origin, or a file with no usable
    /// hash.
    ///
    /// The same predicates the search itself filtered on, so the two cannot
    /// drift apart into a gap that only the second caller can reach.
    #[must_use]
    pub fn is_well_formed(&self) -> bool {
        is_plausible_repo(&self.repo)
            && is_safe_relative_path(&self.file)
            && is_gguf(&self.file)
            && !is_multi_part(&self.file)
            && !is_projector(&self.file)
            && is_sha256(&self.sha256)
            && self.size_bytes > 0
            && self
                .projector
                .as_ref()
                .is_none_or(SearchProjector::is_well_formed)
    }

    /// The name the file would be saved under, without any folders above it.
    ///
    /// A repository may hold `Q4_K_M/model.gguf`; the local library is flat, and
    /// nothing here reconstructs a directory tree from a remote path.
    #[must_use]
    pub fn file_name(&self) -> &str {
        self.file.rsplit('/').next().unwrap_or(&self.file)
    }
}

/// One repository in the search listing.
///
/// Every field optional so a single odd entry cannot fail the whole response;
/// the module treats a body it cannot read as an empty result set.
#[derive(Debug, Deserialize)]
struct RepoSummary {
    /// The repository id, as `owner/name`.
    #[serde(default)]
    id: Option<String>,
}

/// One entry in a repository's file tree.
#[derive(Debug, Deserialize)]
struct TreeEntry {
    /// The path within the repository.
    #[serde(default)]
    path: Option<String>,
    /// The size in bytes, absent for entries that are not files.
    #[serde(default)]
    size: Option<u64>,
    /// The LFS record, absent for a file stored directly rather than in LFS.
    #[serde(default)]
    lfs: Option<Lfs>,
}

/// The LFS record attached to a tree entry.
#[derive(Debug, Deserialize)]
struct Lfs {
    /// The object id, which for a `.gguf` here is its SHA256.
    #[serde(default)]
    oid: Option<String>,
}

/// Searches Hugging Face for downloadable GGUF files matching `query`.
///
/// `limit` bounds the number of **repositories** consulted, not the number of
/// files returned: one repository usually holds the same model at a dozen
/// quantizations, and truncating that list would hide the choice the user came
/// to make.
///
/// # Errors
///
/// [`AcquireError::Network`] or [`AcquireError::HttpStatus`] if the listing
/// itself cannot be fetched. A body that will not parse, and a repository whose
/// tree cannot be read, both yield fewer results rather than a failure — see
/// the module docs.
pub async fn search(
    client: &reqwest::Client,
    query: &str,
    limit: usize,
) -> Result<Vec<SearchResult>, AcquireError> {
    search_at(client, HUGGING_FACE, query, limit).await
}

/// [`search`], against an explicit origin so tests can point at a local socket.
///
/// Private: the public entry point takes no origin, which is what keeps
/// "downloads only ever come from Hugging Face" a property of the type rather
/// than of the caller.
async fn search_at(
    client: &reqwest::Client,
    origin: &str,
    query: &str,
    limit: usize,
) -> Result<Vec<SearchResult>, AcquireError> {
    // The search term is the only user input in this module and it goes into a
    // URL, so it is escaped rather than interpolated. `reqwest`'s own `query`
    // builder would do this, but it is behind a feature that pulls in
    // `serde_urlencoded` for one call — the same trade `download.rs` made when
    // it wrote `hex` by hand.
    let url = format!(
        "{origin}/api/models?search={}&filter=gguf&limit={limit}",
        percent_encoded(query)
    );
    let body = fetch_text(client.get(&url), &url).await?;

    let Ok(repos) = serde_json::from_str::<Vec<RepoSummary>>(&body) else {
        return Ok(Vec::new());
    };

    let mut results = Vec::new();
    for repo in repos.into_iter().take(limit) {
        let Some(id) = repo.id.filter(|id| is_plausible_repo(id)) else {
            continue;
        };

        // Sequential, and a failure here skips the repository rather than
        // failing the search: one repository being unreadable is a smaller
        // result set, not a broken page.
        results.extend(files_in(client, origin, &id).await);
    }

    Ok(results)
}

/// Every downloadable GGUF file in `repo`, or nothing if it cannot be read.
async fn files_in(client: &reqwest::Client, origin: &str, repo: &str) -> Vec<SearchResult> {
    let url = format!("{origin}/api/models/{repo}/tree/main");
    let Ok(body) = fetch_text(client.get(&url), &url).await else {
        return Vec::new();
    };
    let Ok(entries) = serde_json::from_str::<Vec<TreeEntry>>(&body) else {
        return Vec::new();
    };

    let publisher = repo.split('/').next().unwrap_or(repo).to_owned();

    // Two passes over one listing, because a projector belongs to the whole
    // repository rather than to any single file in it: one tree holds a dozen
    // quantizations of the same model and, on a vision model, the one
    // projector every one of them needs. The tree is already in hand, so
    // finding it costs no further request — which is what makes attaching it
    // cheaper than the alternative of leaving this tier text-only.
    let projector = entries
        .iter()
        .filter_map(downloadable)
        .filter(|(file, _, _)| is_projector(file))
        .map(|(file, sha256, size_bytes)| SearchProjector {
            file: file.to_owned(),
            sha256: sha256.to_owned(),
            size_bytes,
        })
        // Several is ordinary: a repository often ships the projector at both
        // F16 and Q8_0. The smallest is taken because a projector's precision
        // moves image quality far less than its size moves the download and
        // the VRAM, and because "smallest" is a rule that can be stated. The
        // name is the tie-break so two of equal size cannot make the choice
        // depend on the order the API happened to list them in.
        .min_by(|left, right| {
            left.size_bytes
                .cmp(&right.size_bytes)
                .then_with(|| left.file.cmp(&right.file))
        });

    entries
        .iter()
        .filter_map(downloadable)
        // A projector is a GGUF with a hash and a size, so every predicate
        // above admits it. Offering one as a model would be offering a
        // download that no chat can load — and it is what this module did
        // until the projector had somewhere to go.
        .filter(|(file, _, _)| !is_projector(file))
        .map(|(file, sha256, size_bytes)| SearchResult {
            repo: repo.to_owned(),
            publisher: publisher.clone(),
            quant_hint: quant_hint(file),
            file: file.to_owned(),
            size_bytes,
            sha256: sha256.to_owned(),
            projector: projector.clone(),
        })
        .collect()
}

/// The path, hash and size of `entry`, if it is a whole GGUF carrying all three.
///
/// One place rather than two so the projector pass and the model pass cannot
/// drift apart into a gap only one of them can reach — the same argument
/// [`SearchResult::is_well_formed`] makes for re-asking the search's own
/// predicates at download time.
///
/// Hash and size are both or nothing. A file without a hash could only be
/// downloaded unverified, and one without a size cannot have its free-space
/// check made before the request, so offering either would be offering a button
/// that fails.
fn downloadable(entry: &TreeEntry) -> Option<(&str, &str, u64)> {
    let file = entry.path.as_deref()?;
    if !is_gguf(file) || is_multi_part(file) {
        return None;
    }

    let sha256 = entry
        .lfs
        .as_ref()?
        .oid
        .as_deref()
        .filter(|oid| is_sha256(oid))?;
    let size_bytes = entry.size.filter(|size| *size > 0)?;

    Some((file, sha256, size_bytes))
}

/// Sends `request` and reads its body, mapping both failures onto the shared
/// error type.
async fn fetch_text(request: reqwest::RequestBuilder, url: &str) -> Result<String, AcquireError> {
    let response = request
        .send()
        .await
        .map_err(|source| AcquireError::Network {
            url: url.to_owned(),
            source,
        })?;

    if !response.status().is_success() {
        return Err(AcquireError::HttpStatus {
            url: url.to_owned(),
            status: response.status().as_u16(),
        });
    }

    response
        .text()
        .await
        .map_err(|source| AcquireError::Network {
            url: url.to_owned(),
            source,
        })
}

/// Escapes `text` for use as a URL query value.
///
/// Everything outside the unreserved set of RFC 3986 is percent-encoded, so a
/// search term containing `&`, `#`, a space or a slash cannot add a parameter
/// or change the path being asked for.
fn percent_encoded(text: &str) -> String {
    use std::fmt::Write as _;

    text.bytes().fold(String::new(), |mut out, byte| {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(byte));
        } else {
            // Writing to a String cannot fail; the Result is discarded knowingly.
            let _ = write!(out, "%{byte:02X}");
        }
        out
    })
}

/// Whether an id is the `owner/name` shape the tree endpoint expects.
///
/// Checked before it is interpolated into a URL, so a listing carrying
/// something else cannot reach out to a path of its choosing.
fn is_plausible_repo(id: &str) -> bool {
    let mut parts = id.split('/');
    let (Some(owner), Some(name), None) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };

    !owner.is_empty()
        && !name.is_empty()
        && !id.contains("..")
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/'))
}

/// Whether `file` is a relative path that stays inside the repository.
///
/// Rejects an empty segment, `.`, `..`, a leading slash, a backslash and a
/// drive letter. It is half of one URL and its last segment names a file that
/// will be created on the user's disk, so neither half may escape.
fn is_safe_relative_path(file: &str) -> bool {
    !file.is_empty()
        && !file.starts_with('/')
        && !file.contains('\\')
        && !file.contains(':')
        && !file.contains('?')
        && !file.contains('#')
        && !file.chars().any(char::is_control)
        && file
            .split('/')
            .all(|segment| !segment.is_empty() && segment != "." && segment != "..")
}

/// Whether a path names a GGUF file.
fn is_gguf(file: &str) -> bool {
    file.rsplit('.')
        .next()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("gguf"))
}

/// Whether a path names a multimodal projector rather than a model.
///
/// Matched on the file name's **prefix**, not as a substring anywhere in the
/// path, for the same reason [`quant_hint`] reads whole tokens: a repository
/// called `someone/mmproj-experiments` would otherwise have every model in it
/// classified as a projector and vanish from the results.
///
/// Conservative on purpose, and the two errors are not symmetric. A projector
/// this misses is offered as a model — a bad download the user can see and
/// delete. A model this wrongly claims is a projector is handed to
/// `llama-server` as `--mmproj`, which loads it and produces answers about
/// nothing, with no way to tell from the outside. So the rule matches the one
/// naming convention the ecosystem actually uses and declines to guess further.
fn is_projector(file: &str) -> bool {
    file.rsplit('/')
        .next()
        .unwrap_or(file)
        .to_ascii_lowercase()
        .starts_with("mmproj")
}

/// Whether a name is one shard of a split model, such as
/// `Model-Q4_K_M-00001-of-00002.gguf`.
///
/// Downloading one shard produces a file that verifies against its own oid and
/// is still not a loadable model. Multi-part files are out of scope, so the
/// honest thing is to leave them out of the results entirely.
fn is_multi_part(file: &str) -> bool {
    let stem = match file.rfind('.') {
        Some(dot) => &file[..dot],
        None => file,
    };

    let mut parts = stem.rsplit('-');
    let (Some(last), Some(middle), Some(first)) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };

    middle.eq_ignore_ascii_case("of") && is_digits(first) && is_digits(last)
}

/// Whether `text` is a non-empty run of ASCII digits.
fn is_digits(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit())
}

/// Whether `oid` could be a hex SHA256.
///
/// An oid of any other shape is not a hash this code could check a download
/// against, and treating it as one would mean a verification step that always
/// fails — or, worse, one that looks like it passed.
fn is_sha256(oid: &str) -> bool {
    oid.len() == SHA256_HEX_LEN && oid.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// The quantization tag `file` carries, if it carries a known one.
fn quant_hint(file: &str) -> Option<String> {
    file.split(['-', '.', '/'])
        .find(|token| QUANT_TAGS.iter().any(|tag| tag.eq_ignore_ascii_case(token)))
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use std::io::{Read as _, Write as _};
    use std::net::TcpListener;

    /// A plausible LFS oid: 64 hex characters, as a real SHA256 is.
    fn oid(seed: char) -> String {
        std::iter::repeat_n(seed, SHA256_HEX_LEN).collect()
    }

    /// Serves `routes` on a loopback port, matching the longest key that is a
    /// substring of the request target, and 404 for anything else.
    ///
    /// A real socket rather than a mocked client, for the same reason
    /// `download.rs` uses one: what is under test includes how the request is
    /// built and how the body is read, and a mock would replace both.
    ///
    /// `Connection: close` on every response so each request arrives on its own
    /// connection, which is what lets one server answer the listing call and
    /// the tree calls that follow it.
    fn serve(routes: Vec<(&'static str, String)>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        std::thread::spawn(move || {
            for accepted in listener.incoming() {
                let Ok(mut stream) = accepted else { continue };

                // Drained before the response is written: replying to an unread
                // socket and closing it sends RST, which reaches the client as
                // a transport error rather than the body under test.
                let mut request = Vec::new();
                let mut byte = [0_u8; 1];
                while let Ok(1) = stream.read(&mut byte) {
                    request.push(byte[0]);
                    if request.ends_with(b"\r\n\r\n") {
                        break;
                    }
                }

                let target = String::from_utf8_lossy(&request)
                    .lines()
                    .next()
                    .unwrap_or_default()
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or_default()
                    .to_owned();

                let matched = routes
                    .iter()
                    .filter(|(key, _)| target.contains(key))
                    .max_by_key(|(key, _)| key.len());

                let (status, body) =
                    matched.map_or(("404 Not Found", ""), |(_, body)| ("200 OK", body.as_str()));

                let header = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(header.as_bytes());
                let _ = stream.write_all(body.as_bytes());
                let _ = stream.flush();
                let _ = stream.shutdown(std::net::Shutdown::Write);
            }
        });

        format!("http://127.0.0.1:{port}")
    }

    /// The listing endpoint's answer for one repository.
    const ONE_REPO: &str = r#"[{"id":"bartowski/Test-GGUF"}]"#;

    /// Where that repository's tree is served from.
    const TREE: &str = "/api/models/bartowski/Test-GGUF/tree/main";

    /// The listing route key. The `?` is what keeps it from also matching the
    /// tree path, which starts with the same characters.
    const LISTING: &str = "/api/models?";

    #[tokio::test]
    async fn results_carry_a_hash_and_a_size() {
        // Without both, a result cannot be downloaded or verified, and
        // offering it would be offering a button that fails.
        let tree = format!(
            r#"[{{"path":"Test-Q4_K_M.gguf","size":4200,"lfs":{{"oid":"{}"}}}}]"#,
            oid('a')
        );
        let origin = serve(vec![(LISTING, ONE_REPO.to_owned()), (TREE, tree)]);

        let results = search_at(&reqwest::Client::new(), &origin, "test", 5)
            .await
            .unwrap();

        assert_eq!(results.len(), 1, "{results:?}");
        assert_eq!(results[0].sha256, oid('a'));
        assert_eq!(results[0].size_bytes, 4200);
        assert_eq!(results[0].repo, "bartowski/Test-GGUF");
        assert_eq!(results[0].publisher, "bartowski");
        assert_eq!(results[0].quant_hint.as_deref(), Some("Q4_K_M"));
    }

    #[tokio::test]
    async fn a_file_without_an_lfs_oid_is_skipped_not_guessed() {
        // Small files are stored directly rather than in LFS and have no oid.
        // A result with no hash must be dropped, never downloaded unverified.
        let tree = format!(
            r#"[{{"path":"Small.gguf","size":900}},
                {{"path":"Big-Q8_0.gguf","size":4200,"lfs":{{"oid":"{}"}}}}]"#,
            oid('b')
        );
        let origin = serve(vec![(LISTING, ONE_REPO.to_owned()), (TREE, tree)]);

        let results = search_at(&reqwest::Client::new(), &origin, "test", 5)
            .await
            .unwrap();

        assert_eq!(results.len(), 1, "{results:?}");
        assert_eq!(results[0].file, "Big-Q8_0.gguf");
        assert!(
            !results.iter().any(|result| result.file == "Small.gguf"),
            "a file with no hash was offered: {results:?}"
        );
    }

    #[tokio::test]
    async fn only_gguf_files_are_returned() {
        // A repository holds READMEs, configs and safetensors. Offering a
        // .safetensors file would produce a download that cannot be loaded.
        let tree = format!(
            r#"[{{"path":"README.md","size":900,"lfs":{{"oid":"{oid}"}}}},
                {{"path":"config.json","size":120,"lfs":{{"oid":"{oid}"}}}},
                {{"path":"model.safetensors","size":9000,"lfs":{{"oid":"{oid}"}}}},
                {{"path":"Test-Q6_K.gguf","size":4200,"lfs":{{"oid":"{oid}"}}}}]"#,
            oid = oid('c')
        );
        let origin = serve(vec![(LISTING, ONE_REPO.to_owned()), (TREE, tree)]);

        let results = search_at(&reqwest::Client::new(), &origin, "test", 5)
            .await
            .unwrap();

        assert_eq!(results.len(), 1, "{results:?}");
        assert_eq!(results[0].file, "Test-Q6_K.gguf");
    }

    #[tokio::test]
    async fn a_multi_part_file_is_skipped() {
        // Names like `-00001-of-00002.gguf` are one shard of a split model.
        // Downloading one shard yields a file that fails its own hash check
        // for a reason nobody could diagnose. Out of scope, so exclude it.
        let tree = format!(
            r#"[{{"path":"Test-Q8_0-00001-of-00002.gguf","size":4200,"lfs":{{"oid":"{oid}"}}}},
                {{"path":"Test-Q8_0-00002-of-00002.gguf","size":4200,"lfs":{{"oid":"{oid}"}}}},
                {{"path":"Test-Q4_K_M.gguf","size":2100,"lfs":{{"oid":"{oid}"}}}}]"#,
            oid = oid('d')
        );
        let origin = serve(vec![(LISTING, ONE_REPO.to_owned()), (TREE, tree)]);

        let results = search_at(&reqwest::Client::new(), &origin, "test", 5)
            .await
            .unwrap();

        assert_eq!(results.len(), 1, "{results:?}");
        assert_eq!(results[0].file, "Test-Q4_K_M.gguf");
        assert!(
            !results.iter().any(|result| result.file.contains("-of-")),
            "one shard of a split model was offered as a whole model: {results:?}"
        );
    }

    #[tokio::test]
    async fn a_malformed_response_yields_no_results_rather_than_an_error() {
        // Search is a convenience; a bad response should show "nothing found",
        // not break the page.
        let origin = serve(vec![(LISTING, "{ not json at all".to_owned())]);

        let results = search_at(&reqwest::Client::new(), &origin, "test", 5)
            .await
            .unwrap();

        assert!(results.is_empty(), "{results:?}");
    }

    #[tokio::test]
    async fn a_repository_whose_tree_cannot_be_read_costs_only_its_own_results() {
        // The tree route is absent, so it 404s. One unreadable repository is a
        // shorter list, not a failed search.
        let origin = serve(vec![(LISTING, ONE_REPO.to_owned())]);

        let results = search_at(&reqwest::Client::new(), &origin, "test", 5)
            .await
            .unwrap();

        assert!(results.is_empty(), "{results:?}");
    }

    #[tokio::test]
    async fn a_listing_that_cannot_be_fetched_is_an_error_the_ui_can_report() {
        // Distinct from an empty result: "Hugging Face did not answer" and
        // "nothing matched" are different things to tell someone.
        let origin = serve(Vec::new());

        let error = search_at(&reqwest::Client::new(), &origin, "test", 5)
            .await
            .unwrap_err();

        assert!(matches!(
            error,
            AcquireError::HttpStatus { status: 404, .. }
        ));
    }

    #[tokio::test]
    async fn a_result_carries_no_verdict_shaped_field() {
        // The governing constraint of the feature, asserted on the wire shape
        // rather than only in review: a searched result shows a size, and
        // nothing that could be mistaken for the calculator's output.
        let tree = format!(
            r#"[{{"path":"Test-Q4_K_M.gguf","size":4200,"lfs":{{"oid":"{}"}}}}]"#,
            oid('e')
        );
        let origin = serve(vec![(LISTING, ONE_REPO.to_owned()), (TREE, tree)]);

        let results = search_at(&reqwest::Client::new(), &origin, "test", 5)
            .await
            .unwrap();

        let json = serde_json::to_value(&results[0]).unwrap();
        let fields: Vec<&String> = json.as_object().unwrap().keys().collect();
        assert_eq!(
            fields,
            vec![
                "file",
                "projector",
                "publisher",
                "quantHint",
                "repo",
                "sha256",
                "sizeBytes"
            ],
            "a field was added that the advisor cannot honestly fill: {fields:?}"
        );
    }

    /// A result with no projector, for the tests that vary one thing about it.
    fn text_only_result() -> SearchResult {
        SearchResult {
            repo: "bartowski/Test-GGUF".to_owned(),
            publisher: "bartowski".to_owned(),
            file: "Test-Q4_K_M.gguf".to_owned(),
            size_bytes: 4200,
            sha256: oid('a'),
            quant_hint: Some("Q4_K_M".to_owned()),
            projector: None,
        }
    }

    #[tokio::test]
    async fn a_vision_repository_attaches_its_projector_to_every_quantization() {
        // The projector belongs to the repository, not to one file in it: a
        // tree holds many quantizations of the same model and the single
        // projector all of them need. Attaching it to only one would make the
        // rest silently blind for no reason a user could see.
        let tree = format!(
            r#"[{{"path":"Test-Q4_K_M.gguf","size":4200,"lfs":{{"oid":"{oid}"}}}},
                {{"path":"Test-Q8_0.gguf","size":8400,"lfs":{{"oid":"{oid}"}}}},
                {{"path":"mmproj-Test-F16.gguf","size":900,"lfs":{{"oid":"{oid}"}}}}]"#,
            oid = oid('a')
        );
        let origin = serve(vec![(LISTING, ONE_REPO.to_owned()), (TREE, tree)]);

        let results = search_at(&reqwest::Client::new(), &origin, "test", 5)
            .await
            .unwrap();

        assert_eq!(results.len(), 2, "{results:?}");
        for result in &results {
            let projector = result
                .projector
                .as_ref()
                .expect("a vision result was left with no projector");
            assert_eq!(projector.file, "mmproj-Test-F16.gguf");
            assert_eq!(projector.size_bytes, 900);
            assert_eq!(projector.sha256, oid('a'));
        }
    }

    #[tokio::test]
    async fn a_projector_is_never_offered_as_a_model_of_its_own() {
        // It is a GGUF with a hash and a size, so every other predicate in this
        // module admits it. Downloading one produces several hundred megabytes
        // that no chat can load.
        let tree = format!(
            r#"[{{"path":"Test-Q4_K_M.gguf","size":4200,"lfs":{{"oid":"{oid}"}}}},
                {{"path":"mmproj-Test-F16.gguf","size":900,"lfs":{{"oid":"{oid}"}}}}]"#,
            oid = oid('b')
        );
        let origin = serve(vec![(LISTING, ONE_REPO.to_owned()), (TREE, tree)]);

        let results = search_at(&reqwest::Client::new(), &origin, "test", 5)
            .await
            .unwrap();

        assert_eq!(results.len(), 1, "{results:?}");
        assert_eq!(results[0].file, "Test-Q4_K_M.gguf");
        assert!(
            !results.iter().any(|result| is_projector(&result.file)),
            "a projector was offered as a downloadable model: {results:?}"
        );
    }

    #[tokio::test]
    async fn the_smallest_projector_is_taken_when_a_repository_ships_several() {
        // F16 and Q8_0 side by side is the ordinary case. Precision moves image
        // quality far less than size moves the download and the VRAM.
        let tree = format!(
            r#"[{{"path":"Test-Q4_K_M.gguf","size":4200,"lfs":{{"oid":"{oid}"}}}},
                {{"path":"mmproj-Test-F16.gguf","size":1600,"lfs":{{"oid":"{oid}"}}}},
                {{"path":"mmproj-Test-Q8_0.gguf","size":800,"lfs":{{"oid":"{oid}"}}}}]"#,
            oid = oid('c')
        );
        let origin = serve(vec![(LISTING, ONE_REPO.to_owned()), (TREE, tree)]);

        let results = search_at(&reqwest::Client::new(), &origin, "test", 5)
            .await
            .unwrap();

        assert_eq!(results.len(), 1, "{results:?}");
        assert_eq!(
            results[0].projector.as_ref().unwrap().file,
            "mmproj-Test-Q8_0.gguf"
        );
    }

    #[tokio::test]
    async fn a_projector_with_no_hash_is_not_attached_rather_than_attached_unverified() {
        // Same rule the weights get. A projector osstat cannot verify must not
        // ride along on a result whose weights it can.
        let tree = format!(
            r#"[{{"path":"Test-Q4_K_M.gguf","size":4200,"lfs":{{"oid":"{}"}}}},
                {{"path":"mmproj-Test-F16.gguf","size":900}}]"#,
            oid('d')
        );
        let origin = serve(vec![(LISTING, ONE_REPO.to_owned()), (TREE, tree)]);

        let results = search_at(&reqwest::Client::new(), &origin, "test", 5)
            .await
            .unwrap();

        assert_eq!(results.len(), 1, "{results:?}");
        assert_eq!(
            results[0].projector, None,
            "an unverifiable projector was attached anyway"
        );
    }

    #[tokio::test]
    async fn a_text_only_repository_attaches_nothing() {
        // What nearly every result is. `None` has to stay the ordinary answer.
        let tree = format!(
            r#"[{{"path":"Test-Q4_K_M.gguf","size":4200,"lfs":{{"oid":"{}"}}}}]"#,
            oid('e')
        );
        let origin = serve(vec![(LISTING, ONE_REPO.to_owned()), (TREE, tree)]);

        let results = search_at(&reqwest::Client::new(), &origin, "test", 5)
            .await
            .unwrap();

        assert_eq!(results.len(), 1, "{results:?}");
        assert_eq!(results[0].projector, None);
    }

    #[test]
    fn a_projector_the_search_would_not_have_produced_is_refused_at_download_time() {
        // The projector's file name reaches a download URL exactly as the
        // weights' does, so it gets the same gauntlet. A webview that could
        // name this file could name a path of its choosing.
        let good = SearchProjector {
            file: "mmproj-Test-F16.gguf".to_owned(),
            sha256: oid('a'),
            size_bytes: 900,
        };
        assert!(good.is_well_formed());
        assert!(
            SearchResult {
                projector: Some(good.clone()),
                ..text_only_result()
            }
            .is_well_formed()
        );

        for bad in [
            SearchProjector {
                file: "../../../etc/passwd.gguf".to_owned(),
                ..good.clone()
            },
            SearchProjector {
                file: "..\\..\\Windows\\System32\\mmproj.gguf".to_owned(),
                ..good.clone()
            },
            // Not a projector name at all: accepting it would let a caller
            // point `--mmproj` at the weights themselves.
            SearchProjector {
                file: "Test-Q4_K_M.gguf".to_owned(),
                ..good.clone()
            },
            SearchProjector {
                file: "mmproj-Test.safetensors".to_owned(),
                ..good.clone()
            },
            SearchProjector {
                sha256: "deadbeef".to_owned(),
                ..good.clone()
            },
            SearchProjector {
                size_bytes: 0,
                ..good.clone()
            },
        ] {
            assert!(
                !bad.is_well_formed(),
                "a projector the search would never produce was accepted: {bad:?}"
            );
            assert!(
                !SearchResult {
                    projector: Some(bad.clone()),
                    ..text_only_result()
                }
                .is_well_formed(),
                "a bad projector rode in on a well-formed result: {bad:?}"
            );
        }
    }

    #[test]
    fn a_result_whose_own_file_is_a_projector_is_refused() {
        // The download-time half of the exclusion above. The search no longer
        // emits one, so nothing that arrives claiming to be one is genuine.
        assert!(
            !SearchResult {
                file: "mmproj-Test-F16.gguf".to_owned(),
                ..text_only_result()
            }
            .is_well_formed()
        );
    }

    #[test]
    fn a_projector_is_recognised_by_its_file_name_prefix_only() {
        assert!(is_projector("mmproj-Qwen2.5-VL-3B-Instruct-Q8_0.gguf"));
        assert!(is_projector("mmproj-model-f16.gguf"));
        assert!(is_projector("MMPROJ-F16.gguf"));
        assert!(is_projector("Q8_0/mmproj-model.gguf"));

        assert!(!is_projector("Qwen2.5-VL-3B-Instruct-Q4_K_M.gguf"));
        // The repository is named for projectors; its models are still models.
        assert!(!is_projector("mmproj-experiments/Llama-3-8B-Q4_K_M.gguf"));
        assert!(!is_projector("Test-with-mmproj-baked-in.gguf"));
    }

    #[test]
    fn a_quantization_tag_is_read_as_a_whole_token_or_not_at_all() {
        assert_eq!(
            quant_hint("Meta-Llama-3-8B-Instruct-Q4_K_M.gguf").as_deref(),
            Some("Q4_K_M")
        );
        assert_eq!(quant_hint("model.Q8_0.gguf").as_deref(), Some("Q8_0"));
        assert_eq!(quant_hint("some-model-f16.gguf").as_deref(), Some("f16"));
        assert_eq!(
            quant_hint("Qwen2.5-Coder-7B.gguf"),
            None,
            "a hint was invented for a name that carries no tag"
        );
    }

    #[tokio::test]
    async fn a_searched_download_still_refuses_a_hash_mismatch() {
        // The weaker guarantee is about WHERE the hash came from, not about
        // whether it is checked. A corrupted transfer must still be rejected.
        //
        // Against `download_resumable` itself, which is the function the
        // searched-download command reuses unchanged — so this is the real
        // check rather than a restatement of it.
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("searched.gguf");
        let part = destination.with_extension("part");

        let result = SearchResult {
            size_bytes: 3,
            ..text_only_result()
        };

        // Three bytes that are not what the oid claims.
        let origin = serve(vec![("/Test-Q4_K_M.gguf", "abc".to_owned())]);
        let url = format!("{origin}/Test-Q4_K_M.gguf");

        let error = crate::download::download_resumable(
            &reqwest::Client::new(),
            &url,
            &part,
            &destination,
            &result.sha256,
            result.size_bytes,
            &mut |_| {},
        )
        .await
        .unwrap_err();

        assert!(matches!(error, AcquireError::ChecksumMismatch { .. }));
        assert!(!error.is_retryable(), "a mismatch must not invite a retry");
        assert!(
            !destination.exists(),
            "a searched download that failed verification was kept"
        );
    }

    #[test]
    fn a_result_the_search_would_not_have_produced_is_refused_at_download_time() {
        // By the time this is asked the value has been through the webview.
        // ADR-012 rests on there being no command that takes a URL, and a
        // `repo` and `file` interpolated into one are a URL unless checked.
        let good = text_only_result();
        assert!(good.is_well_formed());

        for bad in [
            SearchResult {
                repo: "../../evil".to_owned(),
                ..good.clone()
            },
            SearchResult {
                file: "../../../etc/passwd.gguf".to_owned(),
                ..good.clone()
            },
            SearchResult {
                file: "..\\..\\Windows\\System32\\x.gguf".to_owned(),
                ..good.clone()
            },
            SearchResult {
                file: "model.safetensors".to_owned(),
                ..good.clone()
            },
            SearchResult {
                file: "Test-00001-of-00002.gguf".to_owned(),
                ..good.clone()
            },
            SearchResult {
                sha256: "deadbeef".to_owned(),
                ..good.clone()
            },
            SearchResult {
                size_bytes: 0,
                ..good.clone()
            },
        ] {
            assert!(
                !bad.is_well_formed(),
                "a result the search would never produce was accepted: {bad:?}"
            );
        }
    }

    #[test]
    fn a_file_in_a_subdirectory_is_saved_under_its_own_name() {
        // The local library is flat. Nothing rebuilds a remote directory tree.
        let result = SearchResult {
            file: "Q4_K_M/model.gguf".to_owned(),
            ..text_only_result()
        };

        assert!(result.is_well_formed());
        assert_eq!(result.file_name(), "model.gguf");
    }

    #[test]
    fn a_search_term_cannot_add_a_parameter_or_change_the_path() {
        // The one piece of user input that reaches a URL.
        assert_eq!(percent_encoded("llama 3"), "llama%203");
        assert_eq!(
            percent_encoded("a&limit=999&x=/../"),
            "a%26limit%3D999%26x%3D%2F..%2F"
        );
        assert_eq!(percent_encoded("qwen2.5-coder_7b~"), "qwen2.5-coder_7b~");
    }

    #[test]
    fn a_repository_id_that_is_not_owner_slash_name_is_refused() {
        // It is interpolated into a URL. A listing carrying something else must
        // not be able to choose the path osstat asks for.
        assert!(is_plausible_repo("bartowski/Test-GGUF"));
        assert!(!is_plausible_repo("../../etc/passwd"));
        assert!(!is_plausible_repo("owner/name/extra"));
        assert!(!is_plausible_repo("nameonly"));
        assert!(!is_plausible_repo("owner/na me"));
        assert!(!is_plausible_repo("owner/name?x=1"));
    }

    #[test]
    fn an_oid_that_is_not_a_hash_is_not_treated_as_one() {
        assert!(is_sha256(&oid('f')));
        assert!(!is_sha256("deadbeef"));
        assert!(!is_sha256(&"z".repeat(SHA256_HEX_LEN)));
    }

    #[test]
    fn shard_detection_does_not_catch_an_ordinary_name() {
        assert!(is_multi_part("Test-Q8_0-00001-of-00002.gguf"));
        assert!(is_multi_part("Test-1-of-2.gguf"));
        assert!(!is_multi_part("Test-Q4_K_M.gguf"));
        assert!(!is_multi_part("Nous-Hermes-2-of-many.gguf"));
    }
}
