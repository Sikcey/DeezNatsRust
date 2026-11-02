//! Authenticated, read-only Spotify catalog. This module has no dependency on UI types.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fmt,
    time::{Duration, Instant},
};
use url::Url;
#[path = "api_extra.rs"]
mod extra;
pub(crate) use extra::Device;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Error {
    InvalidRequest,
    SignInRequired,
    Forbidden,
    NotFound,
    RateLimited { retry_after: u64 },
    Http(u32),
    Transport,
    InvalidResponse,
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRequest => f.write_str("Invalid Spotify catalog request"),
            Self::SignInRequired => f.write_str("Spotify sign-in is required or has expired"),
            Self::Forbidden => f.write_str(
                "Spotify denied access; check granted scopes, app access, and playlist ownership",
            ),
            Self::NotFound => f.write_str("Spotify item is unavailable"),
            Self::RateLimited { retry_after } => write!(
                f,
                "Spotify rate limited requests; retry after {retry_after} seconds"
            ),
            Self::Http(status) => write!(f, "Spotify HTTP {status}"),
            Self::Transport => f.write_str("Spotify request failed or timed out"),
            Self::InvalidResponse => f.write_str("Invalid Spotify response"),
        }
    }
}
impl std::error::Error for Error {}

pub(crate) struct Response {
    pub status: u32,
    pub retry_after: Option<u64>,
    pub body: Vec<u8>,
}
pub(crate) trait Transport {
    fn token(&mut self, force_refresh: bool) -> Result<String, Error>;
    fn get(&mut self, url: &str, token: &str) -> Result<Response, Error>;
    fn send(
        &mut self,
        method: &str,
        url: &str,
        token: &str,
        body: &[u8],
    ) -> Result<Response, Error> {
        if method == "GET" && body.is_empty() {
            self.get(url, token)
        } else {
            Err(Error::InvalidRequest)
        }
    }
}
pub(crate) struct Live;
impl Transport for Live {
    fn token(&mut self, force: bool) -> Result<String, Error> {
        super::access_token(force).map_err(|_| Error::SignInRequired)
    }
    fn get(&mut self, url: &str, token: &str) -> Result<Response, Error> {
        self.send("GET", url, token, &[])
    }
    fn send(
        &mut self,
        method: &str,
        url: &str,
        token: &str,
        body: &[u8],
    ) -> Result<Response, Error> {
        let authorization = format!("Bearer {token}");
        let response = crate::lyrics::http::response(
            method,
            url,
            &[
                ("Authorization", &authorization),
                ("Content-Type", "application/json"),
            ],
            body,
        )
        .map_err(|_| Error::Transport)?;
        Ok(Response {
            status: response.status,
            retry_after: response.retry_after,
            body: response.body,
        })
    }
}
pub(crate) struct Client<T = Live> {
    transport: T,
    retry_at: Option<Instant>,
}
impl Client<Live> {
    pub fn saved() -> Self {
        Self::new(Live)
    }
}
impl<T: Transport> Client<T> {
    pub fn new(transport: T) -> Self {
        Self {
            transport,
            retry_at: None,
        }
    }
    /// Only Spotify API origins receive credentials, including API-provided pagination links.
    pub fn get(&mut self, address: &str) -> Result<Value, Error> {
        self.request("GET", address, None)
    }
    fn request(
        &mut self,
        method: &str,
        address: &str,
        body: Option<&Value>,
    ) -> Result<Value, Error> {
        let url = api_url(address)?;
        if let Some(deadline) = self.retry_at.filter(|d| *d > Instant::now()) {
            return Err(Error::RateLimited {
                retry_after: deadline
                    .saturating_duration_since(Instant::now())
                    .as_secs()
                    .saturating_add(1),
            });
        }
        let token = self.transport.token(false)?;
        let body = body
            .map(serde_json::to_vec)
            .transpose()
            .map_err(|_| Error::InvalidRequest)?
            .unwrap_or_default();
        let mut response = self.transport.send(method, url.as_str(), &token, &body)?;
        if response.status == 401 {
            let token = self.transport.token(true)?;
            response = self.transport.send(method, url.as_str(), &token, &body)?;
        }
        match response.status {
            200 | 201 if method != "GET" && response.body.is_empty() => Ok(Value::Null),
            200 | 201 => serde_json::from_slice(&response.body).map_err(|_| Error::InvalidResponse),
            202 | 204 => Ok(Value::Null),
            401 => Err(Error::SignInRequired),
            403 => Err(Error::Forbidden),
            404 => Err(Error::NotFound),
            429 => {
                // Do not block a worker thread or replay commands. Share this client to share cooldown.
                let seconds = response.retry_after.unwrap_or(5).clamp(1, 86400);
                self.retry_at = Some(Instant::now() + Duration::from_secs(seconds));
                Err(Error::RateLimited {
                    retry_after: seconds,
                })
            }
            status => Err(Error::Http(status)),
        }
    }
    pub fn profile(&mut self) -> Result<Value, Error> {
        self.get("https://api.spotify.com/v1/me")
    }
    pub fn search(&mut self, query: &str, kind: &str, offset: u32) -> Result<Page, Error> {
        if query.trim().is_empty()
            || query.len() > 1000
            || offset > 1000
            || !matches!(
                kind,
                "track" | "album" | "artist" | "playlist" | "episode" | "show"
            )
        {
            return Err(Error::InvalidRequest);
        }
        let mut url = Url::parse("https://api.spotify.com/v1/search").unwrap();
        url.query_pairs_mut()
            .append_pair("q", query)
            .append_pair("type", kind)
            .append_pair("limit", "10")
            .append_pair("offset", &offset.to_string());
        let value = self.get(url.as_str())?;
        let container = format!("{kind}s");
        let mut page = Page::parse(value.get(&container).ok_or(Error::InvalidResponse)?, None)?;
        page.container = Some(container);
        Ok(page)
    }
    pub fn playlists(&mut self, offset: u32) -> Result<Page, Error> {
        self.page("me/playlists", offset, None)
    }
    pub fn saved_tracks(&mut self, offset: u32) -> Result<Page, Error> {
        self.page("me/tracks", offset, Some("track"))
    }
    pub fn saved_albums(&mut self, offset: u32) -> Result<Page, Error> {
        self.page("me/albums", offset, Some("album"))
    }
    pub fn playlist_items(&mut self, id: &str, offset: u32) -> Result<Page, Error> {
        validate_id(id)?;
        self.page(&format!("playlists/{id}/items"), offset, Some("item"))
    }
    pub fn album_tracks(&mut self, id: &str, offset: u32) -> Result<Page, Error> {
        validate_id(id)?;
        self.page(&format!("albums/{id}/tracks"), offset, None)
    }
    pub fn item(&mut self, kind: &str, id: &str) -> Result<Metadata, Error> {
        validate_id(id)?;
        if !matches!(
            kind,
            "track" | "album" | "artist" | "playlist" | "episode" | "show"
        ) {
            return Err(Error::InvalidRequest);
        }
        Metadata::parse(&self.get(&format!("https://api.spotify.com/v1/{kind}s/{id}"))?)
            .ok_or(Error::InvalidResponse)
    }
    pub fn next(&mut self, page: &Page) -> Result<Option<Page>, Error> {
        page.next
            .as_ref()
            .map(|url| {
                self.get(url).and_then(|value| {
                    let source = match page.container.as_ref() {
                        Some(key) => value.get(key).ok_or(Error::InvalidResponse)?,
                        None => &value,
                    };
                    let mut next = Page::parse(source, page.wrapper.as_deref())?;
                    next.container = page.container.clone();
                    Ok(next)
                })
            })
            .transpose()
    }
    fn page(&mut self, path: &str, offset: u32, wrapper: Option<&str>) -> Result<Page, Error> {
        Page::parse(
            &self.get(&format!(
                "https://api.spotify.com/v1/{path}?limit=50&offset={offset}"
            ))?,
            wrapper,
        )
    }
}
fn api_url(address: &str) -> Result<Url, Error> {
    let url = Url::parse(address).map_err(|_| Error::InvalidRequest)?;
    if url.scheme() != "https"
        || url.host_str() != Some("api.spotify.com")
        || url.port_or_known_default() != Some(443)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || !url.path().starts_with("/v1/")
    {
        return Err(Error::InvalidRequest);
    }
    Ok(url)
}
fn validate_id(id: &str) -> Result<(), Error> {
    if id.len() == 22 && id.bytes().all(|b| b.is_ascii_alphanumeric()) {
        Ok(())
    } else {
        Err(Error::InvalidRequest)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub(crate) struct Metadata {
    /// Preserve Spotify's namespaced URI; never treat this ID as a Deezer ID.
    pub uri: String,
    pub id: String,
    pub kind: String,
    pub title: String,
    pub artists: Vec<String>,
    pub album: String,
    pub artwork: String,
    pub duration_ms: u64,
    pub playable: bool,
    pub external_url: String,
}
impl Metadata {
    pub fn parse(value: &Value) -> Option<Self> {
        let id = value["id"].as_str()?;
        let uri = value["uri"].as_str()?;
        let kind = value["type"].as_str().unwrap_or("track");
        if validate_id(id).is_err() || uri != format!("spotify:{kind}:{id}") {
            return None;
        }
        let images = value.get("images").or_else(|| value["album"].get("images"));
        let artwork = images
            .and_then(Value::as_array)
            .and_then(|images| crate::catalog::largest_artwork(images.iter()))
            .unwrap_or_default();
        Some(Self {
            uri: uri.into(),
            id: id.into(),
            kind: kind.into(),
            title: value["name"].as_str().unwrap_or_default().into(),
            artists: value["artists"]
                .as_array()
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|v| v["name"].as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default(),
            album: value["album"]["name"].as_str().unwrap_or_default().into(),
            artwork: artwork.into(),
            duration_ms: value["duration_ms"].as_u64().unwrap_or_default(),
            playable: matches!(kind, "track" | "episode")
                && value["is_playable"].as_bool() != Some(false)
                && value["is_local"].as_bool() != Some(true)
                && value["restrictions"].is_null(),
            external_url: value["external_urls"]["spotify"]
                .as_str()
                .unwrap_or_default()
                .into(),
        })
    }
}
#[derive(Clone, Debug, Serialize)]
pub(crate) struct Page {
    pub items: Vec<Metadata>,
    pub next: Option<String>,
    pub total: u64,
    pub offset: u64,
    pub skipped: usize,
    #[serde(skip)]
    wrapper: Option<String>,
    #[serde(skip)]
    container: Option<String>,
}
impl Page {
    fn parse(value: &Value, wrapper: Option<&str>) -> Result<Self, Error> {
        let source = value["items"].as_array().ok_or(Error::InvalidResponse)?;
        let mut skipped = 0;
        let items = source
            .iter()
            .filter_map(|v| {
                let item = match wrapper {
                    Some("item") => v
                        .get("item")
                        .or_else(|| v.get("track"))
                        .unwrap_or(&Value::Null),
                    Some(key) => &v[key],
                    None => v,
                };
                let result = Metadata::parse(item);
                if result.is_none() {
                    skipped += 1;
                }
                result
            })
            .collect();
        let next = match value.get("next") {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) => {
                api_url(s)?;
                Some(s.clone())
            }
            _ => return Err(Error::InvalidResponse),
        };
        Ok(Self {
            items,
            next,
            total: value["total"].as_u64().unwrap_or_default(),
            offset: value["offset"].as_u64().unwrap_or_default(),
            skipped,
            wrapper: wrapper.map(str::to_owned),
            container: None,
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    struct Mock {
        replies: VecDeque<Response>,
        tokens: Vec<bool>,
        urls: Vec<String>,
    }
    impl Transport for Mock {
        fn token(&mut self, force: bool) -> Result<String, Error> {
            self.tokens.push(force);
            Ok("secret".into())
        }
        fn get(&mut self, url: &str, _: &str) -> Result<Response, Error> {
            self.urls.push(url.into());
            Ok(self
                .replies
                .pop_front()
                .expect("unexpected network request"))
        }
    }
    fn client(statuses: &[u32]) -> Client<Mock> {
        Client::new(Mock {
            replies: statuses
                .iter()
                .map(|status| Response {
                    status: *status,
                    retry_after: Some(10),
                    body: b"{}".to_vec(),
                })
                .collect(),
            tokens: vec![],
            urls: vec![],
        })
    }
    #[test]
    fn retries_unauthorized_once_and_never_retries_forbidden() {
        let mut c = client(&[401, 200]);
        c.profile().unwrap();
        assert_eq!(c.transport.tokens, [false, true]);
        let mut c = client(&[401, 401]);
        assert_eq!(c.profile().unwrap_err(), Error::SignInRequired);
        assert_eq!(c.transport.urls.len(), 2);
        let mut c = client(&[403]);
        assert_eq!(c.profile().unwrap_err(), Error::Forbidden);
        assert_eq!(c.transport.tokens, [false]);
    }
    #[test]
    fn rate_limit_prevents_more_network_calls() {
        let mut c = client(&[429]);
        assert!(matches!(
            c.profile(),
            Err(Error::RateLimited { retry_after: 10 })
        ));
        assert!(matches!(c.profile(), Err(Error::RateLimited { .. })));
        assert_eq!(c.transport.urls.len(), 1);
    }
    #[test]
    fn rejects_untrusted_pagination_before_reading_credentials() {
        let mut c = client(&[]);
        for url in [
            "https://evil.example/v1/me",
            "https://api.spotify.com.evil/v1/me",
            "http://api.spotify.com/v1/me",
            "https://api.spotify.com:444/v1/me",
            "https://user@api.spotify.com/v1/me",
            "https://api.spotify.com/v1/me#fragment",
        ] {
            assert!(c.get(url).is_err());
        }
        assert!(c.transport.tokens.is_empty());
    }
    #[test]
    fn skips_removed_tracks_and_preserves_provider_identity() {
        let item = serde_json::json!({"id":"0123456789012345678901","uri":"spotify:track:0123456789012345678901","type":"track","name":"Song","duration_ms":1234,"artists":[{"name":"Artist"}]});
        let page = Page::parse(
            &serde_json::json!({"items":[{"item":item}, {"item":null}], "next":null,"total":2}),
            Some("item"),
        )
        .unwrap();
        assert_eq!(page.skipped, 1);
        assert_eq!(page.items[0].duration_ms, 1234);
        assert!(page.items[0].playable);
        assert!(page.items[0].uri.starts_with("spotify:"));
    }
    #[test]
    fn search_encodes_query_and_validates_kind_before_network() {
        let mut c = client(&[200]);
        let _ = c.search("a&token=bad", "track", 0);
        assert!(c.transport.urls[0].contains("q=a%26token%3Dbad"));
        assert_eq!(
            c.search("x", "../me", 0).unwrap_err(),
            Error::InvalidRequest
        );
    }
    #[test]
    fn search_pagination_preserves_nested_result_shape() {
        let first = serde_json::json!({"tracks":{"items":[],"total":0,"next":"https://api.spotify.com/v1/search?q=x&type=track&offset=10"}});
        let second = serde_json::json!({"tracks":{"items":[],"total":0,"next":null,"offset":10}});
        let mut c = client(&[]);
        c.transport.replies = [first, second]
            .into_iter()
            .map(|v| Response {
                status: 200,
                retry_after: None,
                body: serde_json::to_vec(&v).unwrap(),
            })
            .collect();
        let page = c.search("x", "track", 0).unwrap();
        let next = c.next(&page).unwrap().unwrap();
        assert_eq!(next.offset, 10);
        assert!(next.next.is_none());
    }
}
