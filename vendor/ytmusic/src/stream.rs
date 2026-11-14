//! A track's audio read in order while it downloads, one bounded range at a time.
//!
//! The stream host serves a plain GET at a trickle and drops long connections halfway without
//! saying so, but it answers a bounded `Range` request at full speed. So the body is asked for
//! a mebibyte at a time, each read has a deadline, and a range that stalls or breaks is asked
//! for again from the byte it stopped at. A url the host refuses partway through, which is what
//! a cold start token gets after its first megabyte or so, is renewed through the client that
//! handed it out, with a minted token when that client needs one, and read on from the same
//! byte.

use std::sync::{Arc, Weak};
use std::time::Duration;

use anyhow::{Context as _, Result, bail};
use bytes::Bytes;

use crate::client::YtMusic;
use crate::context::Client;
use crate::models::AudioFormat;

/// How much one request asks for.
const CHUNK: u64 = 1024 * 1024;
/// How long a request or a read may go without a byte before the range is asked for again.
const STALL: Duration = Duration::from_secs(8);
/// How many times in a row the same byte may be asked for before the stream gives up.
const RETRIES: u32 = 3;

/// The body of one audio format, handed out chunk by chunk in file order.
pub struct AudioStream {
    http: reqwest::Client,
    url: String,
    agent: &'static str,
    total: Option<u64>,
    /// The next byte to hand out.
    offset: u64,
    /// The last byte the open request covers.
    end: u64,
    response: Option<reqwest::Response>,
    /// How many attempts in a row have failed at `offset`.
    failures: u32,
    /// How to ask for a fresh url once the host refuses this one. `None` for a stream opened
    /// without a client, which then ends at the first refusal.
    renewal: Option<Renewal>,
}

/// What a stream needs to ask the player for a fresh url of the same file.
struct Renewal {
    client: Weak<YtMusic>,
    video_id: String,
    itag: u32,
    /// The client that handed out the url.
    from: Client,
    /// The byte the last renewal was made at. A renewed url refused at that same byte ends the
    /// stream instead of renewing forever.
    at: Option<u64>,
}

impl AudioStream {
    /// Sends the first range request and checks the stream host accepts it. A refusal here is
    /// the same answer a probe would have got, so a caller can fall back to another url.
    pub async fn open(http: reqwest::Client, format: &AudioFormat) -> Result<Self> {
        let mut stream = Self {
            http,
            url: format.url.clone(),
            agent: format.user_agent,
            total: format.content_length,
            offset: 0,
            end: 0,
            response: None,
            failures: 0,
            renewal: None,
        };
        stream.request().await?;
        Ok(stream)
    }

    /// Lets the stream renew its url through `client` when the host refuses it partway through.
    /// `from` is the client that handed the url out, which decides how a new one is asked for.
    pub(crate) fn renewing(
        mut self,
        client: &Arc<YtMusic>,
        video_id: &str,
        itag: u32,
        from: Client,
    ) -> Self {
        self.renewal = Some(Renewal {
            client: Arc::downgrade(client),
            video_id: video_id.to_string(),
            itag,
            from,
            at: None,
        });
        self
    }

    /// The length of the body, when the player response gave one.
    pub fn total(&self) -> Option<u64> {
        self.total
    }

    /// The next part of the body, or `None` once all of it has been handed out. Fails when the
    /// stream host refuses a range and the url cannot be renewed, or the same byte has stalled
    /// `RETRIES` times.
    pub async fn chunk(&mut self) -> Result<Option<Bytes>> {
        loop {
            if self.total.is_some_and(|total| self.offset >= total) {
                return Ok(None);
            }
            if self.response.is_none() {
                match self.request().await {
                    Ok(()) => {}
                    Err(error) if error.is::<Refused>() => {
                        self.renew(error).await?;
                        continue;
                    }
                    Err(error) => {
                        self.retry(error)?;
                        continue;
                    }
                }
            }
            let Some(response) = self.response.as_mut() else {
                continue;
            };
            match tokio::time::timeout(STALL, response.chunk()).await {
                Ok(Ok(Some(chunk))) => {
                    anyhow::ensure!(
                        chunk.len() as u64 <= self.end + 1 - self.offset,
                        "stream body exceeds requested range"
                    );
                    self.offset += chunk.len() as u64;
                    self.failures = 0;
                    return Ok(Some(chunk));
                }
                Ok(Ok(None)) => {
                    self.response = None;
                    if self.offset <= self.end {
                        self.retry(anyhow::anyhow!("stream body ended before requested range"))?;
                        continue;
                    }
                }
                Ok(Err(error)) => self.retry(anyhow::Error::new(error))?,
                Err(elapsed) => self.retry(anyhow::Error::new(elapsed))?,
            }
        }
    }

    /// Asks for the next range from `offset`.
    async fn request(&mut self) -> Result<()> {
        let end = match self.total {
            Some(total) => (self.offset + CHUNK - 1).min(total.saturating_sub(1)),
            None => self.offset + CHUNK - 1,
        };
        let sent = request(&self.http, &self.url, self.agent, self.offset, end);
        let response = tokio::time::timeout(STALL, sent)
            .await
            .context("the stream host did not answer")??;
        if let Some(value) = response
            .headers()
            .get("content-range")
            .and_then(|v| v.to_str().ok())
        {
            if let Some((_, total)) = value.split_once('/') {
                if let Ok(total) = total.parse::<u64>() {
                    if let Some(expected) = self.total {
                        anyhow::ensure!(expected == total, "stream length changed");
                    }
                    self.total = Some(total);
                }
            }
        } else if self.offset == 0 {
            self.total = response.content_length().or(self.total);
        }
        self.end = response
            .headers()
            .get("content-range")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split_once('/'))
            .and_then(|(bounds, _)| bounds.rsplit_once('-'))
            .and_then(|(_, last)| last.parse().ok())
            .unwrap_or_else(|| {
                response
                    .content_length()
                    .map(|length| self.offset + length - 1)
                    .unwrap_or(end)
            });
        self.response = Some(response);
        Ok(())
    }

    /// Swaps in a fresh url for the same file after the host refused this one, or hands back
    /// `refused` when there is no way to renew or a renewed url was already refused here.
    async fn renew(&mut self, refused: anyhow::Error) -> Result<()> {
        let Some(renewal) = self.renewal.as_mut() else {
            return Err(refused);
        };
        if renewal.at == Some(self.offset) {
            return Err(refused);
        }
        let Some(client) = renewal.client.upgrade() else {
            return Err(refused);
        };
        renewal.at = Some(self.offset);
        log::debug!(
            "player: {} was refused at byte {}, renewing the url",
            renewal.video_id,
            self.offset
        );
        let format = client
            .renewed_audio(&renewal.video_id, renewal.itag, renewal.from)
            .await
            .map_err(|error| refused.context(format!("cannot renew the url: {error:#}")))?;
        if format.content_length != self.total {
            bail!(
                "the renewed url for {} is {:?} bytes long, not {:?}",
                renewal.video_id,
                format.content_length,
                self.total
            );
        }
        self.url = format.url;
        self.agent = format.user_agent;
        Ok(())
    }

    /// Drops the broken range so the next read asks again from `offset`, or gives up.
    fn retry(&mut self, error: anyhow::Error) -> Result<()> {
        self.response = None;
        self.failures += 1;
        if self.failures > RETRIES {
            return Err(error.context(format!(
                "the stream stalled at byte {} {RETRIES} times",
                self.offset
            )));
        }
        log::debug!(
            "player: the stream broke at byte {}, asking again: {error:#}",
            self.offset
        );
        Ok(())
    }
}

/// The stream host answered a range with an error status. Asking again gets the same answer.
#[derive(Debug)]
pub(crate) struct Refused(reqwest::StatusCode);

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "stream host refused the download with status {} \
             (the proof-of-origin token it carried was not accepted)",
            self.0
        )
    }
}

impl std::error::Error for Refused {}

/// Sends one range request and hands back the response once its status says the body follows.
pub(crate) async fn request(
    http: &reqwest::Client,
    url: &str,
    agent: &'static str,
    start: u64,
    end: u64,
) -> Result<reqwest::Response> {
    let response = http
        .get(url)
        .header("Range", format!("bytes={start}-{end}"))
        .header("User-Agent", agent)
        .header("Accept-Encoding", "identity")
        .header("Origin", "https://www.youtube.com")
        .header("Referer", "https://www.youtube.com/")
        .send()
        .await
        .context("cannot reach stream host")?;
    let status = response.status();
    if !status.is_success() {
        bail!(Refused(status));
    }
    validate_range_response(status, response.headers(), start, end)?;
    Ok(response)
}

fn validate_range_response(
    status: reqwest::StatusCode,
    headers: &reqwest::header::HeaderMap,
    start: u64,
    end: u64,
) -> Result<()> {
    anyhow::ensure!(end >= start, "invalid requested range");
    if status == reqwest::StatusCode::PARTIAL_CONTENT {
        let value = headers
            .get("content-range")
            .context("missing Content-Range")?
            .to_str()
            .context("invalid Content-Range")?;
        let (bounds, total) = value
            .strip_prefix("bytes ")
            .and_then(|v| v.split_once('/'))
            .context("invalid Content-Range")?;
        let (first, last) = bounds.split_once('-').context("invalid Content-Range")?;
        let first: u64 = first.parse()?;
        let last: u64 = last.parse()?;
        anyhow::ensure!(
            first == start && last >= first && last <= end,
            "unexpected Content-Range"
        );
        if total != "*" {
            let total: u64 = total.parse()?;
            anyhow::ensure!(
                last < total && (last == end || last + 1 == total),
                "incomplete Content-Range"
            );
        } else {
            anyhow::ensure!(last == end, "incomplete range with unknown total");
        }
        if let Some(length) = headers.get("content-length") {
            anyhow::ensure!(
                length.to_str()?.parse::<u64>()? == last - first + 1,
                "range length mismatch"
            );
        }
    } else {
        let length: u64 = headers
            .get("content-length")
            .context("unbounded non-range response")?
            .to_str()?
            .parse()?;
        anyhow::ensure!(
            status == reqwest::StatusCode::OK
                && start == 0
                && length > 0
                && length <= end - start + 1,
            "host ignored requested range"
        );
    }
    Ok(())
}

#[cfg(test)]
mod range_tests {
    use super::*;
    use reqwest::{StatusCode, header::HeaderMap};

    #[test]
    fn range_headers_reject_wrong_offsets_and_unbounded_bodies() {
        let mut headers = HeaderMap::new();
        headers.insert("content-range", "bytes 10-19/100".parse().unwrap());
        headers.insert("content-length", "10".parse().unwrap());
        assert!(validate_range_response(StatusCode::PARTIAL_CONTENT, &headers, 10, 19).is_ok());
        assert!(validate_range_response(StatusCode::PARTIAL_CONTENT, &headers, 0, 9).is_err());
        headers.insert("content-length", "11".parse().unwrap());
        assert!(validate_range_response(StatusCode::PARTIAL_CONTENT, &headers, 10, 19).is_err());
        assert!(validate_range_response(StatusCode::OK, &HeaderMap::new(), 0, 9).is_err());
        assert!(validate_range_response(StatusCode::OK, &headers, 10, 19).is_err());
    }

    #[test]
    fn final_range_may_be_short_only_at_object_end() {
        let mut headers = HeaderMap::new();
        headers.insert("content-range", "bytes 10-14/15".parse().unwrap());
        assert!(validate_range_response(StatusCode::PARTIAL_CONTENT, &headers, 10, 19).is_ok());
        headers.insert("content-range", "bytes 10-14/100".parse().unwrap());
        assert!(validate_range_response(StatusCode::PARTIAL_CONTENT, &headers, 10, 19).is_err());
    }
}
