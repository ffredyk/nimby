use serde_json::Value;

use crate::error::{Error, Result};

const UNRESERVED: &[u8] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~!$&'()*+,;=:@";

fn encode_segment(segment: &str) -> String {
    let mut out = String::with_capacity(segment.len());
    for byte in segment.bytes() {
        if UNRESERVED.contains(&byte) {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

fn encode_rel(rel: &str) -> String {
    rel.split('/')
        .map(encode_segment)
        .collect::<Vec<_>>()
        .join("/")
}

pub struct HttpSource {
    base: String,
    client: reqwest::blocking::Client,
    auth: Option<(String, String)>,
}

impl HttpSource {
    pub fn new(base: &str) -> Result<Self> {
        Self::with_auth(base, None)
    }

    pub fn with_auth(base: &str, auth: Option<(String, String)>) -> Result<Self> {
        let base = if base.ends_with('/') {
            base.to_string()
        } else {
            format!("{base}/")
        };
        let client = reqwest::blocking::Client::builder()
            .user_agent(concat!("nimby/", env!("CARGO_PKG_VERSION")))
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .map_err(|e| Error::Http(e.to_string()))?;
        let auth = auth.filter(|(user, _)| !user.is_empty());
        Ok(Self { base, client, auth })
    }

    fn request(&self, url: &str) -> reqwest::blocking::RequestBuilder {
        let builder = self.client.get(url);
        match &self.auth {
            Some((user, password)) => builder.basic_auth(user, Some(password)),
            None => builder,
        }
    }

    pub fn url_for(&self, rel: &str) -> String {
        format!("{}{}", self.base, encode_rel(rel))
    }

    pub fn get(&self, rel: &str) -> Result<Vec<u8>> {
        let url = self.url_for(rel);
        let response = self
            .request(&url)
            .send()
            .map_err(|e| Error::Http(format!("{url}: {e}")))?;
        let status = response.status();
        if !status.is_success() {
            return Err(Error::HttpStatus {
                url,
                status: status.as_u16(),
            });
        }
        response
            .bytes()
            .map(|b| b.to_vec())
            .map_err(|e| Error::Http(format!("{url}: {e}")))
    }

    /// Like [`HttpSource::get`], but transient failures are retried.
    pub fn get_retrying(&self, rel: &str, attempts: u32) -> Result<Vec<u8>> {
        crate::retry::with_retry(
            crate::retry::RetryPolicy::with_attempts(attempts),
            |_| {},
            |_| self.get(rel),
        )
    }

    /// Fetch a byte range. Returns `None` when the server ignores Range
    /// (HTTP 200 instead of 206), so callers can fall back to a full download.
    pub fn get_range(&self, rel: &str, start: u64, len: u64) -> Result<Option<Vec<u8>>> {
        if len == 0 {
            return Ok(Some(Vec::new()));
        }
        let url = self.url_for(rel);
        let end = start + len - 1;
        let response = self
            .request(&url)
            .header(reqwest::header::RANGE, format!("bytes={start}-{end}"))
            .send()
            .map_err(|e| Error::Http(format!("{url}: {e}")))?;

        match response.status().as_u16() {
            206 => response
                .bytes()
                .map(|b| Some(b.to_vec()))
                .map_err(|e| Error::Http(format!("{url}: {e}"))),
            200 => Ok(None),
            code => Err(Error::HttpStatus { url, status: code }),
        }
    }

    pub fn get_json(&self, rel: &str) -> Result<Value> {
        let bytes = self.get(rel)?;
        let text = String::from_utf8_lossy(&bytes);
        Ok(serde_json::from_str(crate::util::strip_bom(&text))?)
    }

    /// Streaming body, for servers that ignore `Range`. Callers read it in
    /// chunks so they can report progress.
    pub fn get_reader(&self, rel: &str) -> Result<Box<dyn std::io::Read>> {
        let url = self.url_for(rel);
        let response = self
            .request(&url)
            .send()
            .map_err(|e| Error::Http(format!("{url}: {e}")))?;
        let status = response.status();
        if !status.is_success() {
            return Err(Error::HttpStatus {
                url,
                status: status.as_u16(),
            });
        }
        Ok(Box::new(response))
    }

    pub fn base(&self) -> &str {
        &self.base
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_paths() {
        assert_eq!(encode_rel("@cba_ws/mod.srf"), "@cba_ws/mod.srf");
        assert_eq!(encode_rel("@my mod/a b.pbo"), "@my%20mod/a%20b.pbo");
    }
}
