//! Experimental Spotify backend. OAuth/cookie credentials never enter Soloist arguments.
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    io::{self, BufRead},
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
};
use url::Url;

const REDIRECT: &str = "http://127.0.0.1:43821/spotify/callback";
pub(crate) mod api {
    //! Authenticated, read-only Spotify catalog. This module has no dependency on UI types.
    use serde::{Deserialize, Serialize};
    use serde_json::Value;
    use std::{
        fmt,
        time::{Duration, Instant},
    };
    use url::Url;
    mod extra {
        //! Documented Web API operations. No scraping, invented recommendation or Jam endpoints.
        use super::*;
        use serde_json::json;

        #[derive(Clone, Debug, Deserialize, Serialize)]
        pub(crate) struct Device {
            pub id: Option<String>,
            pub name: String,
            #[serde(rename = "type")]
            pub kind: String,
            pub is_active: bool,
            pub is_restricted: bool,
            pub volume_percent: Option<u8>,
            #[serde(default)]
            pub supports_volume: bool,
        }

        impl<T: Transport> Client<T> {
            pub fn update_playlist(
                &mut self,
                id: &str,
                name: &str,
                description: Option<&str>,
            ) -> Result<(), Error> {
                validate_id(id)?;
                if name.trim().is_empty()
                    || name.len() > 300
                    || description.is_some_and(|d| d.len() > 10_000)
                {
                    return Err(Error::InvalidRequest);
                }
                let mut body = json!({"name":name});
                if let Some(description) = description {
                    body["description"] = json!(description);
                }
                self.request(
                    "PUT",
                    &format!("https://api.spotify.com/v1/playlists/{id}"),
                    Some(&body),
                )
                .map(|_| ())
            }
            pub fn recently_played(&mut self, before: Option<u64>) -> Result<Page, Error> {
                let mut url =
                    Url::parse("https://api.spotify.com/v1/me/player/recently-played").unwrap();
                url.query_pairs_mut().append_pair("limit", "50");
                if let Some(before) = before {
                    url.query_pairs_mut()
                        .append_pair("before", &before.to_string());
                }
                Page::parse(&self.get(url.as_str())?, Some("track"))
            }
            pub fn saved_shows(&mut self, offset: u32) -> Result<Page, Error> {
                self.page("me/shows", offset, Some("show"))
            }
            pub fn saved_episodes(&mut self, offset: u32) -> Result<Page, Error> {
                self.page("me/episodes", offset, Some("episode"))
            }
            pub fn show_episodes(&mut self, id: &str, offset: u32) -> Result<Page, Error> {
                validate_id(id)?;
                self.page(&format!("shows/{id}/episodes"), offset, None)
            }
            pub fn artist_albums(&mut self, id: &str, offset: u32) -> Result<Page, Error> {
                validate_id(id)?;
                self.page(&format!("artists/{id}/albums"), offset, None)
            }
            pub fn followed_artists(&mut self, after: Option<&str>) -> Result<Page, Error> {
                let mut url =
                    Url::parse("https://api.spotify.com/v1/me/following?type=artist&limit=50")
                        .unwrap();
                if let Some(id) = after {
                    validate_id(id)?;
                    url.query_pairs_mut().append_pair("after", id);
                }
                let value = self.get(url.as_str())?;
                let mut page = Page::parse(&value["artists"], None)?;
                page.container = Some("artists".into());
                Ok(page)
            }
            pub fn devices(&mut self) -> Result<Vec<Device>, Error> {
                let value = self.get("https://api.spotify.com/v1/me/player/devices")?;
                serde_json::from_value(value["devices"].clone()).map_err(|_| Error::InvalidResponse)
            }
            pub fn transfer_playback(&mut self, device_id: &str, play: bool) -> Result<(), Error> {
                if device_id.is_empty()
                    || device_id.len() > 256
                    || device_id.chars().any(char::is_control)
                {
                    return Err(Error::InvalidRequest);
                }
                self.request(
                    "PUT",
                    "https://api.spotify.com/v1/me/player",
                    Some(&json!({"device_ids":[device_id],"play":play})),
                )
                .map(|_| ())
            }
            pub fn library_contains(&mut self, uris: &[String]) -> Result<Vec<bool>, Error> {
                let url = library_url("me/library/contains", uris)?;
                serde_json::from_value(self.get(url.as_str())?).map_err(|_| Error::InvalidResponse)
            }
            pub fn save_library(&mut self, uris: &[String], save: bool) -> Result<(), Error> {
                let url = library_url("me/library", uris)?;
                self.request(if save { "PUT" } else { "DELETE" }, url.as_str(), None)
                    .map(|_| ())
            }
            pub fn create_playlist(&mut self, name: &str, public: bool) -> Result<Value, Error> {
                if name.trim().is_empty() || name.len() > 300 {
                    return Err(Error::InvalidRequest);
                }
                self.request(
                    "POST",
                    "https://api.spotify.com/v1/me/playlists",
                    Some(&json!({"name":name,"public":public})),
                )
            }
            pub fn add_playlist_items(
                &mut self,
                id: &str,
                uris: &[String],
            ) -> Result<Value, Error> {
                validate_id(id)?;
                validate_uris(uris, true)?;
                self.request(
                    "POST",
                    &format!("https://api.spotify.com/v1/playlists/{id}/items"),
                    Some(&json!({"uris":uris})),
                )
            }
            pub fn remove_playlist_items(
                &mut self,
                id: &str,
                uris: &[String],
                snapshot_id: Option<&str>,
            ) -> Result<Value, Error> {
                validate_id(id)?;
                validate_uris(uris, true)?;
                let mut body =
                    json!({"items":uris.iter().map(|uri|json!({"uri":uri})).collect::<Vec<_>>()});
                if let Some(snapshot) = snapshot_id {
                    body["snapshot_id"] = json!(snapshot);
                }
                self.request(
                    "DELETE",
                    &format!("https://api.spotify.com/v1/playlists/{id}/items"),
                    Some(&body),
                )
            }
        }

        impl Client<super::Live> {
            pub fn upload_playlist_cover(
                &mut self,
                id: &str,
                path: &std::path::Path,
            ) -> Result<(), Error> {
                validate_id(id)?;
                if std::fs::metadata(path)
                    .map_err(|_| Error::InvalidRequest)?
                    .len()
                    > 192 * 1024
                {
                    return Err(Error::InvalidRequest);
                }
                let bytes = std::fs::read(path).map_err(|_| Error::InvalidRequest)?;
                if !bytes.starts_with(&[0xff, 0xd8, 0xff]) {
                    return Err(Error::InvalidRequest);
                }
                use base64::Engine;
                let body = base64::engine::general_purpose::STANDARD.encode(bytes);
                for force in [false, true] {
                    let token = self.transport.token(force)?;
                    let authorization = format!("Bearer {token}");
                    let response = crate::lyrics::http::response(
                        "PUT",
                        &format!("https://api.spotify.com/v1/playlists/{id}/images"),
                        &[
                            ("Authorization", &authorization),
                            ("Content-Type", "image/jpeg"),
                        ],
                        body.as_bytes(),
                    )
                    .map_err(|_| Error::Transport)?;
                    match response.status {
                        200 | 202 | 204 => return Ok(()),
                        401 if !force => continue,
                        401 => return Err(Error::SignInRequired),
                        403 => return Err(Error::Forbidden),
                        404 => return Err(Error::NotFound),
                        429 => {
                            return Err(Error::RateLimited {
                                retry_after: response.retry_after.unwrap_or(5),
                            })
                        }
                        status => return Err(Error::Http(status)),
                    }
                }
                Err(Error::SignInRequired)
            }
        }
        fn validate_uris(uris: &[String], playable_only: bool) -> Result<(), Error> {
            if uris.is_empty() || uris.len() > 100 {
                return Err(Error::InvalidRequest);
            }
            for uri in uris {
                let parts: Vec<_> = uri.split(':').collect();
                if parts.len() != 3
                    || parts[0] != "spotify"
                    || !matches!(
                        parts[1],
                        "track"
                            | "episode"
                            | "album"
                            | "artist"
                            | "playlist"
                            | "show"
                            | "audiobook"
                    )
                    || (playable_only && !matches!(parts[1], "track" | "episode"))
                {
                    return Err(Error::InvalidRequest);
                }
                validate_id(parts[2])?;
            }
            Ok(())
        }
        fn library_url(path: &str, uris: &[String]) -> Result<Url, Error> {
            if uris.len() > 40 || uris.iter().any(|uri| uri.starts_with("spotify:artist:")) {
                return Err(Error::InvalidRequest);
            }
            validate_uris(uris, false)?;
            let mut url = Url::parse(&format!("https://api.spotify.com/v1/{path}")).unwrap();
            url.query_pairs_mut().append_pair("uris", &uris.join(","));
            Ok(url)
        }

        #[cfg(test)]
        mod tests {
            use super::*;
            use std::collections::VecDeque;
            #[derive(Default)]
            struct Mock {
                responses: VecDeque<Response>,
                calls: Vec<(String, String, Vec<u8>)>,
                tokens: Vec<bool>,
            }
            impl Transport for Mock {
                fn token(&mut self, force: bool) -> Result<String, Error> {
                    self.tokens.push(force);
                    Ok("test-token".into())
                }
                fn get(&mut self, url: &str, token: &str) -> Result<Response, Error> {
                    self.send("GET", url, token, &[])
                }
                fn send(
                    &mut self,
                    method: &str,
                    url: &str,
                    _token: &str,
                    body: &[u8],
                ) -> Result<Response, Error> {
                    self.calls.push((method.into(), url.into(), body.to_vec()));
                    self.responses.pop_front().ok_or(Error::Transport)
                }
            }
            fn response(status: u32, body: &str) -> Response {
                Response {
                    status,
                    body: body.as_bytes().to_vec(),
                    retry_after: None,
                }
            }
            #[test]
            fn device_transfer_refreshes_once_and_handles_no_content() {
                let mut mock = Mock::default();
                mock.responses = VecDeque::from([response(401, "{}"), response(204, "")]);
                let mut client = Client::new(mock);
                client.transfer_playback("real-device", true).unwrap();
                assert_eq!(client.transport.tokens, vec![false, true]);
                assert_eq!(client.transport.calls.len(), 2);
                assert_eq!(client.transport.calls[0].0, "PUT");
                assert_eq!(
                    client.transport.calls[0].1,
                    "https://api.spotify.com/v1/me/player"
                );
                assert_eq!(
                    serde_json::from_slice::<Value>(&client.transport.calls[0].2).unwrap(),
                    json!({"device_ids":["real-device"],"play":true})
                );
            }
            #[test]
            fn library_is_bounded_and_successful_empty_200_is_valid() {
                let uri = "spotify:track:1234567890123456789012".to_owned();
                let mut mock = Mock::default();
                mock.responses.push_back(response(200, ""));
                let mut client = Client::new(mock);
                assert_eq!(
                    client.save_library(&vec![uri.clone(); 41], true),
                    Err(Error::InvalidRequest)
                );
                assert!(client.transport.calls.is_empty());
                client.save_library(&[uri], false).unwrap();
                assert_eq!(client.transport.calls[0].0, "DELETE");
                assert!(client.transport.calls[0]
                    .1
                    .contains("uris=spotify%3Atrack%3A"));
            }
            #[test]
            fn playlist_removal_uses_current_items_body_and_rejects_foreign_ids() {
                let mut mock = Mock::default();
                mock.responses
                    .push_back(response(200, "{\"snapshot_id\":\"new\"}"));
                let mut client = Client::new(mock);
                assert!(client
                    .remove_playlist_items(
                        "1234567890123456789012",
                        &["deezer:track:1".into()],
                        None
                    )
                    .is_err());
                assert!(client.transport.calls.is_empty());
                client
                    .remove_playlist_items(
                        "1234567890123456789012",
                        &["spotify:track:1234567890123456789012".into()],
                        Some("old"),
                    )
                    .unwrap();
                assert!(client.transport.calls[0].1.ends_with("/items"));
                let body: Value = serde_json::from_slice(&client.transport.calls[0].2).unwrap();
                assert_eq!(
                    body["items"][0]["uri"],
                    "spotify:track:1234567890123456789012"
                );
                assert_eq!(body["snapshot_id"], "old");
                assert!(body.get("tracks").is_none());
            }
        }
    }
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
                200 | 201 => {
                    serde_json::from_slice(&response.body).map_err(|_| Error::InvalidResponse)
                }
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
            let second =
                serde_json::json!({"tracks":{"items":[],"total":0,"next":null,"offset":10}});
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
}
pub(crate) mod capabilities {
    //! Capability means an implemented control surface, not a feature of Spotify's official app.
    use serde::Serialize;
    #[derive(Clone, Copy, Debug, Serialize)]
    pub(crate) struct Capabilities {
        pub downloads: bool,
        pub radio: bool,
        pub connect_devices: bool,
        pub jam_participation: bool,
        pub jam_management: bool,
        pub host_remote_control: bool,
        pub host_audio_stream: bool,
        pub equalizer: bool,
        pub output_selection: bool,
        pub normalization: bool,
        pub crossfade: bool,
    }
    pub(crate) const CAPABILITIES: Capabilities = Capabilities {
        // Soloist's volatile playback cache does not provide offline downloads.
        downloads: false,
        radio: true,
        connect_devices: true,
        jam_participation: true,
        jam_management: true,
        host_remote_control: true,
        host_audio_stream: true,
        // Soloist renders directly through WASAPI. Windows Volume mixer can route
        // its output, but that is not an implemented in-app output/DSP control.
        equalizer: false,
        output_selection: false,
        normalization: false,
        crossfade: false,
    };
}
mod catalog {
    //! Compatibility names are shared with the native catalog; only Spotify endpoints are called.
    use super::api::{Client, Error};
    use serde_json::{json, Value};
    use std::sync::Mutex;
    static CLIENT: Mutex<Option<Client>> = Mutex::new(None);

    fn arg(args: &[Value], index: usize) -> Result<&str, String> {
        args.get(index)
            .and_then(Value::as_str)
            .ok_or_else(|| "Missing Spotify request argument".into())
    }
    fn id<'a>(args: &'a [Value], index: usize, kind: &str) -> Result<&'a str, String> {
        let value = arg(args, index)?;
        let prefix = format!("spotify:{kind}:");
        let id = value
            .strip_prefix(&prefix)
            .ok_or("Select a Spotify item of the correct type")?;
        if id.len() != 22 || !id.bytes().all(|b| b.is_ascii_alphanumeric()) {
            return Err("Invalid Spotify item identifier".into());
        }
        Ok(id)
    }
    fn raw(client: &mut Client, path: &str) -> Result<Value, String> {
        get(client, &format!("https://api.spotify.com/v1/{path}"))
    }
    fn get(client: &mut Client, url: &str) -> Result<Value, String> {
        if super::web::has_session() {
            super::web::rest_get(url)
        } else {
            client.get(url).map_err(|error| error.to_string())
        }
    }
    fn unwrapped(mut value: Value, wrapper: &str) -> Value {
        if let Some(items) = value["items"].as_array_mut() {
            for item in items {
                if let Some(entity) = item
                    .get(wrapper)
                    .cloned()
                    .or_else(|| item.get("track").cloned())
                {
                    let mut entity = entity;
                    if let Some(object) = entity.as_object_mut() {
                        for key in ["played_at", "added_at", "added_by"] {
                            if let Some(value) = item.get(key) {
                                object.insert(key.into(), value.clone());
                            }
                        }
                    }
                    *item = entity;
                }
            }
        }
        value
    }
    pub(crate) fn invoke(name: &str, args: &[Value]) -> Result<Value, String> {
        if name == "DZQueueJSON" && super::web::has_session() {
            let state = super::web_devices::snapshot()?;
            return Ok(
                json!({"items":state.as_ref().and_then(|s|s["next_tracks"].as_array()).cloned().unwrap_or_default(),"next":null}),
            );
        }
        if name == "DZAccountJSON" && super::web::has_session() {
            return super::web::account();
        }
        let web = super::web_catalog::invoke(name, args);
        if let Some(Ok(value)) = &web {
            return Ok(value.clone());
        }
        // Do not replay an indeterminate authenticated mutation through another transport.
        if super::web::has_session() && super::web_catalog::is_mutation(name) {
            if let Some(result) = web {
                return result;
            }
        }
        if super::web::has_session() {
            if let Some(result) = web {
                return result;
            }
        }
        public_invoke(name, args).or_else(|public_error| match web {
            Some(Err(web_error)) => Err(web_error),
            _ => Err(public_error),
        })
    }
    fn public_invoke(name: &str, args: &[Value]) -> Result<Value, String> {
        let mut slot = CLIENT
            .lock()
            .map_err(|_| "Spotify catalog worker unavailable")?;
        let client = slot.get_or_insert_with(Client::saved);
        match name {
        "DZAccountJSON" => client.profile().map_err(|e|e.to_string()),
        "DZFavoritesJSON" => raw(client,"me/tracks?limit=50").map(|v|unwrapped(v,"track")),
        "DZFavoriteAlbumsJSON" => raw(client,"me/albums?limit=50").map(|v|unwrapped(v,"album")),
        "DZPlaylistsJSON" => raw(client,"me/playlists?limit=50"),
        "DZHistoryRecentJSON" => raw(client,"me/player/recently-played?limit=50").map(|v|unwrapped(v,"track")),
        "DZPodcastShelfJSON" => if args.first().and_then(Value::as_u64)==Some(3) {raw(client,"me/episodes?limit=50").map(|v|unwrapped(v,"episode"))} else {raw(client,"me/shows?limit=50").map(|v|unwrapped(v,"show"))},
        "DZTrackMetadataJSON" => raw(client,&format!("tracks/{}",id(args,0,"track")?)),
        "DZArtistProfileJSON" => raw(client,&format!("artists/{}",id(args,0,"artist")?)),
        "DZArtistTopJSON" => raw(client,&format!("artists/{}/top-tracks",id(args,0,"artist")?)),
        "DZArtistSectionJSON" if args.get(1).and_then(Value::as_u64)==Some(0) => raw(client,&format!("artists/{}/albums?limit=50",id(args,0,"artist")?)),
        "DZAlbumTracksJSON" => raw(client,&format!("albums/{}/tracks?limit=50",id(args,0,"album")?)),
        "DZPlaylistTracksJSON" => raw(client,&format!("playlists/{}/items?limit=50",id(args,0,"playlist")?)).map(|v|unwrapped(v,"item")),
        "DZPodcastEpisodesJSON" => raw(client,&format!("shows/{}/episodes?limit=50",id(args,0,"show")?)),
        "SPCatalogItemJSON" => {
            let kind=arg(args,0)?;
            if !matches!(kind,"track"|"album"|"artist"|"playlist"|"show"|"episode") {return Err(Error::InvalidRequest.to_string());}
            raw(client,&format!("{kind}s/{}",id(args,1,kind)?))
        }
        "SPCatalogNextJSON" => {
            let mut value=get(client,arg(args,0)?)?;
            for wrapper in ["track","album","show","episode","item"] { value=unwrapped(value,wrapper); }
            Ok(value)
        }
        "DZSearchJSON" | "DZSearchPodcastsJSON" => {
            let query=arg(args,0)?.trim();
            if query.is_empty() || query.len()>1000 {return Err(Error::InvalidRequest.to_string());}
            let mut url=url::Url::parse("https://api.spotify.com/v1/search").unwrap();
            url.query_pairs_mut().append_pair("q",query).append_pair("type",if name=="DZSearchPodcastsJSON" {"show,episode"} else {"track,album,artist,playlist,show,episode"}).append_pair("limit","10");
            get(client,url.as_str())
        }
        "DZHomeModuleJSON" => match args.first().and_then(Value::as_u64) {
            Some(0)=>raw(client,"me/player/recently-played?limit=50").map(|v|unwrapped(v,"track")),
            Some(1)=>raw(client,"me/tracks?limit=50").map(|v|unwrapped(v,"track")),
            Some(2)=>raw(client,"me/playlists?limit=50"),
            Some(3)|Some(8)=>raw(client,"me/albums?limit=50").map(|v|unwrapped(v,"album")),
            Some(4)=>raw(client,"me/following?type=artist&limit=50").map(|v|v["artists"].clone()),
            Some(5)=>raw(client,"me/top/tracks?limit=50"),
            Some(6)=>raw(client,"me/top/artists?limit=50"),
            Some(7)=>raw(client,"me/shows?limit=50").map(|v|unwrapped(v,"show")),
            Some(9)=>raw(client,"me/episodes?limit=50").map(|v|unwrapped(v,"episode")),
            _=>Err("No Spotify home shelf for this index".into()),
        },
        "DZQueueJSON" => {
            let value=raw(client,"me/player/queue")?;
            Ok(json!({"items":value["queue"],"next":null}))
        }
        "RustPodcastLibraryJSON" if args.is_empty() => raw(client,"me/shows?limit=50").map(|value|json!({"podcasts":unwrapped(value,"show")["items"]})),
        "DZAlbumLibraryJSON" | "DZPlaylistLibraryJSON" | "RustPodcastLibraryJSON" => {
            let kind=match name {"DZAlbumLibraryJSON"=>"album","DZPlaylistLibraryJSON"=>"playlist",_=>"show"};
            id(args,0,kind)?;
            let save=match args.get(1) {Some(Value::Bool(value))=>*value,Some(Value::Number(value))=>value.as_i64().ok_or("Invalid library action")?!=0,_=>return Err("Missing library add/remove action".into())};
            client.save_library(&[arg(args,0)?.to_owned()],save).map(|_|json!(1)).map_err(|e|e.to_string())
        }
        "DZAddFavorite" | "DZRemoveFavorite" => {
            let uri=arg(args,0)?.to_owned();
            client.save_library(&[uri],name=="DZAddFavorite").map(|_|json!(1)).map_err(|e|e.to_string())
        }
        "DZAddToPlaylist" | "DZRemoveFromPlaylist" => {
            let playlist=id(args,0,"playlist")?;
            let uri=arg(args,1)?.to_owned();
            if name=="DZAddToPlaylist" {client.add_playlist_items(playlist,&[uri])} else {client.remove_playlist_items(playlist,&[uri],None)}.map_err(|e|e.to_string())
        }
        "DZCreatePlaylist" => client.create_playlist(arg(args,0)?,false).map(|mut v| {
            let uri = v["uri"].as_str().map(str::to_owned).unwrap_or_else(|| format!("spotify:playlist:{}",v["id"].as_str().unwrap_or_default()));
            v["id"] = json!(uri); v
        }).map_err(|e|e.to_string()),
        "DZRenamePlaylist" | "DZUpdatePlaylistJSON" => client.update_playlist(id(args,0,"playlist")?,arg(args,1)?,if name=="DZUpdatePlaylistJSON" {Some(arg(args,2)?)} else {None}).map(|_|json!(1)).map_err(|e|e.to_string()),
        "DZDeletePlaylist" => client.save_library(&[arg(args,0)?.to_owned()],false).map(|_|json!(1)).map_err(|e|e.to_string()),
        "DZPlaylistCoverJSON" => client.upload_playlist_cover(id(args,0,"playlist")?,std::path::Path::new(arg(args,1)?)).map(|_|json!(1)).map_err(|e|e.to_string()),
        "SPDevicesJSON" if super::web::has_session() => super::web_devices::devices(),
        "SPDevicesJSON" => client.devices().and_then(|d|serde_json::to_value(d).map_err(|_|Error::InvalidResponse)).map_err(|e|e.to_string()),
        "SPTransferPlayback" | "SPTransferPlaybackJSON" if super::web::has_session() => {
            super::web_devices::transfer(arg(args,0)?, args.get(1).and_then(Value::as_bool).unwrap_or(false))
        }
        "SPTransferPlayback" | "SPTransferPlaybackJSON" => {
            let device = arg(args,0)?;
            client.transfer_playback(device,args.get(1).and_then(Value::as_bool).unwrap_or(false)).map_err(|e|e.to_string())?;
            super::remote::select_device(Some(device.into()));
            Ok(Value::Null)
        },
        "SPCapabilitiesJSON" => Ok(json!(super::capabilities::CAPABILITIES)),
        "DZFlowJSON" | "DZFlowForConfigJSON" | "DZTrackMixJSON" | "DZArtistMixJSON" | "SPStyleMixJSON" | "StyleMixGenres" => Err("Spotify Web API does not expose Flow, constrained personal mixes or radio generation for this integration".into()),
        "SPCatalogDiscoveryJSON" => {
            let mut sections=Vec::new();
            for (title,query,kind) in [
                ("Playlist search: moods","mood","playlist"),
                ("Playlist search: genres","genre","playlist"),
                ("Podcast search: music","music","show"),
            ] {
                let mut url=url::Url::parse("https://api.spotify.com/v1/search").unwrap();
                url.query_pairs_mut().append_pair("q",query).append_pair("type",kind).append_pair("limit","10");
                match get(client,url.as_str()) {
                    Ok(value)=>sections.push(json!({"title":title,"items":value[format!("{kind}s")],"error":""})),
                    Err(error)=>sections.push(json!({"title":title,"items":[],"error":error.to_string()})),
                }
            }
            Ok(json!({"sections":sections}))
        }
        "SPCatalogChannelJSON" => Err("Spotify browse categories are unavailable for this integration; use Search".into()),
        _ => Err(format!("Spotify does not expose the requested operation ({name})")),
    }
    }
    pub(super) fn reset() {
        if let Ok(mut slot) = CLIENT.lock() {
            *slot = None;
        }
    }
}
mod directory_cache {
    //! Spotify account namespace for the shared browse disk cache.
    use serde_json::Value;
    pub(super) fn peek(key: &str) -> Option<Value> {
        crate::catalog::disk_cache::peek("spotify", super::web::directory_cache_scope().ok()?, key)
    }
    pub(super) fn load(
        key: &str,
        fetch: impl FnOnce() -> Result<Value, String>,
    ) -> Result<Value, String> {
        match super::web::directory_cache_scope() {
            Ok(scope) if key.starts_with("flow-mood-") => {
                crate::catalog::disk_cache::refresh("spotify", scope, key, fetch)
            }
            Ok(scope) => crate::catalog::disk_cache::load("spotify", scope, key, fetch),
            Err(_) => fetch(),
        }
    }
}
pub(crate) mod flow {
    //! Local Flow preferences; Spotify playlists supply the music.
    use serde::{Deserialize, Serialize};
    use serde_json::{json, Value};

    pub(crate) const PREFIX: &str = "spotify-flow:";

    fn searchable(value: &str) -> String {
        format!(
            " {} ",
            value
                .to_lowercase()
                .chars()
                .map(|c| {
                    if c.is_alphanumeric() {
                        c
                    } else {
                        ' '
                    }
                })
                .collect::<String>()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
        )
    }

    /// Derive mood genres from the provider's current playlist titles/descriptions.
    /// Saved selections always win over discovery, including an explicitly empty selection.
    pub(crate) fn provider_tuner(
        key: &str,
        directory: &Value,
        playlists: &Value,
    ) -> Result<Value, String> {
        let mut selection = Selection::parse(key)?;
        let saved = key.starts_with(PREFIX);
        if !saved && selection.mood != "default" && playlists.as_array().is_none_or(Vec::is_empty) {
            return Err(
                "The provider returned no playlists for this mood. Try again after reconnecting."
                    .into(),
            );
        }
        if !saved {
            selection.genres.clear();
        }
        let mut names: Vec<String> = genres().iter().map(|g| (*g).to_owned()).collect();
        for genre in directory.as_array().into_iter().flatten() {
            if genre["kind"] == "mood" {
                continue;
            }
            if let Some(name) = genre["title"].as_str().filter(|name| valid_genre(name)) {
                if !names.iter().any(|known| known.eq_ignore_ascii_case(name)) {
                    names.push(name.to_owned());
                }
            }
        }
        for name in &selection.genres {
            if !names.iter().any(|known| known.eq_ignore_ascii_case(name)) {
                names.push(name.clone());
            }
        }
        let sources: Vec<_> = playlists
            .as_array()
            .into_iter()
            .flatten()
            .map(|item| {
                (
                    searchable(&format!(
                        "{} {}",
                        item["title"].as_str().unwrap_or_default(),
                        item["description"].as_str().unwrap_or_default()
                    )),
                    item,
                )
            })
            .collect();
        let genres: Vec<_> = names
            .iter()
            .map(|name| {
                let pattern = searchable(name);
                let source = sources
                    .iter()
                    .find(|(text, _)| text.contains(&pattern))
                    .map(|(_, item)| *item);
                let native = directory.as_array().into_iter().flatten().find(|genre| {
                    genre["title"]
                        .as_str()
                        .is_some_and(|title| title.eq_ignore_ascii_case(name))
                });
                let artwork = native
                    .and_then(|g| g["artwork"].as_str())
                    .filter(|s| !s.is_empty())
                    .or_else(|| source.and_then(|item| item["artwork"].as_str()))
                    .unwrap_or_default();
                let enabled = if saved {
                    selection
                        .genres
                        .iter()
                        .any(|g| g.eq_ignore_ascii_case(name))
                } else {
                    selection.mood != "default" && source.is_some()
                };
                json!({"id":name,"title":name,"isEnabled":enabled,"artwork":artwork,
                    "isMoodGenre":selection.mood == "default" || source.is_some()})
            })
            .collect();
        Ok(json!({"id":key,"customizable":true,"genres":genres,"moodSource":"provider-playlists"}))
    }

    pub(crate) fn artwork_key(provider: crate::catalog::Provider, genre: &str) -> String {
        format!(
            "flow-genre:{}:{genre}",
            match provider {
                crate::catalog::Provider::Spotify => "spotify",
                crate::catalog::Provider::YouTube => "youtube",
                crate::catalog::Provider::Deezer => "deezer",
            }
        )
    }

    pub(crate) fn resolve_artwork(
        key: &str,
        engine: Option<&crate::engine::Engine>,
    ) -> Result<String, String> {
        let (provider, genre) = key
            .strip_prefix("flow-genre:")
            .and_then(|s| s.split_once(':'))
            .ok_or("Invalid genre artwork key")?;
        if provider == "image" {
            return if genre.starts_with("https://") {
                Ok(genre.to_owned())
            } else {
                Err("Invalid genre artwork URL".into())
            };
        }
        if !valid_genre(genre) {
            return Err("Invalid genre artwork name".into());
        }
        match provider {
            "spotify" => crate::spotify::genre_artwork(genre),
            "youtube" => crate::youtube::genre_artwork(genre),
            "deezer" => {
                let value = engine
                    .ok_or("Deezer is not connected")?
                    .invoke("DZSearchJSON", &[json!(genre)])?;
                crate::catalog::entries(&value, "track")
                    .into_iter()
                    .find_map(|entry| (!entry.artwork.is_empty()).then_some(entry.artwork))
                    .ok_or_else(|| "Deezer returned no genre artwork".into())
            }
            _ => Err("Unknown genre artwork provider".into()),
        }
    }

    /// Keep the complete provider catalog on disk independently of the visible page.
    pub(crate) fn cached_catalog(provider: crate::catalog::Provider) -> Vec<Value> {
        let Ok(root) = crate::storage::data_dir() else {
            return Vec::new();
        };
        let path = root
            .join("flow-genres")
            .join(format!("{}.json", provider.label().to_ascii_lowercase()));
        if std::fs::metadata(&path).map_or(true, |m| m.len() > 8 * 1024 * 1024) {
            return Vec::new();
        }
        std::fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    pub(crate) fn searchable_catalog(tuner: &Value, cached: &[Value]) -> Value {
        let mut result = tuner.clone();
        let mut genres: Vec<Value> = Vec::new();
        let mut positions = std::collections::HashMap::<String, usize>::new();
        for source in tuner["genres"].as_array().into_iter().flatten() {
            let (Some(id), Some(title)) = (source["id"].as_str(), source["title"].as_str()) else {
                continue;
            };
            let title_key = title
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .to_lowercase();
            if title_key.is_empty() || id.is_empty() {
                continue;
            }
            let configurations =
                source["configurations"]
                    .as_array()
                    .cloned()
                    .unwrap_or_else(|| {
                        vec![json!({"id":id,"isEnabled":source["isEnabled"] == true,
                    "isEditedByUser":source["isEditedByUser"] == true})]
                    });
            if let Some(&index) = positions.get(&title_key) {
                let genre = &mut genres[index];
                genre["isEnabled"] =
                    json!(genre["isEnabled"] == true || source["isEnabled"] == true);
                genre["isEditedByUser"] =
                    json!(genre["isEditedByUser"] == true || source["isEditedByUser"] == true);
                genre["isMoodGenre"] = json!(
                    genre["isMoodGenre"].as_bool().unwrap_or(true)
                        || source["isMoodGenre"].as_bool().unwrap_or(true)
                );
                let aliases = genre["configurations"].as_array_mut().unwrap();
                for configuration in configurations {
                    if !aliases.iter().any(|old| old["id"] == configuration["id"]) {
                        aliases.push(configuration);
                    }
                }
            } else {
                let mut genre = source.clone();
                genre["title"] = json!(title.split_whitespace().collect::<Vec<_>>().join(" "));
                genre["configurations"] = json!(configurations);
                positions.insert(title_key, genres.len());
                genres.push(genre);
            }
        }
        let mut seen: std::collections::HashSet<String> = genres
            .iter()
            .filter_map(|genre| genre["id"].as_str().map(str::to_owned))
            .collect();
        let mut titles: std::collections::HashSet<String> = genres
            .iter()
            .filter_map(|genre| {
                genre["title"].as_str().map(|title| {
                    title
                        .split_whitespace()
                        .collect::<Vec<_>>()
                        .join(" ")
                        .to_lowercase()
                })
            })
            .collect();
        for genre in cached {
            if let (Some(id), Some(title)) = (genre["id"].as_str(), genre["title"].as_str()) {
                if seen.insert(id.to_owned())
                    && titles.insert(
                        title
                            .split_whitespace()
                            .collect::<Vec<_>>()
                            .join(" ")
                            .to_lowercase(),
                    )
                {
                    genres
                        .push(json!({"id":id,"title":title,"isEnabled":false,"isMoodGenre":false}));
                }
            }
        }
        result["genres"] = json!(genres);
        result
    }

    pub(crate) fn persist_catalog(provider: crate::catalog::Provider, tuner: &Value) {
        let tuner = searchable_catalog(tuner, &cached_catalog(provider));
        let Some(genres) = tuner["genres"].as_array() else {
            return;
        };
        let Ok(root) = crate::storage::data_dir() else {
            return;
        };
        let root = root.join("flow-genres");
        if std::fs::create_dir_all(&root).is_err() {
            return;
        }
        let path = root.join(format!("{}.json", provider.label().to_ascii_lowercase()));
        let temp = path.with_extension("tmp");
        let catalog: Vec<_> = genres
            .iter()
            .map(|genre| json!({"id": genre["id"], "title": genre["title"]}))
            .collect();
        if let Ok(bytes) = serde_json::to_vec(&catalog) {
            if std::fs::write(&temp, bytes).is_ok() {
                let _ = std::fs::rename(&temp, &path);
            }
        }
    }
    pub(crate) const MOODS: &[(&str, &str)] = &[
        ("default", "All moods"),
        ("chill", "Chill"),
        ("focus", "Focus"),
        ("party", "Party"),
        ("workout", "Workout"),
        ("happy", "Happy"),
        ("sad", "Sad"),
        ("sleep", "Sleep"),
        ("romance", "Romance"),
    ];
    pub(crate) fn mood_genres(mood: &str) -> Vec<String> {
        let genres: &[&str] = match mood {
            "chill" => &["Chillout", "Lo-fi", "Acoustic"],
            "focus" => &["Ambient", "Classical", "Instrumental"],
            "party" => &["Dance", "Pop", "House"],
            "workout" => &["EDM", "Hip Hop", "Rock"],
            "happy" => &["Pop", "Disco", "Funk"],
            "sad" => &["Acoustic", "Indie", "Soul"],
            "sleep" => &["Ambient", "Classical", "Instrumental"],
            "romance" => &["R&B", "Soul", "Jazz"],
            _ => &[],
        };
        genres.iter().map(|genre| (*genre).to_owned()).collect()
    }

    pub(crate) fn save(key: &str, edits: &str) -> Result<Value, String> {
        let mut selection = Selection::parse(key)?;
        let edits: Vec<Value> =
            serde_json::from_str(edits).map_err(|_| "Invalid Flow genre edits")?;
        if edits.len() > 8192 {
            return Err("Too many Flow genre edits".into());
        }
        for edit in edits {
            let genre = edit["clusterId"]
                .as_str()
                .filter(|g| valid_genre(g))
                .ok_or("Invalid Flow genre")?;
            let enabled = edit["isEnabled"]
                .as_bool()
                .ok_or("Invalid Flow genre state")?;
            selection.genres.retain(|g| !g.eq_ignore_ascii_case(genre));
            if enabled {
                selection.genres.push(genre.to_owned());
            }
        }
        let selection = Selection::parse(&selection.key())?;
        Ok(selection.tuner(&selection.key()))
    }

    pub(crate) fn save_provider_tuner(
        key: &str,
        edits: &str,
        tuner: &Value,
    ) -> Result<Value, String> {
        let mut selection = Selection::parse(key)?;
        selection.genres = tuner["genres"]
            .as_array()
            .ok_or("Provider genre catalog missing")?
            .iter()
            .filter(|genre| genre["isEnabled"] == true)
            .filter_map(|genre| genre["id"].as_str().map(str::to_owned))
            .collect();
        save(&selection.key(), edits)
    }

    pub(crate) const BROWSE_GENRES: &[&str] = &[
        "Pop",
        "Rock",
        "Hip Hop",
        "R&B",
        "Electronic",
        "Dance",
        "House",
        "Techno",
        "Jazz",
        "Blues",
        "Soul",
        "Funk",
        "Classical",
        "Metal",
        "Punk",
        "Indie",
        "Alternative",
        "Folk",
        "Country",
        "Reggae",
        "Latin",
        "Reggaeton",
        "Afrobeats",
        "Disco",
        "Ambient",
        "Lo-fi",
        "K-Pop",
        "J-Pop",
        "Soundtracks",
        "Acoustic",
        "Trance",
        "Drum & Bass",
        "Synthwave",
        "Indie Rock",
        "Indie Pop",
        "Hard Rock",
        "Heavy Metal",
        "Dance Pop",
        "Bluegrass",
        "Chillout",
        "Trip-hop",
        "Deep House",
        "Tech House",
        "Dubstep",
        "Hardstyle",
        "Synthpop",
        "Electropop",
        "Dream Pop",
        "Pop Punk",
        "Post-punk",
        "New Wave",
        "Progressive Rock",
        "Psychedelic Rock",
        "Shoegaze",
        "Metalcore",
        "Death Metal",
        "Black Metal",
        "Trap",
        "Drill",
        "Neo Soul",
    ];

    const EXTRA_GENRES: &[&str] = &[
        "Latin Pop",
        "Salsa",
        "Bachata",
        "Amapiano",
        "Dancehall",
        "Dub",
        "Ska",
        "Bossa Nova",
        "Samba",
        "Tango",
        "Flamenco",
        "Fado",
        "Traditional",
        "Instrumental",
        "Gospel",
        "Opera",
        "Cumbia",
        "Merengue",
        "Bolero",
        "Afropop",
        "Highlife",
        "Zouk",
        "Rap",
        "EDM",
        "Singer-Songwriter",
        "World Music",
        "Turkish Pop",
        "Arabic Pop",
        "Turkish Rock",
        "Arabesk",
        "Progressive Metal",
        "Symphonic Metal",
        "Grunge",
        "Emo",
        "Garage Rock",
        "Classic Rock",
        "Progressive House",
        "Electro Swing",
        "Bhangra",
        "Bollywood",
    ];

    /// Full Every Noise index, with existing display names retained for saved choices.
    pub(crate) fn genres() -> &'static [&'static str] {
        static CATALOG: std::sync::OnceLock<Vec<&'static str>> = std::sync::OnceLock::new();
        CATALOG.get_or_init(|| {
            let mut entries = BROWSE_GENRES.to_vec();
            entries.extend_from_slice(EXTRA_GENRES);
            let mut seen: std::collections::HashSet<_> =
                entries.iter().map(|g| g.to_lowercase()).collect();
            for line in include_str!("../../../../assets/spotify-flow-genres.tsv").lines() {
                let name = line.split('\t').next().unwrap_or("");
                if !name.is_empty() && seen.insert(name.to_lowercase()) {
                    entries.push(name);
                }
            }
            entries
        })
    }
    pub(crate) fn genre_radio(genre: &str) -> Option<&'static str> {
        include_str!("../../../../assets/spotify-flow-genres.tsv")
            .lines()
            .find_map(|line| {
                let (name, uri) = line.split_once('\t')?;
                (name.eq_ignore_ascii_case(genre) && !uri.is_empty()).then_some(uri)
            })
    }
    pub(crate) fn valid_genre(genre: &str) -> bool {
        !genre.trim().is_empty()
            && genre == genre.trim()
            && genre.chars().count() <= 100
            && !genre.chars().any(char::is_control)
    }

    #[derive(Clone, Default, Serialize, Deserialize)]
    pub(crate) struct Selection {
        pub mood: String,
        pub genres: Vec<String>,
    }
    impl Selection {
        pub fn parse(key: &str) -> Result<Self, String> {
            let mut selection: Self = if let Some(value) = key.strip_prefix(PREFIX) {
                if value.len() > 65_536 {
                    return Err("Invalid Flow selection".into());
                }
                serde_json::from_str(value).map_err(|_| "Invalid Flow selection")?
            } else {
                Self {
                    mood: key.into(),
                    genres: mood_genres(key),
                }
            };
            // Migrate the old personal-playlist default to the genre/mood picker.
            if selection.mood.starts_with("spotify:playlist:") || selection.mood.is_empty() {
                selection.mood = "default".into();
            }
            if !MOODS.iter().any(|(id, _)| *id == selection.mood)
                || selection.genres.len() > 128
                || selection.genres.iter().any(|g| !valid_genre(g))
            {
                return Err("Invalid Flow mood or genre".into());
            }
            selection.genres.sort();
            selection.genres.dedup();
            Ok(selection)
        }
        pub fn key(&self) -> String {
            format!("{PREFIX}{}", serde_json::to_string(self).unwrap())
        }
        pub fn tuner(&self, key: &str) -> Value {
            json!({"id":key,"customizable":true,"genres":genres().iter().copied().chain(self.genres.iter().map(String::as_str).filter(|g| !genres().iter().any(|known| known.eq_ignore_ascii_case(g)))).map(|g|
            json!({"id":g,"title":g,"isEnabled":self.genres.iter().any(|s| s.eq_ignore_ascii_case(g))})
        ).collect::<Vec<_>>()})
        }
        pub fn queries(&self) -> Vec<String> {
            let mood = if self.mood == "default" {
                ""
            } else {
                &self.mood
            };
            if self.genres.is_empty() {
                vec![if mood.is_empty() {
                    "popular hits".into()
                } else {
                    mood.into()
                }]
            } else {
                self.genres
                    .iter()
                    .map(|g| format!("{mood} {g}").trim().to_owned())
                    .collect()
            }
        }
    }

    #[cfg(test)]
    mod tests {
        #[test]
        #[ignore = "requires a saved Spotify web session and network access"]
        fn live_spotify_mood_catalog_and_genre_artwork() {
            let tuner =
                crate::spotify::catalog_invoke("DZFlowTunerJSON", &[json!("focus")]).unwrap();
            assert_eq!(tuner["moodSource"], "provider-playlists");
            assert!(tuner["genres"]
                .as_array()
                .unwrap()
                .iter()
                .any(|g| g["isEnabled"] == true));
            assert!(crate::spotify::genre_artwork("Jazz")
                .unwrap()
                .starts_with("https://"));
        }

        #[test]
        #[ignore = "requires a saved YouTube Music session and network access"]
        fn live_youtube_mood_catalog_and_genre_artwork() {
            let tuner = crate::youtube::invoke("DZFlowTunerJSON", &[json!("focus")]).unwrap();
            assert_eq!(tuner["moodSource"], "provider-playlists");
            assert!(tuner["genres"]
                .as_array()
                .unwrap()
                .iter()
                .any(|g| g["isEnabled"] == true));
            assert!(crate::youtube::genre_artwork("Jazz")
                .unwrap()
                .starts_with("https://"));
        }

        #[test]
        fn provider_mood_results_replace_presets_and_preserve_explicit_edits() {
            let directory = json!([
                {"title":"New provider genre","kind":"genre"},
                {"title":"A provider mood","kind":"mood"}
            ]);
            let spotify = provider_tuner(
                "focus",
                &directory,
                &json!([{"title":"Ambient Focus","artwork":"https://example.test/ambient.jpg"}]),
            )
            .unwrap();
            let youtube = provider_tuner(
                "focus",
                &directory,
                &json!([{"title":"Jazz for Focus","artwork":"https://example.test/jazz.jpg"}]),
            )
            .unwrap();
            let enabled = |value: &Value, genre: &str| {
                value["genres"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|g| g["id"] == genre && g["isEnabled"] == true)
            };
            assert!(enabled(&spotify, "Ambient"));
            assert!(!enabled(&spotify, "Classical"));
            assert!(enabled(&youtube, "Jazz"));
            assert!(!enabled(&youtube, "Ambient"));
            assert!(spotify["genres"]
                .as_array()
                .unwrap()
                .iter()
                .any(|g| g["id"] == "New provider genre"));
            assert!(!spotify["genres"]
                .as_array()
                .unwrap()
                .iter()
                .any(|g| g["id"] == "A provider mood"));
            let key = Selection {
                mood: "focus".into(),
                genres: vec!["My emerging subgenre".into()],
            }
            .key();
            let saved =
                provider_tuner(&key, &directory, &json!([{"title":"Ambient Focus"}])).unwrap();
            assert!(enabled(&saved, "My emerging subgenre"));
            assert!(!enabled(&saved, "Ambient"));
            let empty = Selection {
                mood: "focus".into(),
                genres: vec![],
            }
            .key();
            assert!(
                provider_tuner(&empty, &directory, &json!([])).unwrap()["genres"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|g| g["isEnabled"] == false)
            );
            assert!(provider_tuner("focus", &directory, &json!([])).is_err());
        }

        #[test]
        fn provider_genre_matching_uses_word_boundaries_and_image_keys_require_https() {
            let tuner =
                provider_tuner("party", &json!([]), &json!([{"title":"Popular hip-hop"}])).unwrap();
            assert!(!tuner["genres"]
                .as_array()
                .unwrap()
                .iter()
                .any(|g| g["id"] == "Pop" && g["isEnabled"] == true));
            assert_eq!(
                resolve_artwork("flow-genre:image:https://example.test/cover.jpg", None).unwrap(),
                "https://example.test/cover.jpg"
            );
            assert!(resolve_artwork("flow-genre:image:file:///private", None).is_err());
        }

        #[test]
        fn flow_catalog_keeps_other_mood_genres_searchable_without_showing_them() {
            let tuner = json!({"id":"focus","genres":[{"id":"focus-jazz","title":"Jazz","isEnabled":true}]});
            let cached = vec![
                json!({"id":"default-jazz","title":"Jazz"}),
                json!({"id":"default-rock","title":"Rock"}),
            ];
            let merged = searchable_catalog(&tuner, &cached);
            let genres = merged["genres"].as_array().unwrap();
            assert_eq!(genres.len(), 2);
            assert_eq!(genres[0]["id"], "focus-jazz");
            assert_eq!(genres[0]["isEnabled"], true);
            assert_eq!(genres[1]["title"], "Rock");
            assert_eq!(genres[1]["isEnabled"], false);
            assert_eq!(genres[1]["isMoodGenre"], false);
        }

        #[test]
        fn repeated_provider_genres_share_one_card_and_keep_all_configuration_ids() {
            let tuner = json!({"id":"party","genres":[
                {"id":"dub-a","title":"Dubstep","isEnabled":false},
                {"id":"dub-b","title":" dubstep ","isEnabled":true,"isEditedByUser":true},
                {"id":"dub-b","title":"DUBSTEP","isEnabled":true},
                {"id":"jungle","title":"Jungle","isEnabled":false}
            ]});
            let merged = searchable_catalog(&tuner, &[]);
            let genres = merged["genres"].as_array().unwrap();
            assert_eq!(genres.len(), 2);
            assert_eq!(genres[0]["id"], "dub-a");
            assert_eq!(genres[0]["isEnabled"], true);
            assert_eq!(genres[0]["configurations"].as_array().unwrap().len(), 2);
            assert_eq!(searchable_catalog(&merged, &[]), merged);
        }

        use super::*;
        #[test]
        fn mood_presets_and_saved_edits_roundtrip() {
            for (mood, _) in MOODS.iter().filter(|(id, _)| *id != "default") {
                let selection = Selection::parse(mood).unwrap();
                assert!(!selection.genres.is_empty());
                assert!(selection
                    .genres
                    .iter()
                    .all(|g| genres().contains(&g.as_str())));
            }
            let saved = save(
            "chill",
            r#"[{"clusterId":"Lo-fi","isEnabled":false},{"clusterId":"Jazz","isEnabled":true}]"#,
        )
        .unwrap();
            let restored = Selection::parse(saved["id"].as_str().unwrap()).unwrap();
            assert!(restored.genres.contains(&"Jazz".to_owned()));
            assert!(!restored.genres.contains(&"Lo-fi".to_owned()));
            let empty = Selection {
                mood: "chill".into(),
                genres: vec![],
            };
            assert!(Selection::parse(&empty.key()).unwrap().genres.is_empty());
            assert!(save("chill", r#"[{"clusterId":"Jazz"}]"#).is_err());
        }
        #[test]
        fn flow_includes_style_mix_genres_and_preserves_existing_choices() {
            for (genre, _) in crate::style_mix::GENRES {
                let genre = match *genre {
                    "Hip-hop" => "Hip Hop",
                    "Soundtrack" => "Soundtracks",
                    genre => genre,
                };
                assert!(genres().contains(&genre), "Missing Flow genre: {genre}");
            }
            let selection = Selection {
                mood: "default".into(),
                genres: genres().iter().take(128).map(|g| (*g).to_owned()).collect(),
            };
            let restored = Selection::parse(&selection.key()).unwrap();
            assert_eq!(restored.genres.len(), 128);
            assert_eq!(
                genres()
                    .iter()
                    .collect::<std::collections::HashSet<_>>()
                    .len(),
                genres().len()
            );
            for genre in ["Hip Hop", "Soundtracks", "K-Pop", "J-Pop", "Latin", "Dance"] {
                assert!(genres().contains(&genre));
            }
        }
        #[test]
        fn selection_roundtrip_and_validation() {
            let s = Selection {
                mood: "chill".into(),
                genres: vec!["Jazz".into(), "Soul".into()],
            };
            let restored = Selection::parse(&s.key()).unwrap();
            assert_eq!(restored.queries(), ["chill Jazz", "chill Soul"]);
            assert_eq!(
                restored.tuner(&s.key())["genres"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|g| g["isEnabled"] == true)
                    .count(),
                2
            );
            assert!(Selection::parse("spotify-flow:{\"mood\":\"bogus\",\"genres\":[]}").is_err());
            assert_eq!(
                Selection::parse("spotify:playlist:old").unwrap().mood,
                "default"
            );
        }
        #[test]
        fn full_catalog_and_custom_genres_are_available() {
            assert_eq!(BROWSE_GENRES.len(), 60);
            assert!(genres().len() >= 6291);
            for genre in [
                "turkish pop",
                "arabesk",
                "turkish rock",
                "melodic death metal",
            ] {
                assert!(
                    genres().iter().any(|name| name.eq_ignore_ascii_case(genre)),
                    "Missing {genre}"
                );
            }
            assert!(genre_radio("turkish pop")
                .unwrap()
                .starts_with("spotify:playlist:"));
            let selection = Selection {
                mood: "default".into(),
                genres: vec!["My emerging subgenre".into()],
            };
            let restored = Selection::parse(&selection.key()).unwrap();
            assert_eq!(restored.queries(), ["My emerging subgenre"]);
            assert!(restored.tuner(&restored.key())["genres"]
                .as_array()
                .unwrap()
                .iter()
                .any(|g| g["title"] == "My emerging subgenre" && g["isEnabled"] == true));
            assert!(!valid_genre("bad\ngenre"));
            let saved = Selection {
                mood: "default".into(),
                genres: vec!["turkish pop".into()],
            };
            assert!(saved.tuner(&saved.key())["genres"]
                .as_array()
                .unwrap()
                .iter()
                .any(|g| g["title"] == "Turkish Pop" && g["isEnabled"] == true));
        }
    }
}
pub(crate) mod jam {
    //! Native Jam control over Spotify's social-connect protocol; no WebView2.
    use super::jam_auth as auth;
    use crate::lyrics::http;
    use serde::{Deserialize, Serialize};
    use serde_json::Value;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
        Arc, Mutex,
    };
    static OPERATIONS: Mutex<()> = Mutex::new(());
    static HOST: Mutex<Option<(String, u64)>> = Mutex::new(None);

    pub(crate) struct Request {
        pub receiver: Receiver<Result<Option<Session>, String>>,
        cancel: Arc<AtomicBool>,
    }
    impl Drop for Request {
        fn drop(&mut self) {
            self.cancel.store(true, Ordering::Relaxed);
        }
    }
    pub(crate) fn spawn(operation: Operation, identity: Option<String>) -> Result<Request, String> {
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let (tx, receiver) = mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("spotify-jam".into())
            .spawn(move || {
                let _ = tx.send(run_inner(operation, flag, identity));
            })
            .map_err(|_| "Could not start Spotify Jam worker")?;
        Ok(Request { receiver, cancel })
    }

    #[derive(Clone, Default, Serialize, Deserialize)]
    pub(crate) struct Member {
        pub id: String,
        pub name: String,
        pub owner: bool,
    }
    #[derive(Clone, Default, Serialize, Deserialize)]
    pub(crate) struct Session {
        pub id: String,
        pub active: bool,
        pub owner: bool,
        pub invite: Option<String>,
        #[serde(skip)]
        pub invite_uri: Option<String>,
        pub members: Vec<Member>,
    }
    #[derive(Clone)]
    pub(crate) enum Operation {
        Refresh,
        Start,
        Invite(String),
        End(String),
        Leave(String),
        Remove { session: String, member: String },
    }
    fn text(value: &Value, key: &str) -> String {
        value[key]
            .as_str()
            .unwrap_or_default()
            .chars()
            .filter(|c| !c.is_control())
            .take(256)
            .collect()
    }
    fn identifier(value: &str) -> bool {
        !value.is_empty()
            && value.len() <= 256
            && value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
    }
    fn parse(value: Value) -> Result<Session, String> {
        let id = value["session_id"].as_str().unwrap_or_default().to_string();
        if !identifier(&id) {
            return Err("Spotify returned an invalid Jam session.".into());
        }
        let owner_id = text(&value, "session_owner_id");
        let invite = value["join_session_url"]
            .as_str()
            .filter(|s| {
                s.len() <= 4096
                    && url::Url::parse(s).is_ok_and(|u| {
                        u.scheme() == "https"
                            && matches!(
                                u.host_str(),
                                Some(
                                    "spotify.link"
                                        | "open.spotify.com"
                                        | "spotify.com"
                                        | "www.spotify.com"
                                )
                            )
                            && u.username().is_empty()
                            && u.password().is_none()
                            && u.port().is_none()
                            && u.fragment().is_none()
                    })
            })
            .map(str::to_owned);
        let invite_uri = value["join_session_uri"]
            .as_str()
            .filter(|s| {
                s.strip_prefix("spotify:socialsession:")
                    .is_some_and(identifier)
            })
            .map(str::to_owned);
        let members = value["session_members"]
            .as_array()
            .into_iter()
            .flatten()
            .take(100)
            .map(|m| {
                let id = m["id"]
                    .as_str()
                    .filter(|s| identifier(s))
                    .unwrap_or_default()
                    .to_owned();
                let name = text(m, "display_name");
                Member {
                    owner: id == owner_id,
                    id,
                    name: if name.is_empty() {
                        "Guest".into()
                    } else {
                        name
                    },
                }
            })
            .collect();
        Ok(Session {
            id,
            active: value["active"] == true,
            owner: value["is_session_owner"] == true,
            invite,
            invite_uri,
            members,
        })
    }
    fn valid_host(host: &str) -> bool {
        host.ends_with("-spclient.spotify.com")
            && host.len() < 100
            && host
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'-'))
    }
    fn host() -> Result<String, String> {
        let mut cache = HOST.lock().map_err(|_| "Spotify Jam resolver is busy")?;
        if let Some((host, expiry)) = cache.as_ref() {
            if *expiry > auth::now() {
                return Ok(host.clone());
            }
        }
        let bytes = auth::checked(http::response(
            "GET",
            "https://apresolve.spotify.com/?type=spclient",
            &[],
            &[],
        )?)?;
        let value: Value = serde_json::from_slice(&bytes)
            .map_err(|_| "Invalid Spotify service resolver response")?;
        let host = value["spclient"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .filter_map(|s| s.strip_suffix(":443"))
            .find(|s| valid_host(s))
            .ok_or("Spotify Jam service is unavailable")?
            .to_string();
        *cache = Some((host.clone(), auth::now().saturating_add(3600)));
        Ok(host)
    }
    struct Client {
        host: String,
        token: String,
        identity: String,
        cancel: Arc<AtomicBool>,
    }
    impl Client {
        fn new(cancel: Arc<AtomicBool>) -> Result<Self, String> {
            auth::check_cooldown()?;
            let identity = auth::cookie()?;
            Ok(Self {
                host: host()?,
                token: auth::token()?,
                identity,
                cancel,
            })
        }
    }
    trait Transport {
        fn send_json(
            &self,
            method: &str,
            path: &str,
            query: Option<(&str, &str)>,
            body: Option<Value>,
        ) -> Result<Option<Value>, String>;
        fn send(
            &self,
            method: &str,
            path: &str,
            query: Option<(&str, &str)>,
        ) -> Result<Option<Value>, String> {
            self.send_json(method, path, query, None)
        }
        fn current(&self) -> Result<Option<Session>, String> {
            self.send("GET", "/social-connect/v2/sessions/current", None)?
                .map(parse)
                .transpose()
        }
        fn owned(&self, expected: &str) -> Result<Session, String> {
            let session = self.current()?.ok_or("This Jam has ended. Refresh Jam.")?;
            if session.id != expected {
                return Err("Your Jam changed. Refresh before managing it.".into());
            }
            if !session.owner {
                return Err("Only the Jam host can manage participants or end the Jam.".into());
            }
            Ok(session)
        }
    }
    impl Transport for Client {
        fn send_json(
            &self,
            method: &str,
            path: &str,
            query: Option<(&str, &str)>,
            body: Option<Value>,
        ) -> Result<Option<Value>, String> {
            if self.cancel.load(Ordering::Relaxed) {
                return Err("Spotify Jam request cancelled; refresh its state.".into());
            }
            auth::check_cooldown()?;
            if auth::cookie()? != self.identity {
                return Err("Spotify sign-in changed; refresh Jam.".into());
            }
            let mut url = url::Url::parse(&format!("https://{}/", self.host)).unwrap();
            url.set_path(path);
            if let Some((key, value)) = query {
                url.query_pairs_mut().append_pair(key, value);
            }
            let bearer = format!("Bearer {}", self.token);
            let body = body.map(|v| v.to_string().into_bytes()).unwrap_or_default();
            let response = http::response(
                method,
                url.as_str(),
                &[
                    ("Authorization", &bearer),
                    ("Accept", "application/json"),
                    ("Content-Type", "application/json"),
                ],
                &body,
            )?;
            if method == "GET"
                && path == "/social-connect/v2/sessions/current"
                && matches!(response.status, 404 | 410)
            {
                return Ok(None);
            }
            if response.status == 403 {
                return Err(
                    "Spotify denied Jam access. An eligible account and device are required."
                        .into(),
                );
            }
            if response.status == 401 {
                auth::invalidate();
                return Err(
                    "Spotify rejected Jam authorization. Refresh Jam to renew your sign-in.".into(),
                );
            }
            let bytes = auth::checked(response)?;
            if bytes.is_empty() {
                return Ok(None);
            }
            serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(|_| "Spotify returned an invalid Jam response.".into())
        }
    }
    pub(crate) fn run(operation: Operation) -> Result<Option<Session>, String> {
        run_inner(operation, Arc::new(AtomicBool::new(false)), None)
    }
    fn run_inner(
        operation: Operation,
        cancel: Arc<AtomicBool>,
        identity: Option<String>,
    ) -> Result<Option<Session>, String> {
        let _guard = OPERATIONS.lock().map_err(|_| "Spotify Jam is busy")?;
        if cancel.load(Ordering::Relaxed) {
            return Err("Spotify Jam request cancelled.".into());
        }
        if identity
            .as_ref()
            .is_some_and(|expected| account_key().as_ref() != Some(expected))
        {
            return Err("Spotify sign-in changed. Refresh Jam.".into());
        }
        let client = Client::new(cancel).map_err(|e| format!("Spotify Jam authorization: {e}"))?;
        run_on(&client, operation)
    }
    fn run_on(client: &impl Transport, operation: Operation) -> Result<Option<Session>, String> {
        let mutation = !matches!(operation, Operation::Refresh);
        let result = (|| match operation {
            Operation::Refresh => client.current(),
            Operation::Start => {
                if let Some(session) = client.current()? {
                    if session.active {
                        if session.owner {
                            return Ok(Some(session));
                        }
                        return Err("You are already participating in another Jam.".into());
                    }
                }
                // Native create-session request. This GET is a mutation: never retry it.
                let value = client
                    .send(
                        "GET",
                        "/social-connect/v2/sessions/current_or_new",
                        Some(("activate", "true")),
                    )?
                    .ok_or("Spotify did not return the created Jam")?;
                let created = parse(value)?;
                if !created.active || !created.owner {
                    return Err("Spotify did not confirm an active Jam hosted by you.".into());
                }
                Ok(Some(created))
            }
            Operation::End(expected) => {
                let session = client.owned(&expected)?;
                client.send(
                    "DELETE",
                    &format!("/social-connect/v3/sessions/{}", session.id),
                    None,
                )?;
                let current = client.current()?;
                if current
                    .as_ref()
                    .is_some_and(|s| s.id == expected && s.active)
                {
                    return Err("Spotify has not confirmed that the Jam ended. Refresh Jam.".into());
                }
                Ok(current)
            }
            Operation::Leave(expected) => {
                let session = client
                    .current()?
                    .ok_or("This Jam has ended. Refresh Jam.")?;
                if session.id != expected {
                    return Err("Your Jam changed. Refresh before leaving it.".into());
                }
                if session.owner {
                    return Err("Use End Jam to stop hosting this Jam.".into());
                }
                client.send(
                    "POST",
                    &format!("/social-connect/v3/sessions/{}/leave", session.id),
                    None,
                )?;
                let current = client.current()?;
                if current
                    .as_ref()
                    .is_some_and(|s| s.id == expected && s.active)
                {
                    return Err(
                        "Spotify has not confirmed that you left the Jam. Refresh Jam.".into(),
                    );
                }
                Ok(current)
            }
            Operation::Invite(expected) => {
                let mut session = client
                    .current()?
                    .ok_or("This Jam has ended. Refresh Jam.")?;
                if !session.active || session.id != expected {
                    return Err("Your Jam changed. Refresh before inviting guests.".into());
                }
                let uri = session
                    .invite_uri
                    .as_deref()
                    .ok_or("Spotify did not provide a Jam invite URI")?;
                let token = uri.strip_prefix("spotify:socialsession:").unwrap();
                let body = serde_json::json!({"spotify_uri":uri,
                "custom_data":[{"key":"ssp","value":"1"},{"key":"app_destination","value":"socialsession"}],
                "link_preview":{"title":"Join my Spotify Jam","image_url":format!("https://shareables.scdn.co/publish/socialsession/{token}")},
                "utm_parameters":{"utm_campaign":null,"utm_term":null,"utm_medium":"share-link","utm_content":null,"utm_source":"share-options-sheet"}});
                let response = client
                    .send_json("POST", "/url-dispenser/v1/generate-url", None, Some(body))?
                    .ok_or("Spotify did not return an invite link")?;
                let candidate = serde_json::json!({"session_id":session.id,"join_session_url":response["shareable_url"]});
                session.invite = parse(candidate)?.invite;
                if session.invite.is_none() {
                    return Err("Spotify returned an invalid invite link.".into());
                }
                Ok(Some(session))
            }
            Operation::Remove { session, member } => {
                let current = client.owned(&session)?;
                if !identifier(&member)
                    || !current.members.iter().any(|m| m.id == member && !m.owner)
                {
                    return Err("This guest is no longer removable. Refresh Jam.".into());
                }
                client.send(
                    "POST",
                    &format!(
                        "/social-connect/v3/sessions/{}/member/{member}/kick",
                        current.id
                    ),
                    None,
                )?;
                let updated = client.current()?;
                if updated
                    .as_ref()
                    .is_some_and(|s| s.id == session && s.members.iter().any(|m| m.id == member))
                {
                    return Err("Spotify has not confirmed the guest removal. Refresh Jam.".into());
                }
                Ok(updated)
            }
        })();
        result.map_err(|e: String| {
            if mutation {
                format!("{e} The request was not retried; refresh Jam before trying again.")
            } else {
                e
            }
        })
    }
    pub(crate) fn account_key() -> Option<String> {
        use sha2::{Digest, Sha256};
        auth::cookie()
            .ok()
            .map(|s| format!("{:x}", Sha256::digest(s.as_bytes())))
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn dropping_a_request_cancels_it_before_authorization_or_network() {
            let cancel = Arc::new(AtomicBool::new(false));
            let (_tx, receiver) = mpsc::sync_channel(1);
            let request = Request {
                receiver,
                cancel: cancel.clone(),
            };
            drop(request);
            assert!(cancel.load(Ordering::Relaxed));
            assert!(run_inner(Operation::Start, cancel, None)
                .err()
                .unwrap()
                .contains("cancelled"));
        }
        use std::{cell::RefCell, collections::VecDeque};
        struct Mock(RefCell<VecDeque<(&'static str, &'static str, Result<Option<Value>, String>)>>);
        impl Transport for Mock {
            fn send_json(
                &self,
                method: &str,
                path: &str,
                _query: Option<(&str, &str)>,
                _body: Option<Value>,
            ) -> Result<Option<Value>, String> {
                let (expected_method, expected_path, result) = self
                    .0
                    .borrow_mut()
                    .pop_front()
                    .expect("unexpected or retried request");
                assert_eq!(method, expected_method);
                assert_eq!(path, expected_path);
                result
            }
        }
        fn mock(replies: Vec<(&'static str, &'static str, Result<Option<Value>, String>)>) -> Mock {
            Mock(RefCell::new(replies.into()))
        }
        fn fixture(id: &str, owner: bool, active: bool) -> Value {
            serde_json::json!({"session_id":id,"is_session_owner":owner,"active":active})
        }
        #[test]
        fn stale_session_and_guest_permissions_never_send_mutations() {
            for value in [
                fixture("other", true, true),
                fixture("expected", false, true),
            ] {
                let client = mock(vec![(
                    "GET",
                    "/social-connect/v2/sessions/current",
                    Ok(Some(value)),
                )]);
                assert!(run_on(&client, Operation::End("expected".into())).is_err());
                assert!(client.0.borrow().is_empty());
            }
        }
        #[test]
        fn indeterminate_creation_is_never_retried() {
            let client = mock(vec![
                ("GET", "/social-connect/v2/sessions/current", Ok(None)),
                (
                    "GET",
                    "/social-connect/v2/sessions/current_or_new",
                    Err("timeout".into()),
                ),
            ]);
            let error = run_on(&client, Operation::Start).err().unwrap();
            assert!(error.contains("not retried"));
            assert!(client.0.borrow().is_empty());
        }
        #[test]
        fn leaving_as_a_guest_uses_the_verified_leave_route_and_confirms_state() {
            let client = mock(vec![
                (
                    "GET",
                    "/social-connect/v2/sessions/current",
                    Ok(Some(fixture("id", false, true))),
                ),
                ("POST", "/social-connect/v3/sessions/id/leave", Ok(None)),
                ("GET", "/social-connect/v2/sessions/current", Ok(None)),
            ]);
            assert!(run_on(&client, Operation::Leave("id".into()))
                .unwrap()
                .is_none());
            assert!(client.0.borrow().is_empty());
        }
        #[test]
        fn host_cannot_accidentally_leave_instead_of_ending_a_jam() {
            let client = mock(vec![(
                "GET",
                "/social-connect/v2/sessions/current",
                Ok(Some(fixture("id", true, true))),
            )]);
            assert!(run_on(&client, Operation::Leave("id".into())).is_err());
            assert!(client.0.borrow().is_empty());
        }
        #[test]
        fn inactive_guest_record_does_not_prevent_starting_a_new_jam() {
            let client = mock(vec![
                (
                    "GET",
                    "/social-connect/v2/sessions/current",
                    Ok(Some(fixture("old", false, false))),
                ),
                (
                    "GET",
                    "/social-connect/v2/sessions/current_or_new",
                    Ok(Some(fixture("new", true, true))),
                ),
            ]);
            let started = run_on(&client, Operation::Start).unwrap().unwrap();
            assert_eq!(started.id, "new");
            assert!(started.owner && started.active);
            assert!(client.0.borrow().is_empty());
        }
        #[test]
        fn active_guest_cannot_start_a_second_jam() {
            let client = mock(vec![(
                "GET",
                "/social-connect/v2/sessions/current",
                Ok(Some(fixture("joined", false, true))),
            )]);
            assert!(run_on(&client, Operation::Start).is_err());
            assert!(client.0.borrow().is_empty());
        }
        #[test]
        fn deleting_a_jam_accepts_the_confirmed_inactive_session_record() {
            let client = mock(vec![
                (
                    "GET",
                    "/social-connect/v2/sessions/current",
                    Ok(Some(fixture("id", true, true))),
                ),
                ("DELETE", "/social-connect/v3/sessions/id", Ok(None)),
                (
                    "GET",
                    "/social-connect/v2/sessions/current",
                    Ok(Some(fixture("id", true, false))),
                ),
            ]);
            let ended = run_on(&client, Operation::End("id".into()))
                .unwrap()
                .unwrap();
            assert!(!ended.active);
            assert!(client.0.borrow().is_empty());
        }
        #[test]
        fn removing_host_is_rejected_before_the_kick_request() {
            let value = serde_json::json!({"session_id":"id","is_session_owner":true,"session_owner_id":"host","session_members":[{"id":"host"}]});
            let client = mock(vec![(
                "GET",
                "/social-connect/v2/sessions/current",
                Ok(Some(value)),
            )]);
            assert!(run_on(
                &client,
                Operation::Remove {
                    session: "id".into(),
                    member: "host".into()
                }
            )
            .is_err());
            assert!(client.0.borrow().is_empty());
        }
        #[test]
        fn rejects_foreign_invites_and_malformed_session_ids() {
            let value = serde_json::json!({"session_id":"abc-123", "join_session_url":"https://evil.test/invite", "active":true,"is_session_owner":true});
            let session = parse(value).unwrap();
            assert!(session.invite.is_none());
            assert!(session.active && session.owner);
            assert!(parse(serde_json::json!({"session_id":"../other?activate=true"})).is_err());
        }
        #[test]
        fn resolver_does_not_allow_credentials_to_escape_spotify() {
            assert!(valid_host("gew4-spclient.spotify.com"));
            for host in [
                "evil-spclient.spotify.com.attacker.test",
                "spotify.com",
                "a@gew4-spclient.spotify.com",
                "gew4-spclient.spotify.com:443",
            ] {
                assert!(!valid_host(host));
            }
        }
        #[test]
        fn preserves_guest_identity_and_host_permissions() {
            let session = parse(serde_json::json!({"session_id":"id", "session_owner_id":"host", "session_members":[{"id":"host","display_name":"Host"},{"id":"guest","display_name":"Guest"}], "join_session_url":"https://spotify.link/invite"})).unwrap();
            assert!(session.members[0].owner);
            assert!(!session.members[1].owner);
            assert_eq!(session.members[1].id, "guest");
            assert!(session.invite.is_some());
        }
    }
}
mod jam_auth {
    //! Direct cookie-to-token renewal. No browser, dealer registration or playback.
    use crate::lyrics::http;
    use hmac::{Hmac, Mac};
    use serde_json::Value;
    use sha1::Sha1;
    use std::{
        sync::Mutex,
        time::{SystemTime, UNIX_EPOCH},
    };

    static CACHE: Mutex<Option<(String, String, u64)>> = Mutex::new(None);
    static RETRY_AT: Mutex<u64> = Mutex::new(0);
    pub(super) fn invalidate() {
        if let Ok(mut cache) = CACHE.lock() {
            *cache = None;
        }
    }
    pub(super) fn now() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }
    pub(super) fn cookie() -> Result<String, String> {
        let saved = crate::session::provider_saved("spotify-web")
            .map_err(|_| "Spotify Jam needs a saved Spotify sign-in.".to_string())?;
        let parsed = serde_json::from_str::<Value>(&saved).ok();
        let cookie = parsed
            .as_ref()
            .and_then(|v| v.as_str().or_else(|| v["sp_dc"].as_str()))
            .unwrap_or(&saved);
        if cookie.len() < 16
            || cookie.len() > 4096
            || !cookie
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
        {
            return Err("Saved Spotify sign-in is invalid.".into());
        }
        Ok(cookie.into())
    }
    pub(super) fn checked(response: http::Response) -> Result<Vec<u8>, String> {
        if response.status == 429 {
            let wait = response.retry_after.unwrap_or(60).clamp(1, 3600);
            if let Ok(mut retry) = RETRY_AT.lock() {
                *retry = now().saturating_add(wait);
            }
            return Err(format!("Spotify is rate limited; retry in {wait} seconds."));
        }
        if !(200..300).contains(&response.status) {
            return Err(format!(
                "Spotify Jam request failed (HTTP {}).",
                response.status
            ));
        }
        Ok(response.body)
    }
    pub(super) fn check_cooldown() -> Result<(), String> {
        let retry = *RETRY_AT
            .lock()
            .map_err(|_| "Spotify Jam authorization is busy")?;
        if retry > now() {
            return Err(format!(
                "Spotify is rate limited; retry in {} seconds.",
                retry - now()
            ));
        }
        Ok(())
    }
    fn get(url: &str) -> Result<Vec<u8>, String> {
        check_cooldown()?;
        checked(http::response("GET", url, &[], &[])?)
    }
    fn json(bytes: &[u8]) -> Result<Value, String> {
        serde_json::from_slice(bytes)
            .map_err(|_| "Spotify returned an invalid authorization response.".into())
    }
    /// Discover the current bootstrap rather than embedding an expiring TOTP version.
    fn bootstrap() -> Result<(Vec<u8>, u32), String> {
        let html = String::from_utf8(get("https://open.spotify.com/")?)
            .map_err(|_| "Invalid Spotify page")?;
        let url = script_url(&html).ok_or("Spotify authorization bootstrap changed.")?;
        let script = String::from_utf8(checked(http::spotify_script(url)?)?)
            .map_err(|_| "Invalid Spotify script")?;
        secret(&script)
    }
    fn script_url(html: &str) -> Option<&str> {
        [
            "https://open.spotifycdn.com/cdn/build/web-player/web-player.",
            "https://open.spotifycdn.com/cdn/build/mobile-web-player/mobile-web-player.",
        ]
        .iter()
        .find_map(|prefix| {
            html.match_indices(prefix).find_map(|(start, _)| {
                let candidate = html[start..].split(['"', '\'', '<', '>', ' ']).next()?;
                (candidate.len() <= 512 && candidate.ends_with(".js")).then_some(candidate)
            })
        })
    }
    fn secret(script: &str) -> Result<(Vec<u8>, u32), String> {
        for (_, tail) in script
            .match_indices("secret:")
            .map(|(i, _)| (i, &script[i + 7..]))
        {
            let Some(quote @ (b'\'' | b'"')) = tail.bytes().next() else {
                continue;
            };
            let mut chars = tail[1..].char_indices();
            let mut value = String::new();
            let mut end = None;
            while let Some((i, ch)) = chars.next() {
                if ch as u32 == quote as u32 {
                    end = Some(i + 2);
                    break;
                }
                if ch == '\\' {
                    match chars.next().map(|(_, c)| c) {
                        Some(c @ ('\\' | '\'' | '"' | '/')) => value.push(c),
                        _ => {
                            value.clear();
                            break;
                        }
                    }
                } else {
                    value.push(ch);
                }
                if value.len() > 256 {
                    break;
                }
            }
            let Some(end) = end else { continue };
            let Some(rest) = tail[end..].strip_prefix(",version:") else {
                continue;
            };
            let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            let Ok(version) = digits.parse::<u32>() else {
                continue;
            };
            if value.is_empty() || !value.is_ascii() || version == 0 {
                continue;
            }
            let key: String = value
                .bytes()
                .enumerate()
                .map(|(i, b)| (b ^ ((i % 33 + 9) as u8)).to_string())
                .collect();
            return Ok((key.into_bytes(), version));
        }
        Err("Spotify authorization bootstrap changed; update DeezNats.".into())
    }
    fn totp(key: &[u8], seconds: u64) -> String {
        let mut mac =
            Hmac::<Sha1>::new_from_slice(key).expect("HMAC accepts arbitrary key lengths");
        mac.update(&(seconds / 30).to_be_bytes());
        let digest = mac.finalize().into_bytes();
        let offset = (digest[19] & 15) as usize;
        let code = u32::from_be_bytes(digest[offset..offset + 4].try_into().unwrap()) & 0x7fff_ffff;
        format!("{:06}", code % 1_000_000)
    }
    pub(super) fn token() -> Result<String, String> {
        check_cooldown()?;
        let cookie = cookie()?;
        // Serialize renewal; credentials never leave this module or enter diagnostics.
        let mut cache = CACHE
            .lock()
            .map_err(|_| "Spotify Jam authorization is busy")?;
        if let Some((identity, token, expiry)) = cache.as_ref() {
            if identity == &cookie && *expiry > now().saturating_add(60) {
                return Ok(token.clone());
            }
        }
        let (key, version) = bootstrap()?;
        let server = json(&get("https://open.spotify.com/api/server-time")?)?["serverTime"]
            .as_u64()
            .ok_or("Invalid Spotify server time")?;
        let mut url = url::Url::parse("https://open.spotify.com/api/token").unwrap();
        url.query_pairs_mut()
            .append_pair("reason", "transport")
            .append_pair("productType", "web-player")
            .append_pair("totp", &totp(&key, now()))
            .append_pair("totpServer", &totp(&key, server))
            .append_pair("totpVer", &version.to_string());
        let header = format!("sp_dc={cookie}");
        let value = json(&checked(http::response(
            "GET",
            url.as_str(),
            &[("Cookie", &header)],
            &[],
        )?)?)?;
        if value["isAnonymous"] != false {
            return Err("Spotify sign-in expired. Sign in again before using Jam.".into());
        }
        let token = value["accessToken"]
            .as_str()
            .filter(|s| !s.is_empty() && !s.contains(['\r', '\n']))
            .ok_or("Spotify did not grant a Jam token")?;
        let expiry = value["accessTokenExpirationTimestampMs"]
            .as_u64()
            .ok_or("Spotify token has no expiry")?
            / 1000;
        if expiry <= now().saturating_add(60) {
            return Err("Spotify returned an expired Jam token.".into());
        }
        *cache = Some((cookie, token.into(), expiry));
        Ok(token.into())
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn rfc6238_sha1_vectors_use_six_digits() {
            assert_eq!(totp(b"12345678901234567890", 59), "287082");
            assert_eq!(totp(b"12345678901234567890", 1111111109), "081804");
        }
        #[test]
        fn discovers_only_a_versioned_secret_and_decodes_js_quotes() {
            let (key, version) = secret("secret:unrelated;[{secret:'a\\\'b',version:61}]").unwrap();
            assert_eq!(version, 61);
            assert_eq!(key, b"10445105");
            assert!(secret("secret:'abc',version:0").is_err());
            assert!(secret("secret:'abc',other:61").is_err());
        }
        #[test]
        fn bootstrap_ignores_stylesheets_and_finds_the_actual_entry_script() {
            let url =
                "https://open.spotifycdn.com/cdn/build/mobile-web-player/mobile-web-player.abc.js";
            let html = format!("<link href='https://open.spotifycdn.com/cdn/build/mobile-web-player/mobile-web-player.abc.css'><script src='{url}'></script>");
            assert_eq!(script_url(&html), Some(url));
            assert!(script_url("https://evil.test/player.js").is_none());
        }
    }
}
pub(crate) mod playback {
    //! Audio worker adapter: Spotify controls never enter the Deezer ABI or PCM pipeline.
    use super::{service::Handle, soloist::Control};
    use crate::{
        audio::{Command, Snapshot},
        catalog::Entry,
    };
    use serde_json::{json, Value};
    mod playback_queue {
        //! Confirmed Spotify queue replacement. All I/O runs on the host audio worker.
        use super::{Command, Entry, Snapshot};
        use crate::spotify::{state, web_devices};
        use serde_json::{json, Value};
        use std::{
            sync::atomic::{AtomicU64, Ordering},
            sync::Arc,
            time::{Duration, Instant},
        };

        const STATION: &str = "spotify_station";
        #[derive(Default)]
        pub(crate) struct Queue {
            items: Arc<Vec<Entry>>,
            index: usize,
            window_end: usize,
            confirmed: Option<Instant>,
            awaiting_play: bool,
            refill_at: Option<Instant>,
            device: Option<String>,
            generation: Option<(Arc<AtomicU64>, u64)>,
        }
        fn playable(entry: &Entry) -> bool {
            let mut parts = entry.id.split(':');
            parts.next() == Some("spotify")
                && matches!(parts.next(), Some("track" | "episode"))
                && parts.next().is_some_and(|id| {
                    id.len() == 22 && id.bytes().all(|c| c.is_ascii_alphanumeric())
                })
                && parts.next().is_none()
        }
        fn selected(entry: &Entry, items: &[Entry]) -> Result<usize, String> {
            if let Some(index) = entry.raw["spotify_selection_index"].as_u64() {
                let index =
                    usize::try_from(index).map_err(|_| "Invalid Spotify selection index")?;
                return items
                    .get(index)
                    .filter(|item| item.id == entry.id)
                    .map(|_| index)
                    .ok_or_else(|| {
                        "Spotify selection index does not match the supplied collection".into()
                    });
            }
            // Exact raw metadata distinguishes repeated occurrences where the caller supplies it.
            items
                .iter()
                .position(|e| e.id == entry.id && e.raw == entry.raw)
                .or_else(|| items.iter().position(|e| e.id == entry.id))
                .ok_or_else(|| {
                    "Selected Spotify item is absent from the supplied collection".into()
                })
        }
        fn body(items: &[Entry], index: usize, position: u64) -> Result<Value, String> {
            if items.is_empty() || index >= items.len() || !items.iter().all(playable) {
                return Err(
                    "Spotify queue requires Spotify tracks or episodes and a valid selection"
                        .into(),
                );
            }
            // Reject rather than silently truncate a caller's collection.
            if items.len() > 80 {
                return Err("Spotify queue replacement is limited to 80 verifiable items".into());
            }
            Ok(
                json!({"context":{"uri":"spotify:internal:harmony-play-tracks","metadata":{},
        "pages":[{"tracks":items.iter().map(|e|json!({"uri":e.id,"metadata":{}})).collect::<Vec<_>>()}]},
        "play_origin":{"feature_identifier":"harmony"},
        "options":{"license":"","skip_to":{"track_index":index},"seek_to":position,
            "initially_paused":false,"player_options_override":{"shuffling_context":false}}}),
            )
        }
        fn window_body(
            items: &[Entry],
            index: usize,
            position: u64,
        ) -> Result<(Value, usize), String> {
            if items.is_empty()
                || items.len() > 10_000
                || index >= items.len()
                || !items.iter().all(playable)
            {
                return Err("Spotify queue requires valid Spotify items and a selection".into());
            }
            let end = (index + 80).min(items.len());
            Ok((body(&items[index..end], 0, position)?, end))
        }
        fn station(entry: &Entry) -> Option<&Value> {
            let marker = entry.raw.get(STATION)?;
            let name = marker["name"].as_str()?;
            if !matches!(
                name,
                "DZFlowForConfigJSON" | "SPStyleMixJSON" | "DZTrackMixJSON" | "DZArtistMixJSON"
            ) || !marker["args"].is_array()
            {
                return None;
            }
            Some(marker)
        }
        fn metadata(target: &mut Entry, original: &Entry) {
            for name in [STATION, crate::style_mix::MARKER, "added_by"] {
                if let Some(value) = original.raw.get(name) {
                    target.raw[name] = value.clone();
                }
            }
        }
        fn queue_matches(value: &Value, items: &[Entry], index: usize) -> bool {
            value["uri"].as_str() == Some(items[index].id.as_str())
                && value["next_tracks"].as_array().is_some_and(|upcoming| {
                    upcoming.len() >= (items.len() - index - 1).min(10)
                        && upcoming.len() <= items.len() - index - 1
                        && upcoming
                            .iter()
                            .zip(&items[index + 1..])
                            .all(|(v, e)| v["uri"].as_str() == Some(e.id.as_str()))
                })
        }
        impl Queue {
            fn reset(&mut self) {
                let generation = self.generation.clone();
                *self = Self {
                    generation,
                    ..Default::default()
                };
            }
            pub fn bind_generation(&mut self, live: Arc<AtomicU64>, epoch: u64) {
                self.generation = Some((live, epoch));
            }
            fn check_current(&self) -> Result<(), String> {
                if self.generation.as_ref().is_some_and(|(live, epoch)| {
                    *epoch == 0 || live.load(Ordering::SeqCst) != *epoch
                }) {
                    return Err(
                        "Spotify playback request cancelled after device/provider change".into(),
                    );
                }
                Ok(())
            }
            pub fn dispatch(
                &mut self,
                command: &Command,
                s: &mut Snapshot,
            ) -> Result<bool, String> {
                let (mut items, index, position, paused) = match command {
                    Command::PlayEntry(entry, supplied) => {
                        let mut items = if supplied.is_empty() {
                            vec![(**entry).clone()]
                        } else {
                            supplied.clone()
                        };
                        let index = selected(entry, &items)?;
                        crate::audio::mark_queue_fallback(&mut items);
                        (items, index, 0, false)
                    }
                    Command::PlayQueueIndex(index) => (s.queue.as_ref().clone(), *index, 0, false),
                    Command::QueueMove { from, to, expected } => {
                        if s.queue.iter().map(|e| &e.id).ne(expected.iter()) {
                            return Err("Queue changed while dragging; try again".into());
                        }
                        if *from <= s.queue_index
                            || *to <= s.queue_index
                            || *from >= s.queue.len()
                            || *to >= s.queue.len()
                        {
                            return Err("Only upcoming Spotify items can be reordered".into());
                        }
                        let mut items = s.queue.as_ref().clone();
                        let item = items.remove(*from);
                        items.insert(*to, item);
                        (items, s.queue_index, s.position, s.state != "Playing")
                    }
                    Command::QueueClear => {
                        if !playable(&s.now) {
                            return Err(
                                "Spotify has no current item whose upcoming queue can be cleared"
                                    .into(),
                            );
                        }
                        (vec![s.now.clone()], 0, s.position, s.state != "Playing")
                    }
                    Command::QueueInsertNext(entry) | Command::QueueAddLast(entry) => {
                        if !playable(entry) {
                            return Err("Only Spotify tracks and episodes can be queued".into());
                        }
                        let mut items = s.queue.as_ref().clone();
                        if items.is_empty() && playable(&s.now) {
                            items.push(s.now.clone());
                        }
                        if items.is_empty() {
                            return Err("Start Spotify playback before editing its queue".into());
                        }
                        let at = if matches!(command, Command::QueueInsertNext(_)) {
                            (s.queue_index + 1).min(items.len())
                        } else {
                            crate::audio::queue_add_position(&items, s.queue_index)
                        };
                        items.insert(at, crate::audio::explicit_queue_entry((**entry).clone()));
                        (items, s.queue_index, s.position, s.state != "Playing")
                    }
                    Command::Play(uri) => {
                        if !uri.starts_with("spotify:") {
                            return Err("Stale non-Spotify playback request rejected".into());
                        }
                        self.reset();
                        return Ok(false);
                    }
                    Command::Stop => {
                        self.reset();
                        return Ok(false);
                    }
                    _ => return Ok(false),
                };
                if let Command::PlayEntry(entry, _) = command {
                    if let Some(marker) = station(entry) {
                        for item in &mut items {
                            item.raw[STATION] = marker.clone();
                            metadata(item, entry);
                        }
                    }
                }
                self.replace(
                    s,
                    items,
                    index,
                    position,
                    paused,
                    !matches!(command, Command::PlayEntry(..) | Command::PlayQueueIndex(_)),
                )?;
                Ok(true)
            }
            fn replace(
                &mut self,
                s: &mut Snapshot,
                items: Vec<Entry>,
                index: usize,
                mut position: u64,
                mut paused: bool,
                preserve: bool,
            ) -> Result<(), String> {
                crate::spotify_diagnostics::record(
                    "queue.replace",
                    &format!(
                        "items={} index={} preserve={}",
                        items.len(),
                        index,
                        preserve
                    ),
                );
                window_body(&items, index, position)?;
                self.check_current()?;
                let before = web_devices::playback_target().map_err(|e| {
                    crate::spotify_diagnostics::record(
                        "queue.target.error",
                        crate::spotify_diagnostics::error_kind(&e),
                    );
                    e
                })?;
                crate::spotify_diagnostics::record("queue.target", "ok");
                if s.now.raw["spotify_local_device_name"]
                    .as_str()
                    .is_some_and(|name| before["device"]["name"].as_str() != Some(name))
                {
                    return Err(
                        "Spotify active device changed; refresh before editing playback".into(),
                    );
                }
                if s.now.raw["spotify_device_id"]
                    .as_str()
                    .is_some_and(|id| before["device"]["id"].as_str() != Some(id))
                {
                    return Err(
                        "Spotify active device changed; refresh before editing playback".into(),
                    );
                }
                if preserve {
                    if before["uri"].as_str() != Some(items[index].id.as_str()) {
                        return Err(
                            "Spotify current item changed; refresh before editing the queue".into(),
                        );
                    }
                    position = before["position_ms"]
                        .as_u64()
                        .ok_or("Spotify returned no playback position")?;
                    paused = !before["is_playing"]
                        .as_bool()
                        .ok_or("Spotify returned no playback state")?;
                }
                let (mut request, window_end) = window_body(&items, index, position)?;
                let device = before["device"]["id"]
                    .as_str()
                    .ok_or("Spotify returned no active device")?
                    .to_owned();
                if !preserve {
                    request["options"]["initially_paused"] = json!(paused);
                    self.check_current()?;
                    web_devices::play(request, &device).map_err(|e| {
                        crate::spotify_diagnostics::record(
                            "queue.play.error",
                            crate::spotify_diagnostics::error_kind(&e),
                        );
                        e
                    })?;
                    crate::spotify_diagnostics::record("queue.play", "acknowledged");
                    self.items = Arc::new(items);
                    self.device = Some(device.clone());
                    self.index = index;
                    self.window_end = window_end;
                    self.confirmed = Some(Instant::now());
                    self.awaiting_play = true;
                    s.queue = self.items.clone();
                    s.queue_index = index;
                    s.now = self.items[index].clone();
                    s.position = position;
                    s.duration = s.now.duration;
                    s.state = if paused { "Paused" } else { "Playing" }.into();
                    s.radio_active = station(&s.now).is_some();
                    s.message = "Spotify playback started".into();
                    return Ok(());
                }
                // SetQueue replaces upcoming tracks without restarting current playback. Revision
                // and device guards reject concurrent changes instead of overwriting another player.
                let mut observed = before;
                let next: Vec<_> = items[index + 1..window_end]
                    .iter()
                    .map(|e| json!({"uri":e.id,"metadata":{}}))
                    .collect();
                self.check_current()?;
                web_devices::set_queue(&observed, &next).map_err(|e| {
                    crate::spotify_diagnostics::record(
                        "queue.set.error",
                        crate::spotify_diagnostics::error_kind(&e),
                    );
                    e
                })?;
                crate::spotify_diagnostics::record("queue.set", "acknowledged");
                let mut verified = false;
                for _ in 0..5 {
                    self.check_current()?;
                    observed =
                        web_devices::snapshot()?.ok_or("Spotify active device disappeared")?;
                    if observed["device"]["id"].as_str() != Some(&device) {
                        return Err("Spotify active device changed during queue replacement".into());
                    }
                    if queue_matches(&observed, &items[..window_end], index)
                        && observed["is_playing"].as_bool() == Some(!paused)
                        && observed["position_ms"]
                            .as_u64()
                            .is_some_and(|p| p.abs_diff(position) < 5000)
                    {
                        verified = true;
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(150));
                }
                if !verified {
                    self.items = Arc::default();
                    return Err("Spotify did not confirm queue order, position and playback state; refresh the queue (remote playback may have changed)".into());
                }
                self.items = Arc::new(items);
                self.device = Some(device.into());
                self.index = index;
                self.window_end = window_end;
                self.confirmed = Some(Instant::now());
                s.queue = self.items.clone();
                s.queue_index = index;
                s.now = self.items[index].clone();
                s.position = observed["position_ms"].as_u64().unwrap_or(position);
                s.state = if paused { "Paused" } else { "Playing" }.into();
                s.radio_active = station(&s.now).is_some();
                s.message = "Spotify queue confirmed".into();
                Ok(())
            }
            pub fn observe(&mut self, s: &mut Snapshot, state: &state::Snapshot) -> bool {
                if self.items.is_empty() {
                    return false;
                }
                let matching = |index: usize| {
                    index < self.window_end
                        && self.items[index].id == s.now.id
                        && state
                            .upcoming
                            .iter()
                            .zip(&self.items[index + 1..self.window_end])
                            .all(|(v, e)| v["item"]["uri"].as_str() == Some(e.id.as_str()))
                        && state.upcoming.len().min(10) == (self.window_end - index - 1).min(10)
                };
                let waiting = self.awaiting_play
                    && self
                        .confirmed
                        .is_some_and(|at| at.elapsed() < Duration::from_secs(3));
                let index = (self.index..self.items.len())
                    .chain(0..self.index)
                    .find(|&i| (!waiting || i == self.index) && matching(i));
                if let Some(index) = index {
                    self.awaiting_play = false;
                    self.index = index;
                    let catalog = &self.items[index];
                    if s.now.title.is_empty() {
                        s.now.title.clone_from(&catalog.title);
                    }
                    if s.now.subtitle.is_empty() {
                        s.now.subtitle.clone_from(&catalog.subtitle);
                    }
                    if s.now.artwork.is_empty()
                        || (!catalog.artwork.is_empty() && s.now.artwork != catalog.artwork)
                    {
                        s.now.artwork.clone_from(&catalog.artwork);
                    }
                    if s.now.album.is_empty() {
                        s.now.album.clone_from(&catalog.album);
                    }
                    if s.now.artist_id.is_empty() {
                        s.now.artist_id.clone_from(&catalog.artist_id);
                    }
                    if s.now.album_id.is_empty() {
                        s.now.album_id.clone_from(&catalog.album_id);
                    }
                    if s.now.duration == 0 {
                        s.now.duration = catalog.duration;
                    }
                    if s.duration == 0 {
                        s.duration = s.now.duration;
                    }
                    s.title.clone_from(&s.now.title);
                    metadata(&mut s.now, catalog);
                    s.queue = self.items.clone();
                    s.queue_index = index;
                    s.radio_active = station(&s.now).is_some();
                } else if self
                    .confirmed
                    .is_some_and(|at| at.elapsed() < Duration::from_secs(3))
                {
                    // Acknowledged REST state can precede the corresponding Soloist events.
                    s.queue = self.items.clone();
                    s.queue_index = self.index;
                    s.now = self.items[self.index].clone();
                    s.title = s.now.title.clone();
                    s.duration = s.now.duration;
                    if waiting {
                        s.position = 0;
                    }
                    if s.state == "Stopped" {
                        s.state = "Playing".into();
                    }
                    s.radio_active = station(&s.now).is_some();
                } else {
                    self.items = Arc::default();
                    return false;
                }
                true
            }
            pub fn observe_remote(&mut self, s: &mut Snapshot, remote: &Value) {
                if self
                    .device
                    .as_deref()
                    .is_some_and(|id| remote["device"]["id"].as_str() != Some(id))
                {
                    self.reset();
                    return;
                }
                let upcoming = remote["next_tracks"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|t| json!({"item":{"uri":t["uri"]}}))
                    .collect();
                self.observe(
                    s,
                    &state::Snapshot {
                        upcoming,
                        ..Default::default()
                    },
                );
            }
            pub fn refill(&mut self, s: &mut Snapshot) {
                if self.check_current().is_err() {
                    return;
                }
                if self.items.is_empty()
                    || s.state != "Playing"
                    || s.repeat == 2
                    || self.window_end.saturating_sub(self.index + 1) > 2
                    || self
                        .refill_at
                        .is_some_and(|at| at.elapsed() < Duration::from_secs(30))
                {
                    return;
                }
                if self.window_end < self.items.len() {
                    self.refill_at = Some(Instant::now());
                    if let Err(error) = self.replace(
                        s,
                        self.items.as_ref().clone(),
                        self.index,
                        s.position,
                        s.state != "Playing",
                        true,
                    ) {
                        s.message = format!("Spotify queue continuation: {error}");
                    }
                    return;
                }
                let Some(marker) = station(&s.now).cloned() else {
                    return;
                };
                self.refill_at = Some(Instant::now());
                let result = (|| {
                    let value = crate::spotify::catalog_invoke(
                        marker["name"].as_str().unwrap(),
                        marker["args"].as_array().unwrap(),
                    )?;
                    let mut tracks = station_tracks(&value)?;
                    for track in &mut tracks {
                        if let (Some(expected), Some(actual)) = (
                            s.now.raw.get(crate::style_mix::MARKER),
                            track.raw.get(crate::style_mix::MARKER),
                        ) {
                            if expected != actual {
                                return Err(
                                    "Spotify station refill returned a different style selection"
                                        .into(),
                                );
                            }
                        }
                        track.raw[STATION] = marker.clone();
                        if let Some(key) = s.now.raw.get(crate::style_mix::MARKER) {
                            track.raw[crate::style_mix::MARKER] = key.clone();
                        }
                    }
                    // Retain current and upcoming occurrences; history cannot grow without bound.
                    let mut items = s.queue[s.queue_index..].to_vec();
                    if items.len() + tracks.len() > 10_000 {
                        return Err(
                            "Station refill exceeds the verifiable Spotify queue size".into()
                        );
                    }
                    items.extend(tracks);
                    self.replace(s, items, 0, s.position, false, true)
                })();
                if let Err(error) = result {
                    s.message = format!("Spotify station refill: {error}");
                }
            }
        }
        fn station_tracks(value: &Value) -> Result<Vec<Entry>, String> {
            let values = value
                .as_array()
                .or_else(|| value["data"].as_array())
                .or_else(|| value["tracks"].as_array())
                .or_else(|| value["items"].as_array())
                .ok_or("Spotify station returned no track list")?;
            let entries = values
                .iter()
                .map(|v| {
                    crate::catalog::entry(v, "track")
                        .filter(playable)
                        .ok_or_else(|| "Spotify station returned an invalid track".to_owned())
                })
                .collect::<Result<Vec<_>, _>>()?;
            if entries.is_empty() {
                return Err("Spotify station returned no playable tracks".into());
            }
            Ok(entries)
        }

        #[cfg(test)]
        mod tests {
            use super::*;
            #[test]
            fn provider_generation_cancels_between_queue_requests_and_survives_reset() {
                let live = Arc::new(AtomicU64::new(3));
                let mut queue = Queue::default();
                queue.bind_generation(live.clone(), 3);
                assert!(queue.check_current().is_ok());
                queue.reset();
                live.store(4, Ordering::SeqCst);
                assert!(queue.check_current().is_err());
            }
            fn track(id: char) -> Entry {
                Entry {
                    id: format!("spotify:track:{}", id.to_string().repeat(22)),
                    kind: "track".into(),
                    raw: json!({}),
                    ..Default::default()
                }
            }
            #[test]
            fn supplied_order_repeats_offset_and_position_are_preserved() {
                let a = track('a');
                let b = track('b');
                let items = vec![a.clone(), b.clone(), a.clone()];
                let wire = body(&items, 2, 12345).unwrap();
                assert_eq!(
                    wire["context"]["pages"][0]["tracks"],
                    json!([{"uri":a.id,"metadata":{}},{"uri":b.id,"metadata":{}},{"uri":a.id,"metadata":{}}])
                );
                assert_eq!(wire["options"]["skip_to"]["track_index"], 2);
                assert_eq!(wire["options"]["seek_to"], 12345);
                let mut selected = a;
                selected.raw["spotify_selection_index"] = json!(2);
                assert_eq!(super::selected(&selected, &items).unwrap(), 2);
                selected.raw["spotify_selection_index"] = json!(1);
                assert!(super::selected(&selected, &items).is_err());
            }
            #[test]
            fn foreign_identity_bounds_and_empty_lists_fail_before_io() {
                let mut foreign = track('a');
                foreign.id = "12345".into();
                assert!(body(&[foreign], 0, 0).is_err());
                assert!(body(&[], 0, 0).is_err());
                assert!(body(&[track('a')], 1, 0).is_err());
                assert!(body(&vec![track('a'); 81], 0, 0).is_err());
            }
            #[test]
            fn confirmation_rejects_wrong_occurrence_order_or_leftover_manual_queue() {
                let a = track('a');
                let b = track('b');
                let c = track('c');
                let items = vec![a.clone(), b.clone(), a.clone()];
                let mut response = json!({"uri":a.id,"next_tracks":[{"uri":b.id},{"uri":a.id}]});
                assert!(queue_matches(&response, &items, 0));
                response["next_tracks"] = json!([{"uri":a.id},{"uri":b.id}]);
                assert!(!queue_matches(&response, &items, 0));
                response["next_tracks"] = json!([{"uri":b.id},{"uri":a.id},{"uri":c.id}]);
                assert!(!queue_matches(&response, &items, 0));
            }
            #[test]
            fn station_marker_and_attribution_survive_normalization() {
                let mut source = track('a');
                source.raw = json!({"spotify_station":{"name":"SPStyleMixJSON","args":["1:2"]},"style_mix":"1:2","added_by":"Host"});
                let mut telemetry = track('a');
                metadata(&mut telemetry, &source);
                assert_eq!(telemetry.raw, source.raw);
                assert!(station(&telemetry).is_some());
                telemetry.raw[STATION]["name"] = json!("GenericSearch");
                assert!(station(&telemetry).is_none());
            }
            #[test]
            fn long_collections_use_overlapping_windows_without_losing_queue_entries() {
                let items: Vec<_> = (0..170)
                    .map(|i| Entry {
                        id: format!("spotify:track:{i:022}"),
                        kind: "track".into(),
                        ..Default::default()
                    })
                    .collect();
                let (request, end) = window_body(&items, 78, 1234).unwrap();
                assert_eq!(end, 158);
                assert_eq!(
                    request["context"]["pages"][0]["tracks"]
                        .as_array()
                        .unwrap()
                        .len(),
                    80
                );
                assert_eq!(
                    request["context"]["pages"][0]["tracks"][0]["uri"],
                    items[78].id
                );
                assert_eq!(request["options"]["skip_to"]["track_index"], 0);
                assert_eq!(request["options"]["seek_to"], 1234);
                let queue = json!({"uri":items[78].id,"next_tracks":items[79..89].iter().map(|e|json!({"uri":e.id})).collect::<Vec<_>>()});
                assert!(queue_matches(&queue, &items[..end], 78));
                let (last, end) = window_body(&items, 158, 0).unwrap();
                assert_eq!(end, 170);
                assert_eq!(
                    last["context"]["pages"][0]["tracks"]
                        .as_array()
                        .unwrap()
                        .len(),
                    12
                );
                assert_eq!(items.len(), 170);
            }
            #[test]
            fn pending_play_ignores_stale_previous_track_and_position() {
                let a = track('a');
                let b = track('b');
                let mut queue = Queue {
                    items: Arc::new(vec![a.clone(), b.clone()]),
                    index: 1,
                    window_end: 2,
                    confirmed: Some(Instant::now()),
                    awaiting_play: true,
                    ..Default::default()
                };
                let mut snapshot = Snapshot {
                    now: a,
                    position: 42_000,
                    state: "Playing".into(),
                    ..Default::default()
                };
                let state = state::Snapshot {
                    upcoming: vec![json!({"item":{"uri":b.id}})],
                    ..Default::default()
                };
                assert!(queue.observe(&mut snapshot, &state));
                assert_eq!(snapshot.now.id, b.id);
                assert_eq!(snapshot.position, 0);
                assert_eq!(snapshot.queue_index, 1);
                let state = state::Snapshot::default();
                assert!(queue.observe(&mut snapshot, &state));
                assert!(!queue.awaiting_play);
            }

            #[test]
            fn observation_distinguishes_repeated_occurrences_and_retains_markers() {
                let mut a = track('a');
                let b = track('b');
                a.raw[STATION] = json!({"name":"DZTrackMixJSON","args":[a.id]});
                let mut queue = Queue {
                    items: Arc::new(vec![a.clone(), b.clone(), a.clone()]),
                    window_end: 3,
                    ..Default::default()
                };
                let mut snapshot = Snapshot {
                    now: track('a'),
                    ..Default::default()
                };
                let state = state::Snapshot {
                    upcoming: vec![],
                    ..Default::default()
                };
                assert!(queue.observe(&mut snapshot, &state));
                assert_eq!(snapshot.queue_index, 2);
                assert_eq!(snapshot.now.raw[STATION], a.raw[STATION]);
                assert!(snapshot.radio_active);
                let published = snapshot.queue.clone();
                assert!(Arc::ptr_eq(&published, &queue.items));
                assert!(queue.observe(&mut snapshot, &state));
                assert!(Arc::ptr_eq(&published, &snapshot.queue));
                snapshot.now = track('c');
                assert!(!queue.observe(&mut snapshot, &state));
                assert!(queue.items.is_empty());
                assert_eq!(published.len(), 3);
            }
        }
    }
    pub(crate) use playback_queue::Queue;

    pub(crate) fn dispatch(
        command: &Command,
        handle: &Handle,
        snapshot: &mut Snapshot,
        queue: &mut Queue,
    ) -> Result<(), String> {
        if matches!(
            command,
            Command::PlayEntry(..)
                | Command::PlayQueueIndex(_)
                | Command::QueueAddLast(_)
                | Command::QueueInsertNext(_)
                | Command::QueueClear
                | Command::QueueMove { .. }
        ) {
            // Explicit local play must address the paired Soloist, even when another
            // Connect device was active. Never move playback for a queue-only edit.
            let local = handle
                .snapshot()
                .filter(|s| s.connected && s.logged_in)
                .ok_or("Pair Soloist in Spotify before starting local playback")?;
            if local.is_active {
                // The queue request obtains a fresh active device; validate it there.
                snapshot.now.raw["spotify_device_id"] = Value::Null;
                snapshot.now.raw["spotify_local_device_name"] = json!(local.device_name);
            } else {
                let devices = super::web_devices::devices()?;
                let matches: Vec<_> = devices
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|d| {
                        d["name"].as_str() == Some(local.device_name.as_str())
                            && d["is_restricted"] == false
                    })
                    .collect();
                let target = matches
                .iter()
                .find(|d| d["is_active"] == true)
                .copied()
                .or_else(|| (matches.len() == 1).then(|| matches[0]))
                .ok_or(
                    "Soloist device is unavailable or ambiguous; refresh Spotify Connect devices",
                )?;
                let device = target["id"].as_str().ok_or("Invalid Soloist device")?;
                if target["is_active"] != true {
                    if !matches!(command, Command::PlayEntry(..) | Command::PlayQueueIndex(_)) {
                        return Err(
                        "Soloist is no longer the active device; refresh before editing its queue"
                            .into(),
                    );
                    }
                    super::web::device_command(
                        json!({"action":"transfer","device":device,"playing":false,"confirm":false}),
                    )?;
                }
                snapshot.now.raw["spotify_device_id"] = json!(device);
                snapshot.now.raw["spotify_local_device_name"] = Value::Null;
            }
        }
        // Queue edits use native HTTP; Soloist remains the playback device.
        if queue.dispatch(command, snapshot)? {
            return Ok(());
        }
        let control = match command {
            Command::SwitchProvider(_) => return Ok(()),
            Command::Play(uri) => Control::Play(Some(uri.clone())),
            Command::PlayEntry(..) => unreachable!(),
            Command::PlayQueueIndex(_) => unreachable!(),
            Command::Toggle => {
                if snapshot.state == "Playing" {
                    Control::Pause
                } else {
                    Control::Play(None)
                }
            }
            Command::SetPlaying(true) => Control::Play(None),
            Command::SetPlaying(false) | Command::Stop => Control::Pause,
            Command::Next => Control::Next,
            Command::Previous => Control::Previous,
            Command::Seek(percent) => Control::Seek(
                (snapshot.duration.saturating_mul((*percent).min(100) as u64) / 100)
                    .min(u32::MAX as u64) as u32,
            ),
            Command::SeekMs(ms) => Control::Seek(
                (if snapshot.duration > 0 {
                    (*ms).min(snapshot.duration)
                } else {
                    *ms
                })
                .min(u32::MAX as u64) as u32,
            ),
            Command::Volume(value) => Control::Volume((*value).min(100) as u8),
            Command::Shuffle(enabled) => Control::Shuffle(*enabled),
            Command::Repeat(mode) => Control::Repeat(
                match mode {
                    0 => "off",
                    1 => "context",
                    2 => "track",
                    _ => return Err("Invalid repeat mode".into()),
                }
                .into(),
            ),
            Command::QueueAddLast(_)
            | Command::QueueInsertNext(_)
            | Command::QueueClear
            | Command::QueueMove { .. } => unreachable!(),
            Command::Autoplay(_) => {
                return Err(
                    "Change Spotify Autoplay in the Spotify app; Soloist has no local control"
                        .into(),
                )
            }
            Command::Api(name, args) => return super::catalog_invoke(name, args).map(|_| ()),
            Command::Login(_) | Command::LoginSaved | Command::Logout => {
                return Err("Use Spotify account authorization for this provider".into())
            }
            Command::Discord(_) | Command::Quit => return Ok(()),
        };
        handle.control(&control).map(|_| ())
    }

    pub(crate) fn update(snapshot: &mut Snapshot, handle: &Handle, managed: &mut Queue) {
        let status = handle.status();
        let Some(state) = handle.snapshot().filter(|_| {
            !matches!(
                status.phase,
                super::service::Phase::Failed | super::service::Phase::Stopped
            )
        }) else {
            snapshot.playback_connected = false;
            snapshot.state = "Stopped".into();
            snapshot.message = status.message;
            return;
        };
        snapshot.playback_connected = state.connected && state.logged_in;
        snapshot.state = if !state.is_active {
            "Stopped"
        } else {
            match state.status.as_str() {
                "playing" => "Playing",
                "paused" => "Paused",
                "loading" | "buffering" => "Loading",
                _ => "Stopped",
            }
        }
        .into();
        snapshot.position = state.position_ms;
        snapshot.duration = state.duration_ms;
        snapshot.shuffle = state.shuffle;
        snapshot.repeat = match state.repeat.as_str() {
            "context" => 1,
            "track" => 2,
            _ => 0,
        };
        snapshot.radio_active = false;
        snapshot.eq = Value::Null;
        snapshot.quality.clear(); // No documented Soloist quality telemetry.
        snapshot.now = entity_entry(&state.item).unwrap_or_default();
        snapshot.title = snapshot.now.title.clone();
        if !managed.observe(snapshot, &state) {
            let mut queue = Vec::with_capacity(state.upcoming.len() + 1);
            if !snapshot.now.id.is_empty() {
                queue.push(snapshot.now.clone());
            }
            queue.extend(
                state
                    .upcoming
                    .iter()
                    .filter_map(|v| entity_entry(&v["item"])),
            );
            replace_queue_if_changed(snapshot, queue);
            snapshot.queue_index = 0;
        }
        // Soloist may omit decorations in an incremental item event. Catalog
        // metadata belongs to the same URI and is a valid duration fallback.
        if snapshot.duration == 0 {
            snapshot.duration = snapshot.now.duration;
        }
        if snapshot.duration > 0 {
            snapshot.position = snapshot.position.min(snapshot.duration);
        }
        if let Some(error) = &state.last_error {
            snapshot.message.clone_from(error);
        }
    }
    fn replace_queue_if_changed(snapshot: &mut Snapshot, queue: Vec<Entry>) {
        if *snapshot.queue != queue {
            snapshot.queue = std::sync::Arc::new(queue);
        }
    }

    #[cfg(test)]
    mod queue_update_tests {
        use super::*;
        use std::sync::Arc;

        #[test]
        fn unchanged_queue_keeps_identity_and_changed_queue_is_published() {
            let mut snapshot = Snapshot::default();
            let entry = Entry {
                id: "spotify:track:first".into(),
                ..Entry::default()
            };
            replace_queue_if_changed(&mut snapshot, vec![entry.clone()]);
            let original = snapshot.queue.clone();
            replace_queue_if_changed(&mut snapshot, vec![entry]);
            assert!(Arc::ptr_eq(&original, &snapshot.queue));
            replace_queue_if_changed(&mut snapshot, Vec::new());
            assert!(!Arc::ptr_eq(&original, &snapshot.queue));
            assert!(snapshot.queue.is_empty());
        }
    }
    fn entity_entry(value: &Value) -> Option<Entry> {
        let uri = value["uri"].as_str()?;
        let kind = value["entity_type"]
            .as_str()
            .or_else(|| uri.split(':').nth(1))?;
        if !matches!(kind, "track" | "episode") || !uri.starts_with(&format!("spotify:{kind}:")) {
            return None;
        }
        let decorations = &value["decorations"];
        let parent = &decorations["parent"]["entity"];
        let creators: Vec<_> = decorations["creators"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|v| &v["entity"])
            .collect();
        let artwork = crate::catalog::largest_artwork(
            decorations["visual_identity"]["cover"]
                .as_array()
                .into_iter()
                .flatten(),
        )
        .unwrap_or_default();
        let mut raw = json!({"id":uri,"uri":uri,"provider":"spotify","type":kind,"name":decorations["identity"]["name"],"is_playable":true});
        raw["album"] = json!({"id":parent["uri"],"uri":parent["uri"],"name":parent["decorations"]["identity"]["name"],"provider":"spotify"});
        raw["artists"] = json!(creators.iter().map(|v|json!({"id":v["uri"],"uri":v["uri"],"name":v["decorations"]["identity"]["name"],"provider":"spotify"})).collect::<Vec<_>>());
        Some(Entry {
            id: uri.into(),
            kind: kind.into(),
            title: decorations["identity"]["name"]
                .as_str()
                .unwrap_or_default()
                .into(),
            subtitle: creators
                .iter()
                .filter_map(|v| v["decorations"]["identity"]["name"].as_str())
                .collect::<Vec<_>>()
                .join(", "),
            artwork: artwork.into(),
            duration: decorations["playback"]["duration_ms"].as_u64().unwrap_or(0),
            album: parent["decorations"]["identity"]["name"]
                .as_str()
                .unwrap_or_default()
                .into(),
            album_id: parent["uri"].as_str().unwrap_or_default().into(),
            artist_id: creators
                .first()
                .and_then(|v| v["uri"].as_str())
                .unwrap_or_default()
                .into(),
            raw,
        })
    }
}
pub(crate) mod remote {
    //! Global Connect state/control. Browser readiness never blocks the audio worker.
    use crate::audio::{Command, Snapshot};
    use crate::catalog::Entry;
    use serde_json::{json, Value};
    use std::{
        sync::{
            atomic::{AtomicU64, Ordering},
            mpsc, Arc,
        },
        time::{Duration, SystemTime, UNIX_EPOCH},
    };

    type Reply = (u64, Result<Option<Value>, String>, Option<String>);
    static SELECTED_DEVICE: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);
    pub(crate) fn select_device(id: Option<String>) {
        if let Ok(mut selected) = SELECTED_DEVICE.lock() {
            *selected = id;
        }
    }
    pub(crate) fn selected_device() -> Option<String> {
        SELECTED_DEVICE.lock().ok()?.clone()
    }
    fn selected_target_matches(selected: Option<&str>, state: &Value) -> bool {
        selected.is_some_and(|id| state["device"]["id"].as_str() == Some(id))
    }
    pub(crate) struct Worker {
        epoch: Arc<AtomicU64>,
        enabled: bool,
        generation: u64,
        serial: u64,
        commands: mpsc::SyncSender<(u64, Command, Snapshot)>,
        updates: mpsc::Receiver<Reply>,
        state: Option<Value>,
    }

    fn current(epoch: u64, live: u64) -> bool {
        epoch != 0 && epoch == live
    }

    impl Worker {
        pub(crate) fn new() -> Self {
            let epoch = Arc::new(AtomicU64::new(0));
            let live = epoch.clone();
            let (commands, rx) = mpsc::sync_channel::<(u64, Command, Snapshot)>(8);
            let (tx, updates) = mpsc::channel();
            let (metadata_tx, metadata_rx) = mpsc::sync_channel::<(u64, String)>(1);
            let (entry_tx, entry_rx) = mpsc::channel();
            let metadata_live = epoch.clone();
            std::thread::spawn(move || {
                while let Ok((epoch, uri)) = metadata_rx.recv() {
                    if !current(epoch, metadata_live.load(Ordering::SeqCst)) {
                        continue;
                    }
                    let kind = if uri.starts_with("spotify:episode:") {
                        "episode"
                    } else {
                        "track"
                    };
                    let result = if kind == "track" {
                        super::catalog_invoke("DZTrackMetadataJSON", &[json!(uri)])
                    } else {
                        super::catalog_invoke("SPCatalogItemJSON", &[json!(kind), json!(uri)])
                    };
                    if let Ok(value) = result {
                        if let Some(entry) =
                            crate::catalog::entry(&value, kind).filter(|e| e.id == uri)
                        {
                            if current(epoch, metadata_live.load(Ordering::SeqCst)) {
                                let _ = entry_tx.send((epoch, uri, entry));
                            }
                        }
                    }
                }
            });
            std::thread::spawn(move || {
                let mut queue = super::playback::Queue::default();
                let mut generation = 0;
                let mut requested_uri = String::new();
                let mut metadata: Option<Entry> = None;
                loop {
                    let command = match rx.recv_timeout(Duration::from_secs(1)) {
                        Ok(c) => Some(c),
                        Err(mpsc::RecvTimeoutError::Timeout) => None,
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    };
                    let epoch = live.load(Ordering::SeqCst);
                    if epoch == 0 {
                        continue;
                    }
                    if generation != epoch {
                        queue = Default::default();
                        metadata = None;
                        requested_uri.clear();
                        generation = epoch;
                    }
                    queue.bind_generation(live.clone(), epoch);
                    if command.as_ref().is_some_and(|c| !current(c.0, epoch)) {
                        continue;
                    }
                    let mut error = None;
                    let result = (|| -> Result<Option<Value>, String> {
                        let mut state = match super::web_devices::snapshot()? {
                            Some(state) => state,
                            None if command.as_ref().is_some_and(|c| {
                                matches!(c.1, Command::Play(_) | Command::PlayEntry(..))
                            }) =>
                            {
                                super::web_devices::playback_target()?
                            }
                            None => {
                                queue = Default::default();
                                return Ok(None);
                            }
                        };
                        if !selected_target_matches(selected_device().as_deref(), &state) {
                            return Err("The selected Spotify Connect device is no longer active; select it again or play on this computer".into());
                        }
                        let mut s = Snapshot::default();
                        apply(&mut s, Some(&state), now_ms());
                        queue.observe_remote(&mut s, &state);
                        s.now.raw["spotify_device_id"] = state["device"]["id"].clone();
                        if let Some((_, cmd, expected)) = command {
                            if !current(epoch, live.load(Ordering::SeqCst)) {
                                return Ok(None);
                            }
                            error = dispatch(&cmd, &expected, &mut s, &mut queue).err();
                            state = super::web_devices::snapshot()?
                                .ok_or("Spotify active device disappeared")?;
                            apply(&mut s, Some(&state), now_ms());
                            queue.observe_remote(&mut s, &state);
                        }
                        if !current(epoch, live.load(Ordering::SeqCst)) {
                            return Ok(None);
                        }
                        queue.refill(&mut s);
                        apply_queue_attribution(&mut s, &state);
                        if s.message.starts_with("Spotify station refill:")
                            || s.message.starts_with("Spotify queue continuation:")
                        {
                            error.get_or_insert(s.message.clone());
                        }
                        let uri = state["uri"].as_str().unwrap_or_default().to_owned();
                        if requested_uri != uri {
                            if metadata_tx.try_send((epoch, uri.clone())).is_ok() {
                                requested_uri = uri.clone();
                            }
                            metadata = None;
                        }
                        while let Ok((e, u, entry)) = entry_rx.try_recv() {
                            if metadata_matches(e, epoch, &u, &state) {
                                metadata = Some(entry);
                            }
                        }
                        if let Some(entry) = metadata.as_ref().filter(|e| e.id == uri) {
                            s.now = entry.clone();
                            if s.duration == 0 {
                                s.duration = entry.duration;
                            }
                            s.now.duration = s.duration;
                        }
                        if s.now.title.is_empty() {
                            if let Some(entry) = s.queue.get(s.queue_index).filter(|e| e.id == uri)
                            {
                                s.now = entry.clone();
                            }
                        }
                        state["entry"] = serde_json::to_value(&s.now).unwrap_or(Value::Null);
                        state["duration_ms"] = json!(s.duration);
                        state["managed_queue"] =
                            serde_json::to_value(s.queue.as_ref()).unwrap_or(Value::Null);
                        state["queue_index"] = json!(s.queue_index);
                        state["radio_active"] = json!(s.radio_active);
                        Ok(Some(state))
                    })();
                    if current(epoch, live.load(Ordering::SeqCst)) {
                        let _ = tx.send((epoch, result, error));
                    }
                }
            });
            Self {
                epoch,
                enabled: false,
                generation: 0,
                serial: 0,
                commands,
                updates,
                state: None,
            }
        }

        /// Disable on local activation/provider change. One worker remains parked;
        /// an in-flight browser request may finish, but its result cannot be adopted.
        pub(crate) fn enable(&mut self, enabled: bool, generation: u64) {
            if enabled != self.enabled || generation != self.generation {
                self.serial = self.serial.wrapping_add(1).max(1);
                self.epoch
                    .store(if enabled { self.serial } else { 0 }, Ordering::SeqCst);
                self.state = None;
            }
            self.generation = generation;
            self.enabled = enabled;
        }

        pub(crate) fn dispatch(&self, command: &Command, s: &Snapshot) -> Result<(), String> {
            match command {
                Command::SwitchProvider(_) | Command::Discord(_) => return Ok(()),
                Command::Api(name, args) => return super::catalog_invoke(name, args).map(|_| ()),
                Command::Autoplay(_) => return Err(
                    "Spotify autoplay preferences are not exposed by the verified Connect service"
                        .into(),
                ),
                _ => {}
            }
            if !self.enabled
                || (self.state.is_none()
                    && !matches!(command, Command::Play(_) | Command::PlayEntry(..)))
            {
                return Err("Waiting for current remote Spotify playback state".into());
            }
            self.commands
                .try_send((
                    self.epoch.load(Ordering::SeqCst),
                    command.clone(),
                    s.clone(),
                ))
                .map_err(|_| "Spotify remote control queue is busy".into())
        }

        pub(crate) fn update(&mut self, s: &mut Snapshot) {
            while let Ok((epoch, result, error)) = self.updates.try_recv() {
                if !current(epoch, self.epoch.load(Ordering::SeqCst)) {
                    continue;
                }
                match result {
                    Ok(state) => {
                        if state.is_some() && self.state.is_none() {
                            s.message.clear();
                        }
                        self.state = state;
                    }
                    Err(error) => {
                        self.state = None;
                        s.message = error;
                    }
                }
                if let Some(error) = error {
                    s.message = error;
                }
            }
            apply(s, self.state.as_ref(), now_ms());
        }
    }

    fn metadata_matches(epoch: u64, live: u64, uri: &str, state: &Value) -> bool {
        current(epoch, live) && state["uri"].as_str() == Some(uri)
    }
    fn dispatch(
        command: &Command,
        expected: &Snapshot,
        s: &mut Snapshot,
        queue: &mut super::playback::Queue,
    ) -> Result<(), String> {
        if expected.now.raw["spotify_device_id"]
            .as_str()
            .is_some_and(|id| s.now.raw["spotify_device_id"].as_str() != Some(id))
        {
            return Err(
                "Spotify active device changed; refresh before controlling playback".into(),
            );
        }
        if matches!(
            command,
            Command::PlayQueueIndex(_)
                | Command::QueueClear
                | Command::QueueMove { .. }
                | Command::QueueInsertNext(_)
                | Command::QueueAddLast(_)
        ) && (expected.now.id != s.now.id
            || expected.queue_index != s.queue_index
            || expected
                .queue
                .iter()
                .map(|e| &e.id)
                .ne(s.queue.iter().map(|e| &e.id)))
        {
            return Err("Spotify queue changed; refresh before editing".into());
        }
        if queue.dispatch(command, s)? {
            return Ok(());
        }
        let (action, value) = match command {
            Command::Play(uri) => {
                return super::web_devices::play_context(
                    uri,
                    s.now.raw["spotify_device_id"]
                        .as_str()
                        .ok_or("Spotify returned no active device")?,
                )
            }
            Command::Toggle => (
                if s.state == "Playing" {
                    "pause"
                } else {
                    "resume"
                },
                None,
            ),
            Command::SetPlaying(true) => ("resume", None),
            Command::SetPlaying(false) | Command::Stop => ("pause", None),
            Command::Next => ("next", None),
            Command::Previous => ("previous", None),
            Command::Seek(percent) if s.duration > 0 => (
                "seek",
                Some(s.duration.saturating_mul((*percent).min(100) as u64) / 100),
            ),
            Command::SeekMs(ms) => (
                "seek",
                Some(if s.duration > 0 {
                    (*ms).min(s.duration)
                } else {
                    *ms
                }),
            ),
            Command::Volume(v) => ("volume", Some((*v).min(100) as u64)),
            Command::Shuffle(v) => ("shuffle", Some(u64::from(*v))),
            Command::Repeat(v) => ("repeat", Some(*v as u64)),
            _ => return Err("This command is unavailable on the remote Spotify device".into()),
        };
        super::web_devices::control_on(action, value, s.now.raw["spotify_device_id"].as_str())
    }
    fn now_ms() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .min(u64::MAX as u128) as u64
    }

    // Attribution belongs to a queue occurrence, not to the track URI: the
    // same track can be queued more than once by different listeners.
    fn apply_queue_attribution(s: &mut Snapshot, state: &Value) {
        let Some(upcoming) = state["next_tracks"].as_array() else {
            return;
        };
        let Some(start) = s.queue_index.checked_add(1) else {
            return;
        };
        let queue = Arc::make_mut(&mut s.queue);
        for (entry, track) in queue.iter_mut().skip(start).zip(upcoming) {
            if track["uri"].as_str() != Some(entry.id.as_str()) {
                break;
            }
            let name = [
                &track["queuedBy"],
                &track["queued_by"],
                &track["added_by"],
                &track["metadata"]["queuedBy"],
                &track["metadata"]["queued_by"],
                &track["metadata"]["added_by"],
            ]
            .into_iter()
            .find_map(|value| {
                value["name"]
                    .as_str()
                    .or_else(|| value["display_name"].as_str())
                    .or_else(|| value.as_str())
            })
            .map(|name| {
                name.chars()
                    .filter(|c| !c.is_control())
                    .take(64)
                    .collect::<String>()
            })
            .filter(|name| !name.trim().is_empty());
            if !entry.raw.is_object() {
                entry.raw = json!({});
            }
            if let Some(name) = name {
                entry.raw["added_by"] = json!(name.trim());
            } else if let Some(raw) = entry.raw.as_object_mut() {
                raw.remove("added_by");
            }
        }
    }

    fn apply(s: &mut Snapshot, state: Option<&Value>, now: u64) {
        s.playback_connected = state.is_some();
        s.state = "Stopped".into();
        s.now = Default::default();
        s.title.clear();
        s.position = 0;
        s.duration = 0;
        s.queue = Default::default();
        s.queue_index = 0;
        s.shuffle = false;
        s.repeat = 0;
        s.radio_active = false;
        s.eq = Value::Null;
        s.quality.clear();
        let Some(state) = state else {
            return;
        };
        let playing = state["is_playing"].as_bool() == Some(true);
        s.state = if playing { "Playing" } else { "Paused" }.into();
        s.duration = state["duration_ms"].as_u64().unwrap_or(0);
        let elapsed = if playing {
            state["timestamp_ms"]
                .as_u64()
                .map(|t| now.saturating_sub(t))
                .unwrap_or(0)
        } else {
            0
        };
        s.position = state["position_ms"]
            .as_u64()
            .unwrap_or(0)
            .saturating_add(elapsed);
        if s.duration > 0 {
            s.position = s.position.min(s.duration);
        }
        let uri = state["uri"].as_str().unwrap_or_default();
        s.now = serde_json::from_value::<Entry>(state["entry"].clone())
            .ok()
            .filter(|e| e.id == uri)
            .unwrap_or_else(|| Entry {
                id: uri.into(),
                kind: if uri.starts_with("spotify:episode:") {
                    "episode"
                } else {
                    "track"
                }
                .into(),
                ..Default::default()
            });
        if s.duration == 0 {
            s.duration = s.now.duration;
            if s.duration > 0 {
                s.position = s.position.min(s.duration);
            }
        }
        s.now.duration = s.duration;
        s.now.raw["spotify_device_id"] = state["device"]["id"].clone();
        s.title = s.now.title.clone();
        s.shuffle = state["options"]["shuffling_context"]
            .as_bool()
            .unwrap_or(false);
        s.repeat = if state["options"]["repeating_track"].as_bool() == Some(true) {
            2
        } else if state["options"]["repeating_context"].as_bool() == Some(true) {
            1
        } else {
            0
        };
        s.queue = serde_json::from_value::<Vec<Entry>>(state["managed_queue"].clone())
            .map(Arc::new)
            .unwrap_or_else(|_| {
                let mut entries = vec![s.now.clone()];
                entries.extend(
                    state["next_tracks"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|t| {
                            let id = t["uri"].as_str()?;
                            if !id.starts_with("spotify:track:")
                                && !id.starts_with("spotify:episode:")
                            {
                                return None;
                            }
                            Some(Entry {
                                id: id.into(),
                                kind: if id.starts_with("spotify:episode:") {
                                    "episode"
                                } else {
                                    "track"
                                }
                                .into(),
                                ..Default::default()
                            })
                        }),
                );
                Arc::new(entries)
            });
        s.queue_index = state["queue_index"].as_u64().unwrap_or(0) as usize;
        s.radio_active = state["radio_active"].as_bool() == Some(true);
    }

    #[cfg(test)]
    mod tests {
        #[test]
        fn remote_control_requires_the_explicitly_selected_device() {
            let state = serde_json::json!({"device":{"id":"speaker"}});
            assert!(!super::selected_target_matches(None, &state));
            assert!(!super::selected_target_matches(Some("browser"), &state));
            assert!(super::selected_target_matches(Some("speaker"), &state));
            assert!(!super::selected_target_matches(
                Some("speaker"),
                &serde_json::Value::Null
            ));
        }
        use super::*;
        use serde_json::json;
        #[test]
        fn queue_attribution_follows_occurrences_and_clears_missing_names() {
            let mut s = Snapshot::default();
            let state = json!({"uri":"spotify:track:current", "next_tracks":[
                {"uri":"spotify:track:same", "queuedBy":{"name":"Alice"}},
                {"uri":"spotify:track:same", "metadata":{"queued_by":{"display_name":"Bob"}}},
                {"uri":"spotify:track:other"}
            ]});
            apply(&mut s, Some(&state), 0);
            apply_queue_attribution(&mut s, &state);
            assert_eq!(s.queue[1].raw["added_by"], "Alice");
            assert_eq!(s.queue[2].raw["added_by"], "Bob");
            assert!(s.queue[3].raw["added_by"].is_null());
            let mut changed = state.clone();
            changed["next_tracks"][0]["queuedBy"] = Value::Null;
            apply_queue_attribution(&mut s, &changed);
            assert!(s.queue[1].raw["added_by"].is_null());
            assert_eq!(s.queue[2].raw["added_by"], "Bob");
        }
        #[test]
        fn metadata_options_and_uri_epoch_guards() {
            let mut s = Snapshot::default();
            let mut state = json!({"uri":"spotify:track:fixture","device":{"id":"remote"},
            "options":{"shuffling_context":true,"repeating_context":true},
        "entry":Entry{id:"spotify:track:fixture".into(),title:"Song".into(),subtitle:"Artist".into(),artwork:"https://art".into(),duration:210000,..Default::default()}});
            apply(&mut s, Some(&state), 0);
            assert_eq!(
                (
                    s.title.as_str(),
                    s.now.subtitle.as_str(),
                    s.now.artwork.as_str()
                ),
                ("Song", "Artist", "https://art")
            );
            assert!(s.shuffle);
            assert_eq!((s.duration, s.now.duration), (210000, 210000));
            assert_eq!(s.repeat, 1);
            assert!(metadata_matches(3, 3, "spotify:track:fixture", &state));
            assert!(!metadata_matches(2, 3, "spotify:track:fixture", &state));
            assert!(!metadata_matches(3, 3, "spotify:track:old", &state));
            state["uri"] = json!("spotify:track:new");
            state["options"] = Value::Null;
            apply(&mut s, Some(&state), 0);
            assert!(s.title.is_empty() && !s.shuffle && s.repeat == 0);
        }
        #[test]
        fn global_position_time_and_options() {
            let mut s = Snapshot {
                connected: true,
                shuffle: true,
                repeat: 2,
                autoplay: true,
                ..Default::default()
            };
            let mut state = json!({"uri":"spotify:track:fixture","is_playing":true,"position_ms":1000,"timestamp_ms":5000,"duration_ms":10000,"device":{"id":"remote"}});
            apply(&mut s, Some(&state), 7000);
            assert_eq!(
                (s.position, s.duration, s.state.as_str()),
                (3000, 10000, "Playing")
            );
            assert!(s.connected && s.playback_connected);
            assert!(!s.shuffle && s.autoplay && s.repeat == 0 && s.queue.len() == 1);
            assert_eq!(s.now.id, "spotify:track:fixture");
            state["is_playing"] = json!(false);
            apply(&mut s, Some(&state), 50000);
            assert_eq!((s.position, s.state.as_str()), (1000, "Paused"));
            state["is_playing"] = json!(true);
            apply(&mut s, Some(&state), 50000);
            assert_eq!(s.position, 10000);
            apply(&mut s, Some(&state), 4000);
            assert_eq!(s.position, 1000);
            apply(&mut s, None, 7000);
            assert!(!s.playback_connected && s.connected && s.now.id.is_empty());
        }
        #[test]
        fn stale_provider_and_disabled_results_are_rejected() {
            assert!(!current(1, 3));
            assert!(!current(1, 0));
            assert!(!current(0, 0));
            assert!(current(3, 3));
        }

        #[test]
        fn provider_round_trip_discards_old_reply_and_controls_are_not_optimistic() {
            let (commands, requests) = mpsc::sync_channel(8);
            let (replies, updates) = mpsc::sync_channel(1);
            let mut worker = Worker {
                epoch: Arc::new(AtomicU64::new(0)),
                enabled: false,
                generation: 0,
                serial: 0,
                commands,
                updates,
                state: None,
            };
            worker.enable(true, 1);
            let old = worker.epoch.load(Ordering::SeqCst);
            worker.enable(false, 2);
            worker.enable(true, 3);
            replies
                .send((
                    old,
                    Ok(Some(json!({"uri":"spotify:track:stale","is_playing":true}))),
                    None,
                ))
                .unwrap();
            let mut s = Snapshot {
                message: "Configure Soloist for local playback".into(),
                ..Default::default()
            };
            worker.update(&mut s);
            assert!(!s.playback_connected && s.now.id.is_empty());
            let live = worker.epoch.load(Ordering::SeqCst);
            replies.send((live, Ok(Some(json!({"uri":"spotify:track:live","is_playing":false,"position_ms":20,"duration_ms":1000}))), None)).unwrap();
            worker.update(&mut s);
            worker.dispatch(&Command::Toggle, &s).unwrap();
            assert!(s.message.is_empty());
            assert!(matches!(requests.try_recv().unwrap().1, Command::Toggle));
            assert_eq!(s.state, "Paused");
            worker.dispatch(&Command::Seek(50), &s).unwrap();
            assert!(matches!(requests.try_recv().unwrap().1, Command::Seek(50)));
            worker.dispatch(&Command::Volume(200), &s).unwrap();
            assert!(matches!(
                requests.try_recv().unwrap().1,
                Command::Volume(200)
            ));
            assert!(worker.dispatch(&Command::QueueClear, &s).is_ok());
            assert!(worker
                .dispatch(&Command::Play("spotify:track:new".into()), &s)
                .is_ok());
            // Local activation followed by deactivation in the same provider epoch
            // also invalidates a browser request that was already in flight.
            worker.enable(false, 3);
            worker.enable(true, 3);
            replies
                .send((live, Ok(Some(json!({"uri":"spotify:track:stale"}))), None))
                .unwrap();
            worker.update(&mut s);
            assert!(!s.playback_connected);
        }
    }
}
pub(crate) mod style_mix {
    //! Filter only explicit track language metadata; never fetch lyrics for discovery.
    use serde_json::Value;

    fn languages(region: usize) -> &'static [&'static str] {
        match crate::style_mix::REGIONS[region].0 {
            "Turkish" => &["tr"],
            "Japanese" => &["ja"],
            "Korean" => &["ko"],
            "French" => &["fr"],
            "Arabic" => &["ar"],
            "Brazilian" | "Portuguese" => &["pt"],
            "Spanish" | "Mexican" | "Argentinian" | "Colombian" | "Chilean" | "Peruvian"
            | "Cuban" | "Dominican" | "Puerto Rican" => &["es"],
            "German" => &["de"],
            "Italian" => &["it"],
            "Hindi" => &["hi"],
            "Punjabi" => &["pa"],
            "Persian" => &["fa"],
            "Greek" => &["el"],
            "Russian" => &["ru"],
            "Chinese" => &["zh"],
            "Indonesian" => &["id"],
            "Thai" => &["th"],
            "Polish" => &["pl"],
            "Dutch" => &["nl"],
            "Swedish" => &["sv"],
            "Norwegian" => &["no", "nb", "nn"],
            "Finnish" => &["fi"],
            "Danish" => &["da"],
            "Ukrainian" => &["uk"],
            "Romanian" => &["ro"],
            "Hungarian" => &["hu"],
            "Czech" => &["cs"],
            "Serbian" => &["sr"],
            "Croatian" => &["hr"],
            "Albanian" => &["sq"],
            "Bengali" => &["bn"],
            "Tamil" => &["ta"],
            "Telugu" => &["te"],
            "Urdu" => &["ur"],
            "Vietnamese" => &["vi"],
            "Filipino / Tagalog" => &["tl", "fil"],
            "Malay" => &["ms"],
            "Nepali" => &["ne"],
            "Hebrew" => &["he", "iw"],
            "Azerbaijani" => &["az"],
            "Georgian" => &["ka"],
            "Armenian" => &["hy"],
            "Kurdish" => &["ku", "ckb", "kmr"],
            "Swahili" => &["sw"],
            "Amharic" => &["am"],
            "Icelandic" => &["is"],
            "Slovak" => &["sk"],
            "Slovenian" => &["sl"],
            // These are country choices with multiple vocal languages.
            _ => &[],
        }
    }

    pub(crate) fn retain_language<T>(
        items: &mut Vec<T>,
        selection: crate::style_mix::Selection,
        metadata: impl Fn(&T) -> &Value,
    ) {
        let expected = languages(selection.region);
        if expected.is_empty() {
            return;
        }
        items.retain(|item| {
            let value = metadata(item);
            let language = value["language"]
                .as_str()
                .or_else(|| value.pointer("/lyrics/language").and_then(Value::as_str));
            language.is_none_or(|language| {
                let normalized = language.trim().to_ascii_lowercase();
                normalized.is_empty()
                    || expected.contains(&normalized.split(['-', '_']).next().unwrap_or_default())
            })
        });
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use serde_json::json;

        #[test]
        fn missing_metadata_survives_and_explicit_language_is_respected() {
            let mut tracks = vec![
                json!({}),
                json!({"language":""}),
                json!({"language":"TR-tr"}),
                json!({"lyrics":{"language":"en"}}),
            ];
            retain_language(
                &mut tracks,
                crate::style_mix::Selection::new(0, 1).unwrap(),
                |v| v,
            );
            assert_eq!(tracks.len(), 3);
            assert_eq!(tracks[2]["language"], "TR-tr");
        }

        #[test]
        fn missing_metadata_does_not_truncate_playlists() {
            let mut tracks = vec![json!({}); 100];
            retain_language(
                &mut tracks,
                crate::style_mix::Selection::new(0, 1).unwrap(),
                |v| v,
            );
            assert_eq!(tracks.len(), 100);
        }
    }
}
pub(crate) mod web {
    //! Explicit browser session capture plus direct native request dispatch.
    //! Routine requests never start or refresh WebView2 automatically.
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    use serde_json::{json, Value};
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    };
    use std::time::Duration;
    use url::Url;
    mod native {
        //! Session system and generic HTTP plumbing. Catalog/Connect integration is separate work.
        use super::*;
        mod connect {
            //! Connect commands over the native authenticated HTTP transport.
            use super::*;
            const ROOT: &str = "https://spclient.wg.spotify.com/connect-state/v1/";

            fn request(
                s: &Value,
                method: &str,
                path: &str,
                body: Option<&Value>,
            ) -> Result<(u32, Value), String> {
                let bytes = body
                    .map(serde_json::to_vec)
                    .transpose()
                    .map_err(|_| "Invalid Connect command")?
                    .unwrap_or_default();
                super::request(
                    s,
                    method,
                    &format!("{ROOT}{path}"),
                    &bytes,
                    "application/json",
                )
            }
            type Reply = Result<(u32, Value), String>;
            fn read(
                send: &mut impl FnMut(&str, &str, Option<&Value>) -> Reply,
            ) -> Result<Value, String> {
                let (_, mut v) = send("GET", "cluster", None)?;
                if v.is_null() {
                    return Ok(json!({"devices":{}}));
                }
                if let Some(cluster) = v.get("cluster") {
                    v = cluster.clone();
                }
                if v.get("devices").is_none()
                    && v["active_device_id"].is_null()
                    && v["timestamp"].is_number()
                    && v["player_state"].is_object()
                {
                    v["devices"] = json!({});
                }
                if !v["devices"].is_object() {
                    return Err("Invalid Spotify Connect cluster".into());
                }
                Ok(v)
            }
            fn id(value: &str) -> bool {
                !value.is_empty()
                    && value.len() <= 256
                    && value
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || b"_%:-".contains(&c))
            }
            fn command_id() -> Result<String, String> {
                use sha2::{Digest, Sha256};
                let random = super::super::super::random()?;
                Ok(format!("{:x}", Sha256::digest(random.as_bytes()))[..32].to_owned())
            }
            fn restriction(state: &Value, key: &str) -> Result<(), String> {
                if state["player_state"]["restrictions"][format!("disallow_{key}_reasons")]
                    .as_array()
                    .is_some_and(|v| !v.is_empty())
                {
                    return Err("Spotify Connect control is restricted".into());
                }
                Ok(())
            }
            pub(super) fn execute(s: &Value, c: &Value) -> Result<Value, String> {
                execute_with(c, &mut |method, path, body| request(s, method, path, body))
            }
            fn execute_with(
                c: &Value,
                send: &mut impl FnMut(&str, &str, Option<&Value>) -> Reply,
            ) -> Result<Value, String> {
                let state = read(send)?;
                let action = c["action"].as_str().ok_or("Missing Connect action")?;
                if action == "state" {
                    return Ok(state);
                }
                let target = if action == "transfer" {
                    &c["device"]
                } else {
                    &state["active_device_id"]
                };
                let target = target
                    .as_str()
                    .filter(|v| id(v))
                    .ok_or("No active Spotify device; select a device first")?;
                let device = state["devices"]
                    .get(target)
                    .ok_or("Spotify Connect target is unavailable")?;
                if device["capabilities"]["is_controllable"] != true {
                    return Err("Spotify device does not allow remote control".into());
                }
                if c["expected"]["device"]
                    .as_str()
                    .is_some_and(|v| v != target)
                {
                    return Err(
                        "Spotify active device changed; refresh before controlling playback".into(),
                    );
                }
                let p = &state["player_state"];
                if c["expected"]["uri"]
                    .as_str()
                    .is_some_and(|v| p["track"]["uri"].as_str() != Some(v))
                {
                    return Err("Spotify current item changed; refresh before editing queue".into());
                }
                // Match the native client protocol: use the observed play origin, with
                // the protocol's `local` origin for a cluster without a play origin.
                // Never depend on a browser registration surviving browser shutdown.
                let sender = p["play_origin"]["device_identifier"]
                    .as_str()
                    .filter(|v| id(v))
                    .unwrap_or("local");
                let (path, method, payload) = if action == "transfer" {
                    if device["can_play"] != true
                        || device["capabilities"]["hidden"] == true
                        || device["is_restricted"] == true
                    {
                        return Err("Spotify device cannot receive playback".into());
                    }
                    restriction(&state, "transferring_playback")?;
                    (
                        "connect/transfer",
                        "POST",
                        json!({"transfer_options":{"restore_paused":if c["playing"] == true {"resume"} else {"pause"}},"command_id":command_id()?}),
                    )
                } else if action == "volume" {
                    let value = c["value"]
                        .as_u64()
                        .filter(|v| *v <= 100)
                        .ok_or("Invalid Spotify volume")?;
                    if device["capabilities"]["disable_volume"] == true {
                        return Err("Spotify volume unavailable".into());
                    }
                    (
                        "connect/volume",
                        "PUT",
                        json!({"volume":(value * 65535 + 50) / 100}),
                    )
                } else {
                    let (endpoint, key) = match action {
                        "pause" => ("pause", "pausing"),
                        "resume" => ("resume", "resuming"),
                        "next" => ("skip_next", "skipping_next"),
                        "previous" => ("skip_prev", "skipping_prev"),
                        "seek" => ("seek_to", "seeking"),
                        "play" => ("play", "interrupting_playback"),
                        "queue" => ("set_queue", "set_queue"),
                        "shuffle" => ("set_options", "toggling_shuffle"),
                        "repeat" => ("set_options", "toggling_repeat_context"),
                        _ => return Err("Unsupported Spotify Connect control".into()),
                    };
                    restriction(&state, key)?;
                    let mut command = json!({"endpoint":endpoint});
                    match action {
                        "seek" => {
                            command["value"] = json!(c["value"]
                                .as_u64()
                                .filter(|v| *v <= 9_007_199_254_740_991)
                                .ok_or("Invalid Spotify seek")?);
                        }
                        "play" => {
                            let descriptor = c["descriptor"]
                                .as_object()
                                .filter(|d| {
                                    d.get("context").is_some_and(Value::is_object)
                                        && d.get("options").is_some_and(Value::is_object)
                                })
                                .ok_or("Invalid Spotify play descriptor")?;
                            command.as_object_mut().unwrap().extend(descriptor.clone());
                            command["endpoint"] = json!(endpoint);
                        }
                        "queue" => {
                            if !c["next_tracks"].is_array()
                                || (!p["prev_tracks"].is_null() && !p["prev_tracks"].is_array())
                            {
                                return Err("Spotify queue state unavailable".into());
                            }
                            if !c["expected"]["revision"].is_null()
                                && c["expected"]["revision"] != p["queue_revision"]
                            {
                                return Err("Spotify queue changed; refresh before editing".into());
                            }
                            command["next_tracks"] = c["next_tracks"].clone();
                            command["prev_tracks"] = p
                                .get("prev_tracks")
                                .filter(|v| v.is_array())
                                .cloned()
                                .unwrap_or(json!([]));
                            command["queue_revision"] = p["queue_revision"].clone();
                        }
                        "shuffle" => {
                            command["shuffling_context"] = json!(
                                c["value"]
                                    .as_u64()
                                    .filter(|v| *v <= 1)
                                    .ok_or("Invalid shuffle")?
                                    == 1
                            );
                        }
                        "repeat" => {
                            restriction(&state, "toggling_repeat_track")?;
                            let value = c["value"]
                                .as_u64()
                                .filter(|v| *v <= 2)
                                .ok_or("Invalid repeat")?;
                            command["repeating_context"] = json!(value == 1);
                            command["repeating_track"] = json!(value == 2);
                        }
                        _ => {}
                    }
                    ("player/command", "POST", json!({"command":command}))
                };
                let (status, response) = send(
                    method,
                    &format!("{path}/from/{sender}/to/{target}"),
                    Some(&payload),
                )?;
                if !((status == 200 && response["ack_id"].as_str().is_some_and(|v| !v.is_empty()))
                    || status == 202)
                {
                    return Err("Spotify Connect did not acknowledge command".into());
                }
                if action == "transfer" {
                    if c["confirm"].as_bool() == Some(false) {
                        return Ok(Value::Null);
                    }
                    for _ in 0..5 {
                        std::thread::sleep(Duration::from_millis(100));
                        let after = read(send)?;
                        if after["active_device_id"] == target
                            && after["player_state"]["is_paused"].as_bool()
                                == Some(c["playing"] != true)
                        {
                            return Ok(Value::Null);
                        }
                    }
                    return Err(
            "Spotify accepted transfer but target state is unconfirmed; refresh devices".into(),
        );
                }
                Ok(Value::Null)
            }

            #[cfg(test)]
            mod tests {
                use super::*;
                #[test]
                fn connect_ids_accept_the_same_targets_as_the_device_selector() {
                    for value in ["speaker", "speaker:room", "device%20name", "room_1-2"] {
                        assert!(id(value));
                    }
                    for value in ["", "../other", "room?query", "room\n"] {
                        assert!(!id(value));
                    }
                }
                #[test]
                fn playing_transfer_addresses_selected_device_and_confirms_playback() {
                    let mut transferred = false;
                    execute_with(
                        &json!({"action":"transfer","device":"speaker:room","playing":true}),
                        &mut |method, path, body| {
                            if method == "GET" {
                                let mut state = cluster();
                                state["devices"]["speaker:room"] =
                                    state["devices"]["soloist"].clone();
                                state["player_state"]["is_paused"] = json!(false);
                                if transferred {
                                    state["active_device_id"] = json!("speaker:room");
                                }
                                return Ok((200, state));
                            }
                            assert_eq!(path, "connect/transfer/from/origin/to/speaker:room");
                            assert_eq!(
                                body.unwrap()["transfer_options"]["restore_paused"],
                                "resume"
                            );
                            transferred = true;
                            Ok((202, Value::Null))
                        },
                    )
                    .unwrap();
                    assert!(transferred);
                }
                fn cluster() -> Value {
                    json!({"active_device_id":"soloist", "devices":{"soloist":{"can_play":true,"capabilities":{"is_controllable":true}}},
            "player_state":{"track":{"uri":"spotify:track:current"},"queue_revision":"rev1","prev_tracks":[],
                "play_origin":{"device_identifier":"origin"}}})
                }
                #[test]
                fn queue_uses_http_origin_and_preserves_revision_without_browser_sender() {
                    let mut writes = 0;
                    execute_with(
                        &json!({"action":"queue","next_tracks":[{"uri":"spotify:track:next"}],
            "expected":{"device":"soloist","uri":"spotify:track:current","revision":"rev1"}}),
                        &mut |method, path, body| {
                            if method == "GET" {
                                assert_eq!(path, "cluster");
                                return Ok((200, cluster()));
                            }
                            writes += 1;
                            assert_eq!(path, "player/command/from/origin/to/soloist");
                            let command = &body.unwrap()["command"];
                            assert_eq!(command["endpoint"], "set_queue");
                            assert_eq!(command["queue_revision"], "rev1");
                            assert_eq!(command["next_tracks"][0]["uri"], "spotify:track:next");
                            assert_eq!(command["prev_tracks"], json!([]));
                            Ok((202, Value::Null))
                        },
                    )
                    .unwrap();
                    assert_eq!(writes, 1);
                }
                #[test]
                fn stale_device_track_or_queue_never_sends_mutation() {
                    for expected in [
                        json!({"device":"other"}),
                        json!({"uri":"spotify:track:other"}),
                        json!({"revision":"rev0"}),
                    ] {
                        let result = execute_with(
                            &json!({"action":"queue","next_tracks":[],"expected":expected}),
                            &mut |method, _, _| {
                                assert_eq!(method, "GET", "stale queue must not send a command");
                                Ok((200, cluster()))
                            },
                        );
                        assert!(result.is_err());
                    }
                }
                #[test]
                fn idle_transfer_uses_native_local_origin_and_requires_observed_target() {
                    let mut calls = 0;
                    execute_with(
                        &json!({"action":"transfer","device":"soloist","playing":false}),
                        &mut |method, path, body| {
                            calls += 1;
                            if method == "GET" {
                                let mut state = cluster();
                                state["player_state"]["play_origin"] = Value::Null;
                                if calls == 1 {
                                    state["active_device_id"] = Value::Null;
                                }
                                state["player_state"]["is_paused"] = json!(true);
                                return Ok((200, state));
                            }
                            assert_eq!(path, "connect/transfer/from/local/to/soloist");
                            assert_eq!(
                                body.unwrap()["transfer_options"]["restore_paused"],
                                "pause"
                            );
                            Ok((202, Value::Null))
                        },
                    )
                    .unwrap();
                    assert!(calls >= 2);
                }
            }
        }
        use std::{
            collections::HashMap,
            sync::atomic::AtomicBool,
            time::{SystemTime, UNIX_EPOCH},
        };

        pub(crate) const REFRESH_REQUIRED: &str = "Spotify session expired or is unavailable. Click Refresh session in Settings > Providers > Spotify.";
        static INVALID: AtomicBool = AtomicBool::new(false);
        const MAX_SESSION_BYTES: usize = 6 * 1024 * 1024;
        static SESSION: Mutex<Option<Value>> = Mutex::new(None);
        static COOLDOWNS: Mutex<Option<HashMap<String, u64>>> = Mutex::new(None);
        fn now() -> u64 {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64
        }
        pub(super) fn reset() {
            if let Ok(mut value) = SESSION.lock() {
                *value = None;
            }
            INVALID.store(true, Ordering::Relaxed);
            let _ = crate::session::provider_remove("spotify-web-api");
        }
        fn normalize_expiry(value: &mut Value) {
            if value["expiry"].as_u64().is_none() {
                if let Some(expiry) = value["expiry"].as_f64().filter(|v| {
                    v.is_finite() && *v >= 0.0 && *v < u64::MAX as f64 && v.fract() == 0.0
                }) {
                    value["expiry"] = json!(expiry as u64);
                }
            }
        }
        fn valid(value: &Value, timestamp: u64) -> bool {
            ["token", "client", "version"].iter().all(|key| {
                value[key]
                    .as_str()
                    .is_some_and(|s| !s.is_empty() && !s.contains(['\r', '\n']))
            }) && value["expiry"]
                .as_u64()
                .is_some_and(|expiry| expiry > timestamp.saturating_add(30_000))
                && value["hashes"].is_object()
        }
        pub(crate) fn save_session(mut value: Value) -> Result<(), String> {
            // WebView2 serializes JavaScript timestamps as floating point numbers.
            normalize_expiry(&mut value);
            if !valid(&value, now()) {
                return Err(format!("Spotify session capture incomplete (token: {}, client: {}, version: {}, expiry: {}, operations: {}). Refresh the session.",
            value["token"].as_str().is_some_and(|v| !v.is_empty()),
            value["client"].as_str().is_some_and(|v| !v.is_empty()),
            value["version"].as_str().is_some_and(|v| !v.is_empty()),
            value["expiry"].as_u64().is_some_and(|v| v > now().saturating_add(30_000)),
            value["hashes"].as_object().map_or(0, |v| v.len())));
            }
            let encoded =
                serde_json::to_string(&value).map_err(|_| "Could not encode Spotify session")?;
            if encoded.len() > MAX_SESSION_BYTES {
                return Err("Spotify session exceeds its size limit".into());
            }
            crate::session::provider_save("spotify-web-api", &encoded)?;
            *SESSION.lock().map_err(|_| "Spotify session unavailable")? = Some(value);
            INVALID.store(false, Ordering::Relaxed);
            Ok(())
        }
        fn session() -> Result<Value, String> {
            if INVALID.load(Ordering::Relaxed) {
                return Err(REFRESH_REQUIRED.into());
            }
            let mut slot = SESSION.lock().map_err(|_| "Spotify session unavailable")?;
            if slot.is_none() {
                *slot = crate::session::provider_saved("spotify-web-api")
                    .ok()
                    .filter(|s| s.len() <= MAX_SESSION_BYTES)
                    .and_then(|s| serde_json::from_str(&s).ok());
            }
            slot.as_ref()
                .filter(|v| valid(v, now()))
                .cloned()
                .ok_or_else(|| REFRESH_REQUIRED.into())
        }
        fn expired() -> String {
            INVALID.store(true, Ordering::Relaxed);
            REFRESH_REQUIRED.into()
        }
        fn request(
            s: &Value,
            method: &str,
            address: &str,
            body: &[u8],
            content: &str,
        ) -> Result<(u32, Value), String> {
            let url = Url::parse(address).map_err(|_| "Invalid Spotify URL")?;
            let host = url.host_str().ok_or("Invalid Spotify host")?;
            if url.scheme() != "https"
                || !url.username().is_empty()
                || url.password().is_some()
                || url.port().is_some()
                || !matches!(
                    host,
                    "api-partner.spotify.com" | "api.spotify.com" | "spclient.wg.spotify.com"
                )
            {
                return Err("Spotify credential destination rejected".into());
            }
            if COOLDOWNS
                .lock()
                .ok()
                .and_then(|c| c.as_ref().and_then(|m| m.get(host).copied()))
                .is_some_and(|until| until > now())
            {
                return Err("Spotify is rate limited; retry later".into());
            }
            let authorization = format!("Bearer {}", s["token"].as_str().ok_or(REFRESH_REQUIRED)?);
            let user_agent = s["userAgent"]
                .as_str()
                .filter(|v| !v.is_empty() && v.len() <= 1024 && !v.contains(['\r', '\n']))
                .unwrap_or("DeezNatsRust/0.1");
            let headers = [
                ("Authorization", authorization.as_str()),
                (
                    "Client-Token",
                    s["client"].as_str().ok_or(REFRESH_REQUIRED)?,
                ),
                (
                    "Spotify-App-Version",
                    s["version"].as_str().ok_or(REFRESH_REQUIRED)?,
                ),
                ("App-Platform", "WebPlayer"),
                ("Origin", "https://open.spotify.com"),
                ("Referer", "https://open.spotify.com/"),
                ("User-Agent", user_agent),
                ("Accept", "application/json"),
                ("Content-Type", content),
            ];
            let response = crate::lyrics::http::response(method, address, &headers, body)?;
            if response.status == 401 {
                return Err(expired());
            }
            if response.status == 429 {
                if let Ok(mut map) = COOLDOWNS.lock() {
                    map.get_or_insert_with(HashMap::new).insert(
                        host.into(),
                        now().saturating_add(
                            response.retry_after.unwrap_or(5).clamp(1, 3600) * 1000,
                        ),
                    );
                }
                return Err("Spotify is rate limited; retry later".into());
            }
            if !(200..300).contains(&response.status) {
                return Err(format!(
                    "Spotify request failed (HTTP {}).{}",
                    response.status,
                    if response.status == 403 {
                        " Click Refresh session; this operation may also be restricted for your account."
                    } else {
                        ""
                    }
                ));
            }
            let value = if response.body.is_empty() {
                Value::Null
            } else {
                serde_json::from_slice::<Value>(&response.body)
                    .map_err(|_| "Invalid Spotify response")?
            };
            if let Some(errors) = value["errors"].as_array().filter(|e| !e.is_empty()) {
                if errors.iter().any(|e| {
                    e["message"].as_str().is_some_and(|m| {
                        let m = m.to_ascii_lowercase();
                        m.contains("unauthenticated") || m.contains("token expired")
                    })
                }) {
                    return Err(expired());
                }
                return Err("Spotify API operation changed or was rejected. Click Refresh session to update the API session.".into());
            }
            Ok((response.status, value))
        }
        pub(super) fn browse_operations() -> Result<Vec<String>, String> {
            let value = session()?;
            Ok(value["hashes"]
                .as_object()
                .into_iter()
                .flat_map(|map| map.keys())
                .filter(|name| name.to_lowercase().contains("browse"))
                .cloned()
                .collect())
        }
        pub(super) fn execute(command: Value) -> Result<Value, String> {
            let s = session()?;
            match command["kind"].as_str() {
                Some("devices") => connect::execute(&s, &command["command"]),
                Some("rest" | "upload") => {
                    let url = command["url"].as_str().ok_or("Missing Spotify URL")?;
                    super::approved_url(url)?;
                    let body = if command["kind"] == "upload" {
                        STANDARD
                            .decode(command["body"].as_str().ok_or("Missing upload")?)
                            .map_err(|_| "Invalid upload")?
                    } else if command["body"].is_null() {
                        Vec::new()
                    } else {
                        command["body"].to_string().into_bytes()
                    };
                    request(
                        &s,
                        command["method"].as_str().ok_or("Missing HTTP method")?,
                        url,
                        &body,
                        command["contentType"]
                            .as_str()
                            .unwrap_or("application/json"),
                    )
                    .map(|(_, v)| v)
                }
                Some("query") => {
                    let operation = command["operation"]
                        .as_str()
                        .ok_or("Missing Spotify operation")?;
                    let hash = s["hashes"][operation]
                        .as_str()
                        .filter(|hash| {
                            hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit())
                        })
                        .or_else(|| match operation {
                            "queryPodcastEpisodes" => Some(
                                "3539d746cf882f3909660de40b4ef472b3f5893a0761d617c282541de5d412c5",
                            ),
                            "queryShowMetadataV2" => Some(
                                "b475447846f37cc426add800b995a7859c169c53c72b62de6338c0c40a5dacea",
                            ),
                            "getEpisodeOrChapter" => Some(
                                "5f77db47e5a2ce6680330bb61317aa09e17ddd1d189513cb218e7cd3e43288be",
                            ),
                            _ => None,
                        })
                        .ok_or_else(|| {
                            format!(
                                "Spotify operation {operation} is unavailable; refresh the session"
                            )
                        })?;
                    let body = serde_json::to_vec(&json!({
                        "operationName": operation, "variables": command["variables"],
                        "extensions": {"persistedQuery": {"version": 1, "sha256Hash": hash}}
                    }))
                    .map_err(|_| "Invalid Spotify query")?;
                    request(
                        &s,
                        "POST",
                        "https://api-partner.spotify.com/pathfinder/v2/query",
                        &body,
                        "application/json",
                    )
                    .map(|(_, value)| value)
                }
                Some("browse") => s
                    .get("browse")
                    .filter(|v| !v.is_null())
                    .cloned()
                    .ok_or_else(|| {
                        "Spotify browse content is unavailable; refresh the session".into()
                    }),
                _ => Err("Unsupported Spotify request".into()),
            }
        }
        #[cfg(test)]
        mod tests {
            use super::*;
            #[test]
            fn webview_timestamps_accept_integral_floats_but_not_invalid_expiries() {
                let mut value = json!({"expiry":1793151028854.0});
                normalize_expiry(&mut value);
                assert_eq!(value["expiry"].as_u64(), Some(1793151028854));
                for expiry in [
                    json!(-1.0),
                    json!(123.5),
                    json!("1793151028854"),
                    json!(1e30),
                ] {
                    let mut value = json!({"expiry":expiry});
                    normalize_expiry(&mut value);
                    assert!(value["expiry"].as_u64().is_none());
                }
            }
            #[test]
            fn expired_and_incomplete_captures_require_explicit_refresh() {
                let mut s = json!({"token":"access","client":"client","version":"version","expiry":100000,"hashes":{}});
                assert!(valid(&s, 100));
                assert!(!valid(&s, 90000));
                s["client"] = json!("bad\r\nheader");
                assert!(!valid(&s, 100));
                assert!(!valid(&Value::Null, 0));
            }
        }
    }
    pub(crate) use native::save_session;

    pub(crate) struct Request {
        pub command: Value,
        pub reply: mpsc::SyncSender<Result<Value, String>>,
    }
    pub(crate) struct Transport {
        pub cookie: String,
        pub requests: mpsc::Receiver<Request>,
        pub cancel: Arc<AtomicBool>,
    }
    struct Worker {
        cookie: String,
        tx: mpsc::SyncSender<Request>,
        cancel: Arc<AtomicBool>,
    }
    impl Drop for Worker {
        fn drop(&mut self) {
            self.cancel.store(true, Ordering::Relaxed);
        }
    }
    static WORKER: Mutex<Option<Worker>> = Mutex::new(None);

    fn cookie() -> Result<String, String> {
        let value = crate::session::provider_saved("spotify-web")
            .ok()
            .and_then(|s| saved_cookie(&s))
            .or_else(|| {
                crate::session::provider_saved("spotify")
                    .ok()
                    .and_then(|s| serde_json::from_str::<super::Credentials>(&s).ok())
                    .and_then(|c| saved_cookie(&c.sp_dc))
            })
            .ok_or("Sign in to Spotify with the Web Player first")?;
        if !crate::spotify_webview::valid_spotify_cookie("sp_dc", ".spotify.com", true, &value) {
            return Err("Spotify web session is invalid; sign in again".into());
        }
        Ok(value)
    }
    fn saved_cookie(saved: &str) -> Option<String> {
        let decoded = serde_json::from_str::<Value>(saved).ok();
        let value = decoded
            .as_ref()
            .and_then(|v| v.as_str().or_else(|| v["sp_dc"].as_str()))
            .unwrap_or(saved);
        crate::spotify_webview::valid_spotify_cookie("sp_dc", ".spotify.com", true, value)
            .then(|| value.to_owned())
    }
    /// Presence only; the next request verifies the grant with Spotify.
    pub(crate) fn has_session() -> bool {
        cookie().is_ok()
    }
    pub(crate) fn reset() {
        native::reset();
        if let Ok(mut worker) = WORKER.lock() {
            *worker = None;
        }
    }
    static REFRESHING: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

    pub(crate) fn refreshing() -> bool {
        REFRESHING.load(Ordering::Relaxed) != 0
    }

    pub(crate) struct RefreshGuard;
    impl Drop for RefreshGuard {
        fn drop(&mut self) {
            REFRESHING.fetch_sub(1, Ordering::Relaxed);
        }
    }
    pub(crate) fn begin_refresh() -> RefreshGuard {
        REFRESHING.fetch_add(1, Ordering::Relaxed);
        RefreshGuard
    }
    pub(crate) fn refresh() -> Result<(), String> {
        let _guard = begin_refresh();
        let result = capture_session();
        if let Ok(mut worker) = WORKER.lock() {
            *worker = None;
        }
        save_session(result?)?;
        Ok(())
    }
    pub(crate) fn account() -> Result<Value, String> {
        let value = query("profileAttributes", json!({}))?;
        let profile = value
            .pointer("/data/me/profile")
            .filter(|p| p.is_object())
            .ok_or("Spotify web session has no authenticated profile; sign in again")?;
        let username = profile["username"]
            .as_str()
            .filter(|v| !v.is_empty())
            .ok_or("Spotify web session has no account identity; sign in again")?;
        Ok(
            json!({"id":username,"userId":username,"uri":profile["uri"],"display_name":profile["name"],"name":profile["name"],"images":profile["avatar"]["sources"],"provider":"spotify"}),
        )
    }
    fn capture_session() -> Result<Value, String> {
        let command = json!({"kind":"export"});
        let saved = cookie()?;
        let mut worker = WORKER
            .lock()
            .map_err(|_| "Spotify web transport unavailable")?;
        if worker
            .as_ref()
            .is_none_or(|w| w.cookie != saved || w.cancel.load(Ordering::Relaxed))
        {
            *worker = None;
            let (tx, requests) = mpsc::sync_channel(16);
            let cancel = Arc::new(AtomicBool::new(false));
            let transport = Transport {
                cookie: saved.clone(),
                requests,
                cancel: cancel.clone(),
            };
            let stopped = cancel.clone();
            std::thread::Builder::new()
                .name("spotify-web-session".into())
                .spawn(move || {
                    let result = crate::spotify_webview::transport(transport);
                    // Never log browser results, account payloads, or credentials.
                    let _ = result;
                    stopped.store(true, Ordering::Relaxed);
                })
                .map_err(|_| "Could not start Spotify WebView2 session")?;
            *worker = Some(Worker {
                cookie: saved,
                tx,
                cancel,
            });
        }
        let (reply, rx) = mpsc::sync_channel(1);
        let tx = worker.as_ref().unwrap().tx.clone();
        let cancel = worker.as_ref().unwrap().cancel.clone();
        drop(worker);
        tx.try_send(Request { command, reply })
            .map_err(|_| "Spotify web session is busy or closed")?;
        let started = std::time::Instant::now();
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err("Spotify web request cancelled or session closed".into());
            }
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(result) => return result,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(
                        "Spotify WebView2 session unavailable; update the Runtime or sign in again"
                            .into(),
                    )
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
            if started.elapsed() >= Duration::from_secs(95) {
                cancel.store(true, Ordering::Relaxed);
                return Err("Spotify web session timed out; retry or sign in again".into());
            }
        }
    }
    // Only replay read-only operations after refreshing an expired web session.
    // A single refresh is shared by concurrent catalog requests.
    static AUTO_REFRESH: Mutex<()> = Mutex::new(());
    fn execute(command: Value) -> Result<Value, String> {
        let result = native::execute(command.clone());
        let read_only = (matches!(command["kind"].as_str(), Some("query" | "browse"))
            && command["mutation"] != true)
            || (command["kind"] == "rest" && command["method"] == "GET");
        if !read_only
            || !result
                .as_ref()
                .is_err_and(|e| e == native::REFRESH_REQUIRED)
        {
            return result;
        }
        let _guard = AUTO_REFRESH
            .lock()
            .map_err(|_| "Spotify session refresh unavailable")?;
        let retry = native::execute(command.clone());
        if !retry.as_ref().is_err_and(|e| e == native::REFRESH_REQUIRED) {
            return retry;
        }
        refresh().map_err(|_| "Spotify session could not be refreshed. Sign in again.")?;
        native::execute(command)
    }
    pub(crate) fn query(operation: &str, variables: Value) -> Result<Value, String> {
        query_inner(operation, variables, false)
    }
    pub(crate) fn mutation(operation: &str, variables: Value) -> Result<Value, String> {
        query_inner(operation, variables, true)
    }
    fn query_inner(operation: &str, variables: Value, mutation: bool) -> Result<Value, String> {
        if operation.is_empty()
            || operation.len() > 100
            || !operation
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_')
        {
            return Err("Invalid Spotify web operation".into());
        }
        if variables.to_string().len() > 512 * 1024 {
            return Err("Spotify request is too large".into());
        }
        execute(
            json!({"kind":"query", "operation":operation, "variables":variables, "mutation":mutation}),
        )
    }
    pub(crate) fn browse() -> Result<Value, String> {
        execute(json!({"kind":"browse"}))
    }
    pub(crate) fn browse_operations() -> Result<Vec<String>, String> {
        let result = native::browse_operations();
        if !result
            .as_ref()
            .is_err_and(|e| e == native::REFRESH_REQUIRED)
        {
            return result;
        }
        let _guard = AUTO_REFRESH
            .lock()
            .map_err(|_| "Spotify session refresh unavailable")?;
        let retry = native::browse_operations();
        if !retry.as_ref().is_err_and(|e| e == native::REFRESH_REQUIRED) {
            return retry;
        }
        refresh().map_err(|_| "Spotify session could not be refreshed. Sign in again.")?;
        native::browse_operations()
    }
    pub(super) fn directory_cache_scope() -> Result<u64, String> {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        cookie()?.hash(&mut hasher);
        Ok(hasher.finish())
    }
    // Device commands use native HTTP without starting a browser worker.
    pub(crate) fn device_command(command: Value) -> Result<Value, String> {
        execute(json!({"kind":"devices", "command":command}))
    }
    fn approved_url(address: &str) -> Result<Url, String> {
        let url = Url::parse(address).map_err(|_| "Invalid Spotify web URL")?;
        let approved = match url.host_str() {
            Some("api.spotify.com") => {
                url.path().starts_with("/v1/")
                    && (!url.path().starts_with("/v1/me/player")
                        || url.path() == "/v1/me/player/recently-played")
            }
            Some("spclient.wg.spotify.com") => [
                "/metadata/",
                "/user-profile-view/",
                "/recently-played/",
                "/inspiredby-mix/",
                "/connect-state/",
                "/player/",
                "/track-playback/",
                "/collection/",
                "/playlist/",
                "/color-lyrics/",
            ]
            .iter()
            .any(|p| url.path().starts_with(p)),
            _ => false,
        };
        if !approved
            || url.scheme() != "https"
            || url.port_or_known_default() != Some(443)
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
            || address.len() > 8192
        {
            return Err("Spotify web URL is outside the approved host/path allowlist".into());
        }
        Ok(url)
    }
    pub(crate) fn rest_get(url: &str) -> Result<Value, String> {
        rest_request("GET", url, None)
    }
    pub(crate) fn rest_request(
        method: &str,
        url: &str,
        body: Option<&Value>,
    ) -> Result<Value, String> {
        approved_url(url)?;
        if !matches!(method, "GET" | "POST" | "PUT" | "DELETE")
            || (method == "GET" && body.is_some())
        {
            return Err("Invalid Spotify web request method/body".into());
        }
        if body.is_some_and(|v| v.to_string().len() > 512 * 1024) {
            return Err("Spotify request is too large".into());
        }
        execute(json!({"kind":"rest", "method":method,"url":url,"body":body}))
    }
    pub(crate) fn rest_upload(url: &str, body: &[u8], content_type: &str) -> Result<Value, String> {
        let parsed = approved_url(url)?;
        if content_type != "image/jpeg"
            || body.len() > 256 * 1024
            || body.len() < 3
            || !body.starts_with(&[0xff, 0xd8, 0xff])
            || parsed.host_str() != Some("api.spotify.com")
            || !parsed.path().starts_with("/v1/playlists/")
            || !parsed.path().ends_with("/images")
        {
            return Err("Spotify playlist cover must be a JPEG of at most 256 KiB".into());
        }
        execute(
            json!({"kind":"upload", "method":"PUT", "url":url,"body":STANDARD.encode(body),"contentType":content_type}),
        )
    }

    // Installed before Spotify's scripts. Tokens remain inside the private browser;
    // native callers receive only the requested API result. The original fetch is
    // retained so our replay does not recursively intercept itself.
    pub(crate) const BOOTSTRAP: &str = r#"
(() => {
  if (location.origin !== 'https://open.spotify.com' || window !== window.top) return;
  const nativeFetch = window.fetch.bind(window);
const s = window.__deeznatsSpotifyWeb = {token:'', client:'', version:'', expiry:0, anonymous:true, hashes:{}, scanned:new Set(), cooldowns:{}, browse:null, result:null};
  const authHost = u => ['api-partner.spotify.com','api.spotify.com','spclient.wg.spotify.com'].includes(u.hostname) && u.protocol === 'https:' && !u.port;
  // Protocol verified in the current Web Player ConnectAPIClient. Observe only
  // browser-owned registrations/dealer traffic; never create another session.
  s.connect={sender:'',cluster:null};
  const deviceId=x=>typeof x==='string' && /^[A-Za-z0-9_%:-]{1,256}$/.test(x);
  function cluster(v) {
    const c=v?.cluster || v;
    if(!c || typeof c.devices!=='object' || !c.devices || Array.isArray(c.devices))return;
    const old=Number(s.connect.cluster?.timestamp), next=Number(c.timestamp);
    if(Number.isFinite(old) && Number.isFinite(next) && next<old)return;
    s.connect.cluster=c;
  }
  const Socket=window.WebSocket;
  window.WebSocket=new Proxy(Socket,{construct(target,args){
    const socket=new target(...args);
    let u;try{u=new URL(args[0]);}catch(_){return socket;}
    if(u.protocol==='wss:' && /(^|\.)dealer\.spotify\.com$/.test(u.hostname))socket.addEventListener('message',event=>{
      if(typeof event.data!=='string' || event.data.length>4*1024*1024)return;
      try {const v=JSON.parse(event.data);if(/\/connect-state\/v1\/cluster$/.test(v.uri||'')) {
              for(let p of v.payloads||[]) {if(typeof p==='string')p=JSON.parse(p);cluster(p?.cluster||p);}
      }}catch(_){}
    });
    return socket;
  }});
  function remember(body) {
    try { if (typeof body === 'string') body=JSON.parse(body);
      const h=body?.extensions?.persistedQuery?.sha256Hash;
      if (/^[a-f0-9]{64}$/.test(h) && /^[A-Za-z0-9_]{1,100}$/.test(body.operationName)) s.hashes[body.operationName]=h;
    } catch (_) {}
  }
  window.fetch = function(input, init) {
    let u; try { u=new URL(typeof input==='string' || input instanceof URL ? input : input.url,location.href); } catch (_) { return nativeFetch(input,init); }
    if (authHost(u)) {
      const headers=new Headers(input instanceof Request ? input.headers : undefined);
      new Headers(init?.headers).forEach((v,k)=>headers.set(k,v));
      const token=headers.get('authorization'); if (token?.startsWith('Bearer ')) s.token=token.slice(7);
      s.client=headers.get('client-token') || s.client;
      s.version=headers.get('spotify-app-version') || s.version;
      if (u.hostname==='api-partner.spotify.com') {
        if (init?.body) remember(init.body);
        if (input instanceof Request && !init?.body) input.clone().text().then(remember).catch(()=>{});
        if (u.searchParams.has('operationName')) remember({operationName:u.searchParams.get('operationName'),extensions:JSON.parse(u.searchParams.get('extensions')||'{}')});
      }
    }
    // Clone registration requests before nativeFetch consumes their body.
    const registrationBody=u.hostname==='spclient.wg.spotify.com' && u.pathname==='/track-playback/v1/devices' && input instanceof Request ? input.clone().text() : null;
    const response=nativeFetch(input,init);
    if(u.hostname==='spclient.wg.spotify.com') {
      // The sender is the local descriptor, not the hobs_ observer ID (which
      // Spotify truncates). Capture it from actual track-player registration.
      if(u.pathname==='/track-playback/v1/devices') {
        const capture=body=>{try{const v=JSON.parse(body);const id=v?.device?.device_info?.device_id||v?.device?.device_id;if(deviceId(id))s.connect.sender=id;}catch(_){}};
        if(typeof init?.body==='string')capture(init.body);
        else if(registrationBody)registrationBody.then(capture).catch(()=>{});
      }
      const registration=u.pathname.match(/^\/connect-state\/v1\/devices\/([^/]+)$/);
      const method=(init?.method||(input instanceof Request?input.method:'GET')).toUpperCase();
      // hobs_ registrations prove only the observer ID; the source truncates it.
      if(registration && method==='PUT' && deviceId(registration[1]) && !registration[1].startsWith('hobs_'))s.connect.sender=registration[1];
      const from=u.pathname.match(/^\/connect-state\/v1\/(?:connect\/[^/]+|player\/command)\/from\/([^/]+)\/to\//);
      if(from && deviceId(from[1]))s.connect.sender=from[1];
      if(/^\/connect-state\/v1\/(cluster|devices\/[^/]+)$/.test(u.pathname))response.then(async r=>{
        if(r.ok && r.status!==204){const v=JSON.parse(await limitedText(r.clone(),4*1024*1024));cluster(v?.cluster||v);}
      }).catch(()=>{});
    }
if(u.hostname==='api-partner.spotify.com') response.then(r=>r.clone().json()).then(v=>{
 if((v?.data?.browse || v?.data?.browseStart) && JSON.stringify(v).length<=4*1024*1024)s.browse=v;
}).catch(()=>{});
    if ((u.origin===location.origin && u.pathname==='/api/token') || u.hostname==='clienttoken.spotify.com') {
      response.then(r=>r.clone().json()).then(v=> {
        if(u.hostname==='clienttoken.spotify.com') s.client=v?.granted_token?.token || s.client;
        else if(v.accessToken) { s.token=v.accessToken; s.expiry=v.accessTokenExpirationTimestampMs||0; s.anonymous=v.isAnonymous !== false; }
      }).catch(()=>{});
    }
    return response;
  };
  const allowedAsset = x => { try { const u=new URL(x,location.href); return u.protocol==='https:' && !u.port && ['open.spotify.com','open.spotifycdn.com'].includes(u.hostname) && u.pathname.endsWith('.js'); } catch(_){return false;} };
  let runtime;
  function chunkURLs() {
    const out=[...document.scripts].map(x=>x.src).concat(performance.getEntriesByType('resource').map(x=>x.name)).filter(allowedAsset);
    if(!runtime) for(const key of Object.keys(window).filter(k=>k.startsWith('webpackChunk') || k==='rspackChunk')) {
      const chunks=window[key]; if(Array.isArray(chunks)) try { chunks.push([['deeznats_'+Date.now()],{},r=>{runtime=r;}]); } catch(_) {}
      if(runtime) break;
    }
    if(runtime?.u) {
      const ids=new Set([...runtime.u.toString().matchAll(/\b(\d+):/g)].map(m=>Number(m[1])));
      for(const id of ids) try { const u=new URL(runtime.p+runtime.u(id),location.href).href; if(allowedAsset(u))out.push(u); } catch(_){}
    }
    return [...new Set(out)].sort((a,b)=>Number(/vendor|locales|moment/.test(a))-Number(/vendor|locales|moment/.test(b)));
  }
  function scan(text) {
    for(const m of text.matchAll(/["']([A-Za-z][A-Za-z0-9_]{0,99})["']\s*,\s*["'](?:query|mutation)["']\s*,\s*["']([a-f0-9]{64})["']/g)) s.hashes[m[1]]=m[2];
    for(const m of text.matchAll(/operationName\s*:\s*["']([A-Za-z][A-Za-z0-9_]{0,99})["'][\s\S]{0,300}?sha256Hash\s*:\s*["']([a-f0-9]{64})["']/g))s.hashes[m[1]]=m[2];
  }
  async function limitedText(r,max) {
    if(!r.ok || Number(r.headers.get('content-length'))>max) throw Error('Spotify web response exceeded its limit or failed');
    const reader=r.body.getReader(); let size=0, parts=[];
    try { for(;;) { const {value,done}=await reader.read(); if(done)break; size+=value.byteLength; if(size>max)throw Error('Spotify web response exceeded its limit'); parts.push(value); } }
    finally { await reader.cancel().catch(()=>{}); }
    const bytes=new Uint8Array(size); let at=0; for(const p of parts){bytes.set(p,at);at+=p.length;} return new TextDecoder().decode(bytes);
  }
  async function discover(op,force=false) {
    if(force){ delete s.hashes[op]; s.scanned.clear(); }
    const deadline=Date.now()+25000; let count=0;
    // Loaded source first, then the active webpack runtime's current lazy chunks.
    const urls=chunkURLs();
    for(let at=0; at<urls.length && count<96 && Date.now()<deadline && !s.hashes[op];at+=4) {
      await Promise.all(urls.slice(at,at+4).map(async url=>{
        if(s.scanned.has(url))return; s.scanned.add(url);count++;
        try { const r=await nativeFetch(url,{credentials:'omit',redirect:'error',signal:AbortSignal.timeout(5000)}); scan(await limitedText(r,6*1024*1024)); } catch(_){}
      }));
    }
    return s.hashes[op];
  }
  s.ready=()=>!!s.token && !!s.client && !!s.version && s.anonymous===false && (!s.expiry || Date.now()+30000<s.expiry);
  s.run=async cmd=> {
    s.result=null;
if(cmd.kind==='browse') {
 const deadline=Date.now()+20000;
 while(!s.browse && Date.now()<deadline)await new Promise(r=>setTimeout(r,100));
 if(!s.browse)throw Error('Spotify search page did not provide browse categories; retry after refreshing');
 s.result={ok:true,value:s.browse};return;
}
    try {
      if(!s.ready()) { s.result={refresh:true};return; }
      if(cmd.kind==='export') {
        await discover('__deeznats_capture_all__');
const browseDeadline=Date.now()+20000;
while(!s.browse && Date.now()<browseDeadline)await new Promise(r=>setTimeout(r,100));
// Header-only captures have no server expiry. Cache them for at most five
// minutes; native requests still validate authorization and invalidate on 401.
s.result={ok:true,value:{token:s.token,client:s.client,version:s.version,expiry:s.expiry||Date.now()+300000,
hashes:s.hashes,browse:s.browse,userAgent:navigator.userAgent,sender:s.connect.sender}};
        return;
      }
      let url=cmd.url, method=cmd.method, body, mutation=false;
      if(cmd.kind==='devices') {
        const c=cmd.command;
        // Always fetch the current account's cluster. Cached dealer state is
        // advisory only and never used to invent an active remote device.
        const request=async (path,method='GET',payload)=>{
          const origin='https://spclient.wg.spotify.com';
          if(s.cooldowns[origin]>Date.now())throw Error('Spotify Connect is rate limited; retry later');
          const r=await nativeFetch(origin+'/connect-state/v1/'+path,{method,headers:{Authorization:'Bearer '+s.token,'Client-Token':s.client,'Spotify-App-Version':s.version,'App-Platform':'WebPlayer','Content-Type':'application/json'},body:payload===undefined?undefined:JSON.stringify(payload),credentials:'omit',redirect:'error',signal:AbortSignal.timeout(15000)});
          if(r.status===429)s.cooldowns[origin]=Date.now()+Math.max(1,Math.min(3600,Number(r.headers.get('retry-after'))||5))*1000;
          if(r.status===401)throw Object.assign(Error('Spotify Connect authorization expired'),{refresh:true});
          if(!r.ok)throw Error('Spotify Connect request failed (HTTP '+r.status+')');
          if(!r.body || r.status===204 || r.headers.get('content-length')==='0')return {status:r.status,value:null};
          const raw=await limitedText(r,4*1024*1024);return {status:r.status,value:raw.trim()?JSON.parse(raw):null};
        };
        const read=async()=>{const r=await request('cluster');const v=r.status===204 || r.value===null?{devices:{}}:(r.value?.cluster||r.value);
          // Proto JSON omits the empty device map for a genuine idle cluster.
          if(v && v.devices===undefined && !v.active_device_id && v.timestamp!==undefined && Number.isFinite(Number(v.timestamp)) && v.player_state && typeof v.player_state==='object' && !Array.isArray(v.player_state) && typeof v.need_full_player_state==='boolean')v.devices={};
          if(!v?.devices || typeof v.devices!=='object' || Array.isArray(v.devices))throw Error('Spotify Connect returned malformed device state');
          cluster(v);return v;};
        const state=await read();
        if(c.action==='state'){s.result={ok:true,value:{...state,deeznats_web_device_id:s.connect.sender}};return;}
        const sender=s.connect.sender;
        if(!deviceId(sender))throw Error('Spotify Web Player has not supplied its Connect sender; open Web Player and retry');
        const id=c.action==='transfer'?c.device:state.active_device_id;
        if(!deviceId(id) || !state.devices[id])throw Error('Spotify Connect target is unavailable');
        if(id===sender)throw Error('The hidden Spotify web session is for catalog access. Pair DeezNats in Spotify for local audio, or select another Spotify Connect device.');
        if(c.expected?.device && c.expected.device!==id)throw Error('Spotify active device changed; refresh before controlling playback');
        if(c.expected?.uri && c.expected.uri!==state.player_state?.track?.uri)throw Error('Spotify current item changed; refresh before editing queue');
        const d=state.devices[id], cap=d.capabilities||{};
        if(cap.is_controllable!==true)throw Error('Spotify Connect target does not allow remote control');
        let path, payload, verb='POST';
        if(c.action==='transfer'){
          if(d.can_play!==true || cap.hidden===true || d.is_restricted===true)throw Error('Spotify Connect target cannot receive playback');
          if(state.player_state?.restrictions?.disallow_transferring_playback_reasons?.length)throw Error('Spotify Connect transfer is restricted');
          path='connect/transfer';payload={transfer_options:{restore_paused:c.playing?'resume':'pause'},command_id:globalThis.crypto.randomUUID().replaceAll('-','')};
        }else if(c.action==='volume'){
          if(cap.disable_volume===true || !Number.isInteger(c.value) || c.value<0 || c.value>100)throw Error('Spotify Connect volume is unavailable');
          verb='PUT';path='connect/volume';payload={volume:Math.round(c.value*65535/100)};
        }else{
          const endpoints={pause:'pause',resume:'resume',next:'skip_next',previous:'skip_prev',seek:'seek_to',play:'play',queue:'set_queue',shuffle:'set_options',repeat:'set_options'};
          const endpoint=endpoints[c.action];if(!endpoint)throw Error('Unsupported Spotify Connect control');
          const restrictions=state.player_state?.restrictions||{};
          const keys={pause:'pausing',resume:'resuming',next:'skipping_next',previous:'skipping_prev',seek:'seeking',play:'interrupting_playback',queue:'set_queue',shuffle:'toggling_shuffle'};
          if(restrictions['disallow_'+keys[c.action]+'_reasons']?.length)throw Error('Spotify Connect control is restricted');
          payload={command:{endpoint}};path='player/command';
          if(c.action==='seek') {if(!Number.isSafeInteger(c.value)||c.value<0)throw Error('Invalid Spotify Connect seek');payload.command.value=c.value;}
          if(c.action==='play') {
            if(!c.descriptor?.context || !c.descriptor?.options)throw Error('Invalid Spotify Connect play descriptor');
            Object.assign(payload.command,c.descriptor,{endpoint});
          }
          if(c.action==='queue') {
            const p=state.player_state;
            if(p?.queue_revision==null || !Array.isArray(c.next_tracks) || (p.prev_tracks!=null && !Array.isArray(p.prev_tracks)))throw Error('Spotify has not supplied a replaceable queue revision');
            if(c.expected?.revision!=null && c.expected.revision!==p.queue_revision)throw Error('Spotify queue changed; refresh before editing');
            Object.assign(payload.command,{next_tracks:c.next_tracks,prev_tracks:p.prev_tracks||[],queue_revision:p.queue_revision});
          }
          if(c.action==='shuffle') {
            if(c.value!==0 && c.value!==1)throw Error('Invalid Spotify shuffle option');
            payload.command.shuffling_context=!!c.value;
          }
          if(c.action==='repeat') {
            if(![0,1,2].includes(c.value))throw Error('Invalid Spotify repeat option');
            if(restrictions.disallow_toggling_repeat_context_reasons?.length || restrictions.disallow_toggling_repeat_track_reasons?.length)throw Error('Spotify repeat control is restricted');
            Object.assign(payload.command,{repeating_context:c.value===1,repeating_track:c.value===2});
          }
          if(globalThis.crypto?.randomUUID)payload.command.logging_params={command_id:globalThis.crypto.randomUUID()};
        }
        const r=await request(path+'/from/'+encodeURIComponent(sender)+'/to/'+encodeURIComponent(id),verb,payload);
        if(!((r.status===200 && r.value?.ack_id) || (r.status===202 && !r.value?.ack_id)))throw Error('Spotify Connect did not acknowledge the command');
        if(c.action==='transfer') {
          // An acknowledgement alone is not evidence of a successful transfer.
          for(let i=0;i<5;i++) {await new Promise(r=>setTimeout(r,300));const v=await read();
            if(v.active_device_id===id && typeof v.player_state?.is_paused==='boolean' && v.player_state.is_paused===!c.playing){s.result={ok:true,value:null};return;}}
          throw Error('Spotify Connect accepted transfer but has not confirmed the target state; refresh devices');
        }
        s.result={ok:true,value:null};return;
      }
      if(cmd.kind==='query') {
        const hash=await discover(cmd.operation);
        if(!hash) throw Error('Spotify Web Player operation is unavailable in the current deployment: '+cmd.operation);
        url='https://api-partner.spotify.com/pathfinder/v2/query'; method='POST';
        body=JSON.stringify({operationName:cmd.operation,variables:cmd.variables,extensions:{persistedQuery:{version:1,sha256Hash:hash}}});
        mutation=cmd.mutation===true || /^(add|remove|create|update|edit|delete|change|set|follow|unfollow)/i.test(cmd.operation);
      } else body=cmd.kind==='upload' ? cmd.body : cmd.body==null ? undefined : JSON.stringify(cmd.body);
      const u=new URL(url); if(!authHost(u)||u.username||u.password||u.hash)throw Error('Spotify web URL rejected');
if(s.cooldowns[u.origin]>Date.now())throw Error('Spotify web rate limited; retry after '+Math.ceil((s.cooldowns[u.origin]-Date.now())/1000)+' seconds');
      for(let attempt=0;attempt<2;attempt++) {
        const headers={'Authorization':'Bearer '+s.token,'Client-Token':s.client,'Spotify-App-Version':s.version,'App-Platform':'WebPlayer','Accept':'application/json'};
        if(body!==undefined)headers['Content-Type']=cmd.contentType||'application/json;charset=UTF-8';
        const r=await nativeFetch(url,{method,headers,body,credentials:'omit',redirect:'error',signal:AbortSignal.timeout(15000)});
        if(r.status===401 || (r.status===400 && r.headers.get('client-token-error')==='INVALID_CLIENTTOKEN')) { s.result={refresh:true};return; }
if(r.status===429) { const wait=Math.max(1,Math.min(3600,Number(r.headers.get('retry-after'))||5)); s.cooldowns[u.origin]=Date.now()+wait*1000; throw Error('Spotify web rate limited; retry after '+wait+' seconds'); }
        if(r.status>=500 && attempt===0 && (method==='GET'||(cmd.kind==='query'&&!mutation))) { await new Promise(r=>setTimeout(r,400));continue; }
        if(!r.ok)throw Error(r.status===403 ? 'Spotify web request is forbidden for this account (HTTP 403)' : 'Spotify web request failed (HTTP '+r.status+')');
        if(r.status===204 || r.headers.get('content-length')==='0') {s.result={ok:true,value:null};return;}
        const raw=await limitedText(r,4*1024*1024); const value=raw.trim()?JSON.parse(raw):null;
        if(value?.errors?.length) {
          if(attempt===0 && value.errors.every(e=>/persistedquerynotfound|persistedquerynotsupported|unknown operation/i.test((e.message||'').replace(/\s/g,'')))) {
            const hash=await discover(cmd.operation,true);if(!hash)throw Error('Spotify web operation changed; retry after signing in again');
            const payload=JSON.parse(body);payload.extensions.persistedQuery.sha256Hash=hash;body=JSON.stringify(payload);continue;
          }
          if(value.errors.some(e=>/unauthenticated|token expired/i.test(e.message||''))){s.result={refresh:true};return;}
          throw Error('Spotify Web Player rejected operation '+cmd.operation);
        }
        s.result={ok:true,value};return;
      }
      throw Error('Spotify web request could not be completed');
    } catch(e) { s.result=e?.refresh ? {refresh:true} : {ok:false,error: e?.name==='TimeoutError' ? 'Spotify web request timed out' : String(e.message||'Spotify web request failed')}; }
  };
})();
"#;

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn credentials_only_go_to_approved_https_services() {
            for u in [
                "http://api.spotify.com/v1/me",
                "https://api.spotify.com.evil/v1/me",
                "https://user@api.spotify.com/v1/me",
                "https://api.spotify.com:444/v1/me",
                "https://spclient.wg.spotify.com/redirect/",
                "https://api.spotify.com/v1/me#secret",
                "https://open.spotify.com/v1/me",
            ] {
                assert!(approved_url(u).is_err());
            }
            for u in ["https://api.spotify.com/v1/me", "https://api.spotify.com/v1/me/player/recently-played", "https://spclient.wg.spotify.com/inspiredby-mix/v2/seed_to_playlist/spotify:track:abc?response-format=json", "https://spclient.wg.spotify.com/recently-played/v3/user/u/recently-played?format=json"] { assert!(approved_url(u).is_ok()); }
            for path in ["", "/play", "/pause", "/queue", "/devices"] {
                assert!(
                    approved_url(&format!("https://api.spotify.com/v1/me/player{path}")).is_err()
                );
            }
        }
    }
}
mod web_catalog {
    //! Web-player catalog adapter. Transport owns cookies, token refresh and persisted hashes.
    //! Shapes verified against the current Spotify desktop bundle and SpotAPI/Meld consumers.
    mod flow_queue {
        use super::*;
        use serde::{Deserialize, Serialize};
        use std::collections::VecDeque;

        pub(super) const PREFIX: &str = "spotify-flow-page:";
        #[derive(Serialize, Deserialize)]
        struct Source {
            uri: String,
            offset: u64,
        }
        #[derive(Serialize, Deserialize)]
        struct Queue {
            config: String,
            sources: VecDeque<Source>,
            pending: VecDeque<Value>,
            seen: HashSet<String>,
        }

        fn editorial(v: &Value) -> bool {
            v["type"] == "playlist"
                && v.pointer("/owner/uri").and_then(Value::as_str) == Some("spotify:user:spotify")
        }

        pub(super) fn start(config: &str) -> ResultValue {
            let selection = super::super::flow::Selection::parse(config)?;
            let mut sources = VecDeque::new();
            let mut seen = HashSet::new();
            for query in selection.queries() {
                // Spotify's editorial search results are the authority for the selected topic.
                // Do not substitute arbitrary user playlists or unrelated Home mixes.
                let value = search(&query)?;
                for item in collect(&value["playlists"])
                    .into_iter()
                    .filter(editorial)
                    .take(3)
                {
                    let uri = text(&item, &["uri"]).to_owned();
                    if seen.insert(uri.clone()) {
                        sources.push_back(Source { uri, offset: 0 });
                    }
                }
            }
            if selection.mood == "default" {
                for genre in &selection.genres {
                    if let Some(uri) = super::super::flow::genre_radio(genre) {
                        if seen.insert(uri.to_owned()) {
                            sources.push_back(Source {
                                uri: uri.to_owned(),
                                offset: 0,
                            });
                        }
                    }
                }
            }
            if sources.is_empty() {
                return Err("Spotify returned no editorial playlists for these Flow selections. Try another mood or genre.".into());
            }
            batch(
                Queue {
                    config: selection.key(),
                    sources,
                    pending: VecDeque::new(),
                    seen: HashSet::new(),
                },
                fetch,
            )
        }

        pub(super) fn next(cursor: &str) -> ResultValue {
            if cursor.len() > 256_000 {
                return Err("Flow cursor is too large. Refresh Flow.".into());
            }
            let queue: Queue =
                serde_json::from_str(cursor.strip_prefix(PREFIX).ok_or("Invalid Flow cursor")?)
                    .map_err(|_| "Invalid Flow cursor")?;
            super::super::flow::Selection::parse(&queue.config)?;
            if queue.sources.len() > 128 * 4
                || queue.pending.len() > 10
                || queue.seen.len() > 3000
                || queue
                    .sources
                    .iter()
                    .any(|s| !s.uri.starts_with("spotify:playlist:") || s.offset > 100_000)
            {
                return Err("Invalid Flow cursor".into());
            }
            batch(queue, fetch)
        }

        fn fetch(uri: &str, offset: u64) -> ResultValue {
            page(
                "fetchPlaylist",
                json!({"uri":uri,"offset":offset,"limit":10,"enableWatchFeedEntrypoint":false}),
                "/data/playlistV2/content",
            )
        }

        fn batch(mut queue: Queue, mut fetch: impl FnMut(&str, u64) -> ResultValue) -> ResultValue {
            let mut items = Vec::new();
            let mut reads = 0;
            loop {
                while items.len() < 10 {
                    let Some(mut track) = queue.pending.pop_front() else {
                        break;
                    };
                    let uri = text(&track, &["uri"]).to_owned();
                    if track["type"] == "track"
                        && !uri.is_empty()
                        && track["is_playable"] != false
                        && queue.seen.insert(uri)
                    {
                        track["spotify_station"] =
                            json!({"name":"DZFlowForConfigJSON","args":[queue.config]});
                        items.push(track);
                    }
                }
                if items.len() == 10
                    || queue.sources.is_empty()
                    || reads == 30
                    || queue.seen.len() >= 3000
                {
                    break;
                }
                let mut source = queue.sources.pop_front().unwrap();
                let value = fetch(&source.uri, source.offset)?;
                reads += 1;
                queue.pending.extend(collect(&value).into_iter().take(10));
                if let Some(next) = value["next"]
                    .as_str()
                    .and_then(|s| s.strip_prefix(CURSOR))
                    .and_then(|s| serde_json::from_str::<Value>(s).ok())
                    .and_then(|v| v["vars"]["offset"].as_u64())
                    .filter(|n| *n > source.offset && *n <= 100_000)
                {
                    source.offset = next;
                    queue.sources.push_back(source);
                }
            }
            let more =
                (!queue.sources.is_empty() || !queue.pending.is_empty()) && queue.seen.len() < 3000;
            let next = more.then(|| format!("{PREFIX}{}", serde_json::to_string(&queue).unwrap()));
            Ok(json!({"items":items,"next":next}))
        }

        #[cfg(test)]
        mod tests {
            use super::*;
            #[test]
            fn ten_tracks_then_more_without_duplicates_or_episodes() {
                let queue = Queue {
                    config: "default".into(),
                    sources: VecDeque::from([Source {
                        uri: "spotify:playlist:test".into(),
                        offset: 0,
                    }]),
                    pending: VecDeque::new(),
                    seen: HashSet::new(),
                };
                let fetch = |_: &str, offset: u64| -> ResultValue {
                    let start = if offset == 0 { 0 } else { 8 };
                    Ok(
                        json!({"items":(start..start+10).map(|n| json!({"type":"track","name":format!("Track {n}"),"uri":format!("spotify:track:{n}")})).collect::<Vec<_>>(),
                "next":if offset == 0 { Some(format!("{CURSOR}{}",json!({"vars":{"offset":10}}))) } else { None }}),
                    )
                };
                let first = batch(queue, fetch).unwrap();
                assert_eq!(first["items"].as_array().unwrap().len(), 10);
                let cursor = first["next"]
                    .as_str()
                    .unwrap()
                    .strip_prefix(PREFIX)
                    .unwrap();
                let second = batch(serde_json::from_str(cursor).unwrap(), fetch).unwrap();
                assert_eq!(second["items"].as_array().unwrap().len(), 8);
                assert!(second["next"].is_null());
                assert_eq!(second["items"][0]["uri"], "spotify:track:10");
            }
            #[test]
            fn editorial_owner_is_required() {
                assert!(editorial(
                    &json!({"type":"playlist","owner":{"uri":"spotify:user:spotify"}})
                ));
                assert!(!editorial(
                    &json!({"type":"playlist","name":"Jazz Radio","owner":{"uri":"spotify:user:someone"}})
                ));
            }
            #[test]
            fn skips_unplayable_tracks_and_episodes_and_rotates_sources() {
                let queue = Queue {
                    config: "default".into(),
                    sources: VecDeque::from([
                        Source {
                            uri: "spotify:playlist:one".into(),
                            offset: 0,
                        },
                        Source {
                            uri: "spotify:playlist:two".into(),
                            offset: 0,
                        },
                    ]),
                    pending: VecDeque::new(),
                    seen: HashSet::new(),
                };
                let value = batch(queue, |uri, _| {
            Ok(json!({"items": if uri.ends_with("one") {
                vec![json!({"uri":"spotify:track:blocked","name":"Blocked","is_playable":false}),
                    json!({"uri":"spotify:episode:talk","name":"Talk"})]
            } else {
                vec![json!({"uri":"spotify:track:good","name":"Good"})]
            }}))
        })
        .unwrap();
                assert_eq!(value["items"].as_array().unwrap().len(), 1);
                assert_eq!(value["items"][0]["uri"], "spotify:track:good");
                assert!(value["next"].is_null());
            }
        }
    }
    use super::web;
    use serde_json::{json, Value};
    use std::collections::HashSet;

    type ResultValue = Result<Value, String>;
    const CURSOR: &str = "spotify-web:";

    fn text<'a>(v: &'a Value, keys: &[&str]) -> &'a str {
        keys.iter().find_map(|k| v.get(*k)?.as_str()).unwrap_or("")
    }
    fn arg(args: &[Value], n: usize) -> Result<&str, String> {
        args.get(n)
            .and_then(Value::as_str)
            .ok_or_else(|| "Missing Spotify request argument".into())
    }
    fn playlist_details_request(name: &str, args: &[Value]) -> Result<(String, Value), String> {
        let playlist = uri(args, 0, &["playlist"])?;
        let title = arg(args, 1)?.trim();
        if title.is_empty() || title.len() > 300 {
            return Err("Enter a playlist name".into());
        }
        let mut body = json!({"playlistUri": playlist, "newName": title});
        if name == "DZUpdatePlaylistJSON" {
            body["newDescription"] = json!(arg(args, 2)?);
        }
        Ok(("editPlaylistAttributes".into(), body))
    }
    fn uri(args: &[Value], n: usize, kinds: &[&str]) -> Result<String, String> {
        let value = arg(args, n)?;
        if value.len() == 22 && value.bytes().all(|b| b.is_ascii_alphanumeric()) && kinds.len() == 1
        {
            return Ok(format!("spotify:{}:{}", kinds[0], value));
        }
        let parts: Vec<_> = value.split(':').collect();
        if parts.len() != 3
            || parts[0] != "spotify"
            || !kinds.contains(&parts[1])
            || parts[2].len() != 22
            || !parts[2].bytes().all(|b| b.is_ascii_alphanumeric())
        {
            return Err("Select a valid Spotify item of the correct type".into());
        }
        Ok(value.into())
    }
    fn number(v: &Value) -> Option<u64> {
        v.as_u64().or_else(|| v.as_str()?.parse().ok())
    }
    fn images(v: &Value) -> Value {
        fn gather(v: &Value, out: &mut Vec<Value>) {
            if let Some(url) = v["url"].as_str().filter(|s| s.starts_with("https://")) {
                out.push(json!({"url":url,"width":v["width"],"height":v["height"]}));
            } else if let Some(a) = v.as_array() {
                for v in a {
                    gather(v, out);
                }
            } else {
                for key in ["sources", "items"] {
                    if let Some(v) = v.get(key) {
                        gather(v, out);
                    }
                }
            }
        }
        let mut out = Vec::new();
        gather(v, &mut out);
        json!(out)
    }

    /// Unwrap only known entity wrappers; never manufacture an entity from an error response.
    pub(super) fn entity(v: &Value) -> Option<Value> {
        if !v.is_object() || text(v, &["__typename"]).contains("Error") {
            return None;
        }
        let own_uri = text(v, &["uri", "_uri"]);
        let own_name =
            !text(v, &["name", "title"]).is_empty() || v.pointer("/profile/name").is_some();
        if own_uri.is_empty() || !own_name {
            for key in [
                "content", "itemV2", "item", "track", "episode", "album", "data", "entity",
            ] {
                if let Some(child) = v.get(key).filter(|c| c.is_object()) {
                    let mut child = child.clone();
                    if text(&child, &["uri", "_uri"]).is_empty() && !own_uri.is_empty() {
                        child["uri"] = json!(own_uri);
                    }
                    if let Some(mut result) = entity(&child) {
                        if result["uri"].as_str().unwrap_or("").is_empty() {
                            result["uri"] = json!(text(v, &["_uri", "uri"]));
                        }
                        for key in ["uid", "addedAt", "added_at", "played_at", "timestampMs"] {
                            if let Some(value) = v.get(key) {
                                result[key] = value.clone();
                            }
                        }
                        return Some(result);
                    }
                }
            }
        }
        let uri = text(v, &["uri", "_uri"]);
        let typ = uri.split(':').nth(1)?;
        if !matches!(
            typ,
            "track"
                | "album"
                | "artist"
                | "playlist"
                | "show"
                | "episode"
                | "page"
                | "genre"
                | "section"
                | "category"
        ) {
            return None;
        }
        let name = [
            v.get("name"),
            v.get("title"),
            v.pointer("/profile/name"),
            v.pointer("/data/cardRepresentation/title/transformedLabel"),
            v.pointer("/cardRepresentation/title/transformedLabel"),
        ]
        .into_iter()
        .flatten()
        .find_map(Value::as_str)?;
        let kind = if matches!(typ, "page" | "genre" | "section") {
            "category"
        } else {
            typ
        };
        let mut out = json!({"uri":uri,"id":uri,"type":kind,"name":name,"provider":"spotify"});
        if kind == "track" {
            crate::spotify_diagnostics::record(
                "catalog.track",
                &format!(
                    "duration_ms={} durationMs={} duration_object={} duration_numeric={}",
                    v.get("duration_ms").is_some(),
                    v.get("durationMs").is_some(),
                    v.pointer("/duration/totalMilliseconds").is_some(),
                    v.get("duration").is_some_and(Value::is_number)
                ),
            );
        }
        for key in [
            "description",
            "publisher",
            "is_playable",
            "language",
            "is_local",
            "explicit",
            "format",
            "attributes",
            "uid",
            "added_at",
            "played_at",
            "track_number",
            "disc_number",
            "external_ids",
            "genres",
        ] {
            if let Some(value) = v.get(key) {
                out[key] = value.clone();
            }
        }
        if let Some(publisher) = v.get("publisher") {
            if let Some(name) = publisher
                .as_str()
                .or_else(|| publisher.get("name").and_then(Value::as_str))
            {
                out["publisher"] = json!(name);
            }
        }
        if out.get("description").is_none() {
            if let Some(desc) = v.get("htmlDescription") {
                out["description"] = desc.clone();
            }
        }
        let art = [
            v.get("images"),
            v.get("coverArt"),
            v.get("artwork"),
            v.pointer("/visuals/avatarImage"),
            v.pointer("/data/cardRepresentation/artwork"),
            v.pointer("/cardRepresentation/artwork"),
            v.pointer("/data/cardRepresentation/backgroundImage"),
            v.pointer("/cardRepresentation/backgroundImage"),
        ]
        .into_iter()
        .flatten()
        .map(images)
        .find(|v| v.as_array().is_some_and(|a| !a.is_empty()));
        if let Some(art) = art {
            out["images"] = art;
        }
        if let Some(ms) = v
            .get("duration_ms")
            .and_then(number)
            .or_else(|| v.get("durationMs").and_then(number))
            .or_else(|| v.pointer("/duration/totalMilliseconds").and_then(number))
        {
            out["duration_ms"] = json!(ms);
        }
        if let Some(artists) = v.get("artists") {
            out["artists"] = json!(collect(artists));
        }
        if let Some(album) = v.get("albumOfTrack").or_else(|| v.get("album")) {
            if let Some(album) = entity(album) {
                out["album"] = album;
            } else {
                // Artist top-track albums can omit their name while retaining cover art.
                out["album"] = json!({"uri": text(album, &["uri", "_uri"]), "images": images(&album["coverArt"])});
            }
        }
        if let Some(explicit) = v.pointer("/contentRating/label").and_then(Value::as_str) {
            if matches!(explicit, "EXPLICIT" | "NONE" | "CLEAN") {
                out["explicit"] = json!(explicit == "EXPLICIT");
            }
        }
        if let Some(track_number) = v.get("trackNumber").and_then(number) {
            out["track_number"] = json!(track_number);
        }
        if let Some(show) = v.pointer("/podcastV2/data").or_else(|| v.get("show")) {
            if let Some(show) = entity(show) {
                out["show"] = show;
            }
        }
        if let Some(owner) = v.pointer("/ownerV2/data").or_else(|| v.get("owner")) {
            out["owner"] =
                json!({"uri":owner["uri"],"display_name":text(owner,&["name","display_name"])});
        }
        for (target, paths) in [
            (
                "monthlyListeners",
                &["/stats/monthlyListeners", "/monthlyListeners"][..],
            ),
            ("followers", &["/stats/followers", "/followers/total"][..]),
            (
                "playCount",
                &["/playcount", "/playCount", "/playCount/total"][..],
            ),
            (
                "total_tracks",
                &[
                    "/content/totalCount",
                    "/tracks/totalCount",
                    "/tracksV2/totalCount",
                ][..],
            ),
        ] {
            if let Some(n) = paths.iter().find_map(|p| v.pointer(p).and_then(number)) {
                out[target] = json!(n);
            }
        }
        if let Some(date) = v
            .get("release_date")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| {
                v.pointer("/releaseDate/isoString")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .or_else(|| {
                v.pointer("/date/isoString")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .or_else(|| number(&v["date"]["year"]).map(|y| y.to_string()))
        {
            out["release_date"] = json!(date);
        }
        if kind == "track" && out["release_date"].is_null() {
            if let Some(date) = out.pointer("/album/release_date").cloned() {
                out["release_date"] = date;
            }
        }
        Some(out)
    }

    pub(super) fn collect(v: &Value) -> Vec<Value> {
        fn visit(v: &Value, out: &mut Vec<Value>, depth: usize) {
            if depth > 12 {
                return;
            }
            if let Some(item) = entity(v) {
                out.push(item);
                return;
            }
            if let Some(a) = v.as_array() {
                for item in a {
                    visit(item, out, depth + 1);
                }
            } else {
                for key in [
                    "items",
                    "releases",
                    "tracks",
                    "tracksV2",
                    "albums",
                    "artists",
                    "playlists",
                    "shows",
                    "episodes",
                    "episodesV2",
                    "entity",
                ] {
                    if let Some(child) = v.get(key) {
                        visit(child, out, depth + 1);
                    }
                }
            }
        }
        let mut out = Vec::new();
        visit(v, &mut out, 0);
        // Playlist duplicates have distinct UIDs and must survive normalization.
        out
    }
    fn title(v: &Value) -> &str {
        [
            v.pointer("/data/title/transformedLabel"),
            v.pointer("/data/title/text"),
            v.pointer("/data/title/translatedBaseText"),
            v.pointer("/title/transformedLabel"),
            v.get("title"),
            v.get("name"),
        ]
        .into_iter()
        .flatten()
        .find_map(Value::as_str)
        .unwrap_or("")
    }
    pub(super) fn sections(v: &Value) -> Vec<Value> {
        v.as_array()
            .into_iter()
            .flatten()
            .map(|section| {
                let items = collect(&section["sectionItems"]);
                let failed = text(&section["data"], &["__typename"]).contains("Error");
                json!({"title":title(section),"uri":section["uri"],"items":items,
            "offset":section["sectionItems"]["items"].as_array().map_or(0,Vec::len),
            "total":section["sectionItems"]["totalCount"],
            "error":if failed {"Spotify could not load this section"} else {""}})
            })
            .filter(|s| {
                !s["title"].as_str().unwrap_or("").is_empty()
                    || !s["items"].as_array().unwrap().is_empty()
            })
            .collect()
    }
    fn at(value: &Value, pointer: &str) -> ResultValue {
        value
            .pointer(pointer)
            .filter(|v| !v.is_null())
            .cloned()
            .ok_or_else(|| "Spotify web response is missing the requested content".into())
    }
    fn append_home_shelves(home: &mut Vec<Value>, extra: Vec<Value>) {
        let mut titles: HashSet<String> = home
            .iter()
            .filter_map(|s| s["title"].as_str().map(str::to_owned))
            .collect();
        let mut shown: HashSet<String> = home
            .iter()
            .flat_map(|s| s["items"].as_array().into_iter().flatten())
            .filter_map(|item| item["uri"].as_str().map(str::to_owned))
            .collect();
        for mut shelf in extra {
            let title = shelf["title"].as_str().unwrap_or("").to_owned();
            if titles.contains(&title) {
                continue;
            }
            if let Some(items) = shelf["items"].as_array_mut() {
                items.retain(|item| {
                    item["uri"]
                        .as_str()
                        .is_none_or(|uri| shown.insert(uri.to_owned()))
                });
                if !items.is_empty() {
                    titles.insert(title);
                    home.push(shelf);
                }
            }
        }
    }
    fn home() -> ResultValue {
        let primary = home_feed();
        // Sonora supplements the personalized feed with Made For You's missing shelves.
        let made = browse(Some("spotify:page:0JQ5DAt0tbjZptfcdMSKl3"));
        match (primary, made) {
            (Ok(mut feed), extra) => {
                if let Ok(extra) = extra {
                    let mut shelves = feed["sections"].as_array().cloned().unwrap_or_default();
                    append_home_shelves(
                        &mut shelves,
                        extra["sections"].as_array().cloned().unwrap_or_default(),
                    );
                    feed["sections"] = json!(shelves);
                }
                Ok(feed)
            }
            (Err(_), Ok(extra)) if extra["sections"].as_array().is_some_and(|s| !s.is_empty()) => {
                Ok(extra)
            }
            (Err(error), _) => Err(error),
        }
    }
    fn discovery_feed(personal: ResultValue, directory: ResultValue) -> ResultValue {
        match (personal, directory) {
            (Ok(mut feed), Ok(directory)) => {
                let mut shelves = feed["sections"].as_array().cloned().unwrap_or_default();
                let mut titles: HashSet<String> = shelves
                    .iter()
                    .filter_map(|s| s["title"].as_str().map(str::to_owned))
                    .collect();
                for shelf in directory["sections"].as_array().into_iter().flatten() {
                    if titles.insert(shelf["title"].as_str().unwrap_or("").to_owned()) {
                        shelves.push(shelf.clone());
                    }
                }
                feed["sections"] = json!(shelves);
                Ok(feed)
            }
            (Ok(feed), Err(_)) | (Err(_), Ok(feed)) => Ok(feed),
            (Err(error), Err(_)) => Err(error),
        }
    }
    fn discovery() -> ResultValue {
        discovery_feed(
            browse(Some("spotify:page:0JQ5DAt0tbjZptfcdMSKl3")),
            browse(None),
        )
    }
    fn home_feed() -> ResultValue {
        let value = web::query(
            "home",
            json!({"homeEndUserIntegration":"INTEGRATION_WEB_PLAYER","timeZone":"UTC","sp_t":"","facet":"","sectionItemsLimit":20,"includeEpisodeContentRatingsV2":false}),
        )?;
        let root = at(&value, "/data/home")?;
        let shelves = at(&root, "/sectionContainer/sections/items")?;
        Ok(json!({"sections":sections(&shelves),"title":root["greeting"]["transformedLabel"]}))
    }
    fn browse(page: Option<&str>) -> ResultValue {
        if let Some(page) = page {
            if page.len() > 300
                || !page.starts_with("spotify:")
                || page.chars().any(char::is_control)
            {
                return Err("Invalid Spotify browse page".into());
            }
        }
        super::directory_cache::load(&format!("browse-v3:{}", page.unwrap_or("root")), || {
            browse_uncached(page)
        })
    }
    fn browse_uncached(page: Option<&str>) -> ResultValue {
        if page.is_none() {
            let value = web::browse()?;
            let root = value
                .pointer("/data/browse")
                .or_else(|| value.pointer("/data/browseStart"))
                .ok_or("Spotify browse page returned no categories")?;
            return Ok(json!({"sections":sections(&at(root,"/sections/items")?)}));
        }
        let mut vars = json!({"pagePagination":{"offset":0,"limit":20},"sectionPagination":{"offset":0,"limit":8},"browseEndUserIntegration":"INTEGRATION_WEB_PLAYER","includeEpisodeContentRatingsV2":false});
        let (op, path) = if let Some(page) = page {
            if page.len() > 300
                || !page.starts_with("spotify:")
                || page.chars().any(char::is_control)
            {
                return Err("Invalid Spotify browse page".into());
            }
            vars["uri"] = json!(page);
            ("browsePage", "/data/browse/sections/items")
        } else {
            unreachable!()
        };
        let v = web::query(op, vars.clone())?;
        Ok(json!({"sections":sections(&at(&v,path)?)}))
    }
    fn browse_section(uri: &str, offset: u64, limit: u64) -> ResultValue {
        if !uri.starts_with("spotify:section:")
            || uri.len() > 300
            || offset > 10_000
            || !(1..=100).contains(&limit)
        {
            return Err("Invalid Spotify section pagination".into());
        }
        super::directory_cache::load(&format!("section-v1:{uri}:{offset}:{limit}"), || {
            let value = web::query(
                "browseSection",
                json!({"uri":uri,"pagination":{"offset":offset,"limit":limit},
            "browseEndUserIntegration":"INTEGRATION_WEB_PLAYER","includeEpisodeContentRatingsV2":false}),
            )?;
            let raw = at(&value, "/data/browseSection")?;
            let count = raw["sectionItems"]["items"].as_array().map_or(0, Vec::len);
            let mut normalized = sections(&json!([raw]))
                .into_iter()
                .next()
                .ok_or("Spotify returned no section")?;
            normalized["offset"] = json!(offset + count as u64);
            if count == 0 {
                normalized["total"] = normalized["offset"].clone();
            }
            Ok(normalized)
        })
    }

    fn recent() -> ResultValue {
        if let Ok(value) =
            web::rest_get("https://api.spotify.com/v1/me/player/recently-played?limit=50")
        {
            if let Some(items) = recent_tracks(&value) {
                return Ok(json!({"items": items, "next": null}));
            }
        }
        let account = web::account()?;
        let user = account["id"]
            .as_str()
            .ok_or("Spotify account identity is unavailable")?;
        let mut url =
            url::Url::parse("https://spclient.wg.spotify.com/recently-played/v3/user/").unwrap();
        url.path_segments_mut()
            .map_err(|_| "Invalid recently played address")?
            .pop_if_empty()
            .push(user)
            .push("recently-played");
        url.query_pairs_mut().extend_pairs([
            ("format", "json"),
            ("offset", "0"),
            ("limit", "50"),
            ("filter", "default,collection-new-episodes"),
        ]);
        let value = web::rest_get(url.as_str())?;
        let root = value.as_array().and_then(|a| a.first()).unwrap_or(&value);
        let contexts = root["playContexts"]
            .as_array()
            .ok_or("Spotify returned no recently played history")?;
        let uris: Vec<_> = contexts
            .iter()
            .filter_map(|c| c["uri"].as_str())
            .filter(|u| u.starts_with("spotify:") && !u.starts_with("spotify:collection:"))
            .take(50)
            .collect();
        if uris.is_empty() {
            return Ok(json!({"items":[],"next":null}));
        }
        let response = web::query("fetchEntitiesForRecentlyPlayed", json!({"uris":uris}))?;
        let mut items = collect(&at(&response, "/data/lookup")?);
        for item in &mut items {
            if let Some(context) = contexts.iter().find(|c| c["uri"] == item["uri"]) {
                item["played_at"] = context["lastPlayedTime"].clone();
            }
        }
        Ok(json!({"items":items,"next":null}))
    }

    fn recent_tracks(value: &Value) -> Option<Vec<Value>> {
        Some(
            value
                .get("items")?
                .as_array()?
                .iter()
                .filter_map(|play| {
                    let mut track = play.get("track")?.as_object()?.clone();
                    if let Some(played_at) = play.get("played_at") {
                        track.insert("played_at".into(), played_at.clone());
                    }
                    Some(Value::Object(track))
                })
                .collect(),
        )
    }
    fn page(op: &str, vars: Value, pointer: &str) -> ResultValue {
        let value = web::query(op, vars.clone())?;
        let root = at(&value, pointer)?;
        let items = collect(&root);
        let offset = vars["offset"].as_u64().unwrap_or(0);
        let limit = vars["limit"].as_u64().unwrap_or(50);
        let total = root["totalCount"].as_u64();
        let next_offset = root
            .pointer("/pagingInfo/nextOffset")
            .and_then(Value::as_u64)
            .or_else(|| {
                total
                    .filter(|t| offset + limit < *t)
                    .map(|_| offset + limit)
            });
        let next = next_offset
            .filter(|n| *n > offset && *n <= 100_000)
            .map(|n| {
                let mut vars = vars.clone();
                vars["offset"] = json!(n);
                format!("{CURSOR}{}", json!({"op":op,"vars":vars,"pointer":pointer}))
            });
        Ok(json!({"items":items,"total":total,"next":next}))
    }
    fn library(filter: &str) -> ResultValue {
        page(
            "libraryV3",
            json!({"filters":[filter],"order":null,"textFilter":"","features":["LIKED_SONGS","YOUR_EPISODES_V2","PRERELEASES","EVENTS"],"limit":50,"offset":0,"flatten":true,"expandedFolders":[],"folderUri":null,"includeFoldersWhenFlattening":false}),
            "/data/me/libraryV3",
        )
    }
    fn playlist(uri: &str, offset: u64) -> ResultValue {
        page(
            "fetchPlaylist",
            json!({"uri":uri,"offset":offset,"limit":100,"enableWatchFeedEntrypoint":false}),
            "/data/playlistV2/content",
        )
    }
    fn artist(uri: &str) -> ResultValue {
        at(
            &web::query(
                "queryArtistOverview",
                json!({"uri":uri,"locale":"","preReleaseV2":true}),
            )?,
            "/data/artistUnion",
        )
    }
    fn search(query: &str) -> ResultValue {
        search_with_limit(query, 20)
    }
    fn search_with_limit(query: &str, limit: u32) -> ResultValue {
        if query.trim().is_empty() || query.len() > 1000 {
            return Err("Enter a Spotify search".into());
        }
        let vars = json!({"searchTerm":query,"offset":0,"limit":limit,"numberOfTopResults":5,"includeAudiobooks":true,"includeArtistHasConcertsField":false,"includePreReleases":true,"includeLocalConcertsField":false,"includeAuthors":false});
        let value = web::query("searchDesktop", vars)?;
        let root = at(&value, "/data/searchV2")?;
        Ok(
            json!({"tracks":{"items":collect(root.get("tracksV2").unwrap_or(&root["tracks"]))},
        "albums":{"items":collect(&root["albums"])},"artists":{"items":collect(&root["artists"])},
        "playlists":{"items":collect(&root["playlists"])},"shows":{"items":collect(root.get("podcasts").unwrap_or(&root["shows"]))},"episodes":{"items":collect(&root["episodes"])}}),
        )
    }
    fn mark(mut value: Value, name: &str, args: &[Value]) -> Value {
        if let Some(items) = value["items"].as_array_mut() {
            for item in items {
                item["spotify_station"] = json!({"name":name,"args":args});
            }
        }
        value
    }
    fn radio(uri: &str) -> ResultValue {
        let value = web::rest_get(&format!(
        "https://spclient.wg.spotify.com/inspiredby-mix/v2/seed_to_playlist/{uri}?response-format=json"
    ))?;
        let playlist_uri = [
            value.get("uri"),
            value.get("playlist_uri"),
            value.pointer("/playlist/uri"),
        ]
        .into_iter()
        .flatten()
        .find_map(Value::as_str)
        .filter(|s| s.starts_with("spotify:playlist:"))
        .ok_or("Spotify radio did not return a playlist")?;
        playlist(playlist_uri, 0)
    }
    fn flow_options() -> ResultValue {
        Ok(json!({"options":super::flow::MOODS.iter().map(|(id,title)|
        json!({"id":id,"title":title,"kind":"mood"})).collect::<Vec<_>>()}))
    }

    fn mood_playlists(mood: &str) -> ResultValue {
        super::directory_cache::load(&format!("flow-mood-v1:{mood}"), || {
            let value = search_with_limit(mood, 100)?;
            Ok(json!(collect(&value["playlists"])
                .into_iter()
                .filter(|item| {
                    item.pointer("/owner/uri").and_then(Value::as_str)
                        == Some("spotify:user:spotify")
                })
                .map(|item| json!({"title":text(&item, &["name", "title"]),
            "description":text(&item, &["description"]),
            "artwork":item.pointer("/images/0/url").and_then(Value::as_str).unwrap_or_default()}))
                .collect::<Vec<_>>()))
        })
    }

    pub(super) fn genre_artwork(genre: &str) -> Result<String, String> {
        let value = super::directory_cache::load(&format!("flow-art-v1:{genre}"), || {
            if let Some(uri) = super::flow::genre_radio(genre) {
                if let Ok(value) = web::query(
                    "fetchPlaylist",
                    json!({"uri":uri,"offset":0,"limit":1,
                "enableWatchFeedEntrypoint":false}),
                ) {
                    if let Some(item) = value.pointer("/data/playlistV2").and_then(entity) {
                        if let Some(url) = item
                            .pointer("/images/0/url")
                            .and_then(Value::as_str)
                            .filter(|url| url.starts_with("https://"))
                        {
                            return Ok(json!(url));
                        }
                    }
                }
            }
            let items = search(&format!("{genre} Mix"))?;
            let entries = collect(&items["playlists"]);
            let item = entries
                .iter()
                .find(|item| {
                    item.pointer("/owner/uri").and_then(Value::as_str)
                        == Some("spotify:user:spotify")
                })
                .or_else(|| entries.first())
                .ok_or("Spotify returned no genre image")?;
            let url = item
                .pointer("/images/0/url")
                .and_then(Value::as_str)
                .filter(|url| url.starts_with("https://"))
                .ok_or("Spotify genre image unavailable")?;
            Ok(json!(url))
        })?;
        value
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| "Invalid Spotify genre image".into())
    }

    fn words(s: &str) -> String {
        format!(
            " {} ",
            s.to_lowercase()
                .chars()
                .map(|c| match c {
                    'ü' | 'ú' | 'ù' => 'u',
                    'ö' | 'ó' | 'ô' => 'o',
                    'ç' => 'c',
                    'ñ' => 'n',
                    'ä' | 'å' | 'ã' | 'á' | 'à' | 'â' => 'a',
                    'ş' => 's',
                    'ğ' => 'g',
                    'ı' | 'ï' => 'i',
                    'é' | 'è' | 'ê' => 'e',
                    c if c.is_alphanumeric() => c,
                    _ => ' ',
                })
                .collect::<String>()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
        )
    }
    fn matches_style(v: &Value, s: crate::style_mix::Selection) -> bool {
        let name = words(text(v, &["name"]));
        let has = |aliases: &[&str]| aliases.iter().any(|a| name.contains(&words(a)));
        has(crate::style_mix::REGIONS[s.region].1) && has(crate::style_mix::GENRES[s.genre].1)
    }
    fn style_mix(key: &str) -> ResultValue {
        let selected = crate::style_mix::Selection::parse(key)?;
        let query = format!(
            "{} {} Mix",
            crate::style_mix::REGIONS[selected.region].1[0],
            crate::style_mix::GENRES[selected.genre].1[0]
        );
        let mut sources = vec![None, None, None];
        crate::catalog::parallel::load(
            3,
            |index| match index {
                0 => home(),
                1 => library("Playlists"),
                _ => search(&query),
            },
            |index, result| {
                sources[index] = Some(result);
                Ok(())
            },
        )?;
        let mut candidates = Vec::new();
        if let Some(Ok(h)) = sources[0].take() {
            for section in h["sections"].as_array().into_iter().flatten() {
                candidates.extend(collect(&section["items"]));
            }
        }
        if let Some(Ok(v)) = sources[1].take() {
            candidates.extend(collect(&v));
        }
        match sources[2].take().expect("search completed") {
            Ok(value) => candidates.extend(collect(&value["playlists"])),
            Err(error) if !candidates.iter().any(|v| matches_style(v, selected)) => {
                return Err(error);
            }
            Err(_) => {}
        }

        let candidates: Vec<_> = candidates
            .iter()
            .filter(|v| matches_style(v, selected))
            .take(3)
            .collect();
        let mut playlists = vec![None; candidates.len()];
        crate::catalog::parallel::load(
            candidates.len(),
            |index| playlist(text(candidates[index], &["uri"]), 0),
            |index, result| {
                playlists[index] = Some(result);
                Ok(())
            },
        )?;
        let mut items = Vec::new();
        let mut seen = HashSet::new();
        let mut failure = None;
        for result in playlists.into_iter().flatten() {
            match result {
                Ok(v) => {
                    for mut track in collect(&v) {
                        if track["type"] == "track"
                            && track["is_playable"] != false
                            && seen.insert(text(&track, &["uri"]).to_owned())
                        {
                            track[crate::style_mix::MARKER] = json!(key);
                            items.push(track);
                        }
                    }
                }
                Err(e) => failure = Some(e),
            }
        }
        super::style_mix::retain_language(&mut items, selected, |track| track);
        if items.is_empty() {
            return Err(failure.unwrap_or_else(|| {
                "No playable tracks found in matching playlists. Try another language or genre."
                    .into()
            }));
        }
        Ok(json!({"items":items}))
    }
    fn mutate_library(uri: &str, save: bool) -> ResultValue {
        mutate_library_with(uri, save, web::mutation)
    }
    fn mutate_library_with(
        uri: &str,
        save: bool,
        mut call: impl FnMut(&str, Value) -> ResultValue,
    ) -> ResultValue {
        let op = if save {
            "addToLibrary"
        } else {
            "removeFromLibrary"
        };
        let result = call(op, json!({"libraryItemUris":[uri]}))?;
        confirmed(&result)?;
        Ok(json!(1))
    }
    fn confirmed(v: &Value) -> Result<(), String> {
        // HTTP success alone does not make a GraphQL error/union failure a successful mutation.
        fn failed(v: &Value) -> bool {
            match v {
                Value::Object(o) => {
                    text(v, &["__typename"]).contains("Error")
                        || o.get("success") == Some(&Value::Bool(false))
                        || o.iter().any(|(k, v)| {
                            k == "errors" && v.as_array().is_some_and(|a| !a.is_empty())
                                || failed(v)
                        })
                }
                Value::Array(a) => a.iter().any(failed),
                _ => false,
            }
        }
        if failed(v) || v.get("data").is_none_or(Value::is_null) {
            Err("Spotify did not confirm the requested change".into())
        } else {
            Ok(())
        }
    }

    pub(super) fn is_mutation(name: &str) -> bool {
        matches!(
            name,
            "DZAlbumLibraryJSON"
                | "DZPlaylistLibraryJSON"
                | "RustPodcastLibraryJSON"
                | "DZAddFavorite"
                | "DZRemoveFavorite"
                | "DZDeletePlaylist"
                | "DZAddToPlaylist"
                | "DZRemoveFromPlaylist"
                | "DZRenamePlaylist"
                | "DZUpdatePlaylistJSON"
                | "DZCreatePlaylist"
                | "DZPlaylistCoverJSON"
                | "DZSaveFlowTunerJSON"
                | "DZFlowConfigSaveJSON"
        )
    }

    /// None means this adapter does not handle the operation; callers may use the public API.
    pub(super) fn invoke(name: &str, args: &[Value]) -> Option<ResultValue> {
        let supported = matches!(
            name,
            "SPCatalogHomeJSON"
                | "SPCatalogDiscoveryJSON"
                | "SPCatalogChannelJSON"
                | "SPCatalogDirectoryCachedJSON"
                | "SPCatalogSectionJSON"
                | "SPCatalogNextJSON"
                | "DZFavoritesJSON"
                | "DZFavoriteAlbumsJSON"
                | "DZPlaylistsJSON"
                | "DZPodcastShelfJSON"
                | "DZArtistProfileJSON"
                | "DZArtistTopJSON"
                | "DZArtistSectionJSON"
                | "DZAlbumTracksJSON"
                | "DZPlaylistTracksJSON"
                | "DZPodcastEpisodesJSON"
                | "DZTrackMetadataJSON"
                | "SPCatalogItemJSON"
                | "DZSearchJSON"
                | "DZSearchPodcastsJSON"
                | "DZTrackMixJSON"
                | "DZArtistMixJSON"
                | "DZFlowJSON"
                | "DZFlowForConfigJSON"
                | "DZFlowAppendJSON"
                | "DZFlowConfigsJSON"
                | "DZFlowTunerJSON"
                | "DZFlowConfigTunerJSON"
                | "DZSaveFlowTunerJSON"
                | "DZFlowConfigSaveJSON"
                | "SPStyleMixJSON"
                | "StyleMixGenres"
                | "DZAlbumLibraryJSON"
                | "DZPlaylistLibraryJSON"
                | "RustPodcastLibraryJSON"
                | "DZAddFavorite"
                | "DZRemoveFavorite"
                | "DZDeletePlaylist"
                | "DZAddToPlaylist"
                | "DZRemoveFromPlaylist"
                | "DZRenamePlaylist"
                | "DZUpdatePlaylistJSON"
                | "DZCreatePlaylist"
                | "DZPlaylistCoverJSON"
                | "DZHistoryRecentJSON"
                | "DZHomeModuleJSON"
        );
        if !supported
            || name == "SPCatalogNextJSON"
                && !args
                    .first()
                    .and_then(Value::as_str)
                    .is_some_and(|s| s.starts_with(CURSOR) || s.starts_with(flow_queue::PREFIX))
        {
            return None;
        }
        Some((|| match name {
            "SPCatalogHomeJSON" => home(),
            "DZHomeModuleJSON" => match args.first().and_then(Value::as_u64) {
                Some(0) => recent(),
                Some(1) => page(
                    "fetchLibraryTracks",
                    json!({"offset":0,"limit":50}),
                    "/data/me/library/tracks",
                ),
                Some(2) => library("Playlists"),
                Some(3) | Some(8) => library("Albums"),
                Some(4) => library("Artists"),
                Some(7) => library("Podcasts"),
                Some(9) => library("Episodes"),
                _ => {
                    return Err(
                        "This Spotify shelf is not available through the web library".into(),
                    )
                }
            },
            "SPCatalogDiscoveryJSON" => discovery(),
            "SPCatalogDirectoryCachedJSON" => Ok(super::directory_cache::peek(&format!(
                "browse-v3:{}",
                arg(args, 0)?
            ))
            .unwrap_or(Value::Null)),
            "SPCatalogSectionJSON" => browse_section(
                arg(args, 0)?,
                args.get(1)
                    .and_then(Value::as_u64)
                    .ok_or("Missing offset")?,
                args.get(2).and_then(Value::as_u64).ok_or("Missing limit")?,
            ),
            "SPCatalogChannelJSON" => browse(Some(arg(args, 0)?)),
            "SPCatalogNextJSON" if arg(args, 0)?.starts_with(flow_queue::PREFIX) => {
                flow_queue::next(arg(args, 0)?)
            }
            "SPCatalogNextJSON" => {
                let cursor = arg(args, 0)?
                    .strip_prefix(CURSOR)
                    .ok_or("Invalid Spotify cursor")?;
                if cursor.len() > 4000 {
                    return Err("Invalid Spotify cursor".into());
                }
                let v: Value =
                    serde_json::from_str(cursor).map_err(|_| "Invalid Spotify cursor")?;
                let op = text(&v, &["op"]);
                let pointer = text(&v, &["pointer"]);
                if !matches!(
                    (op, pointer),
                    ("libraryV3", "/data/me/libraryV3")
                        | ("fetchLibraryTracks", "/data/me/library/tracks")
                        | ("fetchPlaylist", "/data/playlistV2/content")
                        | ("getAlbum", "/data/albumUnion/tracksV2")
                        | ("queryPodcastEpisodes", "/data/podcastUnionV2/episodesV2")
                ) || v["vars"]["offset"].as_u64().is_none_or(|n| n > 100_000)
                {
                    return Err("Invalid Spotify cursor".into());
                }
                page(op, v["vars"].clone(), pointer)
            }
            "DZFavoritesJSON" => page(
                "fetchLibraryTracks",
                json!({"offset":0,"limit":50}),
                "/data/me/library/tracks",
            ),
            "DZFavoriteAlbumsJSON" => library("Albums"),
            "DZPlaylistsJSON" => library("Playlists"),
            "DZPodcastShelfJSON" => library(if args.first().and_then(Value::as_u64) == Some(3) {
                "Episodes"
            } else {
                "Podcasts & Shows"
            }),
            "DZPlaylistTracksJSON" => playlist(&uri(args, 0, &["playlist"])?, 0),
            "DZAlbumTracksJSON" => page(
                "getAlbum",
                json!({"uri":uri(args,0,&["album"])?,"offset":0,"limit":50}),
                "/data/albumUnion/tracksV2",
            ),
            "DZPodcastEpisodesJSON" => {
                let offset = args.get(1).and_then(Value::as_u64).unwrap_or(0);
                page(
                    "queryPodcastEpisodes",
                    json!({"uri":uri(args,0,&["show"])?,"offset":offset,"limit":50}),
                    "/data/podcastUnionV2/episodesV2",
                )
            }
            "DZTrackMetadataJSON" => {
                let v = web::query(
                    "getTrack",
                    json!({"uri":uri(args,0,&["track"])?,"includePrerelease":true}),
                )?;
                entity(&at(&v, "/data/trackUnion")?)
                    .ok_or_else(|| "Spotify track metadata unavailable".into())
            }
            "DZArtistProfileJSON" => {
                let v = artist(&uri(args, 0, &["artist"])?)?;
                Ok(
                    json!({"artist":entity(&v),"top":{"items":collect(&v["discography"]["topTracks"])}}),
                )
            }
            "DZArtistTopJSON" => Ok(
                json!({"items":collect(&artist(&uri(args,0,&["artist"])?)?["discography"]["topTracks"])}),
            ),
            "DZArtistSectionJSON" => {
                let v = artist(&uri(args, 0, &["artist"])?)?;
                let index = args.get(1).and_then(Value::as_u64).unwrap_or(0);
                let p = match index {
                    0 => "/discography/albums",
                    2 => "/discography/singles",
                    3 => "/relatedContent/appearsOn",
                    4 => "/relatedContent/relatedArtists",
                    5 => "/relatedContent/featuring",
                    _ => return Err("Unknown Spotify artist section".into()),
                };
                let items = v.pointer(p).map(collect).unwrap_or_default();
                if index == 5 && items.is_empty() {
                    let name = v
                        .pointer("/profile/name")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    if !name.is_empty() {
                        return Ok(json!({"items": collect(&search(name)?["playlists"])}));
                    }
                }
                Ok(json!({"items":items}))
            }
            "SPCatalogItemJSON" => {
                let kind = arg(args, 0)?;
                let u = uri(
                    args,
                    1,
                    &["track", "album", "artist", "playlist", "show", "episode"],
                )?;
                let (op, pointer) = match kind {
                    "album" => ("getAlbum", "/data/albumUnion"),
                    "artist" => ("queryArtistOverview", "/data/artistUnion"),
                    "playlist" => ("fetchPlaylist", "/data/playlistV2"),
                    "track" => ("getTrack", "/data/trackUnion"),
                    "show" => ("queryShowMetadataV2", "/data/podcastUnionV2"),
                    "episode" => ("getEpisodeOrChapter", "/data/episodeUnionV2"),
                    _ => {
                        return web::rest_get(&format!(
                            "https://api.spotify.com/v1/{kind}s/{}",
                            u.rsplit(':').next().unwrap()
                        ));
                    }
                };
                let vars = match op {
                    "queryShowMetadataV2" => json!({
                        "uri": u,
                        "includeContentCapabilityTrait": false,
                        "includeEpisodeContentRatingsV2": false,
                    }),
                    "getEpisodeOrChapter" => json!({
                        "uri": u,
                        "includeEpisodeContentRatingsV2": false,
                    }),
                    _ => json!({
                        "uri": u,
                        "offset": 0,
                        "limit": 50,
                        "enableWatchFeedEntrypoint": false,
                        "locale": "",
                        "preReleaseV2": true,
                        "includePrerelease": true,
                    }),
                };
                entity(&at(&web::query(op, vars)?, pointer)?)
                    .ok_or_else(|| "Spotify item metadata unavailable".into())
            }
            "DZSearchJSON" | "DZSearchPodcastsJSON" => search(arg(args, 0)?),
            "DZHistoryRecentJSON" => recent(),
            "DZTrackMixJSON" | "DZArtistMixJSON" => Ok(mark(
                radio(&uri(args, 0, &["track", "artist"])?)?,
                name,
                args,
            )),
            "DZFlowConfigsJSON" => flow_options(),
            "DZFlowJSON" | "DZFlowForConfigJSON" | "DZFlowAppendJSON" => {
                flow_queue::start(args.first().and_then(Value::as_str).unwrap_or("default"))
            }
            "DZFlowTunerJSON" | "DZFlowConfigTunerJSON" => {
                let key = args.first().and_then(Value::as_str).unwrap_or("default");
                let selection = super::flow::Selection::parse(key)?;
                let playlists =
                    if selection.mood == "default" || key.starts_with(super::flow::PREFIX) {
                        json!([])
                    } else {
                        mood_playlists(&selection.mood)?
                    };
                super::flow::provider_tuner(key, &json!([]), &playlists)
            }
            "DZSaveFlowTunerJSON" | "DZFlowConfigSaveJSON" => {
                let key = arg(args, 0)?;
                if key.starts_with(super::flow::PREFIX) {
                    super::flow::save(key, arg(args, 1)?)
                } else {
                    let selection = super::flow::Selection::parse(key)?;
                    let playlists = if selection.mood == "default" {
                        json!([])
                    } else {
                        mood_playlists(&selection.mood)?
                    };
                    let tuner = super::flow::provider_tuner(key, &json!([]), &playlists)?;
                    super::flow::save_provider_tuner(key, arg(args, 1)?, &tuner)
                }
            }
            "SPStyleMixJSON" => Ok(mark(style_mix(arg(args, 0)?)?, name, args)),
            "StyleMixGenres" => {
                let region = args
                    .first()
                    .and_then(Value::as_u64)
                    .ok_or("Choose a language")? as usize;
                crate::style_mix::discover_genres(
                    region,
                    |selection| {
                        let query = format!(
                            "{} {} Mix",
                            crate::style_mix::REGIONS[selection.region].1[0],
                            crate::style_mix::GENRES[selection.genre].0
                        );
                        let value = search(&query)?;
                        let mut failure = None;
                        for candidate in collect(&value["playlists"])
                            .iter()
                            .filter(|candidate| matches_style(candidate, selection))
                            .take(3)
                        {
                            match playlist(text(candidate, &["uri"]), 0) {
                                Ok(value) => {
                                    let mut tracks: Vec<_> = collect(&value)
                                        .into_iter()
                                        .filter(|track| {
                                            track["type"] == "track"
                                                && track["is_playable"] != false
                                                && text(track, &["uri"])
                                                    .starts_with("spotify:track:")
                                        })
                                        .collect();
                                    super::style_mix::retain_language(
                                        &mut tracks,
                                        selection,
                                        |track| track,
                                    );
                                    if !tracks.is_empty() {
                                        return Ok(true);
                                    }
                                }
                                Err(error) => failure = Some(error),
                            }
                        }
                        match failure {
                            Some(error) => Err(error),
                            None => Ok(false),
                        }
                    },
                    || true,
                )
            }
            "RustPodcastLibraryJSON" if args.is_empty() => {
                library("Podcasts & Shows").map(|value| json!({"podcasts":value["items"]}))
            }
            "DZAlbumLibraryJSON" | "DZPlaylistLibraryJSON" | "RustPodcastLibraryJSON" => {
                let u = uri(args, 0, &["album", "playlist", "show"])?;
                let save = match args.get(1) {
                    Some(Value::Bool(b)) => *b,
                    Some(Value::Number(n)) => n.as_i64().ok_or("Invalid library action")? != 0,
                    _ => return Err("Missing library action".into()),
                };
                mutate_library(&u, save)
            }
            "DZAddFavorite" | "DZRemoveFavorite" | "DZDeletePlaylist" => mutate_library(
                &uri(args, 0, &["track", "episode", "playlist"])?,
                name == "DZAddFavorite",
            ),
            "DZAddToPlaylist" => {
                let v = web::mutation(
                    "addToPlaylist",
                    json!({"playlistUri":uri(args,0,&["playlist"])?,"playlistItemUris":[uri(args,1,&["track","episode"])?],"newPosition":{"moveType":"BOTTOM_OF_PLAYLIST","fromUid":null}}),
                )?;
                confirmed(&v)?;
                Ok(json!(1))
            }
            "DZRemoveFromPlaylist" => {
                let u = uri(args, 0, &["playlist"])?;
                let target = uri(args, 1, &["track", "episode"])?;
                let mut offset = 0;
                let mut uids = Vec::new();
                loop {
                    let v = playlist(&u, offset)?;
                    for t in v["items"].as_array().into_iter().flatten() {
                        if t["uri"] == target {
                            if let Some(uid) = t["uid"].as_str() {
                                uids.push(uid.to_owned());
                            }
                        }
                    }
                    if v["next"].is_null() {
                        break;
                    }
                    offset += 100;
                    if offset >= 10_000 {
                        return Err(
                            "Playlist is too large to safely resolve all removal positions".into(),
                        );
                    }
                }
                if uids.is_empty() {
                    return Err("Track was not found in the Spotify playlist".into());
                }
                let v = web::mutation("removeFromPlaylist", json!({"playlistUri":u,"uids":uids}))?;
                confirmed(&v)?;
                Ok(json!(1))
            }
            "DZRenamePlaylist" | "DZUpdatePlaylistJSON" => {
                let (operation, variables) = playlist_details_request(name, args)?;
                let result = web::mutation(&operation, variables)?;
                confirmed(&result)?;
                Ok(json!(1))
            }
            "DZCreatePlaylist" => {
                let name = arg(args, 0)?.trim();
                if name.is_empty() || name.len() > 300 {
                    return Err("Enter a playlist name".into());
                }
                let v = web::rest_request(
                    "POST",
                    "https://api.spotify.com/v1/me/playlists",
                    Some(&json!({"name":name,"public":false})),
                )?;
                let u = text(&v, &["uri"]);
                if u.starts_with("spotify:playlist:") {
                    Ok(json!({"id":u,"uri":u}))
                } else {
                    Err("Spotify did not return the created playlist URI".into())
                }
            }
            "DZPlaylistCoverJSON" => {
                let u = uri(args, 0, &["playlist"])?;
                let p = std::path::Path::new(arg(args, 1)?);
                if std::fs::metadata(p)
                    .map_err(|_| "Playlist cover could not be read")?
                    .len()
                    > 256 * 1024
                {
                    return Err("Spotify playlist cover must be at most 256 KB".into());
                }
                let bytes = std::fs::read(p).map_err(|_| "Playlist cover could not be read")?;
                if !bytes.starts_with(&[0xff, 0xd8, 0xff]) {
                    return Err("Spotify requires a JPEG playlist cover".into());
                }
                web::rest_upload(
                    &format!(
                        "https://api.spotify.com/v1/playlists/{}/images",
                        u.rsplit(':').next().unwrap()
                    ),
                    &bytes,
                    "image/jpeg",
                )?;
                Ok(json!(1))
            }
            _ => unreachable!(),
        })())
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn playlist_details_use_web_player_fields_and_validate_id() {
            let id = "1234567890123456789012";
            let (operation, body) = playlist_details_request(
                "DZUpdatePlaylistJSON",
                &[
                    json!(format!("spotify:playlist:{id}")),
                    json!("  New name  "),
                    json!(""),
                ],
            )
            .unwrap();
            assert_eq!(operation, "editPlaylistAttributes");
            assert_eq!(
                body,
                json!({"playlistUri":format!("spotify:playlist:{id}"), "newName":"New name", "newDescription":""})
            );
            let (_, body) =
                playlist_details_request("DZRenamePlaylist", &[json!(id), json!("Name")]).unwrap();
            assert_eq!(
                body,
                json!({"playlistUri":format!("spotify:playlist:{id}"), "newName":"Name"})
            );
            assert!(playlist_details_request(
                "DZRenamePlaylist",
                &[json!("invalid"), json!("Name")]
            )
            .is_err());
        }
        #[test]
        fn library_mutations_send_library_item_uris_and_require_confirmation() {
            for kind in ["playlist", "album", "track", "episode", "show"] {
                let uri = format!("spotify:{kind}:1234567890123456789012");
                for save in [true, false] {
                    let result = mutate_library_with(&uri, save, |operation, variables| {
                        assert_eq!(
                            operation,
                            if save {
                                "addToLibrary"
                            } else {
                                "removeFromLibrary"
                            }
                        );
                        assert_eq!(variables, json!({"libraryItemUris":[uri]}));
                        Ok(json!({"data":{"mutation":{"__typename":"LibraryMutationPayload"}}}))
                    });
                    assert_eq!(result.unwrap(), json!(1));
                }
            }
            for response in [
                json!({"data":{"mutation":{"__typename":"GenericError"}}}),
                json!({"errors":[{"message":"Invalid variables"}]}),
                Value::Null,
            ] {
                assert!(mutate_library_with(
                    "spotify:playlist:1234567890123456789012",
                    true,
                    |_, _| Ok(response.clone())
                )
                .is_err());
            }
            assert!(mutate_library_with(
                "spotify:playlist:1234567890123456789012",
                true,
                |_, _| Err("HTTP 403".into())
            )
            .is_err());
        }
        #[test]
        fn discover_keeps_personal_mix_shelves_and_browse_categories() {
            let personal = json!({"sections":[
                {"title":"Your Decade Mixes","items":[{"uri":"spotify:playlist:decade"}]},
                {"title":"Your Mood Mixes","items":[{"uri":"spotify:playlist:mood"}]},
                {"title":"Your Niche Mixes","items":[{"uri":"spotify:playlist:niche"}]}
            ]});
            let directory =
                json!({"sections":[{"title":"Browse all","items":[{"uri":"spotify:page:rock"}]}]});
            let feed = discovery_feed(Ok(personal.clone()), Ok(directory.clone())).unwrap();
            let titles: Vec<_> = feed["sections"]
                .as_array()
                .unwrap()
                .iter()
                .map(|s| s["title"].as_str().unwrap())
                .collect();
            assert_eq!(
                titles,
                [
                    "Your Decade Mixes",
                    "Your Mood Mixes",
                    "Your Niche Mixes",
                    "Browse all"
                ]
            );
            assert_eq!(
                discovery_feed(Ok(personal.clone()), Err("offline".into())).unwrap(),
                personal
            );
            assert_eq!(
                discovery_feed(Err("offline".into()), Ok(directory.clone())).unwrap(),
                directory
            );
            assert!(discovery_feed(Err("offline".into()), Err("offline".into())).is_err());
        }

        #[test]
        fn home_supplement_preserves_context_and_adds_only_missing_cards() {
            let mut shelves =
                vec![json!({"title":"More like Artist","items":[{"uri":"spotify:playlist:a"}]})];
            append_home_shelves(
                &mut shelves,
                vec![
                    json!({"title":"More like Artist","items":[{"uri":"spotify:playlist:b"}]}),
                    json!({"title":"Your top mixes","items":[{"uri":"spotify:playlist:a"},{"uri":"spotify:playlist:c"}]}),
                    json!({"title":"Already shown","items":[{"uri":"spotify:playlist:c"}]}),
                ],
            );
            assert_eq!(shelves.len(), 2);
            assert_eq!(shelves[0]["title"], "More like Artist");
            assert_eq!(shelves[1]["title"], "Your top mixes");
            assert_eq!(shelves[1]["items"], json!([{"uri":"spotify:playlist:c"}]));
        }

        #[test]
        fn recent_tracks_preserves_order_and_play_times() {
            let value = json!({"items":[
                {"track":{"uri":"spotify:track:first","name":"First"},"played_at":"2026-11-03T12:00:00Z"},
                {"track":{"uri":"spotify:track:second","name":"Second"},"played_at":"2026-11-03T11:00:00Z"}
            ]});
            let tracks = recent_tracks(&value).unwrap();
            assert_eq!(tracks[0]["name"], "First");
            assert_eq!(tracks[0]["played_at"], "2026-11-03T12:00:00Z");
            assert_eq!(tracks[1]["name"], "Second");
            assert!(recent_tracks(&json!({"error": {}})).is_none());
        }
        #[test]
        fn style_mix_playlist_matching_requires_both_country_and_genre() {
            let selected = crate::style_mix::Selection::new(0, 1).unwrap();
            assert!(matches_style(&json!({"name":"Turkish Rock Mix"}), selected));
            assert!(!matches_style(&json!({"name":"Turkish Pop Mix"}), selected));
            assert!(!matches_style(
                &json!({"name":"Japanese Rock Mix"}),
                selected
            ));
            assert!(!matches_style(&json!({"name":"Rock Mix"}), selected));
        }
        #[test]
        fn browse_category_wrappers_keep_real_card_artwork_and_all_items() {
            let items: Vec<_> = (0..67).map(|i| json!({
            "uri":format!("spotify:page:category{i}"),
            "content":{"__typename":"BrowseSectionContainerWrapper","data":{
                "__typename":"BrowseSectionContainer","data":{"cardRepresentation":{
                    "title":{"transformedLabel":format!("Category {i}")},
                    "artwork":{"sources":[{"url":format!("https://i.scdn.co/image/category{i}"),"width":300,"height":300}]}
                }}
            }}
        })).collect();
            let shelves = sections(
                &json!([{"data":{"title":{"transformedLabel":"Browse all"}},"sectionItems":{"items":items}}]),
            );
            assert_eq!(shelves[0]["items"].as_array().unwrap().len(), 67);
            for (i, item) in shelves[0]["items"].as_array().unwrap().iter().enumerate() {
                assert_eq!(
                    item["images"][0]["url"],
                    format!("https://i.scdn.co/image/category{i}")
                );
                assert_eq!(item["images"][0]["width"], 300);
            }
        }
        #[test]
        fn nested_playlist_wrappers_preserve_uri_uid_metrics_and_credits() {
            let v = json!({"items":[{"uid":"row1","itemV2":{"_uri":"spotify:track:1111111111111111111111","data":{"uri":"spotify:track:1111111111111111111111","name":"Song","playcount":"12345","duration":{"totalMilliseconds":192000},"artists":{"items":[{"uri":"spotify:artist:2222222222222222222222","profile":{"name":"Artist"}}]},"albumOfTrack":{"uri":"spotify:album:3333333333333333333333","name":"Album","coverArt":{"sources":[{"url":"https://i.scdn.co/image/example"}]}}}}}]});
            let rows = collect(&v);
            assert_eq!(rows.len(), 1);
            let t = &rows[0];
            assert_eq!(t["uid"], "row1");
            assert_eq!(t["playCount"], 12345);
            assert_eq!(t["duration_ms"], 192000);
            assert_eq!(t["artists"][0]["name"], "Artist");
            assert_eq!(t["album"]["type"], "album");
        }
        #[test]
        fn artist_top_track_keeps_cover_when_album_has_no_name() {
            let track = json!({"uri":"spotify:track:1111111111111111111111","name":"Song","albumOfTrack":{"uri":"spotify:album:3333333333333333333333","coverArt":{"sources":[{"url":"https://i.scdn.co/image/cover"}]}}});
            let normalized = entity(&track).unwrap();
            assert_eq!(
                normalized["album"]["images"][0]["url"],
                "https://i.scdn.co/image/cover"
            );
            assert_eq!(
                crate::catalog::entry(&normalized, "track").unwrap().artwork,
                "https://i.scdn.co/image/cover"
            );
        }
        #[test]
        fn partial_home_errors_keep_valid_shelves_and_supplied_titles() {
            let v = json!([{"data":{"title":{"transformedLabel":"Your real mix"}},"sectionItems":{"items":[{"content":{"__typename":"PlaylistResponseWrapper","data":{"uri":"spotify:playlist:1111111111111111111111","name":"Daily mix","images":{"items":[{"sources":[{"url":"https://i.scdn.co/image/mix"}]}]}}}}]}},{"data":{"__typename":"GenericError","title":{"text":"Unavailable"}},"sectionItems":{"items":[]}}]);
            let shelves = sections(&v);
            assert_eq!(shelves.len(), 2);
            assert_eq!(shelves[0]["title"], "Your real mix");
            assert_eq!(
                shelves[0]["items"][0]["images"][0]["url"],
                "https://i.scdn.co/image/mix"
            );
            assert!(!shelves[1]["error"].as_str().unwrap().is_empty());
        }
        #[test]
        fn artist_metrics_are_independent_and_errors_are_not_items() {
            let v = json!({"uri":"spotify:artist:1111111111111111111111","profile":{"name":"Artist"},"stats":{"monthlyListeners":900,"followers":100}});
            let e = entity(&v).unwrap();
            assert_eq!(e["monthlyListeners"], 900);
            assert_eq!(e["followers"], 100);
            assert!(e.get("playCount").is_none());
            assert!(entity(&json!({"__typename":"NotFound","message":"Private"})).is_none());
            assert!(
                confirmed(&json!({"data":{"addToLibrary":{"__typename":"GenericError"}}})).is_err()
            );
            assert!(confirmed(
                &json!({"data":{"addToLibrary":{"__typename":"AddToLibraryPayload"}}})
            )
            .is_ok());
        }

        #[test]
        fn track_details_keep_release_credit_and_content_fields() {
            let v = json!({"uri":"spotify:track:1111111111111111111111","name":"Song",
            "trackNumber":3,"contentRating":{"label":"EXPLICIT"},
            "external_ids":{"isrc":"TEST123"},
            "albumOfTrack":{"uri":"spotify:album:2222222222222222222222","name":"Album",
                "date":{"isoString":"2011-03-21T00:00:00Z"}}});
            let e = entity(&v).unwrap();
            assert_eq!(e["track_number"], 3);
            assert_eq!(e["explicit"], true);
            assert_eq!(e["external_ids"]["isrc"], "TEST123");
            assert_eq!(e["release_date"], "2011-03-21T00:00:00Z");
        }

        #[test]
        fn podcast_episode_and_show_wrappers_unwrap_correctly() {
            let payload = json!({
                "items": [
                    {
                        "entity": {
                            "__typename": "EpisodeResponseWrapper",
                            "data": {
                                "__typename": "Episode",
                                "uri": "spotify:episode:5x11111111111111111111",
                                "name": "Test Episode #123",
                                "description": "Episode description text",
                                "duration": { "totalMilliseconds": 3600000 },
                                "releaseDate": { "isoString": "2024-02-15T12:00:00Z" },
                                "coverArt": { "sources": [{ "url": "https://i.scdn.co/image/ep123", "width": 640, "height": 640 }] },
                                "podcastV2": {
                                    "data": {
                                        "__typename": "Podcast",
                                        "uri": "spotify:show:7x22222222222222222222",
                                        "name": "The Great Show",
                                        "coverArt": { "sources": [{ "url": "https://i.scdn.co/image/show123", "width": 640, "height": 640 }] }
                                    }
                                }
                            }
                        }
                    }
                ]
            });
            let episodes = collect(&payload);
            assert_eq!(episodes.len(), 1);
            let ep = &episodes[0];
            assert_eq!(ep["uri"], "spotify:episode:5x11111111111111111111");
            assert_eq!(ep["name"], "Test Episode #123");
            assert_eq!(ep["type"], "episode");
            assert_eq!(ep["duration_ms"], 3600000);
            assert_eq!(ep["release_date"], "2024-02-15T12:00:00Z");
            assert_eq!(ep["images"][0]["url"], "https://i.scdn.co/image/ep123");
            assert_eq!(ep["show"]["name"], "The Great Show");
            assert_eq!(ep["show"]["uri"], "spotify:show:7x22222222222222222222");

            let show_payload = json!({
                "__typename": "Podcast",
                "uri": "spotify:show:7x22222222222222222222",
                "name": "The Great Show",
                "publisher": { "name": "Show Host" },
                "htmlDescription": "<p>A wonderful show</p>",
                "coverArt": { "sources": [{ "url": "https://i.scdn.co/image/show123", "width": 640, "height": 640 }] }
            });
            let show = entity(&show_payload).unwrap();
            assert_eq!(show["type"], "show");
            assert_eq!(show["name"], "The Great Show");
            assert_eq!(show["publisher"], "Show Host");
            assert_eq!(show["description"], "<p>A wonderful show</p>");
            assert_eq!(show["images"][0]["url"], "https://i.scdn.co/image/show123");
        }
    }
}

pub(crate) fn genre_artwork(genre: &str) -> Result<String, String> {
    web_catalog::genre_artwork(genre)
}
pub(crate) mod web_devices {
    //! Private Web Player Connect bridge. No OAuth fallback while a web account is active.
    use serde_json::{json, Value};

    fn state() -> Result<Value, String> {
        if !super::web::has_session() {
            return Err("Spotify Web Player session required for remote control".into());
        }
        super::web::device_command(json!({"action":"state"}))
    }

    pub(crate) fn devices() -> Result<Value, String> {
        normalize_devices(&state()?)
    }

    fn normalize_devices(cluster: &Value) -> Result<Value, String> {
        let devices = cluster["devices"]
            .as_object()
            .ok_or("Invalid Spotify Connect device state")?;
        let active = cluster["active_device_id"].as_str();
        let mut result = Vec::new();
        for (key, d) in devices {
            // Our hidden WebView runs the catalog/Connect transport, not local audio.
            if cluster["deeznats_web_device_id"].as_str() == Some(key.as_str()) {
                continue;
            }
            let id = d["device_id"]
                .as_str()
                .filter(|id| *id == key && valid_id(id));
            let cap = &d["capabilities"];
            // Observers are not playback targets. Missing capabilities stay restricted.
            if cap["hidden"].as_bool() == Some(true)
                || cap["can_be_player"].as_bool() == Some(false)
            {
                continue;
            }
            let restricted = id.is_none()
                || d["can_play"].as_bool() != Some(true)
                || cap["is_controllable"].as_bool() != Some(true)
                || d["is_restricted"].as_bool() == Some(true);
            let volume = if cap["disable_volume"].as_bool() == Some(true) {
                None
            } else {
                d["volume"]
                    .as_f64()
                    .filter(|n| n.is_finite() && (0.0..=65535.0).contains(n))
                    .map(|n| (n * 100.0 / 65535.0).round() as u64)
            };
            result.push(json!({"id":id,"name":d["name"].as_str().unwrap_or("Spotify device"),
            "type":d["device_type"].as_str().unwrap_or("unknown").to_lowercase(),
            "is_active":id.is_some() && id==active,"is_restricted":restricted,"volume_percent":volume}));
        }
        Ok(Value::Array(result))
    }

    fn valid_id(id: &str) -> bool {
        !id.is_empty()
            && id.len() <= 256
            && id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_%:-".contains(&b))
    }

    pub(crate) fn transfer(device: &str, playing: bool) -> Result<Value, String> {
        if !valid_id(device) {
            return Err("Invalid Spotify device".into());
        }
        if !super::web::has_session() {
            return Err("Spotify Web Player session required".into());
        }
        // The app may still show stopped/local playback while another Connect
        // device is playing. Preserve the observed session rather than pausing it
        // as a side effect of selecting a different output.
        let playing = transfer_playing(&state()?, playing);
        let result = super::web::device_command(
            json!({"action":"transfer","device":device,"playing":playing}),
        )?;
        super::remote::select_device(Some(device.into()));
        Ok(result)
    }

    fn transfer_playing(cluster: &Value, fallback: bool) -> bool {
        cluster["player_state"]["is_paused"]
            .as_bool()
            .map(|paused| !paused)
            .unwrap_or(fallback)
    }

    #[cfg(test)]
    mod transfer_tests {
        use super::*;
        #[test]
        fn transfer_preserves_live_playback_instead_of_stale_local_state() {
            assert!(transfer_playing(
                &json!({"player_state":{"is_paused":false}}),
                false
            ));
            assert!(!transfer_playing(
                &json!({"player_state":{"is_paused":true}}),
                true
            ));
            assert!(transfer_playing(&json!({}), true));
            assert!(!transfer_playing(&json!({}), false));
        }
    }

    /// Fresh, global playback state, independent of the local Soloist instance.
    /// `None` means no observable active device/track, never a synthetic local state.
    /// Position is the server's sampled position; callers may advance it from timestamp.
    pub(crate) fn snapshot() -> Result<Option<Value>, String> {
        normalize_snapshot(&state()?)
    }
    pub(crate) fn playback_target() -> Result<Value, String> {
        let cluster = state()?;
        if cluster["active_device_id"]
            .as_str()
            .is_some_and(|id| cluster["deeznats_web_device_id"].as_str() == Some(id))
        {
            return Err(
            "Pair DeezNats in Spotify for local audio, or select another Spotify Connect device"
                .into(),
        );
        }
        if let Some(snapshot) = normalize_snapshot(&cluster)? {
            return Ok(snapshot);
        }
        let devices = normalize_devices(&cluster)?;
        let device = devices
            .as_array()
            .and_then(|d| {
                d.iter()
                    .find(|d| d["is_active"] == true && d["is_restricted"] == false)
            })
            .ok_or("No active Spotify playback device; select a device first")?;
        Ok(json!({"device":device,"uri":"","is_playing":false,"position_ms":0}))
    }

    fn normalize_snapshot(cluster: &Value) -> Result<Option<Value>, String> {
        let devices = normalize_devices(cluster)?;
        let Some(device) = devices
            .as_array()
            .and_then(|ds| ds.iter().find(|d| d["is_active"] == true))
        else {
            return Ok(None);
        };
        let id = device["id"]
            .as_str()
            .ok_or("Invalid Spotify active device")?;
        if cluster["devices"][id]["capabilities"]["is_observable"].as_bool() != Some(true) {
            return Ok(None);
        }
        let p = &cluster["player_state"];
        let Some(uri) = p["track"]["uri"]
            .as_str()
            .filter(|s| s.starts_with("spotify:track:") || s.starts_with("spotify:episode:"))
        else {
            return Ok(None);
        };
        let Some(paused) = p["is_paused"].as_bool() else {
            return Ok(None);
        };
        let number = |key: &str| {
            p[key]
                .as_u64()
                .or_else(|| p[key].as_str()?.parse::<u64>().ok())
        };
        Ok(Some(json!({"device":device,"uri":uri,"is_playing":!paused,
        "position_ms":number("position_as_of_timestamp").or_else(||number("position")),
        "timestamp_ms":number("timestamp"),"duration_ms":number("duration"),
        "restrictions":p["restrictions"],"options":p["options"],"next_tracks":p["next_tracks"],
        "prev_tracks":p["prev_tracks"],"queue_revision":p["queue_revision"],"track_uid":p["track"]["uid"],
        "context_uri":p["context_uri"]})))
    }

    /// Controls the freshly observed global active device, never a cached transfer target.
    /// Supported actions: pause/resume/next/previous (None), seek (milliseconds),
    /// volume (integer 0..100). Success is command acceptance; poll snapshot for state.
    pub(crate) fn control(action: &str, value: Option<u64>) -> Result<(), String> {
        control_on(action, value, None)
    }
    pub(crate) fn control_on(
        action: &str,
        value: Option<u64>,
        device: Option<&str>,
    ) -> Result<(), String> {
        let valid = match action {
            "pause" | "resume" | "next" | "previous" => value.is_none(),
            "seek" => value.is_some_and(|v| v <= 9_007_199_254_740_991),
            "volume" => value.is_some_and(|v| v <= 100),
            "shuffle" => value.is_some_and(|v| v <= 1),
            "repeat" => value.is_some_and(|v| v <= 2),
            _ => false,
        };
        if !valid {
            return Err("Invalid Spotify remote control".into());
        }
        if !super::web::has_session() {
            return Err("Spotify Web Player session required".into());
        }
        super::web::device_command(
            json!({"action":action,"value":value,"expected":{"device":device}}),
        )
        .map(|_| ())
    }

    // Source: current-vendor~web-player.19d7605d.js Controller.playURI/playTracks,
    // _generatePlayOptions, ConnectAPIClient.play/setQueue/_sendPlayerCommand.
    pub(crate) fn play(descriptor: Value, device: &str) -> Result<(), String> {
        super::web::device_command(json!({"action":"play","descriptor":descriptor,
        "expected":{"device":device}}))
        .map(|_| ())
    }
    pub(crate) fn play_context(uri: &str, device: &str) -> Result<(), String> {
        let mut parts = uri.split(':');
        if parts.next() != Some("spotify")
            || !matches!(
                parts.next(),
                Some("track" | "episode" | "album" | "playlist" | "artist" | "show")
            )
            || !parts
                .next()
                .is_some_and(|s| s.len() == 22 && s.bytes().all(|b| b.is_ascii_alphanumeric()))
            || parts.next().is_some()
        {
            return Err("Invalid Spotify playback URI".into());
        }
        play(
            json!({"context":{"uri":uri,"url":format!("context://{uri}"),"metadata":{}},
        "play_origin":{"feature_identifier":"harmony"},"options":{"license":""}}),
            device,
        )
    }
    pub(crate) fn set_queue(before: &Value, next: &[Value]) -> Result<(), String> {
        super::web::device_command(json!({"action":"queue","next_tracks":next,
        "expected":{"device":before["device"]["id"],"uri":before["uri"],"revision":before["queue_revision"]}})).map(|_| ())
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        fn fixture() -> Value {
            json!({"active_device_id":"speaker","devices":{
            "observer":{"device_id":"observer","capabilities":{"hidden":true}},
            "speaker":{"device_id":"speaker","name":"Room","device_type":"SPEAKER","can_play":true,"volume":32768,
                "capabilities":{"is_controllable":true,"is_observable":true}},
            "missing":{"name":"Unselectable"}},
            "player_state":{"track":{"uri":"spotify:track:fixture"},"is_paused":false,"position_as_of_timestamp":"1234","timestamp":"5678","duration":"9000"}})
        }
        #[test]
        fn cluster_devices_are_minimal_and_observers_hidden() {
            let v = normalize_devices(&fixture()).unwrap();
            let ds = v.as_array().unwrap();
            assert_eq!(ds.len(), 2);
            let active = ds.iter().find(|d| d["is_active"] == true).unwrap();
            assert_eq!(active["volume_percent"], 50);
            assert_eq!(active.as_object().unwrap().len(), 6);
            assert!(ds
                .iter()
                .any(|d| d["id"].is_null() && d["is_restricted"] == true));
        }
        #[test]
        fn hidden_catalog_browser_is_never_an_audio_target_or_playback_snapshot() {
            let mut cluster = fixture();
            cluster["deeznats_web_device_id"] = json!("speaker");
            let devices = normalize_devices(&cluster).unwrap();
            assert!(!devices
                .as_array()
                .unwrap()
                .iter()
                .any(|device| device["id"] == "speaker"));
            assert!(normalize_snapshot(&cluster).unwrap().is_none());
        }
        #[test]
        fn global_snapshot_requires_observed_active_device_and_track() {
            let mut c = fixture();
            let s = normalize_snapshot(&c).unwrap().unwrap();
            assert_eq!(s["position_ms"], 1234);
            assert_eq!(s["is_playing"], true);
            c["active_device_id"] = json!("absent");
            assert!(normalize_snapshot(&c).unwrap().is_none());
            c["active_device_id"] = json!("speaker");
            c["devices"]["speaker"]["capabilities"]["is_observable"] = json!(false);
            assert!(normalize_snapshot(&c).unwrap().is_none());
        }
        #[test]
        fn guards_and_disabled_volume() {
            for id in ["", "a/b", "a?b", "a\nb", "@local"] {
                assert!(!valid_id(id));
            }
            let mut c = fixture();
            c["devices"]["speaker"]["capabilities"]["disable_volume"] = json!(true);
            assert!(normalize_devices(&c)
                .unwrap()
                .as_array()
                .unwrap()
                .iter()
                .find(|d| d["is_active"] == true)
                .unwrap()["volume_percent"]
                .is_null());
            assert!(normalize_devices(&json!({})).is_err());
            assert!(control("logout", None).is_err());
            assert!(control("volume", Some(101)).is_err());
        }
    }
}
pub(crate) use catalog::invoke as catalog_invoke;
mod process {
    //! Hidden, bounded child processes tied to the application's Windows job lifetime.
    use std::{
        io::Read,
        os::windows::{
            io::{AsHandle, AsRawHandle, OwnedHandle},
            process::CommandExt,
        },
        process::{Child, Command, ExitStatus, Stdio},
        sync::{Arc, Mutex},
        thread,
        time::{Duration, Instant},
    };
    use windows_sys::Win32::{
        Foundation::{CloseHandle, HANDLE},
        System::JobObjects::*,
    };
    const NO_WINDOW: u32 = 0x08000000;
    pub(super) const MAX_OUTPUT: usize = 2 * 1024 * 1024;

    /// A duplicated handle pins the supervised process identity, including after exit.
    /// Never trust a PID file or discover an arbitrary process by executable name.
    pub(crate) struct AudioTarget {
        pub pid: u32,
        process: OwnedHandle,
    }
    impl AudioTarget {
        pub(crate) fn from_child(child: &Child) -> Result<Self, String> {
            let process = child
                .as_handle()
                .try_clone_to_owned()
                .map_err(|_| "Could not retain the supervised Soloist process")?;
            Ok(Self {
                pid: child.id(),
                process,
            })
        }
        pub fn is_alive(&self) -> bool {
            unsafe {
                windows_sys::Win32::System::Threading::WaitForSingleObject(
                    self.process.as_raw_handle(),
                    0,
                ) == windows_sys::Win32::Foundation::WAIT_TIMEOUT
            }
        }
    }

    struct Job(HANDLE);
    unsafe impl Send for Job {}
    impl Drop for Job {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
    impl Job {
        fn new(child: &Child) -> Result<Self, String> {
            unsafe {
                let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
                if handle.is_null() {
                    return Err("Could not create Soloist process job".into());
                }
                let job = Self(handle);
                let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
                limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                if SetInformationJobObject(
                    handle,
                    JobObjectExtendedLimitInformation,
                    (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                    std::mem::size_of_val(&limits) as u32,
                ) == 0
                    || AssignProcessToJobObject(handle, child.as_raw_handle()) == 0
                {
                    return Err("Could not bind Soloist to the app lifetime".into());
                }
                Ok(job)
            }
        }
    }
    pub(super) struct Process {
        pub child: Child,
        job: Option<Job>,
        stderr: Arc<Mutex<Vec<u8>>>,
        stderr_reader: Option<thread::JoinHandle<()>>,
    }
    impl Process {
        pub fn audio_target(&self) -> Result<AudioTarget, String> {
            AudioTarget::from_child(&self.child)
        }
        pub fn spawn(command: &mut Command) -> Result<Self, String> {
            let mut child = command
                .stdin(Stdio::null())
                .stderr(Stdio::piped())
                .creation_flags(NO_WINDOW)
                .spawn()
                .map_err(|error| format!("Could not start Soloist process: {error}"))?;
            match Job::new(&child) {
                Ok(job) => {
                    let mut pipe = child
                        .stderr
                        .take()
                        .ok_or("Soloist diagnostic stream unavailable")?;
                    let stderr = Arc::new(Mutex::new(Vec::new()));
                    let output = stderr.clone();
                    let reader = thread::spawn(move || {
                        let mut buffer = [0u8; 4096];
                        while let Ok(count) = pipe.read(&mut buffer) {
                            if count == 0 {
                                break;
                            }
                            if let Ok(mut bytes) = output.lock() {
                                bytes.extend_from_slice(&buffer[..count]);
                                let excess = bytes.len().saturating_sub(16384);
                                bytes.drain(..excess);
                            }
                        }
                    });
                    Ok(Self {
                        child,
                        job: Some(job),
                        stderr,
                        stderr_reader: Some(reader),
                    })
                }
                Err(error) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    Err(error)
                }
            }
        }
        #[cfg(test)]
        pub fn job_for_test_close(&mut self) {
            self.job.take();
        }
        pub fn stop(&mut self) {
            self.job.take();
            let _ = self.child.kill();
            let _ = self.child.wait();
            if let Some(reader) = self.stderr_reader.take() {
                let _ = reader.join();
            }
        }
        pub fn diagnostics(&self) -> String {
            self.stderr
                .lock()
                .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
                .unwrap_or_default()
        }
    }
    impl Drop for Process {
        fn drop(&mut self) {
            self.stop();
        }
    }

    pub(super) fn capture(
        command: &mut Command,
        timeout: Duration,
    ) -> Result<(ExitStatus, Vec<u8>), String> {
        let mut process = Process::spawn(command.stdout(Stdio::piped()))?;
        let output = process
            .child
            .stdout
            .take()
            .ok_or("Soloist output unavailable")?;
        let reader = thread::spawn(move || {
            let mut bytes = Vec::new();
            output
                .take(MAX_OUTPUT as u64 + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| "Could not read Soloist response")?;
            if bytes.len() > MAX_OUTPUT {
                Err("Soloist response exceeded its size limit")
            } else {
                Ok(bytes)
            }
        });
        let started = Instant::now();
        let status = loop {
            match process.child.try_wait() {
                Ok(Some(status)) => break Ok(status),
                Ok(None) if started.elapsed() < timeout => thread::sleep(Duration::from_millis(20)),
                Ok(None) => break Err("Soloist request timed out"),
                Err(_) => break Err("Could not inspect Soloist process"),
            }
        };
        // Close the job before joining the reader, including any descendants holding stdout.
        process.stop();
        let bytes = reader
            .join()
            .map_err(|_| "Soloist response reader stopped")?
            .map_err(str::to_owned)?;
        Ok((status.map_err(str::to_owned)?, bytes))
    }
    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn capture_bounds_output_and_times_out_hung_process() {
            let mut command = Command::new("powershell.exe");
            command.args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "[Console]::Write('ok')",
            ]);
            let (status, output) = capture(&mut command, Duration::from_secs(10)).unwrap();
            assert!(status.success());
            assert_eq!(output, b"ok");
            let mut command = Command::new("powershell.exe");
            command.args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Start-Sleep -Seconds 30",
            ]);
            let start = Instant::now();
            assert!(capture(&mut command, Duration::from_millis(150))
                .unwrap_err()
                .contains("timed out"));
            assert!(start.elapsed() < Duration::from_secs(5));
            let mut command = Command::new("powershell.exe");
            command.args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "[Console]::Write(('x' * 2200000))",
            ]);
            assert!(capture(&mut command, Duration::from_secs(10))
                .unwrap_err()
                .contains("size limit"));
        }
    }
}
pub(crate) mod service {
    //! Pairing supervisor. Startup/restoration is automatic; first Connect selection remains user-driven.
    use super::{
        soloist::{Config, Control, Soloist},
        state::Snapshot,
    };
    mod output {
        //! Observe the Windows default used by Soloist's local audio stream.
        use std::time::{Duration, Instant};
        use windows::Win32::{
            Media::Audio::{eConsole, eRender, IMMDeviceEnumerator, MMDeviceEnumerator},
            System::Com::{
                CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_ALL,
                COINIT_MULTITHREADED,
            },
        };

        pub(super) struct Watcher {
            enumerator: Option<IMMDeviceEnumerator>,
            initialized: bool,
            endpoint: Option<String>,
            checked: Instant,
        }
        impl Watcher {
            pub fn new() -> Self {
                let initialized = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).is_ok() };
                let enumerator =
                    unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok() };
                let mut watcher = Self {
                    enumerator,
                    initialized,
                    endpoint: None,
                    checked: Instant::now(),
                };
                watcher.endpoint = watcher.read().ok().flatten();
                watcher
            }
            fn read(&self) -> Result<Option<String>, windows::core::Error> {
                let Some(enumerator) = &self.enumerator else {
                    return Ok(None);
                };
                unsafe {
                    let device = match enumerator.GetDefaultAudioEndpoint(eRender, eConsole) {
                        Ok(device) => device,
                        Err(error) if error.code().0 as u32 == 0x80070490 => return Ok(None),
                        Err(error) => return Err(error),
                    };
                    let id = device.GetId()?;
                    let result = id.to_string();
                    CoTaskMemFree(Some(id.0.cast()));
                    Ok(Some(result?))
                }
            }
            pub fn changed(&mut self) -> bool {
                if self.checked.elapsed() < Duration::from_secs(1) {
                    return false;
                }
                self.checked = Instant::now();
                let Ok(endpoint) = self.read() else {
                    return false;
                };
                observe(&mut self.endpoint, endpoint)
            }
        }
        fn observe(previous: &mut Option<String>, endpoint: Option<String>) -> bool {
            let changed = endpoint.is_some() && *previous != endpoint;
            *previous = endpoint;
            changed
        }
        impl Drop for Watcher {
            fn drop(&mut self) {
                self.enumerator.take();
                if self.initialized {
                    unsafe { CoUninitialize() };
                }
            }
        }
        #[cfg(test)]
        mod tests {
            use super::observe;
            #[test]
            fn detects_switch_and_reconnection_without_restarting_on_every_poll() {
                let mut previous = Some("speakers".into());
                assert!(!observe(&mut previous, Some("speakers".into())));
                assert!(observe(&mut previous, Some("headphones".into())));
                assert!(!observe(&mut previous, None));
                assert!(observe(&mut previous, Some("headphones".into())));
                assert!(!observe(&mut previous, Some("headphones".into())));
            }
        }
    }
    use std::{
        sync::{
            atomic::{AtomicBool, Ordering},
            mpsc, Arc, Mutex, Weak,
        },
        thread::{self, JoinHandle},
        time::{Duration, Instant},
    };

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(crate) enum Phase {
        Starting,
        Connecting,
        Pairing,
        Ready,
        Failed,
        Stopped,
    }
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub(crate) struct Status {
        pub phase: Phase,
        pub message: String,
    }
    impl Status {
        fn new(phase: Phase, message: impl Into<String>) -> Self {
            Self {
                phase,
                message: message.into(),
            }
        }
    }
    pub(crate) struct Service {
        status: Arc<Mutex<Status>>,
        stop: Arc<AtomicBool>,
        worker: Option<JoinHandle<()>>,
        handle: Arc<Handle>,
    }
    static ACTIVE: Mutex<Option<Weak<Handle>>> = Mutex::new(None);
    pub(crate) fn reopen_output() -> Result<(), String> {
        let handle = ACTIVE
            .lock()
            .ok()
            .and_then(|active| active.as_ref().and_then(Weak::upgrade))
            .ok_or("Start local Spotify playback first")?;
        if matches!(handle.status().phase, Phase::Failed | Phase::Stopped) {
            return Err("Start local Spotify playback first".into());
        }
        handle.reopen_output.store(true, Ordering::Release);
        Ok(())
    }
    pub(crate) use super::process::AudioTarget;

    /// Read-only access: this never starts Soloist or extends the service lease.
    pub(crate) fn audio_target() -> Option<Arc<AudioTarget>> {
        let handle = ACTIVE.lock().ok()?.as_ref()?.upgrade()?;
        if handle.stop.load(Ordering::Acquire) {
            return None;
        }
        if !handle
            .snapshot()
            .is_some_and(|state| state.connected && state.logged_in && state.is_active)
        {
            return None;
        }
        let target = handle.audio_target.lock().ok()?.clone()?;
        target.is_alive().then_some(target)
    }

    struct PublishedTarget(Arc<Mutex<Option<Arc<AudioTarget>>>>);
    impl Drop for PublishedTarget {
        fn drop(&mut self) {
            if let Ok(mut target) = self.0.lock() {
                *target = None;
            }
        }
    }
    struct Request {
        command: Control,
        reply: mpsc::SyncSender<Result<serde_json::Value, String>>,
        deadline: Instant,
    }
    pub(crate) struct Handle {
        reopen_output: Arc<AtomicBool>,
        audio_target: Arc<Mutex<Option<Arc<AudioTarget>>>>,
        state: Arc<Mutex<Option<Arc<Snapshot>>>>,
        status: Arc<Mutex<Status>>,
        commands: mpsc::SyncSender<Request>,
        stop: Arc<AtomicBool>,
    }
    impl Drop for Handle {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
        }
    }
    impl Handle {
        pub fn snapshot(&self) -> Option<Arc<Snapshot>> {
            self.state.lock().ok()?.clone()
        }
        pub fn status(&self) -> Status {
            self.status
                .lock()
                .map(|s| s.clone())
                .unwrap_or_else(|_| Status::new(Phase::Failed, "Spotify status unavailable"))
        }
        pub fn control(&self, command: &Control) -> Result<serde_json::Value, String> {
            let (reply, rx) = mpsc::sync_channel(1);
            self.commands
                .try_send(Request {
                    command: command.clone(),
                    reply,
                    deadline: Instant::now() + Duration::from_secs(4),
                })
                .map_err(|_| "Spotify playback worker busy or unavailable")?;
            rx.recv_timeout(Duration::from_secs(5))
                .map_err(|_| "Spotify playback request timed out")?
        }
    }
    impl Service {
        pub fn start() -> Result<Self, String> {
            let mut active = ACTIVE
                .lock()
                .map_err(|_| "Spotify supervisor unavailable")?;
            if let Some(handle) = active
                .as_ref()
                .and_then(Weak::upgrade)
                .filter(|h| !matches!(h.status().phase, Phase::Failed | Phase::Stopped))
            {
                return Ok(Self {
                    status: handle.status.clone(),
                    stop: handle.stop.clone(),
                    worker: None,
                    handle,
                });
            }
            let service =
                Self::start_with(|stop| Soloist::start_cancellable(Config::load()?, stop))?;
            *active = Some(Arc::downgrade(&service.handle));
            Ok(service)
        }
        pub fn handle(&self) -> Arc<Handle> {
            self.handle.clone()
        }
        fn start_with(
            factory: impl Fn(&AtomicBool) -> Result<Soloist, String> + Send + 'static,
        ) -> Result<Self, String> {
            let status = Arc::new(Mutex::new(Status::new(
                Phase::Starting,
                "Starting Soloist...",
            )));
            let stop = Arc::new(AtomicBool::new(false));
            let output = status.clone();
            let stopped = stop.clone();
            let state = Arc::new(Mutex::new(None));
            let snapshots = state.clone();
            let audio_target = Arc::new(Mutex::new(None));
            let published_target = audio_target.clone();
            let (commands, requests) = mpsc::sync_channel::<Request>(16);
            let reopen_output = Arc::new(AtomicBool::new(false));
            let reopen_requested = reopen_output.clone();
            let handle = Arc::new(Handle {
                reopen_output,
                audio_target,
                state,
                status: status.clone(),
                commands,
                stop: stop.clone(),
            });
            let worker = thread::Builder::new()
            .name("spotify-pairing".into())
            .spawn(move || {
                let publish = |state: Status| {
                    if let Ok(mut current) = output.lock() {
                        *current = state;
                    }
                };
                // Normal startup restores saved pairing. Never use --pair, which clears that session.
                let mut output_watcher = output::Watcher::new();
                let started = factory(&stopped);
                if stopped.load(Ordering::Relaxed) {
                    drop(started);
                    publish(Status::new(Phase::Stopped, "Spotify playback stopped."));
                    return;
                }
                let mut player = match started {
                    Ok(player) => player,
                    Err(error) => {
                        publish(Status::new(Phase::Failed, error));
                        return;
                    }
                };
                // Cleared on every supervisor exit, before the player is destroyed.
                let _published = PublishedTarget(published_target);
                match player.audio_target() {
                    Ok(target) => {
                        if let Ok(mut current) = _published.0.lock() {
                            *current = Some(Arc::new(target));
                        }
                    }
                    Err(error) => {
                        publish(Status::new(Phase::Failed, error));
                        return;
                    }
                }
                let mut awaiting_since = None;
                let mut last_connected = Instant::now();
                let mut restore: Option<OutputRecovery> = None;
                while !stopped.load(Ordering::Relaxed) {
                    let requested = reopen_requested.swap(false, Ordering::AcqRel);
                    let output_changed = output_watcher.changed();
                    if requested || output_changed {
                        let previous = restore.take().map(|recovery| recovery.previous).or_else(||
                            player.snapshot().ok().filter(|s| s.connected && s.logged_in && s.is_active
                                && matches!(s.status.as_str(), "playing" | "paused")));
                        publish(Status::new(Phase::Connecting, "Reopening Spotify audio output..."));
                        if let Ok(mut target) = _published.0.lock() { *target = None; }
                        // Keep the last confirmed track visible while the output is reopened.
                        // Clearing it also makes the audio worker discard its managed queue.
                        // Release the process, monitor and exclusive data lock before restarting.
                        drop(player);
                        player = match factory(&stopped) {
                            Ok(player) => player,
                            Err(error) => { publish(Status::new(Phase::Failed, error)); return; }
                        };
                        if stopped.load(Ordering::Relaxed) { break; }
                        match player.audio_target() {
                            Ok(target) => {
                                if let Ok(mut current) = _published.0.lock() { *current = Some(Arc::new(target)); }
                            }
                            Err(error) => { publish(Status::new(Phase::Failed, error)); return; }
                        }
                        restore = previous.map(OutputRecovery::new);
                        last_connected = Instant::now();
                        awaiting_since = None;
                    }
                    let mut request = requests.recv_timeout(Duration::from_millis(150)).ok();
                    while let Some(req) = request.take() {
                        let result = if Instant::now() < req.deadline {
                            // Keep new sound settings without discarding the interrupted track.
                            match (&req.command, &mut restore) {
                                (Control::Volume(value), Some(recovery)) => recovery.previous.volume = *value,
                                (Control::Shuffle(value), Some(recovery)) => recovery.previous.shuffle = *value,
                                (Control::Repeat(value), Some(recovery)) => recovery.previous.repeat.clone_from(value),
                                (Control::Status | Control::Now | Control::Queue(_), _) => {},
                                _ => restore = None,
                            }
                            player.control(&req.command)
                        } else {
                            Err("Spotify playback request expired before dispatch".into())
                        };
                        let _ = req.reply.send(result);
                        request = requests.try_recv().ok();
                    }
                    match player.snapshot() {
                        Ok(snapshot) => {
                            if let Some(recovery) = &mut restore {
                                match recovery.advance(&snapshot, |command| player.control(command).map(|_| ())) {
                                    Ok(true) => { restore = None; continue; }
                                    Ok(false) => { thread::sleep(Duration::from_millis(150)); continue; }
                                    Err(error) => {
                                        publish(Status::new(Phase::Failed, format!("Could not restore Spotify after output change: {error}")));
                                        return;
                                    }
                                }
                            }
                            let snapshot = Arc::new(snapshot);
                            if let Ok(mut current) = snapshots.lock() {
                                *current = Some(snapshot.clone());
                            }
                            if snapshot.connected {
                                last_connected = Instant::now();
                            } else if last_connected.elapsed() >= Duration::from_secs(15) {
                                publish(Status::new(
                                    Phase::Failed,
                                    "Could not reconnect to Soloist. Try starting Spotify again.",
                                ));
                                return;
                            }
                            let mut status = connection_status(&snapshot);
                            if status.phase == Phase::Pairing {
                                let since = awaiting_since.get_or_insert_with(Instant::now);
                                if since.elapsed() < Duration::from_secs(2) {
                                    status = Status::new(
                                        Phase::Connecting,
                                        "Restoring Spotify session...",
                                    );
                                }
                            } else {
                                awaiting_since = None;
                            }
                            publish(status);
                        }
                        Err(error) => {
                            publish(Status::new(Phase::Failed, error));
                            return;
                        }
                    }
                }
                drop(_published);
                drop(player);
                publish(Status::new(Phase::Stopped, "Spotify playback stopped."));
            })
            .map_err(|_| "Could not start Spotify pairing")?;
            Ok(Self {
                status,
                stop,
                worker: Some(worker),
                handle,
            })
        }
        pub fn status(&self) -> Status {
            self.status
                .lock()
                .map(|status| status.clone())
                .unwrap_or_else(|_| {
                    Status::new(Phase::Failed, "Spotify pairing status unavailable.")
                })
        }
    }
    impl Drop for Service {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
            if let Some(worker) = self.worker.take() {
                let _ = worker.join();
            }
        }
    }
    fn connection_status(snapshot: &Snapshot) -> Status {
        if !snapshot.connected {
            Status::new(Phase::Connecting, "Connecting to Soloist...")
        } else if !snapshot.logged_in {
            Status::new(
                Phase::Pairing,
                "In Spotify on this network, select DeezNats in the device picker.",
            )
        } else {
            Status::new(Phase::Ready, "Spotify paired and ready.")
        }
    }
    struct OutputRecovery {
        previous: Snapshot,
        started: Instant,
        requested: bool,
    }
    impl OutputRecovery {
        fn new(previous: Snapshot) -> Self {
            Self {
                previous,
                started: Instant::now(),
                requested: false,
            }
        }
        fn advance(
            &mut self,
            current: &Snapshot,
            mut send: impl FnMut(&Control) -> Result<(), String>,
        ) -> Result<bool, String> {
            if self.started.elapsed() >= Duration::from_secs(15) {
                return Err("Playback restoration timed out; start Spotify again".into());
            }
            if !current.connected || !current.logged_in {
                return Ok(false);
            }
            let previous = &self.previous;
            let Some(uri) = previous.item["uri"].as_str() else {
                return Ok(true);
            };
            if !self.requested {
                send(&Control::Volume(previous.volume))?;
                send(&Control::Activate)?;
                send(&Control::Play(Some(uri.into())))?;
                self.requested = true;
                return Ok(false);
            }
            // Play is asynchronous: never seek into a stale or still-loading item.
            if current.item["uri"].as_str() != Some(uri)
                || !matches!(current.status.as_str(), "playing" | "paused")
            {
                return Ok(false);
            }
            send(&Control::Seek(
                previous.position_ms.min(u32::MAX as u64) as u32
            ))?;
            send(&Control::Shuffle(previous.shuffle))?;
            send(&Control::Repeat(previous.repeat.clone()))?;
            for entry in previous.upcoming.iter().take(80) {
                if let Some(uri) = entry["item"]["uri"]
                    .as_str()
                    .filter(|uri| uri.starts_with("spotify:track:"))
                {
                    send(&Control::AddToQueue(uri.into()))?;
                }
            }
            if previous.status == "paused" {
                send(&Control::Pause)?;
            }
            Ok(true)
        }
    }
    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn output_recovery_waits_for_auth_and_track_before_restoring_position_and_pause() {
            let previous = Snapshot {
                item: serde_json::json!({"uri":"spotify:track:1234567890123456789012"}),
                position_ms: 3456,
                volume: 42,
                shuffle: true,
                repeat: "track".into(),
                status: "paused".into(),
                ..Snapshot::default()
            };
            let mut recovery = OutputRecovery::new(previous.clone());
            let mut commands = Vec::new();
            let mut send = |command: &Control| {
                commands.push(command.clone());
                Ok(())
            };
            assert!(!recovery.advance(&Snapshot::default(), &mut send).unwrap());
            let mut current = Snapshot {
                connected: true,
                logged_in: true,
                ..Snapshot::default()
            };
            assert!(!recovery.advance(&current, &mut send).unwrap());
            assert!(!recovery.advance(&current, &mut send).unwrap());
            current.item = previous.item.clone();
            current.status = "buffering".into();
            assert!(!recovery.advance(&current, &mut send).unwrap());
            current.status = "playing".into();
            assert!(recovery.advance(&current, &mut send).unwrap());
            assert_eq!(
                commands,
                vec![
                    Control::Volume(42),
                    Control::Activate,
                    Control::Play(Some("spotify:track:1234567890123456789012".into())),
                    Control::Seek(3456),
                    Control::Shuffle(true),
                    Control::Repeat("track".into()),
                    Control::Pause,
                ]
            );
        }
        #[test]
        fn output_recovery_has_a_deadline_and_propagates_command_failure() {
            let previous = Snapshot {
                item: serde_json::json!({"uri":"spotify:track:1234567890123456789012"}),
                ..Snapshot::default()
            };
            let current = Snapshot {
                connected: true,
                logged_in: true,
                ..Snapshot::default()
            };
            let mut recovery = OutputRecovery::new(previous);
            assert_eq!(
                recovery.advance(&current, |_| Err("device unavailable".into())),
                Err("device unavailable".into())
            );
            recovery.started = Instant::now() - Duration::from_secs(16);
            assert!(recovery
                .advance(&current, |_| panic!(
                    "expired recovery must not send controls"
                ))
                .is_err());
        }
        #[test]
        fn authentication_events_determine_pairing_and_do_not_require_active_playback() {
            let mut snapshot = Snapshot::default();
            assert_eq!(connection_status(&snapshot).phase, Phase::Connecting);
            snapshot.connected = true;
            assert_eq!(connection_status(&snapshot).phase, Phase::Pairing);
            snapshot.logged_in = true;
            snapshot.is_active = false;
            assert_eq!(connection_status(&snapshot).phase, Phase::Ready);
            snapshot.connected = false;
            assert_eq!(connection_status(&snapshot).phase, Phase::Connecting);
        }
        #[test]
        fn failed_startup_is_reported_without_blocking_the_caller() {
            let mut service = Service::start_with(|_| Err("fixture failure".into())).unwrap();
            service.worker.take().unwrap().join().unwrap();
            assert_eq!(
                service.status(),
                Status::new(Phase::Failed, "fixture failure")
            );
        }
        #[test]
        fn cancellation_during_startup_cannot_publish_ready() {
            let mut service = Service::start_with(|_| {
                thread::sleep(Duration::from_millis(50));
                Err("cancelled startup".into())
            })
            .unwrap();
            service.stop.store(true, Ordering::Relaxed);
            service.worker.take().unwrap().join().unwrap();
            assert_eq!(service.status().phase, Phase::Stopped);
        }
        #[test]
        fn detects_first_pairing_and_restores_the_saved_session_on_restart() {
            use std::{
                os::windows::fs::OpenOptionsExt,
                path::PathBuf,
                process::Command,
                time::{Instant, SystemTime, UNIX_EPOCH},
            };
            struct Temporary(PathBuf);
            impl Drop for Temporary {
                fn drop(&mut self) {
                    if self.0.parent().and_then(|p| p.canonicalize().ok())
                        == std::env::temp_dir().canonicalize().ok()
                        && self
                            .0
                            .file_name()
                            .unwrap()
                            .to_string_lossy()
                            .starts_with("deeznats-pairing-test-")
                    {
                        let _ = std::fs::remove_dir_all(&self.0);
                    }
                }
            }
            let temporary = Temporary(std::env::temp_dir().join(format!(
                "deeznats-pairing-test-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            )));
            std::fs::create_dir(&temporary.0).unwrap();
            let executable = temporary.0.join("fixture.exe");
            let mut compiler = Command::new("rustc");
            compiler
                .arg("--edition=2021")
                .arg(
                    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                        .join("tests/fixtures/soloist_mock.rs"),
                )
                .arg("-o")
                .arg(&executable);
            assert!(
                super::super::process::capture(&mut compiler, Duration::from_secs(30))
                    .unwrap()
                    .0
                    .success()
            );
            let data = temporary.0.join("data");
            let start = || {
                let config = Config {
                    executable: executable.clone(),
                    api_key: "pair-later".into(),
                };
                let data = data.clone();
                Service::start_with(move |_| {
                    Soloist::start_at(config.clone(), data.clone(), Duration::from_secs(5))
                })
                .unwrap()
            };
            let mut service = start();
            let began = Instant::now();
            while service.status().phase != Phase::Pairing {
                assert!(
                    began.elapsed() < Duration::from_secs(8),
                    "{:?}",
                    service.status()
                );
                thread::sleep(Duration::from_millis(20));
            }
            std::fs::write(data.join("fixture-pairing"), "paired").unwrap();
            let began = Instant::now();
            while service.status().phase != Phase::Ready {
                assert!(began.elapsed() < Duration::from_secs(3));
                thread::sleep(Duration::from_millis(20));
            }
            let original_pid = service
                .handle
                .audio_target
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .pid;
            service.handle.reopen_output.store(true, Ordering::Release);
            let began = Instant::now();
            loop {
                let pid = service
                    .handle
                    .audio_target
                    .lock()
                    .unwrap()
                    .as_ref()
                    .map(|target| target.pid);
                if pid.is_some_and(|pid| pid != original_pid)
                    && service.status().phase == Phase::Ready
                {
                    break;
                }
                assert!(
                    began.elapsed() < Duration::from_secs(10),
                    "output recovery: {:?}",
                    service.status()
                );
                thread::sleep(Duration::from_millis(20));
            }
            let handle = service.handle();
            let target = handle.audio_target.lock().unwrap().clone().unwrap();
            drop(service);
            assert!(!target.is_alive());
            assert_eq!(handle.status().phase, Phase::Stopped);
            let lock = std::fs::OpenOptions::new()
                .write(true)
                .share_mode(0)
                .open(data.join("deeznats.lock"))
                .unwrap();
            drop(lock);
            let mut restored = start();
            let began = Instant::now();
            while restored.status().phase != Phase::Ready {
                assert_ne!(
                    restored.status().phase,
                    Phase::Pairing,
                    "saved pairing must not prompt for a new pairing"
                );
                assert!(began.elapsed() < Duration::from_secs(5));
                thread::sleep(Duration::from_millis(20));
            }
            assert_eq!(
                std::fs::read_to_string(data.join("fixture-pairing")).unwrap(),
                "paired"
            );
            restored.stop.store(true, Ordering::Relaxed);
            restored.worker.take().unwrap().join().unwrap();
        }
    }
}
pub(crate) mod setup {
    //! User-local Soloist installation automation. No network downloads or shared API keys.
    mod archive {
        //! Local ZIP import with strict Windows path, entry, expansion, and CRC limits.
        use super::build::MAX_FILE;
        use std::os::windows::fs::OpenOptionsExt;
        use std::{
            collections::HashSet,
            fs::{File, OpenOptions},
            io::{Read, Seek, SeekFrom, Write},
            path::{Path, PathBuf},
            time::{Duration, Instant},
        };
        const MAX_ARCHIVE: u64 = 512 * 1024 * 1024;
        const MAX_EXPANDED: u64 = 768 * 1024 * 1024;
        const MAX_ENTRIES: usize = 2048;

        pub(super) fn archive_file(path: &Path) -> Result<File, String> {
            let mut file = OpenOptions::new()
                .read(true)
                .share_mode(1)
                .open(path)
                .map_err(|_| "Could not open the downloaded Soloist ZIP")?;
            let size = file
                .metadata()
                .map_err(|_| "Could not inspect Soloist ZIP")?
                .len();
            if !(22..=MAX_ARCHIVE).contains(&size) {
                return Err("Soloist ZIP is empty or exceeds 512 MB".into());
            }
            // Bound directory allocation before the ZIP library reads attacker-controlled metadata.
            let length = size.min(65557) as usize;
            file.seek(SeekFrom::End(-(length as i64)))
                .map_err(|_| "Invalid ZIP trailer")?;
            let mut tail = vec![0; length];
            file.read_exact(&mut tail)
                .map_err(|_| "Truncated ZIP trailer")?;
            let offset = (0..=tail.len() - 22)
                .rev()
                .find(|i| {
                    tail[*i..*i + 4] == *b"PK\x05\x06"
                        && *i
                            + 22
                            + u16::from_le_bytes(tail[*i + 20..*i + 22].try_into().unwrap())
                                as usize
                            == tail.len()
                })
                .ok_or("Invalid ZIP directory trailer")?;
            if offset >= 20 && &tail[offset - 20..offset - 16] == b"PK\x06\x07" {
                return Err("ZIP64 Soloist archives are not supported".into());
            }
            let header = &tail[offset..offset + 22];
            let u16_at = |i| u16::from_le_bytes(header[i..i + 2].try_into().unwrap());
            let u32_at = |i| u32::from_le_bytes(header[i..i + 4].try_into().unwrap());
            if u16_at(4) != 0
                || u16_at(6) != 0
                || u16_at(8) != u16_at(10)
                || !(1..=MAX_ENTRIES).contains(&(u16_at(10) as usize))
                || u32_at(12) > 2 * 1024 * 1024
                || u32_at(16) as u64 + u32_at(12) as u64 > size
            {
                return Err(
                    "Unsupported multi-volume, ZIP64, or excessive Soloist ZIP directory".into(),
                );
            }
            file.seek(SeekFrom::Start(0))
                .map_err(|_| "Could not rewind Soloist ZIP")?;
            Ok(file)
        }
        pub(super) fn safe_relative(name: &str) -> Result<PathBuf, String> {
            if name.is_empty() || name.len() > 220 || !name.is_ascii() {
                return Err("Unsupported Soloist ZIP path".into());
            }
            let normalized = name.replace('\\', "/");
            let normalized = normalized.strip_suffix('/').unwrap_or(&normalized);
            let mut path = PathBuf::new();
            for component in normalized.split('/') {
                let stem = component
                    .split('.')
                    .next()
                    .unwrap_or_default()
                    .trim_end_matches(['.', ' '])
                    .to_ascii_uppercase();
                let reserved = matches!(
                    stem.as_str(),
                    "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$" | "CLOCK$"
                ) || (stem.len() == 4
                    && (stem.starts_with("COM") || stem.starts_with("LPT"))
                    && matches!(stem.as_bytes()[3], b'1'..=b'9'));
                if component.is_empty()
                    || matches!(component, "." | "..")
                    || component.ends_with(['.', ' '])
                    || component
                        .chars()
                        .any(|c| c.is_control() || "<>:\"|?*".contains(c))
                    || reserved
                {
                    return Err("Unsafe Windows path in Soloist ZIP".into());
                }
                path.push(component);
            }
            Ok(path)
        }
        pub(super) fn extract(file: File, destination: &Path) -> Result<PathBuf, String> {
            let mut zip =
                zip::ZipArchive::new(file).map_err(|_| "Invalid or unsupported Soloist ZIP")?;
            if zip.is_empty()
                || zip.len() > MAX_ENTRIES
                || zip.has_duplicate_names()
                || zip
                    .has_overlapping_files()
                    .map_err(|_| "Invalid ZIP file layout")?
            {
                return Err("Soloist ZIP has duplicate, overlapping, or excessive entries".into());
            }
            let mut names = HashSet::new();
            let mut files = Vec::new();
            let mut declared = 0u64;
            let mut executable = None;
            // Validate every entry before writing the first one.
            for i in 0..zip.len() {
                let entry = zip
                    .by_index(i)
                    .map_err(|_| "Unreadable or encrypted Soloist ZIP entry")?;
                let name = entry
                    .name()
                    .map_err(|_| "Soloist ZIP filename is not supported")?
                    .into_owned();
                let relative = safe_relative(&name)?;
                let kind = entry.unix_mode().unwrap_or(0) & 0o170000;
                if entry.encrypted()
                    || entry.is_symlink()
                    || !matches!(kind, 0 | 0o100000 | 0o040000)
                {
                    return Err(
                        "Soloist ZIP must not contain links, devices, or encrypted entries".into(),
                    );
                }
                let key = relative
                    .to_string_lossy()
                    .replace('\\', "/")
                    .to_ascii_lowercase();
                if !names.insert(key) {
                    return Err("Soloist ZIP contains Windows filename collisions".into());
                }
                if entry.size() > MAX_FILE {
                    return Err("Soloist ZIP entry exceeds 256 MB".into());
                }
                declared = declared
                    .checked_add(entry.size())
                    .filter(|v| *v <= MAX_EXPANDED)
                    .ok_or("Soloist ZIP expands beyond 768 MB")?;
                if !entry.is_dir()
                    && relative
                        .file_name()
                        .is_some_and(|s| s.eq_ignore_ascii_case("soloist.exe"))
                {
                    if executable.replace(relative.clone()).is_some() {
                        return Err("Soloist ZIP contains multiple soloist.exe files".into());
                    }
                }
                files.push((i, relative, entry.is_dir(), entry.size()));
            }
            let executable = executable.ok_or("Soloist ZIP does not contain soloist.exe")?;
            let started = Instant::now();
            let mut total = 0u64;
            for (i, relative, directory, expected_size) in files {
                let path = destination.join(relative);
                if directory {
                    std::fs::create_dir_all(&path)
                        .map_err(|_| "Could not create Soloist package directory")?;
                    continue;
                }
                std::fs::create_dir_all(path.parent().unwrap())
                    .map_err(|_| "Could not create Soloist package directory")?;
                let mut output = OpenOptions::new()
                    .create_new(true)
                    .write(true)
                    .open(path)
                    .map_err(|_| "Could not create Soloist package file")?;
                let mut input = zip
                    .by_index(i)
                    .map_err(|_| "Could not read Soloist ZIP file")?;
                let mut count = 0u64;
                let mut bytes = [0u8; 65536];
                loop {
                    if started.elapsed() > Duration::from_secs(30) {
                        return Err("Soloist ZIP extraction timed out".into());
                    }
                    let n = input
                        .read(&mut bytes)
                        .map_err(|_| "Soloist ZIP is corrupt or failed its CRC check")?;
                    if n == 0 {
                        break;
                    }
                    count += n as u64;
                    total += n as u64;
                    if count > expected_size || total > MAX_EXPANDED {
                        return Err("Soloist ZIP exceeded its declared expansion limit".into());
                    }
                    output
                        .write_all(&bytes[..n])
                        .map_err(|_| "Could not write Soloist package file")?;
                }
                if count != expected_size {
                    return Err("Soloist ZIP entry size did not match its directory".into());
                }
                output
                    .sync_all()
                    .map_err(|_| "Could not flush Soloist package file")?;
            }
            Ok(executable)
        }
        #[cfg(test)]
        mod tests {
            use super::*;
            #[test]
            fn rejects_windows_escapes_devices_and_aliases() {
                for name in [
                    "../soloist.exe",
                    "C:/soloist.exe",
                    "/soloist.exe",
                    "a/../soloist.exe",
                    "a\\..\\soloist.exe",
                    "//server/share/soloist.exe",
                    "soloist.exe:stream",
                    "NUL.txt",
                    "NUL .txt",
                    "CONIN$",
                    "CONOUT$",
                    "CLOCK$",
                    "COM1",
                    "LPT9.log",
                    "foo./soloist.exe",
                    "foo /soloist.exe",
                    "a//soloist.exe",
                ] {
                    assert!(safe_relative(name).is_err(), "{name}");
                }
                assert_eq!(
                    safe_relative("package/soloist.exe").unwrap(),
                    PathBuf::from("package/soloist.exe")
                );
            }
        }
    }
    mod build {
        //! Static Windows executable checks and bounded Soloist --version inspection.
        use super::super::process;
        use serde::{Deserialize, Serialize};
        use sha2::{Digest, Sha256};
        use std::{
            fs::File,
            io::{Read, Seek, SeekFrom},
            path::{Path, PathBuf},
            process::Command,
            time::{Duration, SystemTime, UNIX_EPOCH},
        };
        const LIFETIME: u64 = 90 * 86400;
        pub(super) const MAX_FILE: u64 = 256 * 1024 * 1024;

        pub(crate) fn strip_verbatim(path: &Path) -> PathBuf {
            use std::path::{Component, Prefix};
            let mut components = path.components();
            if let Some(Component::Prefix(prefix)) = components.next() {
                match prefix.kind() {
                    Prefix::VerbatimDisk(drive) => {
                        let mut result = PathBuf::from(format!("{}:\\", drive as char));
                        result.push(components.as_path());
                        return result;
                    }
                    Prefix::VerbatimUNC(server, share) => {
                        let mut result = PathBuf::from(r"\\");
                        result.push(server);
                        result.push(share);
                        result.push(components.as_path());
                        return result;
                    }
                    _ => {}
                }
            }
            path.to_path_buf()
        }

        #[derive(Clone, Debug, Serialize, Deserialize)]
        pub(crate) struct Build {
            pub executable: PathBuf,
            pub sha256: String,
            pub version_output: String,
            pub build_timestamp: Option<u64>,
            pub expires_at: Option<u64>,
            pub days_remaining: Option<u64>,
            pub expiry: String,
        }
        impl Build {
            pub fn at(&mut self, now: u64) {
                self.expires_at = self.build_timestamp.and_then(|t| t.checked_add(LIFETIME));
                self.days_remaining = self
                    .expires_at
                    .map(|expiry| expiry.saturating_sub(now).div_ceil(86400));
                self.expiry = match (self.build_timestamp, self.expires_at) {
                    (Some(build), _) if build > now.saturating_add(86400) => {
                        "clock-or-build-date-invalid"
                    }
                    (_, Some(expiry)) if expiry <= now => "expired",
                    (_, Some(expiry)) if expiry - now <= 14 * 86400 => "expiring-soon",
                    (_, Some(_)) => "current",
                    _ => "unknown",
                }
                .into();
            }
            pub fn usable(&self) -> bool {
                !matches!(
                    self.expiry.as_str(),
                    "expired" | "clock-or-build-date-invalid"
                )
            }
        }
        pub(super) fn now() -> u64 {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        }
        pub(super) fn pe_x64(path: &Path) -> Result<(), String> {
            let mut file = File::open(path).map_err(|_| "Could not open the Soloist executable")?;
            let size = file
                .metadata()
                .map_err(|_| "Could not inspect the Soloist executable")?
                .len();
            if !(128..=MAX_FILE).contains(&size) {
                return Err("Soloist executable has an invalid size".into());
            }
            let mut dos = [0u8; 64];
            file.read_exact(&mut dos)
                .map_err(|_| "Truncated Soloist executable")?;
            if &dos[..2] != b"MZ" {
                return Err("Soloist must be a Windows x64 executable".into());
            }
            let offset = u32::from_le_bytes(dos[60..64].try_into().unwrap()) as u64;
            if offset < 64 || offset.saturating_add(26) > size || offset > 1024 * 1024 {
                return Err("Invalid Soloist PE header".into());
            }
            file.seek(SeekFrom::Start(offset))
                .map_err(|_| "Invalid Soloist PE offset")?;
            let mut pe = [0u8; 26];
            file.read_exact(&mut pe)
                .map_err(|_| "Truncated Soloist PE header")?;
            if &pe[..4] != b"PE\0\0"
                || u16::from_le_bytes(pe[4..6].try_into().unwrap()) != 0x8664
                || u16::from_le_bytes(pe[24..26].try_into().unwrap()) != 0x20b
                || u16::from_le_bytes(pe[22..24].try_into().unwrap()) & 0x2000 != 0
            {
                return Err(
                    "Soloist must be an executable Windows x64 PE32+ build, not a DLL".into(),
                );
            }
            Ok(())
        }
        pub(super) fn digest(path: &Path) -> Result<String, String> {
            let mut file =
                File::open(path).map_err(|_| "Could not open Soloist installation file")?;
            let mut hash = Sha256::new();
            let mut bytes = [0u8; 65536];
            loop {
                let count = file
                    .read(&mut bytes)
                    .map_err(|_| "Could not hash Soloist installation file")?;
                if count == 0 {
                    break;
                }
                hash.update(&bytes[..count]);
            }
            Ok(format!("{:x}", hash.finalize()))
        }
        /// Only call for a user-selected/imported binary. Discovery never executes candidates.
        pub(crate) fn inspect(path: &Path) -> Result<Build, String> {
            pe_x64(path)?;
            let canonical = path
                .canonicalize()
                .map_err(|_| "Could not resolve Soloist executable")?;
            let path = strip_verbatim(&canonical);
            let mut command = Command::new(&path);
            command
                .arg("--version")
                .env_remove("DEEZNATS_SOLOIST_API_KEY")
                .env_remove("DEEZNATS_SPOTIFY_CLIENT_ID");
            let (status, bytes) = process::capture(&mut command, Duration::from_secs(3))?;
            if status.code() == Some(10) {
                return Err("Soloist build expired; download a new build from Spotify".into());
            }
            if !status.success() || bytes.len() > 16384 {
                return Err("Soloist --version failed or returned excessive output".into());
            }
            let text = std::str::from_utf8(&bytes)
                .map_err(|_| "Soloist version output is not UTF-8")?
                .trim();
            if !text.to_ascii_lowercase().contains("soloist")
                || text
                    .chars()
                    .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
            {
                return Err("Executable did not identify itself as Soloist".into());
            }
            let mut build = Build {
                executable: path.clone(),
                sha256: digest(&path)?,
                version_output: text.into(),
                build_timestamp: build_timestamp(text),
                expires_at: None,
                days_remaining: None,
                expiry: String::new(),
            };
            build.at(now());
            Ok(build)
        }
        fn build_timestamp(text: &str) -> Option<u64> {
            let mut found = None;
            for line in text.lines() {
                let lower = line.to_ascii_lowercase();
                for key in [
                    "build timestamp",
                    "build_timestamp",
                    "build-timestamp",
                    "build date",
                    "build_date",
                    "built at",
                    "built_at",
                    "build ",
                    "build",
                ] {
                    if let Some(index) = lower.find(key) {
                        let value = line[index + key.len()..]
                            .trim_start_matches([' ', ':', '=', '\t', '"']);
                        if let Some(timestamp) = timestamp(value) {
                            if found.is_some_and(|prior| prior != timestamp) {
                                return None;
                            }
                            found = Some(timestamp);
                        }
                        break;
                    }
                }
            }
            found
        }
        fn timestamp(text: &str) -> Option<u64> {
            let text = text.trim();
            let digits: String = text.chars().take_while(char::is_ascii_digit).collect();
            if digits.len() == 10 || digits.len() == 13 {
                let rest = text[digits.len()..].trim_start();
                if rest.is_empty()
                    || rest.starts_with('(')
                    || rest.starts_with('g')
                    || rest.starts_with('/')
                    || rest.starts_with('-')
                    || rest.starts_with('"')
                {
                    let value = digits.parse::<u64>().ok()?;
                    return Some(if digits.len() == 13 {
                        value / 1000
                    } else {
                        value
                    });
                }
            }
            // Only explicit UTC dates are accepted; unknown version formats stay unknown.
            let bytes = text.as_bytes();
            if bytes.len() < 20
                || bytes[4] != b'-'
                || bytes[7] != b'-'
                || !matches!(bytes[10], b'T' | b' ')
                || bytes[13] != b':'
                || bytes[16] != b':'
            {
                return None;
            }
            let field = |start: usize, end: usize| -> Option<i64> {
                std::str::from_utf8(bytes.get(start..end)?)
                    .ok()?
                    .parse()
                    .ok()
            };
            let (year, month, day, hour, minute, second) = (
                field(0, 4)?,
                field(5, 7)?,
                field(8, 10)?,
                field(11, 13)?,
                field(14, 16)?,
                field(17, 19)?,
            );
            let zone = &text[19..];
            if !matches!(zone, "Z" | "+00:00" | " UTC") {
                return None;
            }
            if !(1970..=9999).contains(&year)
                || !(1..=12).contains(&month)
                || !(0..=23).contains(&hour)
                || !(0..=59).contains(&minute)
                || !(0..=59).contains(&second)
            {
                return None;
            }
            let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
            let month_days = [
                31,
                if leap { 29 } else { 28 },
                31,
                30,
                31,
                30,
                31,
                31,
                30,
                31,
                30,
                31,
            ];
            if day < 1 || day > month_days[(month - 1) as usize] {
                return None;
            }
            // Gregorian civil date to days since 1970-01-01 (400-year eras).
            let y = year - i64::from(month <= 2);
            let era = y.div_euclid(400);
            let yoe = y - era * 400;
            let m = month + if month > 2 { -3 } else { 9 };
            let doy = (153 * m + 2) / 5 + day - 1;
            let days = era * 146097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719468;
            u64::try_from(days * 86400 + hour * 3600 + minute * 60 + second).ok()
        }
        #[cfg(test)]
        mod tests {
            use super::*;
            #[test]
            fn normalizes_windows_paths_without_losing_unc_roots() {
                for (input, expected) in [
                    (r"\\?\C:\Music\soloist.exe", r"C:\Music\soloist.exe"),
                    (
                        r"\\?\UNC\server\share\soloist.exe",
                        r"\\server\share\soloist.exe",
                    ),
                    (r"C:\Music\soloist.exe", r"C:\Music\soloist.exe"),
                    (r"\\server\share\soloist.exe", r"\\server\share\soloist.exe"),
                ] {
                    assert_eq!(strip_verbatim(Path::new(input)), PathBuf::from(expected));
                }
            }
            #[test]
            fn parses_only_explicit_build_dates_and_handles_expiry_boundaries() {
                assert_eq!(timestamp("1970-01-01T00:00:00Z"), Some(0));
                assert_eq!(timestamp("2000-02-29T00:00:00Z"), Some(951782400));
                assert_eq!(timestamp("1900-02-29T00:00:00Z"), None);
                assert_eq!(timestamp("2026-02-29T00:00:00Z"), None);
                assert_eq!(timestamp("2026-01-01T00:00:00+03:00"), None);
                assert_eq!(
                    build_timestamp("Soloist version 1.2.3\nBuild timestamp: 1700000000000"),
                    Some(1700000000)
                );
                assert_eq!(build_timestamp("Soloist copyright 2026"), None);
                assert_eq!(
                    build_timestamp("Build timestamp: 1700000000\nBuild date: 1800000000"),
                    None
                );
                assert_eq!(
            build_timestamp(
                "soloist 1.3.10.21 build 1793124080 (20261027) (g19412df6ea) (windows/x86_64)"
            ),
            Some(1793124080)
        );
                let mut build = Build {
                    executable: PathBuf::new(),
                    sha256: String::new(),
                    version_output: String::new(),
                    build_timestamp: Some(1700000000),
                    expires_at: None,
                    days_remaining: None,
                    expiry: String::new(),
                };
                build.at(1700000000 + LIFETIME - 1);
                assert_eq!(build.expiry, "expiring-soon");
                build.at(1700000000 + LIFETIME);
                assert!(!build.usable());
                assert_eq!(build.days_remaining, Some(0));
                build.build_timestamp = None;
                build.at(1700000000);
                assert_eq!(build.expiry, "unknown");
                assert!(build.usable());
            }
        }
    }
    use super::soloist::Config;
    pub(crate) use build::{strip_verbatim, Build};
    use serde::{Deserialize, Serialize};
    use std::{
        collections::HashSet,
        fs::{File, OpenOptions},
        io::{Read, Write},
        os::windows::fs::OpenOptionsExt,
        path::{Path, PathBuf},
        sync::atomic::{AtomicU64, Ordering},
    };

    pub(crate) const DOWNLOADS: &str =
        "https://developer.spotify.com/documentation/soloist/reference/downloads-and-updates";
    pub(crate) const KEY_PAGE: &str = "https://developer.spotify.com/dashboard/soloist";
    const RECEIPT: &str = "deeznats-receipt.json";
    #[derive(Clone, Debug, Serialize, Deserialize)]
    pub(crate) struct Installation {
        pub id: String,
        pub executable: PathBuf,
        pub build: Build,
        pub imported_at: u64,
    }
    #[derive(Clone, Debug, Serialize, Deserialize)]
    struct Receipt {
        id: String,
        relative_executable: PathBuf,
        build: Build,
        imported_at: u64,
    }
    #[derive(Clone, Debug, Serialize, Deserialize)]
    struct Active {
        id: String,
        previous: Option<String>,
    }
    #[derive(Clone, Debug, Serialize)]
    pub(crate) struct Candidate {
        pub executable: PathBuf,
        pub source: String,
        pub compatible: bool,
        pub error: Option<String>,
    }
    #[derive(Serialize)]
    pub(crate) struct Doctor {
        pub candidates: Vec<Candidate>,
        pub managed_builds: Vec<Installation>,
        pub active: Option<Installation>,
        pub configuration_error: Option<String>,
        pub installation_error: Option<String>,
        pub api_key_configured: bool,
        pub downloads_url: &'static str,
        pub key_url: &'static str,
        pub notes: Vec<&'static str>,
    }
    fn root() -> Result<PathBuf, String> {
        Ok(crate::storage::data_dir()?.join("spotify-soloist-install"))
    }
    fn session_directory() -> Result<PathBuf, String> {
        Ok(crate::storage::data_dir()?.join("spotify-soloist"))
    }
    fn valid_id(id: &str) -> bool {
        id.len() == 64
            && id
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    }
    fn bounded_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, String> {
        let mut bytes = Vec::new();
        File::open(path)
            .map_err(|_| "Soloist installation metadata is unavailable")?
            .take(65537)
            .read_to_end(&mut bytes)
            .map_err(|_| "Could not read Soloist installation metadata")?;
        if bytes.len() > 65536 {
            return Err("Soloist installation metadata exceeds its size limit".into());
        }
        serde_json::from_slice(&bytes).map_err(|_| "Invalid Soloist installation metadata".into())
    }
    fn save_json(path: &Path, value: &impl Serialize) -> Result<(), String> {
        let bytes = serde_json::to_vec(value)
            .map_err(|_| "Could not encode Soloist installation metadata")?;
        let temporary = path.with_extension(format!("{}.tmp", nonce()));
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .map_err(|_| "Could not stage Soloist installation metadata")?;
        let result = (|| {
            file.write_all(&bytes)
                .and_then(|_| file.sync_all())
                .map_err(|_| "Could not write Soloist installation metadata")?;
            drop(file);
            std::fs::rename(&temporary, path)
                .map_err(|_| "Could not activate Soloist installation metadata".to_owned())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(temporary);
        }
        result
    }
    fn nonce() -> String {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        format!(
            "{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        )
    }
    fn lock(directory: &Path, name: &str) -> Result<File, String> {
        std::fs::create_dir_all(directory)
            .map_err(|_| "Could not prepare Soloist management directory")?;
        OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .share_mode(0)
            .open(directory.join(name))
            .map_err(|_| "Soloist installation or playback is busy in another session".into())
    }
    fn load_at(root: &Path, id: &str) -> Result<Installation, String> {
        if !valid_id(id) {
            return Err("Invalid Soloist installation ID".into());
        }
        let directory = root.join("builds").join(id);
        let receipt: Receipt = bounded_json(&directory.join(RECEIPT))?;
        if receipt.id != id
            || archive::safe_relative(
                receipt
                    .relative_executable
                    .to_str()
                    .ok_or("Invalid Soloist executable path")?,
            )? != receipt.relative_executable
            || !receipt
                .relative_executable
                .file_name()
                .is_some_and(|name| name.eq_ignore_ascii_case("soloist.exe"))
        {
            return Err("Invalid Soloist installation receipt".into());
        }
        let executable = directory.join("payload").join(&receipt.relative_executable);
        let executable = build::strip_verbatim(&executable);
        build::pe_x64(&executable)?;
        if build::digest(&executable)? != receipt.build.sha256 {
            return Err(
            "Installed Soloist executable changed since import; import the official archive again"
                .into(),
        );
        }
        let mut build = receipt.build;
        build.executable = executable.clone();
        build.at(build::now());
        Ok(Installation {
            id: id.into(),
            executable,
            build,
            imported_at: receipt.imported_at,
        })
    }
    fn active_at(root: &Path) -> Result<Option<Installation>, String> {
        if !root.join("active.json").exists() {
            return Ok(None);
        }
        let active: Active = bounded_json(&root.join("active.json"))?;
        load_at(root, &active.id).map(Some)
    }
    /// Check the official build at most once per UTC day. A pending build is
    /// activated when the playback lock is available, without interrupting audio.
    pub(crate) fn background_update() -> Result<(), String> {
        if Config::saved().is_err() || std::env::var_os("DEEZNATS_SOLOIST_PATH").is_some() {
            return Ok(());
        }
        let root = root()?;
        let _guard = lock(&root, "update-check.lock")?;
        let pending = root.join("pending-update.json");
        if let Ok(id) = bounded_json::<String>(&pending) {
            if activate(&id).is_ok() {
                let _ = std::fs::remove_file(&pending);
            }
        }
        let Some(current) = active_at(&root)? else {
            return Ok(());
        };
        // An explicit local executable overrides the managed build.
        if preferred_executable()? != current.executable {
            return Ok(());
        }
        let today = build::now() / 86_400;
        let marker = root.join("update-day.json");
        if bounded_json::<u64>(&marker).ok() == Some(today) {
            return Ok(());
        }
        save_json(&marker, &today)?;
        let path = root.join(format!("download-{}.zip", nonce()));
        let result = (|| {
            let mut file = File::create(&path).map_err(|_| "Could not prepare Soloist update")?;
            crate::updater::http::get(
                "https://soloist-builds.spotifycdn.com/soloist_release_windows_x86_64.zip",
                &mut file,
                512 * 1024 * 1024,
                |_| {},
            )?;
            drop(file);
            let new = import_archive(&path)?;
            if new.id != current.id
                && matches!((new.build.build_timestamp, current.build.build_timestamp), (Some(a), Some(b)) if a > b)
            {
                save_json(&pending, &new.id)?;
                if activate(&new.id).is_ok() {
                    let _ = std::fs::remove_file(&pending);
                }
            }
            Ok(())
        })();
        let _ = std::fs::remove_file(path);
        result
    }

    pub(crate) fn active_executable() -> Result<Option<PathBuf>, String> {
        active_at(&root()?).map(|installation| installation.map(|i| i.executable))
    }
    /// Candidate discovery reads headers only. It never runs executables from PATH.
    pub(crate) fn discover() -> Result<Vec<Candidate>, String> {
        let mut paths = Vec::new();
        if let Some(path) = std::env::var_os("DEEZNATS_SOLOIST_PATH") {
            paths.push((PathBuf::from(path), "environment"));
        }
        if let Some(installation) = active_at(&root()?)? {
            paths.push((installation.executable, "managed"));
        }
        if let Ok(config) = Config::saved() {
            paths.push((config.executable, "saved configuration"));
        }
        if let Ok(root) = root() {
            if let Ok(managed) = installations(&root) {
                for installation in managed {
                    paths.push((installation.executable, "managed"));
                }
            }
        }
        if let Ok(current) = std::env::current_dir() {
            let path = current.join("soloist.exe");
            if path.is_file() {
                paths.push((path, "working directory"));
            }
        }
        if let Some(path) = std::env::var_os("PATH") {
            for directory in std::env::split_paths(&path)
                .filter(|p| p.is_absolute())
                .take(128)
            {
                let path = directory.join("soloist.exe");
                if path.is_file() {
                    paths.push((path, "PATH"));
                }
            }
        }
        if let Ok(path) = std::env::current_exe() {
            if let Some(parent) = path.parent() {
                let path = parent.join("soloist.exe");
                if path.is_file() {
                    paths.push((path, "application directory"));
                }
            }
        }
        let mut seen = HashSet::new();
        let mut result = Vec::new();
        for (path, source) in paths {
            let path = if path.is_absolute() {
                path
            } else {
                std::env::current_dir()
                    .map_err(|_| "Could not resolve Soloist path")?
                    .join(path)
            };
            let path = build::strip_verbatim(&path);
            let key = path.to_string_lossy().to_ascii_lowercase();
            if !seen.insert(key) {
                continue;
            }
            let error = build::pe_x64(&path).err();
            result.push(Candidate {
                executable: path,
                source: source.into(),
                compatible: error.is_none(),
                error,
            });
        }
        Ok(result)
    }
    struct Staging {
        path: PathBuf,
        parent: PathBuf,
    }
    impl Staging {
        fn new(root: &Path) -> Result<Self, String> {
            let parent = root.join("staging");
            std::fs::create_dir_all(&parent)
                .map_err(|_| "Could not prepare Soloist staging directory")?;
            let parent = parent
                .canonicalize()
                .map_err(|_| "Could not resolve Soloist staging directory")?;
            let path = parent.join(nonce());
            std::fs::create_dir(&path).map_err(|_| "Could not create Soloist staging directory")?;
            Ok(Self { path, parent })
        }
    }
    impl Drop for Staging {
        fn drop(&mut self) {
            // Remove only the unique directory this importer created, inside the checked staging root.
            if self.path.parent() == Some(self.parent.as_path())
                && self.path.canonicalize().ok().as_ref() == Some(&self.path)
                && std::fs::symlink_metadata(&self.path).is_ok_and(|m| !m.file_type().is_symlink())
            {
                let _ = std::fs::remove_dir_all(&self.path);
            }
        }
    }
    fn import_at(root: &Path, path: &Path) -> Result<Installation, String> {
        let _guard = lock(root, "setup.lock")?;
        let archive = archive::archive_file(path)?;
        let id = build::digest(path)?;
        if root.join("builds").join(&id).exists() {
            return load_at(root, &id);
        }
        let stage = Staging::new(root)?;
        let payload = stage.path.join("payload");
        std::fs::create_dir(&payload).map_err(|_| "Could not create Soloist package directory")?;
        let relative_executable = archive::extract(archive, &payload)?;
        let mut version = build::inspect(&payload.join(&relative_executable))?;
        if !version.usable() {
            return Err(
                "Imported Soloist build is expired or has an invalid future build date".into(),
            );
        }
        let destination = root.join("builds").join(&id);
        version.executable =
            build::strip_verbatim(&destination.join("payload").join(&relative_executable));
        let receipt = Receipt {
            id: id.clone(),
            relative_executable,
            build: version,
            imported_at: build::now(),
        };
        save_json(&stage.path.join(RECEIPT), &receipt)?;
        std::fs::create_dir_all(root.join("builds"))
            .map_err(|_| "Could not create Soloist build directory")?;
        std::fs::rename(&stage.path, &destination)
            .map_err(|_| "Could not commit staged Soloist installation")?;
        load_at(root, &id)
    }
    pub(crate) fn import_archive(path: &Path) -> Result<Installation, String> {
        import_at(&root()?, path)
    }
    fn activate_at(root: &Path, session: &Path, id: &str) -> Result<Installation, String> {
        let _guard = lock(root, "setup.lock")?;
        let _playback_guard = lock(session, "deeznats.lock")?;
        let installation = load_at(root, id)?;
        if !installation.build.usable() {
            return Err("Cannot activate an expired Soloist build or an invalid build date".into());
        }
        let previous = if root.join("active.json").exists() {
            let active: Active = bounded_json(&root.join("active.json"))?;
            if active.id == id {
                return Ok(installation);
            }
            Some(active.id)
        } else {
            None
        };
        save_json(
            &root.join("active.json"),
            &Active {
                id: id.into(),
                previous,
            },
        )?;
        Ok(installation)
    }
    pub(crate) fn activate(id: &str) -> Result<Installation, String> {
        activate_at(&root()?, &session_directory()?, id)
    }
    pub(crate) fn rollback() -> Result<Installation, String> {
        rollback_at(&root()?, &session_directory()?)
    }
    fn rollback_at(root: &Path, session: &Path) -> Result<Installation, String> {
        let active: Active = bounded_json(&root.join("active.json"))?;
        activate_at(
            root,
            session,
            &active
                .previous
                .ok_or("No previous Soloist build is recorded")?,
        )
    }
    /// Resolve a selected/managed/discovered executable without requiring an API key.
    pub(crate) fn preferred_executable() -> Result<PathBuf, String> {
        if let Some(path) = std::env::var_os("DEEZNATS_SOLOIST_PATH") {
            return Ok(build::strip_verbatim(&PathBuf::from(path)));
        }
        let root = root()?;
        let saved = Config::saved().ok().map(|config| config.executable);
        if let Some(path) = selected_at(&root, saved.as_deref())? {
            return Ok(path);
        }
        discover()?
        .into_iter()
        .find(|c| c.compatible)
        .map(|c| build::strip_verbatim(&c.executable))
        .ok_or_else(|| {
            "No Soloist executable found; import and activate the official ZIP or set DEEZNATS_SOLOIST_PATH"
                .into()
        })
    }
    fn selected_at(root: &Path, saved: Option<&Path>) -> Result<Option<PathBuf>, String> {
        // A manually selected executable is authoritative. Saved managed paths follow
        // active.json instead, so upgrades and rollback retain receipt verification.
        if let Some(path) = saved.map(build::strip_verbatim) {
            if !path.starts_with(build::strip_verbatim(&root.join("builds"))) && path.is_file() {
                return Ok(Some(path));
            }
        }
        active_at(root).map(|installation| installation.map(|i| i.executable))
    }
    pub(crate) fn check(path: &Path) -> Result<Build, String> {
        build::inspect(path)
    }
    fn installations(root: &Path) -> Result<Vec<Installation>, String> {
        let directory = root.join("builds");
        if !directory.exists() {
            return Ok(Vec::new());
        }
        let entries =
            std::fs::read_dir(directory).map_err(|_| "Could not list Soloist installations")?;
        let mut result = Vec::new();
        for entry in entries.take(128) {
            let entry = entry.map_err(|_| "Could not list Soloist installation")?;
            if let Some(id) = entry.file_name().to_str().filter(|id| valid_id(id)) {
                result.push(load_at(root, id)?);
            }
        }
        result.sort_by(|a, b| b.imported_at.cmp(&a.imported_at));
        Ok(result)
    }
    pub(crate) fn doctor() -> Result<Doctor, String> {
        let root = root()?;
        let saved = Config::saved().ok();
        let api_key_configured = std::env::var("DEEZNATS_SOLOIST_API_KEY")
            .ok()
            .is_some_and(|key| !key.trim().is_empty())
            || saved.as_ref().is_some_and(|c| !c.api_key.trim().is_empty());
        let configuration_error = Config::load().err();
        let active_result = active_at(&root);
        let list_result = installations(&root);
        let installation_error = active_result
            .as_ref()
            .err()
            .cloned()
            .or_else(|| list_result.as_ref().err().cloned());
        Ok(Doctor { candidates:discover().unwrap_or_default(),managed_builds:list_result.unwrap_or_default(),active:active_result.unwrap_or_default(),configuration_error,installation_error,api_key_configured,downloads_url:DOWNLOADS,key_url:KEY_PAGE,
        notes:vec!["Generate your own Soloist API key; Premium is required for initial setup.","Download Soloist yourself from Spotify, then import that local ZIP. DeezNats does not distribute it.","Import/check executes the selected local binary with --version. Only use a download you trust from Spotify.","SHA-256 detects changes after import; it does not authenticate the publisher.","Expiry is estimated only from an explicit build timestamp. Unknown formats remain unknown; Soloist exit code 10 is authoritative.","Pair by selecting DeezNats in Spotify Connect. API login and captured cookies do not pair Soloist.","Activate updates only after stopping playback; the paired-session and cache directories are preserved."] })
    }
    pub(crate) fn open_page(key: bool) -> Result<(), String> {
        use windows_sys::Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL};
        let wide = |value: &str| value.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
        let action = wide("open");
        let page = wide(if key { KEY_PAGE } else { DOWNLOADS });
        let result = unsafe {
            ShellExecuteW(
                std::ptr::null_mut(),
                action.as_ptr(),
                page.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                SW_SHOWNORMAL,
            )
        };
        if result as isize <= 32 {
            Err("Could not open Spotify's setup page in your browser".into())
        } else {
            Ok(())
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::{io::Write, process::Command, time::Duration};
        use zip::{write::SimpleFileOptions, CompressionMethod, ZipWriter};
        struct Temporary(PathBuf);
        impl Temporary {
            fn new() -> Self {
                let path =
                    std::env::temp_dir().join(format!("deeznats-soloist-setup-test-{}", nonce()));
                std::fs::create_dir(&path).unwrap();
                Self(path.canonicalize().unwrap())
            }
        }
        impl Drop for Temporary {
            fn drop(&mut self) {
                if self.0.parent() == Some(std::env::temp_dir().canonicalize().unwrap().as_path())
                    && self
                        .0
                        .file_name()
                        .unwrap()
                        .to_string_lossy()
                        .starts_with("deeznats-soloist-setup-test-")
                {
                    let _ = std::fs::remove_dir_all(&self.0);
                }
            }
        }
        fn zip_file(path: &Path, files: &[(&str, &[u8])]) {
            let mut writer = ZipWriter::new(File::create(path).unwrap());
            for (name, bytes) in files {
                writer
                    .start_file(
                        *name,
                        SimpleFileOptions::default().compression_method(CompressionMethod::Stored),
                    )
                    .unwrap();
                writer.write_all(bytes).unwrap();
            }
            writer.finish().unwrap();
        }
        #[test]
        fn import_activation_update_and_rollback_preserve_paired_state() {
            let temp = Temporary::new();
            let executable = temp.0.join("fixture.exe");
            let mut compiler = Command::new("rustc");
            compiler
                .arg("--edition=2021")
                .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/soloist_mock.rs"))
                .arg("-o")
                .arg(&executable);
            assert!(
                super::super::process::capture(&mut compiler, Duration::from_secs(30))
                    .unwrap()
                    .0
                    .success()
            );
            let bytes = std::fs::read(&executable).unwrap();
            let zip = temp.0.join("download.zip");
            zip_file(
                &zip,
                &[
                    ("package/soloist.exe", &bytes),
                    ("package/LICENSE.txt", b"fixture notices"),
                    ("package/readme.txt", b"first"),
                ],
            );
            let root = temp.0.join("installation");
            let session = temp.0.join("session");
            std::fs::create_dir(&session).unwrap();
            std::fs::write(
                session.join("paired-session"),
                b"test session; never replace",
            )
            .unwrap();
            let first = import_at(&root, &zip).unwrap();
            assert_eq!(first.build.expiry, "current");
            assert!(active_at(&root).unwrap().is_none());
            assert_eq!(first.id, import_at(&root, &zip).unwrap().id);
            assert_eq!(
                std::fs::read(first.executable.parent().unwrap().join("LICENSE.txt")).unwrap(),
                b"fixture notices"
            );
            activate_at(&root, &session, &first.id).unwrap();
            assert_eq!(
                selected_at(&root, Some(&executable)).unwrap(),
                Some(build::strip_verbatim(&executable))
            );
            // Importing an update remains possible during playback; activation waits for ownership release.
            let busy = lock(&session, "deeznats.lock").unwrap();
            zip_file(
                &zip,
                &[
                    ("soloist.exe", &bytes),
                    ("LICENSE.txt", b"fixture notices"),
                    ("readme.txt", b"second"),
                ],
            );
            let second = import_at(&root, &zip).unwrap();
            assert_ne!(first.id, second.id);
            assert!(activate_at(&root, &session, &second.id)
                .unwrap_err()
                .contains("busy"));
            drop(busy);
            activate_at(&root, &session, &second.id).unwrap();
            assert_eq!(
                selected_at(&root, Some(&first.executable)).unwrap(),
                Some(second.executable.clone())
            );
            assert_eq!(active_at(&root).unwrap().unwrap().id, second.id);
            rollback_at(&root, &session).unwrap();
            assert_eq!(active_at(&root).unwrap().unwrap().id, first.id);
            assert_eq!(
                std::fs::read(session.join("paired-session")).unwrap(),
                b"test session; never replace"
            );
            let before = std::fs::read(root.join("active.json")).unwrap();
            let mut changed = std::fs::read(&second.executable).unwrap();
            let end = changed.len() - 1;
            changed[end] ^= 1;
            std::fs::write(&second.executable, &changed).unwrap();
            assert!(activate_at(&root, &session, &second.id)
                .unwrap_err()
                .contains("changed since import"));
            assert_eq!(std::fs::read(root.join("active.json")).unwrap(), before);
            // A broken managed build must not prevent recovery through a local executable.
            std::fs::write(root.join("active.json"), b"broken metadata").unwrap();
            assert_eq!(
                selected_at(&root, Some(&executable)).unwrap(),
                Some(build::strip_verbatim(&executable))
            );
            assert!(selected_at(&root, None).is_err());
            std::fs::write(root.join("active.json"), &before).unwrap();
            // Known-expired staged builds are rejected without changing the active selection.
            std::fs::write(&second.executable, &bytes).unwrap();
            let receipt_path = root.join("builds").join(&second.id).join(RECEIPT);
            let mut receipt: Receipt = bounded_json(&receipt_path).unwrap();
            receipt.build.build_timestamp = Some(build::now() - 91 * 86400);
            save_json(&receipt_path, &receipt).unwrap();
            assert!(activate_at(&root, &session, &second.id)
                .unwrap_err()
                .contains("expired"));
            assert_eq!(std::fs::read(root.join("active.json")).unwrap(), before);
            receipt.relative_executable = PathBuf::from("../soloist.exe");
            save_json(&receipt_path, &receipt).unwrap();
            assert!(load_at(&root, &second.id).is_err());
        }
        #[test]
        fn invalid_archives_never_commit_a_build_or_leave_staged_files() {
            let temp = Temporary::new();
            let root = temp.0.join("install");
            let zip = temp.0.join("bad.zip");
            for files in [
                vec![("../soloist.exe", b"not an exe".as_slice())],
                vec![
                    ("soloist.exe", b"bad".as_slice()),
                    ("SOLOIST.EXE", b"bad".as_slice()),
                ],
                vec![("readme.txt", b"no executable".as_slice())],
                vec![("soloist.exe", b"not a PE".as_slice())],
            ] {
                zip_file(&zip, &files);
                assert!(import_at(&root, &zip).is_err());
                assert!(!root.join("builds").exists());
                assert!(!root.join("active.json").exists());
                if root.join("staging").exists() {
                    assert_eq!(std::fs::read_dir(root.join("staging")).unwrap().count(), 0);
                }
            }
            let mut writer = ZipWriter::new(File::create(&zip).unwrap());
            writer
                .add_symlink("soloist.exe", "elsewhere", SimpleFileOptions::default())
                .unwrap();
            writer.finish().unwrap();
            assert!(import_at(&root, &zip).is_err());
            zip_file(&zip, &[("soloist.exe", b"payload with CRC")]);
            let mut bytes = std::fs::read(&zip).unwrap();
            let offset = bytes
                .windows(16)
                .position(|s| s == b"payload with CRC")
                .unwrap();
            bytes[offset] ^= 1;
            std::fs::write(&zip, bytes).unwrap();
            assert!(import_at(&root, &zip).unwrap_err().contains("CRC"));
            zip_file(&zip, &[("soloist.exe", b"placeholder")]);
            let mut bytes = std::fs::read(&zip).unwrap();
            let end = bytes.len() - 22;
            bytes[end + 8..end + 12].copy_from_slice(&[255, 255, 255, 255]);
            std::fs::write(&zip, bytes).unwrap();
            assert!(import_at(&root, &zip).unwrap_err().contains("excessive"));
            assert_eq!(std::fs::read_dir(root.join("staging")).unwrap().count(), 0);
        }
        #[test]
        fn executable_architecture_is_checked_without_execution() {
            let temp = Temporary::new();
            let path = temp.0.join("soloist.exe");
            let mut bytes = vec![0u8; 256];
            bytes[..2].copy_from_slice(b"MZ");
            bytes[60..64].copy_from_slice(&128u32.to_le_bytes());
            bytes[128..132].copy_from_slice(b"PE\0\0");
            bytes[132..134].copy_from_slice(&0x8664u16.to_le_bytes());
            bytes[152..154].copy_from_slice(&0x20bu16.to_le_bytes());
            std::fs::write(&path, &bytes).unwrap();
            build::pe_x64(&path).unwrap();
            bytes[132..134].copy_from_slice(&0xaa64u16.to_le_bytes());
            std::fs::write(&path, &bytes).unwrap();
            assert!(build::pe_x64(&path).is_err());
            bytes[132..134].copy_from_slice(&0x8664u16.to_le_bytes());
            bytes[150..152].copy_from_slice(&0x2000u16.to_le_bytes());
            std::fs::write(&path, bytes).unwrap();
            assert!(build::pe_x64(&path).is_err());
        }
    }
}
pub(crate) mod soloist {
    //! Reusable Soloist lifecycle and command API. Owns no Deezer/UI state.
    use super::{
        process::{self, Process, MAX_OUTPUT},
        state::{Snapshot, State},
    };
    use serde::{Deserialize, Serialize};
    use serde_json::Value;
    use std::{
        fs::{File, OpenOptions},
        io::{BufRead, BufReader, Read},
        os::windows::fs::OpenOptionsExt,
        path::PathBuf,
        process::{Command as ProcessCommand, Stdio},
        sync::{Arc, Mutex},
        thread::{self, JoinHandle},
        time::{Duration, Instant},
    };

    #[derive(Clone, Serialize, Deserialize)]
    pub(crate) struct Config {
        pub executable: PathBuf,
        pub api_key: String,
    }
    impl Config {
        pub(super) fn saved() -> Result<Self, String> {
            serde_json::from_str(&crate::session::provider_saved("spotify-soloist")?)
                .map_err(|_| "Saved Soloist configuration is invalid".into())
        }
        pub fn load() -> Result<Self, String> {
            let saved = Self::saved().ok();
            let api_key = personal_key(
                std::env::var("DEEZNATS_SOLOIST_API_KEY").ok().as_deref(),
                saved.as_ref().map(|config| config.api_key.as_str()),
            )?;
            let executable = super::setup::preferred_executable()?;
            let config = Self {
                executable,
                api_key,
            };
            config.validate()?;
            Ok(config)
        }
        fn validate(&self) -> Result<(), String> {
            let clean = super::setup::strip_verbatim(&self.executable);
            if !clean.is_absolute() || !clean.is_file() {
                return Err("Soloist must be an existing absolute executable path".into());
            }
            if self.api_key.trim().is_empty() || self.api_key.chars().any(char::is_control) {
                return Err("Soloist API key is invalid".into());
            }
            Ok(())
        }
        pub fn save(&self) -> Result<(), String> {
            let config = Self {
                executable: self.executable.clone(),
                api_key: self.api_key.trim().to_owned(),
            };
            config.validate()?;
            crate::session::provider_save(
                "spotify-soloist",
                &serde_json::to_string(&config)
                    .map_err(|_| "Could not encode Soloist configuration")?,
            )
        }
    }
    fn personal_key(environment: Option<&str>, saved: Option<&str>) -> Result<String, String> {
        environment.into_iter().chain(saved).map(str::trim).find(|key| !key.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| "Generate your own Soloist API key and enter it in Spotify setup, or set DEEZNATS_SOLOIST_API_KEY locally".into())
    }
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub(crate) enum Control {
        Play(Option<String>),
        Pause,
        Next,
        Previous,
        Seek(u32),
        Volume(u8),
        Shuffle(bool),
        Repeat(String),
        AddToQueue(String),
        Activate,
        Deactivate,
        Status,
        Now,
        Queue(u8),
    }
    impl Control {
        pub fn parse(args: &[&str]) -> Result<Self, String> {
            let command = match args {
                ["play"] => Self::Play(None),
                ["play", uri] if valid_uri(uri, false) => Self::Play(Some((*uri).into())),
                ["pause"] => Self::Pause,
                ["next"] => Self::Next,
                ["prev"] => Self::Previous,
                ["seek", value] => {
                    Self::Seek(value.parse().map_err(|_| "Invalid Spotify seek position")?)
                }
                ["volume", value] => Self::Volume(
                    value
                        .parse::<u8>()
                        .ok()
                        .filter(|v| *v <= 100)
                        .ok_or("Spotify volume must be 0-100")?,
                ),
                ["shuffle", "on"] => Self::Shuffle(true),
                ["shuffle", "off"] => Self::Shuffle(false),
                ["repeat", mode @ ("off" | "context" | "track")] => Self::Repeat((*mode).into()),
                ["add-to-queue", uri] if valid_uri(uri, true) => Self::AddToQueue((*uri).into()),
                ["activate"] => Self::Activate,
                ["deactivate"] => Self::Deactivate,
                ["status"] => Self::Status,
                ["now"] => Self::Now,
                ["queue"] => Self::Queue(10),
                ["queue", limit] => Self::Queue(
                    limit
                        .parse::<u8>()
                        .ok()
                        .filter(|n| (1..=80).contains(n))
                        .ok_or("Spotify queue limit must be 1-80")?,
                ),
                _ => return Err("Invalid Spotify command".into()),
            };
            Ok(command)
        }
        fn args(&self) -> Result<Vec<String>, String> {
            // Validate typed callers too; enum construction cannot bypass command bounds.
            let args: Vec<String> = match self {
                Self::Play(None) => vec!["play".into()],
                Self::Play(Some(uri)) => vec!["play".into(), uri.clone()],
                Self::Pause => vec!["pause".into()],
                Self::Next => vec!["next".into()],
                Self::Previous => vec!["prev".into()],
                Self::Seek(ms) => vec!["seek".into(), ms.to_string()],
                Self::Volume(v) => vec!["volume".into(), v.to_string()],
                Self::Shuffle(value) => {
                    vec!["shuffle".into(), if *value { "on" } else { "off" }.into()]
                }
                Self::Repeat(mode) => vec!["repeat".into(), mode.clone()],
                Self::AddToQueue(uri) => vec!["add-to-queue".into(), uri.clone()],
                Self::Activate => vec!["activate".into()],
                Self::Deactivate => vec!["deactivate".into()],
                Self::Status => vec!["status".into()],
                Self::Now => vec!["now".into()],
                Self::Queue(limit) => vec!["queue".into(), limit.to_string()],
            };
            Self::parse(&args.iter().map(String::as_str).collect::<Vec<_>>())?;
            Ok(args)
        }
    }
    fn valid_uri(uri: &str, track_only: bool) -> bool {
        let parts: Vec<_> = uri.split(':').collect();
        parts.len() == 3
            && parts[0] == "spotify"
            && (if track_only {
                parts[1] == "track"
            } else {
                matches!(parts[1], "track" | "episode" | "album" | "playlist")
            })
            && parts[2].len() == 22
            && parts[2].bytes().all(|b| b.is_ascii_alphanumeric())
    }
    struct Monitor {
        process: Process,
        reader: Option<JoinHandle<()>>,
    }
    impl Drop for Monitor {
        fn drop(&mut self) {
            self.process.stop();
            if let Some(reader) = self.reader.take() {
                let _ = reader.join();
            }
        }
    }
    pub(crate) struct Soloist {
        process: Process,
        executable: PathBuf,
        data: PathBuf,
        state: Arc<Mutex<State>>,
        monitor: Option<Monitor>,
        last_reconnect: Instant,
        _lock: File,
    }
    impl Drop for Soloist {
        fn drop(&mut self) {
            self.monitor.take();
            self.process.stop();
        }
    }
    impl Soloist {
        pub(super) fn audio_target(&self) -> Result<super::process::AudioTarget, String> {
            self.process.audio_target()
        }
        pub fn start(config: Config) -> Result<Self, String> {
            Self::start_at(
                config,
                crate::storage::data_dir()?.join("spotify-soloist"),
                Duration::from_secs(15),
            )
        }
        pub(super) fn start_cancellable(
            config: Config,
            stop: &std::sync::atomic::AtomicBool,
        ) -> Result<Self, String> {
            Self::start_inner(
                config,
                crate::storage::data_dir()?.join("spotify-soloist"),
                Duration::from_secs(15),
                Some(stop),
            )
        }
        pub(super) fn start_at(
            config: Config,
            data: PathBuf,
            timeout: Duration,
        ) -> Result<Self, String> {
            Self::start_inner(config, data, timeout, None)
        }
        fn start_inner(
            config: Config,
            data: PathBuf,
            timeout: Duration,
            stop: Option<&std::sync::atomic::AtomicBool>,
        ) -> Result<Self, String> {
            config.validate()?;
            let data = super::setup::strip_verbatim(&data);
            std::fs::create_dir_all(&data)
                .map_err(|_| "Could not create Soloist data directory")?;
            let lock = OpenOptions::new()
                .create(true)
                .truncate(false)
                .write(true)
                .share_mode(0)
                .open(data.join("deeznats.lock"))
                .map_err(|_| "Soloist is already managed by another DeezNats session")?;
            let mut command = clean_command(&config.executable);
            command
                .args([
                    "--device-name",
                    "DeezNats",
                    "--api-key",
                    &config.api_key,
                    "--ws",
                    "127.0.0.1:0",
                    "--cache-size",
                    "512",
                ])
                .arg("--data-dir")
                .arg(&data)
                .arg("--cache-dir")
                .arg(data.join("cache"))
                .stdout(Stdio::null());
            let process = Process::spawn(&mut command)?;
            let mut player = Self {
                process,
                executable: config.executable,
                data,
                state: Arc::new(Mutex::new(State::default())),
                monitor: None,
                last_reconnect: Instant::now(),
                _lock: lock,
            };
            let started = Instant::now();
            loop {
                if stop.is_some_and(|stop| stop.load(std::sync::atomic::Ordering::Relaxed)) {
                    return Err("Spotify startup cancelled".into());
                }
                player.check_running()?;
                if player.runtime_ready() && player.control(&Control::Status).is_ok() {
                    break;
                }
                if started.elapsed() >= timeout {
                    player.process.stop();
                    return Err(startup_error(None, &player.process.diagnostics()));
                }
                thread::sleep(Duration::from_millis(100));
            }
            player.connect_monitor()?;
            Ok(player)
        }
        fn check_running(&mut self) -> Result<(), String> {
            if let Some(status) = self
                .process
                .child
                .try_wait()
                .map_err(|_| "Could not inspect Soloist")?
            {
                self.process.stop();
                let error = startup_error(status.code(), &self.process.diagnostics());
                if let Ok(mut state) = self.state.lock() {
                    state.disconnected(&error);
                }
                return Err(error);
            }
            // Key validation may finish after the local API has already started.
            let diagnostics = self.process.diagnostics();
            if api_key_rejected(&diagnostics.to_ascii_lowercase()) {
                return Err(startup_error(None, &diagnostics));
            }
            Ok(())
        }
        fn runtime_ready(&self) -> bool {
            let read =
                |name: &str| std::fs::read_to_string(self.data.join(name)).unwrap_or_default();
            read("soloist.pid").trim().parse::<u32>().ok() == Some(self.process.child.id())
                && read("ws.addr").trim() == "127.0.0.1"
                && read("ws.port").trim().parse::<u16>().is_ok_and(|p| p > 0)
        }
        fn ctl(&self) -> ProcessCommand {
            let mut command = clean_command(&self.executable);
            let clean_data = super::setup::strip_verbatim(&self.data);
            command.arg("ctl").arg("--data-dir").arg(clean_data);
            command
        }
        pub fn control(&mut self, control: &Control) -> Result<Value, String> {
            let args = control.args()?;
            self.check_running()?;
            if !self.runtime_ready() {
                return Err(
                    "Soloist local API discovery is missing, stale, or not loopback-only".into(),
                );
            }
            let query = matches!(control, Control::Status | Control::Now | Control::Queue(_));
            let mut command = self.ctl();
            command.args(args);
            if matches!(control, Control::Now) {
                command.arg("--json");
            }
            let (status, bytes) = process::capture(&mut command, Duration::from_secs(3))?;
            if !status.success() {
                return Err(match status.code() {
                Some(2) => "Soloist local API is unavailable",
                Some(3) => {
                    "Soloist is not paired or did not respond; choose DeezNats in Spotify Connect"
                }
                _ => "Soloist rejected the command; check pairing and available actions",
            }
            .into());
            }
            if query {
                if let Ok(value) = serde_json::from_slice::<Value>(&bytes) {
                    Ok(value)
                } else {
                    let text = String::from_utf8_lossy(&bytes).trim().to_string();
                    Ok(serde_json::json!({ "output": text }))
                }
            } else {
                Ok(Value::Null)
            }
        }
        fn connect_monitor(&mut self) -> Result<(), String> {
            self.monitor.take();
            if !self.runtime_ready() {
                return Err(
                    "Soloist local API discovery is missing, stale, or not loopback-only".into(),
                );
            }
            let mut command = self.ctl();
            command.arg("trace").stdout(Stdio::piped());
            let mut process = Process::spawn(&mut command)?;
            let stdout = process
                .child
                .stdout
                .take()
                .ok_or("Soloist event stream unavailable")?;
            let state = self.state.clone();
            let reader = thread::spawn(move || {
                let mut input = BufReader::new(stdout);
                loop {
                    let mut line = Vec::new();
                    match (&mut input)
                        .take(MAX_OUTPUT as u64 + 1)
                        .read_until(b'\n', &mut line)
                    {
                        Ok(0) | Err(_) => break,
                        Ok(_) if line.len() > MAX_OUTPUT => break,
                        Ok(_) => {
                            let text = String::from_utf8_lossy(&line);
                            let trimmed = text.trim();
                            if trimmed.is_empty() || trimmed.starts_with("connected to ") {
                                continue;
                            }
                            let result = parse_trace(&line).and_then(|event| {
                                state
                                    .lock()
                                    .map_err(|_| "Soloist state unavailable".to_owned())?
                                    .apply(&event)
                            });
                            if result.is_err() {
                                break;
                            }
                        }
                    }
                }
                if let Ok(mut state) = state.lock() {
                    state.disconnected("Soloist event stream disconnected");
                }
            });
            self.monitor = Some(Monitor {
                process,
                reader: Some(reader),
            });
            self.last_reconnect = Instant::now();
            Ok(())
        }
        /// Polling reads a cached snapshot; only reconnects a failed event stream, never restarts audio.
        pub fn snapshot(&mut self) -> Result<Snapshot, String> {
            self.check_running()?;
            let disconnected = self
                .monitor
                .as_ref()
                .is_none_or(|m| m.reader.as_ref().is_none_or(JoinHandle::is_finished));
            if disconnected && self.last_reconnect.elapsed() >= Duration::from_secs(1) {
                self.connect_monitor()?;
            }
            self.state
                .lock()
                .map(|state| state.snapshot())
                .map_err(|_| "Soloist state unavailable".into())
        }
    }
    fn clean_command(executable: &std::path::Path) -> ProcessCommand {
        let clean = super::setup::strip_verbatim(executable);
        let mut command = ProcessCommand::new(clean);
        command
            .env_remove("DEEZNATS_SOLOIST_API_KEY")
            .env_remove("DEEZNATS_SPOTIFY_CLIENT_ID");
        command
    }
    fn startup_error(code: Option<i32>, diagnostics: &str) -> String {
        // Classify locally: never display raw daemon output, which can contain keys,
        // session tokens, or account information.
        let text = diagnostics.to_ascii_lowercase();
        let reason = if code == Some(10) {
            "Soloist build expired; download a new build from Spotify"
        } else if api_key_rejected(&text) {
            "Soloist rejected the personal API key. Generate a Soloist key in Spotify's developer dashboard and replace it in setup; a Web API client ID or client secret will not work"
        } else if text.contains("permission denied")
            || text.contains("access is denied")
            || text.contains("access denied")
        {
            "Soloist cannot access its data, cache, or executable directory. Check folder permissions"
        } else if text.contains("already running")
            || text.contains("already in use")
            || text.contains("lock file")
        {
            "Another Soloist process is using the data directory. Close it and start Spotify again"
        } else if text.contains("unknown option")
            || text.contains("unrecognized option")
            || text.contains("unexpected argument")
            || text.contains("invalid argument")
        {
            "This Soloist build rejected the startup options. Install the latest official Windows x64 build"
        } else if text.contains("websocket")
            && ["bind", "listen", "address in use"]
                .iter()
                .any(|word| text.contains(word))
        {
            "Soloist could not open its local WebSocket API. Check local firewall or endpoint security settings"
        } else if text.contains("premium") {
            "Soloist requires Spotify Premium for initial setup. Check the account used to generate the personal key"
        } else if [
            "resolve host",
            "dns",
            "connection refused",
            "network unreachable",
            "certificate",
            "tls",
        ]
        .iter()
        .any(|word| text.contains(word))
        {
            "Soloist could not connect to Spotify. Check the network, proxy, certificate, and firewall settings"
        } else if code.is_none() {
            "Soloist did not expose its local API before startup timed out. Check the installed build and data-directory access"
        } else {
            "Soloist failed to start. Check the installed build and data-directory access"
        };
        let override_note = if reason.contains("personal API key")
            && std::env::var("DEEZNATS_SOLOIST_API_KEY").is_ok_and(|key| !key.trim().is_empty())
        {
            " DEEZNATS_SOLOIST_API_KEY overrides the saved key; update or remove that environment variable."
        } else {
            ""
        };
        match code {
            Some(code) => format!("{reason} (exit code {code}).{override_note}"),
            None => format!("{reason}{override_note}"),
        }
    }
    fn api_key_rejected(text: &str) -> bool {
        text.lines().any(|line| {
            (line.contains("api key") || line.contains("api-key") || line.contains("api_key"))
                && [
                    "invalid", "reject", "unauthor", "revoked", "expired", "missing",
                ]
                .iter()
                .any(|word| line.contains(word))
        })
    }
    fn parse_trace(line: &[u8]) -> Result<Value, String> {
        let text = std::str::from_utf8(line)
            .map_err(|_| "Invalid Soloist event encoding")?
            .trim();
        let json = if text.starts_with('{') {
            text
        } else {
            let (timestamp, json) = text
                .split_once(' ')
                .ok_or("Invalid Soloist event envelope")?;
            timestamp
                .parse::<u64>()
                .map_err(|_| "Invalid Soloist event timestamp")?;
            json
        };
        serde_json::from_str(json).map_err(|_| "Invalid Soloist event JSON".into())
    }
    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn typed_controls_cannot_bypass_validation() {
            assert!(Control::Volume(101).args().is_err());
            assert!(Control::Repeat("--api-key".into()).args().is_err());
            assert!(
                Control::AddToQueue("spotify:album:0123456789012345678901".into())
                    .args()
                    .is_err()
            );
            assert!(
                Control::Play(Some("spotify:playlist:0123456789012345678901".into()))
                    .args()
                    .is_ok()
            );
            assert!(Control::Queue(81).args().is_err());
            assert!(Control::parse(&["seek", "-1"]).is_err());
        }
        #[test]
        fn trace_accepts_documented_timestamp_envelope_and_rejects_garbage() {
            assert_eq!(
                parse_trace(b"123 {\"type\":\"auth_state\",\"logged_in\":true}\n").unwrap()
                    ["logged_in"],
                true
            );
            assert!(parse_trace(b"secret value").is_err());
            assert!(parse_trace(b"123 not-json").is_err());
        }
        #[test]
        fn personal_keys_ignore_paste_whitespace_and_empty_environment_overrides() {
            assert_eq!(
                personal_key(Some("  \r\n"), Some(" saved-key\n")).unwrap(),
                "saved-key"
            );
            assert_eq!(
                personal_key(Some(" environment-key\r\n"), Some("saved-key")).unwrap(),
                "environment-key"
            );
            assert!(personal_key(None, Some(" ")).is_err());
        }
        #[test]
        fn startup_diagnostics_are_actionable_without_exposing_raw_secrets() {
            assert!(!api_key_rejected(
                "api key accepted\ninvalid playback request"
            ));
            for (diagnostic, expected) in [
                ("invalid API key private-secret", "personal API key"),
                ("permission denied private-secret", "folder permissions"),
                ("unrecognized option private-secret", "startup options"),
                ("TLS handshake failed private-secret", "network"),
                ("unknown problem private-secret", "exit code 1"),
            ] {
                let error = startup_error(Some(1), diagnostic);
                assert!(error.contains(expected), "{error}");
                assert!(!error.contains("private-secret"));
            }
        }
        #[test]
        fn offline_daemon_lifecycle_state_reconnect_and_expiry() {
            struct Temporary(PathBuf);
            impl Drop for Temporary {
                fn drop(&mut self) {
                    let root = std::env::temp_dir().canonicalize().unwrap();
                    if self.0.parent().and_then(|p| p.canonicalize().ok()).as_ref() == Some(&root)
                        && self
                            .0
                            .file_name()
                            .unwrap()
                            .to_string_lossy()
                            .starts_with("deeznats-soloist-test-")
                    {
                        let _ = std::fs::remove_dir_all(&self.0);
                    }
                }
            }
            let nonce = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let directory = Temporary(std::env::temp_dir().join(format!(
                "deeznats-soloist-test-{}-{nonce}",
                std::process::id()
            )));
            std::fs::create_dir(&directory.0).unwrap();
            let executable = directory.0.join("soloist-fixture.exe");
            let mut compiler = ProcessCommand::new("rustc");
            compiler
                .arg("--edition=2021")
                .arg(
                    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                        .join("tests/fixtures/soloist_mock.rs"),
                )
                .arg("-o")
                .arg(&executable);
            assert!(process::capture(&mut compiler, Duration::from_secs(30))
                .unwrap()
                .0
                .success());
            let config = |key: &str| Config {
                executable: executable.clone(),
                api_key: key.into(),
            };
            let data = directory.0.join("data");
            let mut player =
                Soloist::start_at(config("test"), data.clone(), Duration::from_secs(5)).unwrap();
            assert!(
                Soloist::start_at(config("test"), data.clone(), Duration::from_secs(1))
                    .err()
                    .unwrap()
                    .contains("another DeezNats")
            );
            let started = Instant::now();
            loop {
                let snapshot = player.snapshot().unwrap();
                if snapshot.logged_in && snapshot.volume == 42 {
                    assert_eq!(snapshot.position_ms, 1234);
                    break;
                }
                assert!(started.elapsed() < Duration::from_secs(3));
                thread::sleep(Duration::from_millis(20));
            }
            assert_eq!(player.control(&Control::Now).unwrap()["fixture"], true);
            assert!(player
                .control(&Control::Volume(50))
                .unwrap_err()
                .contains("not paired"));
            player.control(&Control::Pause).unwrap();
            // A broken event stream can reconnect while the daemon remains untouched.
            player.monitor.as_mut().unwrap().process.stop();
            let started = Instant::now();
            while !player
                .monitor
                .as_ref()
                .unwrap()
                .reader
                .as_ref()
                .unwrap()
                .is_finished()
            {
                assert!(started.elapsed() < Duration::from_secs(3));
                thread::sleep(Duration::from_millis(10));
            }
            let pid = player.process.child.id();
            player.last_reconnect = Instant::now() - Duration::from_secs(2);
            player.snapshot().unwrap();
            assert_eq!(player.process.child.id(), pid);
            // Simulate the OS closing our job handles on app exit.
            player.process.job_for_test_close();
            let started = Instant::now();
            while player.process.child.try_wait().unwrap().is_none() {
                assert!(started.elapsed() < Duration::from_secs(3));
                thread::sleep(Duration::from_millis(10));
            }
            assert!(player.snapshot().is_err());
            drop(player);
            assert!(Soloist::start_at(
                config("expired"),
                directory.0.join("expired"),
                Duration::from_secs(2)
            )
            .err()
            .unwrap()
            .contains("expired"));
            let invalid_key = Soloist::start_at(
                config("invalid-key-fixture-secret"),
                directory.0.join("invalid-key"),
                Duration::from_secs(2),
            )
            .err()
            .unwrap();
            assert!(invalid_key.contains("personal API key"), "{invalid_key}");
            assert!(!invalid_key.contains("invalid-key-fixture-secret"));
            let late_data = directory.0.join("late-key-rejection");
            let mut late = Soloist::start_at(
                config("late-invalid-key-fixture-secret"),
                late_data.clone(),
                Duration::from_secs(2),
            )
            .unwrap();
            std::fs::write(late_data.join("reject-key"), b"").unwrap();
            let started = Instant::now();
            loop {
                if let Err(error) = late.snapshot() {
                    assert!(error.contains("personal API key"), "{error}");
                    assert!(!error.contains("late-invalid-key-fixture-secret"));
                    break;
                }
                assert!(started.elapsed() < Duration::from_secs(2));
                thread::sleep(Duration::from_millis(10));
            }
            drop(late);
            assert!(Soloist::start_at(
                config("no-api"),
                directory.0.join("missing-api"),
                Duration::from_millis(200)
            )
            .err()
            .unwrap()
            .contains("local API"));
        }
    }
}
pub(crate) mod state {
    //! Soloist event reduction. State changes come from the engine, never optimistic controls.
    use serde::Serialize;
    use serde_json::Value;
    use std::time::Instant;

    #[derive(Clone, Debug, Default, Serialize)]
    pub(crate) struct Snapshot {
        pub connected: bool,
        pub logged_in: bool,
        pub is_active: bool,
        pub device_name: String,
        pub status: String,
        pub item: Value,
        pub context: Value,
        pub position_ms: u64,
        pub duration_ms: u64,
        pub volume: u8,
        pub shuffle: bool,
        pub repeat: String,
        pub available_actions: Value,
        pub previous: Vec<Value>,
        pub upcoming: Vec<Value>,
        pub last_error: Option<String>,
        pub revision: u64,
    }
    pub(crate) struct State {
        snapshot: Snapshot,
        anchor: Instant,
        speed: f64,
    }
    impl Default for State {
        fn default() -> Self {
            Self {
                snapshot: Snapshot {
                    status: "idle".into(),
                    repeat: "off".into(),
                    ..Snapshot::default()
                },
                anchor: Instant::now(),
                speed: 0.0,
            }
        }
    }
    impl State {
        pub fn snapshot(&self) -> Snapshot {
            let mut snapshot = self.snapshot.clone();
            snapshot.position_ms = self.position(Instant::now());
            snapshot
        }
        fn position(&self, at: Instant) -> u64 {
            let advance = if self.snapshot.status == "playing" && self.snapshot.connected {
                at.saturating_duration_since(self.anchor).as_secs_f64() * 1000.0 * self.speed
            } else {
                0.0
            };
            let position = self
                .snapshot
                .position_ms
                .saturating_add(advance.max(0.0) as u64);
            if self.snapshot.duration_ms > 0 {
                position.min(self.snapshot.duration_ms)
            } else {
                position
            }
        }
        fn freeze(&mut self) {
            self.snapshot.position_ms = self.position(Instant::now());
            self.anchor = Instant::now();
        }
        pub fn disconnected(&mut self, error: &str) {
            self.freeze();
            self.snapshot.connected = false;
            self.speed = 0.0;
            self.snapshot.is_active = false;
            self.snapshot.last_error = Some(error.into());
            self.snapshot.revision = self.snapshot.revision.wrapping_add(1);
        }
        pub fn apply(&mut self, event: &Value) -> Result<(), String> {
            let kind = event["type"].as_str().ok_or("Soloist event has no type")?;
            match kind {
                "auth_state" => {
                    let logged_in = event["logged_in"]
                        .as_bool()
                        .ok_or("Invalid Soloist authentication state")?;
                    self.snapshot.connected = true;
                    self.snapshot.logged_in = logged_in;
                    self.snapshot.is_active = event["is_active"].as_bool().unwrap_or(false);
                    self.snapshot.device_name =
                        event["device_name"].as_str().unwrap_or_default().into();
                    self.snapshot.last_error = None;
                    if !logged_in {
                        let revision = self.snapshot.revision;
                        let connected = self.snapshot.connected;
                        let device_name = self.snapshot.device_name.clone();
                        *self = Self::default();
                        self.snapshot.revision = revision;
                        self.snapshot.connected = connected;
                        self.snapshot.device_name = device_name;
                    }
                }
                "playback_state" => {
                    validate_status(&event["status"])?;
                    validate_position(&event["position"])?;
                    let volume = volume(&event["volume"])?;
                    self.freeze();
                    self.snapshot.connected = true;
                    self.snapshot.status = event["status"].as_str().unwrap().into();
                    self.set_item(&event["item"]);
                    self.snapshot.context = event["context"].clone();
                    self.set_position(&event["position"]);
                    self.snapshot.volume = volume;
                    self.snapshot.is_active = event["is_active"].as_bool().unwrap_or(false);
                    self.options(&event["options"]);
                    self.snapshot.available_actions = event["available_actions"].clone();
                }
                "track_changed" => {
                    self.freeze();
                    self.set_item(&event["item"]);
                    self.snapshot.position_ms = 0;
                    self.speed = 0.0;
                }
                "playback_changed" => {
                    validate_status(&event["status"])?;
                    self.freeze();
                    let status = event["status"].as_str().unwrap();
                    if status == "playing" && self.speed == 0.0 {
                        self.speed = 1.0;
                        self.anchor = Instant::now();
                    } else if status == "paused" || status == "idle" {
                        self.speed = 0.0;
                    }
                    self.snapshot.status = status.into();
                }
                "position_sync" => {
                    validate_position(&event["position"])?;
                    self.set_position(&event["position"]);
                }
                "volume_changed" => self.snapshot.volume = volume(&event["volume"])?,
                "device_changed" => {
                    self.snapshot.is_active = event["is_active"]
                        .as_bool()
                        .ok_or("Invalid Soloist device state")?;
                    if let Some(name) = event["device_name"].as_str() {
                        self.snapshot.device_name = name.into();
                    }
                }
                "context_changed" => self.snapshot.context = event["context"].clone(),
                "options_changed" => self.options(&event["options"]),
                "queue_changed" => {
                    let previous = event["previous"]
                        .as_array()
                        .ok_or("Invalid Soloist queue")?;
                    let upcoming = event["upcoming"]
                        .as_array()
                        .ok_or("Invalid Soloist queue")?;
                    self.snapshot.previous = previous.iter().take(80).cloned().collect();
                    self.snapshot.upcoming = upcoming.iter().take(80).cloned().collect();
                }
                "error" => {
                    self.snapshot.last_error = Some(
                        "Soloist rejected a request; check pairing and available actions".into(),
                    )
                }
                _ => return Ok(()), // New event kinds and command acknowledgments do not invent playback state.
            }
            self.snapshot.revision = self.snapshot.revision.wrapping_add(1);
            Ok(())
        }
        fn set_item(&mut self, item: &Value) {
            let previous_duration =
                if item["uri"].as_str().is_some() && item["uri"] == self.snapshot.item["uri"] {
                    self.snapshot.duration_ms
                } else {
                    0
                };
            self.snapshot.item = item.clone();
            self.snapshot.duration_ms = item["decorations"]["playback"]["duration_ms"]
                .as_u64()
                .filter(|duration| *duration > 0)
                .or((previous_duration > 0).then_some(previous_duration))
                .unwrap_or_default();
        }
        fn set_position(&mut self, position: &Value) {
            self.snapshot.position_ms = position["position_ms"].as_u64().unwrap_or_default();
            self.speed = position["speed"].as_f64().unwrap_or_default();
            self.anchor = Instant::now();
        }
        fn options(&mut self, options: &Value) {
            if let Some(shuffle) = options["shuffle"].as_bool() {
                self.snapshot.shuffle = shuffle;
            }
            if let Some(repeat) = options["repeat"]
                .as_str()
                .filter(|s| matches!(*s, "off" | "context" | "track"))
            {
                self.snapshot.repeat = repeat.into();
            }
        }
    }
    fn validate_status(value: &Value) -> Result<(), String> {
        if value
            .as_str()
            .is_some_and(|s| matches!(s, "idle" | "playing" | "paused" | "buffering"))
        {
            Ok(())
        } else {
            Err("Invalid Soloist playback status".into())
        }
    }
    fn validate_position(value: &Value) -> Result<(), String> {
        if value["position_ms"].as_u64().is_some()
            && value["speed"]
                .as_f64()
                .is_some_and(|v| v.is_finite() && (0.0..=16.0).contains(&v))
        {
            Ok(())
        } else {
            Err("Invalid Soloist playback position".into())
        }
    }
    fn volume(value: &Value) -> Result<u8, String> {
        value
            .as_u64()
            .filter(|v| *v <= 100)
            .map(|v| v as u8)
            .ok_or_else(|| "Invalid Soloist volume".into())
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use serde_json::json;
        use std::time::Duration;
        #[test]
        fn sparse_item_updates_keep_duration_only_for_the_same_uri() {
            let mut state = State::default();
            state.set_item(&json!({"uri":"spotify:track:first", "decorations":{"playback":{"duration_ms":210000}}}));
            state.set_item(&json!({"uri":"spotify:track:first"}));
            assert_eq!(state.snapshot().duration_ms, 210000);
            state.set_item(&json!({"uri":"spotify:track:second"}));
            assert_eq!(state.snapshot().duration_ms, 0);
            state.set_item(&Value::Null);
            assert_eq!(state.snapshot().duration_ms, 0);
        }
        #[test]
        fn position_uses_monotonic_time_stops_on_pause_and_clamps_to_duration() {
            let mut state = State::default();
            state
                .apply(&json!({"type":"auth_state","logged_in":true,"is_active":true}))
                .unwrap();
            state.apply(&json!({"type":"playback_state","status":"playing","item":{"decorations":{"playback":{"duration_ms":2000}}},"position":{"position_ms":1000,"timestamp_ms":0,"speed":1.0},"volume":50})).unwrap();
            assert_eq!(state.position(state.anchor + Duration::from_secs(10)), 2000);
            state
                .apply(&json!({"type":"playback_changed","status":"paused"}))
                .unwrap();
            assert_eq!(
                state.position(state.anchor + Duration::from_secs(10)),
                state.snapshot.position_ms
            );
            state.disconnected("lost connection");
            assert!(!state.snapshot().connected);
        }
        #[test]
        fn logout_clears_stale_track_and_queue() {
            let mut state = State::default();
            state
                .apply(&json!({"type":"track_changed","item":{"uri":"spotify:track:old"}}))
                .unwrap();
            state
                .apply(&json!({"type":"queue_changed","previous":[],"upcoming":[{"uid":"next"}]}))
                .unwrap();
            state
                .apply(&json!({"type":"auth_state","logged_in":false,"is_active":false}))
                .unwrap();
            let snapshot = state.snapshot();
            assert!(snapshot.item.is_null());
            assert!(snapshot.upcoming.is_empty());
            assert_eq!(snapshot.position_ms, 0);
        }
        #[test]
        fn ignores_new_events_and_rejects_invalid_values() {
            let mut state = State::default();
            state.apply(&json!({"type":"future_event"})).unwrap();
            assert_eq!(state.snapshot().revision, 0);
            assert!(state
                .apply(&json!({"type":"volume_changed","volume":101}))
                .is_err());
            assert!(state
                .apply(&json!({"type":"position_sync","position":{"position_ms":-1,"speed":1.0}}))
                .is_err());
            state
                .apply(&json!({"type":"command_result","command":"play"}))
                .unwrap();
            assert_eq!(state.snapshot().status, "idle");
        }
        #[test]
        fn malformed_snapshot_does_not_replace_good_state() {
            let mut state = State::default();
            state
                .apply(&json!({"type":"volume_changed","volume":40}))
                .unwrap();
            let before = state.snapshot();
            assert!(state.apply(&json!({"type":"playback_state","status":"playing","position":{"position_ms":12,"speed":1.0},"volume":999})).is_err());
            assert_eq!(state.snapshot().revision, before.revision);
            assert_eq!(state.snapshot().status, before.status);
        }
        #[test]
        fn reconnect_waits_for_a_new_position_anchor() {
            let mut state = State::default();
            state.apply(&json!({"type":"playback_state","status":"playing","position":{"position_ms":1000,"speed":1.0},"volume":50})).unwrap();
            state.disconnected("lost connection");
            let frozen = state.snapshot.position_ms;
            state
                .apply(&json!({"type":"auth_state","logged_in":true,"is_active":true}))
                .unwrap();
            assert_eq!(
                state.position(state.anchor + Duration::from_secs(10)),
                frozen
            );
            state
                .apply(&json!({"type":"position_sync","position":{"position_ms":2000,"speed":1.0}}))
                .unwrap();
            assert_eq!(state.position(state.anchor + Duration::from_secs(1)), 3000);
        }
    }
}
pub(crate) mod wizard {
    //! Isolated, user-driven Spotify Soloist setup with a native secret field.
    use std::{
        cell::{Cell, RefCell},
        collections::{HashMap, VecDeque},
        path::PathBuf,
        rc::Rc,
        sync::atomic::{AtomicBool, Ordering},
        sync::{mpsc, Arc},
        time::{Duration, Instant},
    };
    use webview2_com::{
        AddScriptToExecuteOnDocumentCreatedCompletedHandler, CoTaskMemPWSTR,
        CreateCoreWebView2ControllerCompletedHandler,
        CreateCoreWebView2EnvironmentCompletedHandler, DownloadStartingEventHandler,
        ExecuteScriptCompletedHandler, GetCookiesCompletedHandler,
        Microsoft::Web::WebView2::Win32::*, NavigationStartingEventHandler,
        NewWindowRequestedEventHandler, PermissionRequestedEventHandler, StateChangedEventHandler,
    };
    use windows::core::{Interface, PCWSTR, PWSTR};
    use windows_sys::Win32::{
        Foundation::*,
        Graphics::{Dwm::DwmSetWindowAttribute, Gdi::*},
        System::{Com::*, LibraryLoader::GetModuleHandleW},
        UI::{
            Controls::{Dialogs::*, *},
            Input::KeyboardAndMouse::{EnableWindow, SetActiveWindow, SetFocus},
            WindowsAndMessaging::*,
        },
    };
    struct WizardTheme {
        colors: [u32; 9],
        background: HBRUSH,
        field: HBRUSH,
    }
    impl Drop for WizardTheme {
        fn drop(&mut self) {
            unsafe {
                DeleteObject(self.background as _);
                DeleteObject(self.field as _);
            }
        }
    }
    thread_local! { static THEME: RefCell<Option<WizardTheme>> = const { RefCell::new(None) }; }
    thread_local! { static CONTROLLER:RefCell<Option<ICoreWebView2Controller>>=const {RefCell::new(None)}; static CLOSED:Cell<bool>=const {Cell::new(false)}; static ACTION:Cell<u32>=const {Cell::new(0)}; }
    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }
    unsafe extern "system" fn confirm_proc(hwnd: HWND, msg: u32, wp: usize, lp: isize) -> isize {
        match msg {
            WM_COMMAND if (wp >> 16) == BN_CLICKED as usize => {
                ACTION.set((wp & 0xffff) as u32);
                0
            }
            WM_CLOSE => {
                CLOSED.set(true);
                0
            }
            WM_ERASEBKGND | WM_CTLCOLORSTATIC | WM_DRAWITEM => window_proc(hwnd, msg, wp, lp),
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }

    pub(crate) unsafe fn confirm(
        owner: HWND,
        palette: [u32; 9],
        title: &str,
        message: &str,
        ok: &str,
        cancel: &str,
    ) -> bool {
        CLOSED.set(false);
        ACTION.set(0);
        THEME.with(|slot| {
            *slot.borrow_mut() = Some(WizardTheme {
                colors: palette,
                background: CreateSolidBrush(palette[0]),
                field: CreateSolidBrush(palette[2]),
            })
        });
        let instance = GetModuleHandleW(std::ptr::null());
        let class_name = wide("DeezNatsSpotifyConfirm");
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(confirm_proc),
            hInstance: instance,
            lpszClassName: class_name.as_ptr(),
            hCursor: LoadCursorW(std::ptr::null_mut(), IDC_ARROW),
            ..Default::default()
        });
        let scale = windows_sys::Win32::UI::HiDpi::GetDpiForWindow(owner).max(96) as f64 / 96.0;
        let px = |n: i32| (n as f64 * scale).round() as i32;
        let dc = GetDC(owner);
        let old_font = SelectObject(dc, GetStockObject(DEFAULT_GUI_FONT));
        let mut text_rect = RECT {
            left: 0,
            top: 0,
            right: px(500),
            bottom: 0,
        };
        let mut label = wide(message);
        DrawTextW(
            dc,
            label.as_mut_ptr(),
            -1,
            &mut text_rect,
            DT_CALCRECT | DT_WORDBREAK | DT_NOPREFIX,
        );
        SelectObject(dc, old_font);
        ReleaseDC(owner, dc);
        let width = px(550);
        let height = (text_rect.bottom + px(130)).max(px(260));
        let mut parent_rect = RECT::default();
        GetWindowRect(owner, &mut parent_rect);
        let hwnd = CreateWindowExW(
            WS_EX_DLGMODALFRAME,
            class_name.as_ptr(),
            wide(title).as_ptr(),
            WS_POPUP | WS_CAPTION | WS_SYSMENU,
            parent_rect.left + (parent_rect.right - parent_rect.left - width) / 2,
            parent_rect.top + (parent_rect.bottom - parent_rect.top - height) / 2,
            width,
            height,
            owner,
            std::ptr::null_mut(),
            instance,
            std::ptr::null(),
        );
        if hwnd.is_null() {
            THEME.with(|slot| {
                slot.borrow_mut().take();
            });
            return false;
        }
        let dark = ((palette[0] & 255) * 299
            + ((palette[0] >> 8) & 255) * 587
            + ((palette[0] >> 16) & 255) * 114)
            < 128000;
        let dark_mode = dark as i32;
        DwmSetWindowAttribute(hwnd, 20, &dark_mode as *const _ as _, 4);
        DwmSetWindowAttribute(hwnd, 35, &palette[0] as *const _ as _, 4);
        DwmSetWindowAttribute(hwnd, 36, &palette[4] as *const _ as _, 4);
        let mut client = RECT::default();
        GetClientRect(hwnd, &mut client);
        for (id, class, text, x, y, w, h, style) in [
            (
                200,
                "STATIC",
                message,
                px(20),
                px(20),
                client.right - px(40),
                text_rect.bottom + px(4),
                0,
            ),
            (
                201,
                "BUTTON",
                ok,
                client.right - px(230),
                client.bottom - px(50),
                px(100),
                px(32),
                WS_TABSTOP | BS_OWNERDRAW as u32,
            ),
            (
                202,
                "BUTTON",
                cancel,
                client.right - px(120),
                client.bottom - px(50),
                px(100),
                px(32),
                WS_TABSTOP | BS_OWNERDRAW as u32,
            ),
        ] {
            let child = CreateWindowExW(
                0,
                wide(class).as_ptr(),
                wide(text).as_ptr(),
                WS_CHILD | WS_VISIBLE | style,
                x,
                y,
                w,
                h,
                hwnd,
                id as usize as HMENU,
                instance,
                std::ptr::null(),
            );
            if child.is_null() {
                DestroyWindow(hwnd);
                THEME.with(|slot| {
                    slot.borrow_mut().take();
                });
                return false;
            }
            SendMessageW(
                child,
                WM_SETFONT,
                GetStockObject(DEFAULT_GUI_FONT) as usize,
                1,
            );
        }
        EnableWindow(owner, 0);
        ShowWindow(hwnd, SW_SHOWNORMAL);
        SetFocus(GetDlgItem(hwnd, 202));
        let mut msg = MSG::default();
        while !CLOSED.get() && ACTION.get() == 0 {
            let result = GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0);
            if result <= 0 {
                if result == 0 {
                    PostQuitMessage(msg.wParam as i32);
                }
                break;
            }
            if msg.message == WM_KEYDOWN && msg.wParam == 27 {
                break;
            }
            if IsDialogMessageW(hwnd, &msg) == 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        let accepted = ACTION.get() == 201;
        EnableWindow(owner, 1);
        DestroyWindow(hwnd);
        SetActiveWindow(owner);
        THEME.with(|slot| {
            slot.borrow_mut().take();
        });
        accepted
    }
    unsafe fn text(
        get: impl FnOnce(*mut PWSTR) -> windows::core::Result<()>,
    ) -> windows::core::Result<String> {
        let mut value = PWSTR::null();
        get(&mut value)?;
        Ok(CoTaskMemPWSTR::from(value).to_string())
    }
    unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wp: usize, lp: isize) -> isize {
        match msg {
            WM_ERASEBKGND => THEME.with(|slot| {
                if let Some(theme) = slot.borrow().as_ref() {
                    let mut rect = RECT::default();
                    GetClientRect(hwnd, &mut rect);
                    FillRect(wp as HDC, &rect, theme.background);
                    1
                } else {
                    DefWindowProcW(hwnd, msg, wp, lp)
                }
            }),
            WM_CTLCOLORSTATIC | WM_CTLCOLOREDIT => THEME.with(|slot| {
                if let Some(theme) = slot.borrow().as_ref() {
                    let dc = wp as HDC;
                    let field = msg == WM_CTLCOLOREDIT;
                    SetTextColor(dc, theme.colors[4]);
                    SetBkColor(dc, theme.colors[if field { 2 } else { 0 }]);
                    if field {
                        theme.field as isize
                    } else {
                        theme.background as isize
                    }
                } else {
                    DefWindowProcW(hwnd, msg, wp, lp)
                }
            }),
            WM_DRAWITEM => THEME.with(|slot| {
                let state = slot.borrow();
                let Some(theme) = state.as_ref() else {
                    return 0;
                };
                let draw = &*(lp as *const DRAWITEMSTRUCT);
                let selected = draw.itemState & ODS_SELECTED != 0;
                let disabled = draw.itemState & ODS_DISABLED != 0;
                let brush = CreateSolidBrush(theme.colors[if selected { 6 } else { 2 }]);
                FillRect(draw.hDC, &draw.rcItem, brush);
                DeleteObject(brush as _);
                let border = CreateSolidBrush(theme.colors[3]);
                FrameRect(draw.hDC, &draw.rcItem, border);
                DeleteObject(border as _);
                let mut label = [0u16; 256];
                let len = GetWindowTextW(draw.hwndItem, label.as_mut_ptr(), label.len() as i32);
                let old_font = SelectObject(draw.hDC, GetStockObject(DEFAULT_GUI_FONT));
                SetBkMode(draw.hDC, TRANSPARENT as i32);
                SetTextColor(draw.hDC, theme.colors[if disabled { 5 } else { 4 }]);
                let mut rect = draw.rcItem;
                DrawTextW(
                    draw.hDC,
                    label.as_mut_ptr(),
                    len,
                    &mut rect,
                    DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS,
                );
                if draw.itemState & ODS_FOCUS != 0 {
                    InflateRect(&mut rect, -4, -4);
                    DrawFocusRect(draw.hDC, &rect);
                }
                SelectObject(draw.hDC, old_font);
                1
            }),
            WM_SIZE => {
                layout(hwnd);
                0
            }
            WM_COMMAND => {
                let id = (wp & 0xffff) as u32;
                if ((wp >> 16) & 0xffff) == BN_CLICKED as usize
                    && matches!(id, KEY_PAGE | DOWNLOAD_PAGE | SIGN_IN | FINISH | LOCAL_FILE)
                {
                    ACTION.set(id);
                }
                0
            }
            WM_GETMINMAXINFO => {
                let limits = &mut *(lp as *mut MINMAXINFO);
                let scale =
                    windows_sys::Win32::UI::HiDpi::GetDpiForWindow(hwnd).max(96) as f64 / 96.0;
                limits.ptMinTrackSize.x = (760.0 * scale).round() as i32;
                limits.ptMinTrackSize.y = (620.0 * scale).round() as i32;
                0
            }
            WM_CLOSE => {
                CLOSED.set(true);
                0
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
    unsafe fn pump(hwnd: HWND, cancel: &AtomicBool, started: Instant) -> Result<(), String> {
        if cancel.load(Ordering::Relaxed) || CLOSED.get() {
            return Err("Spotify setup cancelled".into());
        }
        if started.elapsed() > Duration::from_secs(1800) {
            return Err("Spotify setup timed out".into());
        }
        if IsWindow(hwnd) == 0 {
            return Err("Spotify setup window closed".into());
        }
        let mut message = MSG::default();
        while PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
            if message.message == WM_QUIT {
                return Err("Spotify setup window closed".into());
            }
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        if cancel.load(Ordering::Relaxed) || CLOSED.get() {
            return Err("Spotify setup cancelled".into());
        }
        std::thread::sleep(Duration::from_millis(10));
        Ok(())
    }
    unsafe fn wait<T>(
        slot: &Rc<RefCell<Option<Result<T, String>>>>,
        hwnd: HWND,
        cancel: &AtomicBool,
        started: Instant,
    ) -> Result<T, String> {
        loop {
            if let Some(value) = slot.borrow_mut().take() {
                return value;
            }
            pump(hwnd, cancel, started)?;
        }
    }
    struct Window(HWND);
    impl Drop for Window {
        fn drop(&mut self) {
            unsafe {
                CONTROLLER.with(|slot| {
                    if let Some(controller) = slot.borrow_mut().take() {
                        let _ = controller.Close();
                    }
                });
                if IsWindow(self.0) != 0 {
                    SetWindowTextW(GetDlgItem(self.0, KEY as i32), wide("").as_ptr());
                    DestroyWindow(self.0);
                }
                THEME.with(|slot| {
                    slot.borrow_mut().take();
                });
            }
        }
    }

    pub(crate) const NOTICE: &str = "Spotify playback uses Spotify Soloist, installed separately.\n\nYou need Spotify Premium for initial setup and your own personal Soloist API key. Spotify may ask you to accept its terms.\n\nThe setup window opens Spotify's official pages. Download Windows x64 Soloist there and paste your personal key into the setup window. The download will be checked and installed on this PC. Your Spotify web session will also be captured and saved securely.\n\nSoloist builds expire after 90 days. After setup, Soloist starts and opens Spotify for pairing. Select DeezNats once; saved pairing is restored later.\n\nContinue to Spotify setup?";
    const KEY_PAGE: u32 = 101;
    const DOWNLOAD_PAGE: u32 = 102;
    const SIGN_IN: u32 = 103;
    const KEY: u32 = 104;
    const FINISH: u32 = 105;
    const STATUS: u32 = 106;
    const HEADING: u32 = 107;
    const KEY_LABEL: u32 = 108;
    const LOCAL_FILE: u32 = 109;
    const WINDOWS_DOWNLOAD: &str =
        "https://soloist-builds.spotifycdn.com/soloist_release_windows_x86_64.zip";
    type Translations = Arc<HashMap<String, String>>;

    unsafe fn controls(hwnd: HWND, tr: &impl Fn(&str) -> String) -> Result<(), String> {
        let instance = GetModuleHandleW(std::ptr::null());
        for (id, class, label, extra) in [
            (HEADING, "STATIC", "Sign in to Spotify to finish setup.", 0),
            (KEY_PAGE, "BUTTON", "1. Get personal key", WS_TABSTOP),
            (DOWNLOAD_PAGE, "BUTTON", "2. Download Soloist", WS_TABSTOP),
            (SIGN_IN, "BUTTON", "Spotify sign-in", WS_TABSTOP),
            (LOCAL_FILE, "BUTTON", "Use local ZIP / EXE", WS_TABSTOP),
            (KEY_LABEL, "STATIC", "Personal key", 0),
            (
                KEY,
                "EDIT",
                "",
                WS_TABSTOP | ES_PASSWORD as u32 | ES_AUTOHSCROLL as u32 | WS_BORDER,
            ),
            (FINISH, "BUTTON", "Finish setup", WS_TABSTOP),
            (
                STATUS,
                "STATIC",
                "Sign in to Spotify. Setup will continue automatically.",
                0,
            ),
        ] {
            let child = CreateWindowExW(
                0,
                wide(class).as_ptr(),
                wide(&tr(label)).as_ptr(),
                WS_CHILD
                    | WS_VISIBLE
                    | extra
                    | if class == "BUTTON" {
                        BS_OWNERDRAW as u32
                    } else {
                        0
                    },
                0,
                0,
                1,
                1,
                hwnd,
                id as usize as HMENU,
                instance,
                std::ptr::null(),
            );
            if child.is_null() {
                return Err("Could not create Spotify setup controls".into());
            }
            SendMessageW(
                child,
                WM_SETFONT,
                GetStockObject(DEFAULT_GUI_FONT) as usize,
                1,
            );
        }
        SendMessageW(GetDlgItem(hwnd, KEY as i32), EM_SETLIMITTEXT, 4096, 0);
        let initial_key = std::env::var("DEEZNATS_SOLOIST_API_KEY")
            .ok()
            .or_else(|| super::soloist::Config::saved().ok().map(|c| c.api_key));
        if let Some(key) = initial_key {
            SetWindowTextW(GetDlgItem(hwnd, KEY as i32), wide(&key).as_ptr());
        }
        layout(hwnd);
        Ok(())
    }
    unsafe fn layout(hwnd: HWND) {
        let mut rect = RECT::default();
        GetClientRect(hwnd, &mut rect);
        let scale = windows_sys::Win32::UI::HiDpi::GetDpiForWindow(hwnd).max(96) as f64 / 96.0;
        let px = |v: i32| (v as f64 * scale).round() as i32;
        let width = rect.right;
        let height = rect.bottom;
        for (id, x, y, w, h) in [
            (HEADING, px(16), px(10), width - px(32), px(24)),
            (KEY_PAGE, px(16), px(42), px(180), px(32)),
            (DOWNLOAD_PAGE, px(208), px(42), px(180), px(32)),
            (SIGN_IN, px(400), px(42), px(160), px(32)),
            (LOCAL_FILE, px(572), px(42), px(164), px(32)),
            (KEY_LABEL, px(16), height - px(108), px(116), px(24)),
            (
                KEY,
                px(136),
                height - px(112),
                (width - px(336)).max(px(90)),
                px(30),
            ),
            (FINISH, width - px(180), height - px(112), px(164), px(32)),
            (STATUS, px(16), height - px(68), width - px(32), px(56)),
        ] {
            let control = GetDlgItem(hwnd, id as i32);
            if !control.is_null() {
                MoveWindow(control, x, y, w.max(1), h.max(1), 1);
            }
        }
        CONTROLLER.with(|slot| {
            if let Some(controller) = slot.borrow().as_ref() {
                let _ = controller.SetBounds(windows::Win32::Foundation::RECT {
                    left: 0,
                    top: px(88),
                    right: width,
                    bottom: (height - px(132)).max(px(100)),
                });
            }
        });
    }
    unsafe fn key_text(hwnd: HWND) -> String {
        let control = GetDlgItem(hwnd, KEY as i32);
        let count = GetWindowTextLengthW(control).clamp(0, 4096) as usize;
        let mut text = vec![0u16; count + 1];
        let len = GetWindowTextW(control, text.as_mut_ptr(), text.len() as i32).max(0) as usize;
        let value = String::from_utf16_lossy(&text[..len]).trim().to_owned();
        text.fill(0);
        value
    }
    fn allowed_page(uri: &str) -> bool {
        let Ok(url) = url::Url::parse(uri) else {
            return false;
        };
        url.scheme() == "https"
            && url.port_or_known_default() == Some(443)
            && url.username().is_empty()
            && url.password().is_none()
            && url.host_str().is_some_and(|host| {
                host == "spotify.com"
                    || host.ends_with(".spotify.com")
                    || matches!(
                        host,
                        "soloist-builds.spotifycdn.com"
                            | "accounts.google.com"
                            | "appleid.apple.com"
                            | "account.apple.com"
                    )
            })
    }
    fn allowed_download(uri: &str) -> bool {
        let Ok(url) = url::Url::parse(uri) else {
            return false;
        };
        allowed_page(uri)
            && url.host_str() == Some("soloist-builds.spotifycdn.com")
            && url.path().ends_with("/soloist_release_windows_x86_64.zip")
    }
    enum Event {
        Download(PathBuf),
        Status(&'static str),
    }
    unsafe fn local_file(hwnd: HWND) -> Option<PathBuf> {
        let mut path = vec![0u16; 32768];
        let filter =
            wide("Soloist ZIP or executable (*.zip;*.exe)\0*.zip;*.exe\0All files (*.*)\0*.*\0");
        let mut dialog: OPENFILENAMEW = std::mem::zeroed();
        dialog.lStructSize = std::mem::size_of::<OPENFILENAMEW>() as u32;
        dialog.hwndOwner = hwnd;
        dialog.lpstrFilter = filter.as_ptr();
        dialog.lpstrFile = path.as_mut_ptr();
        dialog.nMaxFile = path.len() as u32;
        dialog.Flags = OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR;
        if GetOpenFileNameW(&mut dialog) == 0 {
            return None;
        }
        let end = path.iter().position(|&v| v == 0)?;
        Some(PathBuf::from(String::from_utf16_lossy(&path[..end])))
    }
    struct Downloads(Vec<(ICoreWebView2DownloadOperation, i64)>);
    impl Drop for Downloads {
        fn drop(&mut self) {
            unsafe {
                for (operation, token) in &self.0 {
                    let _ = operation.remove_StateChanged(*token);
                    let mut state = COREWEBVIEW2_DOWNLOAD_STATE_IN_PROGRESS;
                    if operation.State(&mut state).is_ok()
                        && state == COREWEBVIEW2_DOWNLOAD_STATE_IN_PROGRESS
                    {
                        let _ = operation.Cancel();
                    }
                }
            }
        }
    }
    pub(crate) fn run(
        cancel: &Arc<AtomicBool>,
        translations: Translations,
        palette: [u32; 9],
    ) -> Result<(), String> {
        unsafe {
            if CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32) < 0 {
                return Err("Could not initialize Spotify setup".into());
            }
            CLOSED.set(false);
            ACTION.set(0);
            let result = run_sta(cancel, translations, palette);
            CoUninitialize();
            result
        }
    }
    unsafe fn run_sta(
        cancel: &Arc<AtomicBool>,
        translations: Translations,
        palette: [u32; 9],
    ) -> Result<(), String> {
        THEME.with(|slot| {
            *slot.borrow_mut() = Some(WizardTheme {
                colors: palette,
                background: CreateSolidBrush(palette[0]),
                field: CreateSolidBrush(palette[2]),
            })
        });
        let tr = |text: &str| {
            translations
                .get(text)
                .cloned()
                .unwrap_or_else(|| text.into())
        };
        let instance = GetModuleHandleW(std::ptr::null());
        let class_name = wide("DeezNatsSpotifySetup");
        let class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            lpszClassName: class_name.as_ptr(),
            hCursor: LoadCursorW(std::ptr::null_mut(), IDC_ARROW),
            hbrBackground: GetStockObject(BLACK_BRUSH) as HBRUSH,
            ..Default::default()
        };
        RegisterClassW(&class);
        let hwnd = CreateWindowExW(
            0,
            class_name.as_ptr(),
            wide(&tr("Spotify playback setup")).as_ptr(),
            WS_OVERLAPPEDWINDOW,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            900,
            850,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            instance,
            std::ptr::null(),
        );
        if hwnd.is_null() {
            THEME.with(|slot| {
                slot.borrow_mut().take();
            });
            return Err("Could not create Spotify sign-in window".into());
        }
        let _window = Window(hwnd);
        let dark = ((palette[0] & 255) * 299
            + ((palette[0] >> 8) & 255) * 587
            + ((palette[0] >> 16) & 255) * 114)
            < 128000;
        let dark_mode = dark as i32;
        DwmSetWindowAttribute(hwnd, 20, &dark_mode as *const _ as _, 4);
        DwmSetWindowAttribute(hwnd, 35, &palette[0] as *const _ as _, 4);
        DwmSetWindowAttribute(hwnd, 36, &palette[4] as *const _ as _, 4);
        controls(hwnd, &tr)?;
        ShowWindow(hwnd, SW_SHOWNORMAL);
        let started = Instant::now();
        let folder = crate::storage::data_dir()?.join("SpotifySetupWebView");
        std::fs::create_dir_all(&folder)
            .map_err(|_| "Could not prepare the isolated Spotify browser profile")?;
        let folder = wide(&folder.to_string_lossy());
        let environment = Rc::new(RefCell::new(None));
        let output = environment.clone();
        CreateCoreWebView2EnvironmentWithOptions(
            PCWSTR::null(),
            PCWSTR(folder.as_ptr()),
            None,
            &CreateCoreWebView2EnvironmentCompletedHandler::create(Box::new(
                move |result, environment| {
                    *output.borrow_mut() = Some(
                    result
                        .map_err(|_| {
                            "WebView2 Runtime could not start. Use normal browser sign-in instead."
                                .into()
                        })
                        .and_then(|_| {
                            environment.ok_or_else(|| "WebView2 environment is unavailable".into())
                        }),
                );
                    Ok(())
                },
            )),
        )
        .map_err(|_| "WebView2 Runtime is unavailable. Use normal browser sign-in instead.")?;
        let environment: ICoreWebView2Environment = wait(&environment, hwnd, cancel, started)?;
        let modern: ICoreWebView2Environment10 = environment
            .cast()
            .map_err(|_| "Update WebView2 Runtime to use private sign-in")?;
        let options = modern
            .CreateCoreWebView2ControllerOptions()
            .map_err(|_| "Private login options are unavailable")?;
        options
            .SetIsInPrivateModeEnabled(true)
            .map_err(|_| "Could not enable private Spotify sign-in")?;
        let controller = Rc::new(RefCell::new(None));
        let output = controller.clone();
        modern
            .CreateCoreWebView2ControllerWithOptions(
                windows::Win32::Foundation::HWND(hwnd),
                &options,
                &CreateCoreWebView2ControllerCompletedHandler::create(Box::new(
                    move |result, controller| {
                        *output.borrow_mut() = Some(
                            result
                                .map_err(|_| "Spotify WebView2 controller could not start".into())
                                .and_then(|_| {
                                    controller.ok_or_else(|| {
                                        "Spotify WebView2 controller is unavailable".into()
                                    })
                                }),
                        );
                        Ok(())
                    },
                )),
            )
            .map_err(|_| "Could not open private Spotify sign-in")?;
        let controller: ICoreWebView2Controller = wait(&controller, hwnd, cancel, started)?;
        CONTROLLER.with(|slot| *slot.borrow_mut() = Some(controller.clone()));
        layout(hwnd);
        let mut bounds = RECT::default();
        GetClientRect(hwnd, &mut bounds);
        controller
            .SetBounds(windows::Win32::Foundation::RECT {
                left: 0,
                top: 0,
                right: bounds.right,
                bottom: bounds.bottom,
            })
            .map_err(|_| "Could not size the Spotify window")?;
        controller
            .SetIsVisible(true)
            .map_err(|_| "Could not show Spotify sign-in")?;
        let webview = controller
            .CoreWebView2()
            .map_err(|_| "Spotify WebView2 is unavailable")?;
        let installed = Rc::new(RefCell::new(None));
        let output = installed.clone();
        webview
            .AddScriptToExecuteOnDocumentCreated(
                PCWSTR(wide(super::web::BOOTSTRAP).as_ptr()),
                &AddScriptToExecuteOnDocumentCreatedCompletedHandler::create(Box::new(
                    move |result, _| {
                        *output.borrow_mut() = Some(
                            result
                                .map(|_| ())
                                .map_err(|_| "Could not capture Spotify API session".into()),
                        );
                        Ok(())
                    },
                )),
            )
            .map_err(|_| "Could not prepare Spotify session capture")?;
        wait(&installed, hwnd, cancel, started)?;
        let settings = webview
            .Settings()
            .map_err(|_| "Spotify WebView2 settings unavailable")?;
        settings
            .SetAreHostObjectsAllowed(false)
            .map_err(|_| "Could not isolate Spotify sign-in")?;
        settings
            .SetIsWebMessageEnabled(false)
            .map_err(|_| "Could not isolate Spotify messages")?;
        settings
            .SetAreDevToolsEnabled(false)
            .map_err(|_| "Could not configure Spotify sign-in")?;
        settings
            .SetAreDefaultContextMenusEnabled(false)
            .map_err(|_| "Could not configure Spotify context menus")?;
        let mut permission_token = 0;
        webview
            .add_PermissionRequested(
                &PermissionRequestedEventHandler::create(Box::new(|_, args| {
                    if let Some(args) = args {
                        let mut kind = COREWEBVIEW2_PERMISSION_KIND_UNKNOWN_PERMISSION;
                        if args.PermissionKind(&mut kind).is_ok() {
                            match kind {
                                COREWEBVIEW2_PERMISSION_KIND_CAMERA
                                | COREWEBVIEW2_PERMISSION_KIND_MICROPHONE
                                | COREWEBVIEW2_PERMISSION_KIND_GEOLOCATION
                                | COREWEBVIEW2_PERMISSION_KIND_NOTIFICATIONS
                                | COREWEBVIEW2_PERMISSION_KIND_OTHER_SENSORS => {
                                    let _ = args.SetState(COREWEBVIEW2_PERMISSION_STATE_DENY);
                                }
                                _ => {
                                    let _ = args.SetState(COREWEBVIEW2_PERMISSION_STATE_ALLOW);
                                }
                            }
                        } else {
                            let _ = args.SetState(COREWEBVIEW2_PERMISSION_STATE_DENY);
                        }
                    }
                    Ok(())
                })),
                &mut permission_token,
            )
            .map_err(|_| "Could not isolate Spotify permissions")?;
        let events = Rc::new(RefCell::new(VecDeque::<Event>::new()));
        let downloading = Rc::new(Cell::new(false));
        let operations = Rc::new(RefCell::new(Downloads(Vec::new())));
        let modern: ICoreWebView2_4 = webview
            .cast()
            .map_err(|_| "Update WebView2 Runtime to download Soloist")?;
        let mut download_token = 0;
        let download_events = events.clone();
        let download_busy = downloading.clone();
        let download_operations = operations.clone();
        modern
            .add_DownloadStarting(
                &DownloadStartingEventHandler::create(Box::new(move |_, args| {
                    if let Some(args) = args {
                        let operation = args.DownloadOperation()?;
                        let uri = text(|out| operation.Uri(out))?;
                        if !allowed_download(&uri) || download_busy.get() {
                            let _ = args.SetCancel(true);
                            download_events.borrow_mut().push_back(Event::Status(
                                "Only the official Windows x64 Soloist download is supported here.",
                            ));
                            return Ok(());
                        }
                        let mut size = 0;
                        if operation.TotalBytesToReceive(&mut size).is_ok()
                            && size > 512 * 1024 * 1024
                        {
                            let _ = args.SetCancel(true);
                            download_events
                                .borrow_mut()
                                .push_back(Event::Status("The Soloist download is too large."));
                            return Ok(());
                        }
                        let directory = match crate::storage::data_dir().and_then(|path| {
                            let unique = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_nanos();
                            let folder = path
                                .join("spotify-soloist-install")
                                .join("downloads")
                                .join(format!("{}-{unique}", std::process::id()));
                            std::fs::create_dir_all(&folder)
                                .map_err(|_| "Could not prepare the Soloist download".to_owned())?;
                            Ok(folder)
                        }) {
                            Ok(path) => path,
                            Err(_) => {
                                let _ = args.SetCancel(true);
                                download_events.borrow_mut().push_back(Event::Status(
                                    "Could not prepare the Soloist download.",
                                ));
                                return Ok(());
                            }
                        };
                        let path = directory.join("soloist_release_windows_x86_64.zip");
                        let clean_path = super::setup::strip_verbatim(&path).to_path_buf();
                        args.SetResultFilePath(PCWSTR(
                            wide(&clean_path.to_string_lossy()).as_ptr(),
                        ))?;
                        args.SetHandled(true)?;
                        let output = download_events.clone();
                        let busy = download_busy.clone();
                        let saved_path = clean_path.clone();
                        let terminal = Cell::new(false);
                        let mut token = 0;
                        operation.add_StateChanged(
                            &StateChangedEventHandler::create(Box::new(move |sender, _| {
                                if let Some(sender) = sender {
                                    let mut state = COREWEBVIEW2_DOWNLOAD_STATE_IN_PROGRESS;
                                    sender.State(&mut state)?;
                                    if state != COREWEBVIEW2_DOWNLOAD_STATE_IN_PROGRESS
                                        && terminal.replace(true)
                                    {
                                        return Ok(());
                                    }
                                    if state == COREWEBVIEW2_DOWNLOAD_STATE_COMPLETED {
                                        output
                                            .borrow_mut()
                                            .push_back(Event::Download(saved_path.clone()));
                                    } else if state == COREWEBVIEW2_DOWNLOAD_STATE_INTERRUPTED {
                                        busy.set(false);
                                        output.borrow_mut().push_back(Event::Status(
                                        "Soloist download was interrupted. Try the download again.",
                                    ));
                                    }
                                }
                                Ok(())
                            })),
                            &mut token,
                        )?;
                        download_operations.borrow_mut().0.push((operation, token));
                        download_busy.set(true);
                        download_events
                            .borrow_mut()
                            .push_back(Event::Status("Downloading Soloist..."));
                    }
                    Ok(())
                })),
                &mut download_token,
            )
            .map_err(|_| "Could not configure the Soloist download")?;
        let mut nav_token = 0;
        let nav_events = events.clone();
        webview
            .add_NavigationStarting(
                &NavigationStartingEventHandler::create(Box::new(move |_, args| {
                    if let Some(args) = args {
                        let uri = text(|out| args.Uri(out))?;
                        if uri != "about:blank" && !allowed_page(&uri) {
                            args.SetCancel(true)?;
                            nav_events.borrow_mut().push_back(Event::Status(
                                "This setup window only opens Spotify and supported sign-in pages.",
                            ));
                        }
                    }
                    Ok(())
                })),
                &mut nav_token,
            )
            .map_err(|_| "Could not configure Spotify setup navigation")?;
        let mut popup_token = 0;
        let browser = webview.clone();
        webview
            .add_NewWindowRequested(
                &NewWindowRequestedEventHandler::create(Box::new(move |_, args| {
                    if let Some(args) = args {
                        args.SetHandled(true)?;
                        let uri = text(|out| args.Uri(out))?;
                        if allowed_page(&uri) {
                            browser.Navigate(PCWSTR(wide(&uri).as_ptr()))?;
                        }
                    }
                    Ok(())
                })),
                &mut popup_token,
            )
            .map_err(|_| "Could not configure Spotify setup popups")?;
        let cookies = webview
            .cast::<ICoreWebView2_2>()
            .map_err(|_| "Update WebView2 Runtime for Spotify setup")?
            .CookieManager()
            .map_err(|_| "Spotify cookie manager unavailable")?;
        let captured = Rc::new(RefCell::new(None::<String>));
        let pending = Rc::new(Cell::new(false));
        let mut checked = Instant::now() - Duration::from_secs(1);
        let inspect = std::env::args().any(|arg| arg == "--inspect-soloist-dashboard");
        let mut inspected = Instant::now() - Duration::from_secs(3);
        let mut installing: Option<mpsc::Receiver<Result<PathBuf, String>>> = None;
        let mut selected_executable = None;
        layout(hwnd);
        let mut key_checked = Instant::now() - Duration::from_secs(2);
        let key_busy = Rc::new(Cell::new(false));
        let captured_key = Rc::new(RefCell::new(None::<String>));
        let mut auto_finish_attempted = false;
        let mut initial_download = !inspect && super::setup::preferred_executable().is_err();
        let mut download_receiver = if initial_download {
            let (send, receive) = mpsc::sync_channel::<Result<PathBuf, String>>(1);
            downloading.set(true);
            std::thread::Builder::new()
                .name("soloist-download".into())
                .spawn(move || {
                    let result = (|| {
                        let folder = crate::storage::data_dir()?
                            .join("spotify-soloist-install")
                            .join("downloads");
                        std::fs::create_dir_all(&folder)
                            .map_err(|_| "Could not prepare Soloist download".to_owned())?;
                        let path = folder.join(format!(
                            "setup-{}-{}.zip",
                            std::process::id(),
                            std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_nanos()
                        ));
                        let result = (|| {
                            let mut file = std::fs::File::create(&path)
                                .map_err(|_| "Could not prepare Soloist download".to_owned())?;
                            crate::updater::http::get(
                                WINDOWS_DOWNLOAD,
                                &mut file,
                                512 * 1024 * 1024,
                                |_| {},
                            )?;
                            Ok(path.clone())
                        })();
                        if result.is_err() {
                            let _ = std::fs::remove_file(path);
                        }
                        result
                    })();
                    if let Err(mpsc::SendError(Ok(path))) = send.send(result) {
                        let _ = std::fs::remove_file(path);
                    }
                })
                .map_err(|_| "Could not start Soloist download")?;
            Some(receive)
        } else {
            None
        };
        webview
            .Navigate(PCWSTR(wide(super::setup::KEY_PAGE).as_ptr()))
            .map_err(|_| "Could not open Spotify setup")?;
        let result = loop {
            if let Err(error) = pump(hwnd, cancel, started) {
                break Err(error);
            }
            if !inspect
                && key_checked.elapsed() >= Duration::from_secs(2)
                && !key_busy.get()
                && captured_key.borrow().is_none()
            {
                key_checked = Instant::now();
                key_busy.set(true);
                let busy = key_busy.clone();
                let result_key = captured_key.clone();
                let script = r#"(() => {
              if (location.origin !== 'https://developer.spotify.com' || !location.pathname.startsWith('/dashboard/soloist')) return {dashboard:false};
              if (typeof window.__deeznatsSoloistKey === 'string') {
                const key = window.__deeznatsSoloistKey; delete window.__deeznatsSoloistKey; return key;
              }
              const heading = [...document.querySelectorAll('h1,h2,h3')].find(e => /soloist/i.test(e.textContent) && /api key/i.test(e.textContent));
              if (!heading) return {dashboard:true, heading:false};
              let root = heading; for (let i=0; i<4 && root.parentElement; i++) root = root.parentElement;
              const buttons = [...root.querySelectorAll('button')];
              const label = e => [e.innerText, e.getAttribute('aria-label'), e.getAttribute('title')].filter(Boolean).join(' ').toLowerCase();
              const copy = buttons.find(e => /copy/.test(label(e)));
              if (copy && !window.__deeznatsCopyStarted && navigator.clipboard) {
                window.__deeznatsCopyStarted = true;
                const clipboard = navigator.clipboard;
                try {
                  Object.defineProperty(clipboard, 'writeText', {configurable: true, value: function(value) {
                    if (typeof value === 'string') window.__deeznatsSoloistKey = value;
                    delete clipboard.writeText;
                    return Promise.resolve();
                  }});
                  copy.click();
                  setTimeout(() => { delete clipboard.writeText; }, 10000);
                } catch (_) { window.__deeznatsCopyStarted = false; }
              } else if (!copy && !window.__deeznatsGenerateStarted) {
                const generate = buttons.find(e => /generate/.test(label(e)) && /api key/.test(label(e)) && !/regenerate/.test(label(e)));
                if (generate) { window.__deeznatsGenerateStarted = true; generate.click(); }
              }
              return {dashboard:true, heading:true, copy:!!copy, clipboard:!!navigator.clipboard, started:!!window.__deeznatsCopyStarted};
            })()"#;
                if webview
                    .ExecuteScript(
                        PCWSTR(wide(script).as_ptr()),
                        &ExecuteScriptCompletedHandler::create(Box::new(move |result, value| {
                            busy.set(false);
                            if result.is_ok() {
                                if let Ok(serde_json::Value::String(key)) =
                                    serde_json::from_str::<serde_json::Value>(&value.to_string())
                                {
                                    let key = key.trim();
                                    if !key.is_empty()
                                        && key.len() <= 4096
                                        && !key.chars().any(char::is_control)
                                    {
                                        *result_key.borrow_mut() = Some(key.to_owned());
                                    }
                                }
                            }
                            Ok(())
                        })),
                    )
                    .is_err()
                {
                    key_busy.set(false);
                }
            }
            if let Some(result) =
                download_receiver
                    .as_ref()
                    .and_then(|receiver| match receiver.try_recv() {
                        Ok(result) => Some(result),
                        Err(mpsc::TryRecvError::Disconnected) => {
                            Some(Err("Soloist download stopped unexpectedly.".into()))
                        }
                        Err(mpsc::TryRecvError::Empty) => None,
                    })
            {
                download_receiver = None;
                match result {
                    Ok(path) => events.borrow_mut().push_back(Event::Download(path)),
                    Err(error) => {
                        downloading.set(false);
                        initial_download = false;
                        SetWindowTextW(GetDlgItem(hwnd, STATUS as i32), wide(&error).as_ptr());
                    }
                }
            }
            if inspect && inspected.elapsed() >= Duration::from_secs(3) {
                inspected = Instant::now();
                // Only report categorical structure: never read values, cookies, URLs,
                // arbitrary attributes, or page text into the diagnostic file.
                let script = r#"(() => {
              if (location.origin !== 'https://developer.spotify.com' || !location.pathname.startsWith('/dashboard/soloist')) return null;
              const categories = t => {
                t = (t || '').toLowerCase();
                return ['soloist','api key','generate','create','copy','show','reveal','regenerate','terms','accept'].filter(x => t.includes(x));
              };
              const heading = [...document.querySelectorAll('h1,h2,h3')].find(e => /soloist/i.test(e.innerText) && /api key/i.test(e.innerText));
              if (!heading) return null;
              let root = heading; for (let i=0; i<4 && root.parentElement; i++) root = root.parentElement;
              const elements = [root, ...root.querySelectorAll('*')].slice(0, 200);
              return elements.map(e => ({
                tag: e.tagName.toLowerCase(),
                type: ['password','text','button','submit','checkbox'].includes(e.type) ? e.type : '',
                role: e.getAttribute('role') === 'button' ? 'button' : '',
                labels: categories(e.innerText || e.getAttribute('aria-label') || e.getAttribute('placeholder')),
                parent: e.parentElement ? e.parentElement.tagName.toLowerCase() : '',
                sibling: e.parentElement ? [...e.parentElement.children].indexOf(e) : 0,
                key_like: !e.children.length && /^[A-Za-z0-9_-]{20,4096}$/.test((e.value || e.textContent || '').trim())
              }));
            })()"#;
                let _ = webview.ExecuteScript(
                PCWSTR(wide(script).as_ptr()),
                &ExecuteScriptCompletedHandler::create(Box::new(|result, value| {
                    if result.is_ok() {
                        let raw = value.to_string();
                        if let Ok(serde_json::Value::Array(items)) = serde_json::from_str::<serde_json::Value>(&raw) {
                            // Revalidate every value against fixed vocabulary before writing.
                            let tags = ["button", "input", "textarea", "label", "h1", "h2", "h3", "div", "span", "form", "section", "main", "p", "li"];
                            let labels = ["soloist", "api key", "generate", "create", "copy", "show", "reveal", "regenerate", "terms", "accept"];
                            let clean: Vec<_> = items.iter().take(200).filter_map(|item| {
                                let tag = item.get("tag")?.as_str()?;
                                let parent = item.get("parent")?.as_str()?;
                                if !tags.contains(&tag) || (!parent.is_empty() && !tags.contains(&parent)) { return None; }
                                let typ = item.get("type")?.as_str()?;
                                if !["", "password", "text", "button", "submit", "checkbox"].contains(&typ) { return None; }
                                let words: Vec<_> = item.get("labels")?.as_array()?.iter().filter_map(|v| v.as_str()).filter(|v| labels.contains(v)).collect();
                                Some(serde_json::json!({"tag": tag, "type": typ, "parent": parent, "sibling": item.get("sibling")?.as_u64()?.min(200), "key_like": item.get("key_like")?.as_bool()?, "labels": words}))
                            }).collect();
                            if let Ok(bytes) = serde_json::to_vec_pretty(&clean) {
                                let _ = std::fs::write(".privateinfo/soloist-dashboard-structure.json", bytes);
                            }
                        }
                    }
                    Ok(())
                })),
            );
            }
            if checked.elapsed() >= Duration::from_millis(750) && !pending.get() {
                checked = Instant::now();
                pending.set(true);
                let busy = pending.clone();
                let output = captured.clone();
                if cookies
                    .GetCookies(
                        PCWSTR(wide("").as_ptr()),
                        &GetCookiesCompletedHandler::create(Box::new(move |result, list| {
                            busy.set(false);
                            if result.is_ok() {
                                if let Some(list) = list {
                                    let mut count = 0;
                                    list.Count(&mut count)?;
                                    let mut found = None;
                                    for i in 0..count.min(1024) {
                                        let cookie = list.GetValueAtIndex(i)?;
                                        let name = text(|out| cookie.Name(out))?;
                                        if name != "sp_dc" {
                                            continue;
                                        }
                                        let domain = text(|out| cookie.Domain(out))?;
                                        let value = text(|out| cookie.Value(out))?;
                                        let mut secure = windows::core::BOOL(0);
                                        cookie.IsSecure(&mut secure)?;
                                        if crate::spotify_webview::valid_spotify_cookie(
                                            &name,
                                            &domain,
                                            secure.as_bool(),
                                            &value,
                                        ) {
                                            found = Some(value);
                                            break;
                                        }
                                    }
                                    *output.borrow_mut() = found;
                                }
                            }
                            Ok(())
                        })),
                    )
                    .is_err()
                {
                    pending.set(false);
                }
            }
            let queued: Vec<_> = events.borrow_mut().drain(..).collect();
            for event in queued {
                match event {
                    Event::Status(message) => {
                        SetWindowTextW(
                            GetDlgItem(hwnd, STATUS as i32),
                            wide(&tr(message)).as_ptr(),
                        );
                    }
                    Event::Download(path) => {
                        let remove_download = initial_download;
                        initial_download = false;
                        let flag = cancel.clone();
                        let (send, receive) = mpsc::sync_channel(1);
                        match std::thread::Builder::new()
                        .name("soloist-install".into())
                        .spawn(move || {
                            let result = (|| {
                                if flag.load(Ordering::Relaxed) {
                                    return Err("Spotify setup cancelled".into());
                                }
                                if path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("exe")) {
                                    let build = super::setup::check(&path)?;
                                    if !build.usable() {
                                        return Err("Selected Soloist build is expired or has an invalid future build date".into());
                                    }
                                    return Ok(build.executable);
                                }
                                let installation = super::setup::import_archive(&path)?;
                                if flag.load(Ordering::Relaxed) {
                                    return Err("Spotify setup cancelled".into());
                                }
                                super::setup::activate(&installation.id)?;
                                Ok(installation.executable)
                            })();
                            if remove_download { let _ = std::fs::remove_file(&path); }
                            let _ = send.send(result);
                        }) {
                        Ok(_) => {
                            installing = Some(receive);
                            EnableWindow(GetDlgItem(hwnd, FINISH as i32), 0);
                            SetWindowTextW(
                                GetDlgItem(hwnd, STATUS as i32),
                                wide(&tr("Checking and installing Soloist...")).as_ptr(),
                            );
                        }
                        Err(_) => {
                            downloading.set(false);
                            SetWindowTextW(
                                GetDlgItem(hwnd, STATUS as i32),
                                wide(&tr("Could not start the Soloist installation.")).as_ptr(),
                            );
                        }
                    }
                    }
                }
            }
            let installed = installing
                .as_ref()
                .and_then(|receive| match receive.try_recv() {
                    Ok(result) => Some(result),
                    Err(mpsc::TryRecvError::Disconnected) => {
                        Some(Err("Soloist installation stopped unexpectedly.".into()))
                    }
                    Err(_) => None,
                });
            if let Some(result) = installed {
                installing = None;
                downloading.set(false);
                EnableWindow(GetDlgItem(hwnd, FINISH as i32), 1);
                let message = match result {
                    Ok(path) => {
                        selected_executable = Some(path);
                        tr("Soloist installed. Sign in to finish setup.")
                    }
                    Err(error) => tr(&error),
                };
                SetWindowTextW(GetDlgItem(hwnd, STATUS as i32), wide(&message).as_ptr());
            }
            if !inspect
                && !auto_finish_attempted
                && !initial_download
                && installing.is_none()
                && !downloading.get()
                && captured.borrow().is_some()
                && captured_key.borrow().is_some()
            {
                if let Some(key) = captured_key.borrow().as_deref() {
                    SetWindowTextW(GetDlgItem(hwnd, KEY as i32), wide(key).as_ptr());
                    auto_finish_attempted = true;
                    ACTION.set(FINISH);
                }
            }
            match ACTION.replace(0) {
                KEY_PAGE => {
                    let _ = webview.Navigate(PCWSTR(wide(super::setup::KEY_PAGE).as_ptr()));
                }
                DOWNLOAD_PAGE => {
                    if !downloading.get() && installing.is_none() {
                        if webview
                            .Navigate(PCWSTR(wide(WINDOWS_DOWNLOAD).as_ptr()))
                            .is_err()
                        {
                            SetWindowTextW(GetDlgItem(hwnd, STATUS as i32), wide(&tr("Could not start the Soloist download. Use a local ZIP or EXE instead.")).as_ptr());
                        }
                    }
                }
                LOCAL_FILE if installing.is_none() && !downloading.get() => {
                    if let Some(path) = local_file(hwnd) {
                        downloading.set(true);
                        events.borrow_mut().push_back(Event::Download(path));
                    }
                }
                SIGN_IN => {
                    let _ = webview.Navigate(PCWSTR(wide("https://open.spotify.com/").as_ptr()));
                }
                FINISH if installing.is_none() && !downloading.get() => {
                    let key = key_text(hwnd);
                    let cookie = captured.borrow().clone();
                    let save = (|| {
                        if key.is_empty() {
                            return Err(
                                "Paste your personal Soloist key into the field below.".to_owned()
                            );
                        }
                        let executable = selected_executable
                            .clone()
                            .map(Ok)
                            .unwrap_or_else(super::setup::preferred_executable)
                            .map_err(|_| {
                                "Download and install Soloist in this window before finishing."
                            })?;
                        if let Some(cookie) = cookie.as_deref() {
                            super::save_web_cookie(cookie)?;
                            crate::spotify_webview::capture_session(&webview, hwnd, cancel)?;
                        }
                        super::soloist::Config {
                            executable,
                            api_key: key,
                        }
                        .save()
                    })();
                    match save {
                        Ok(()) => break Ok(()),
                        Err(error) => {
                            SetWindowTextW(
                                GetDlgItem(hwnd, STATUS as i32),
                                wide(&tr(&error)).as_ptr(),
                            );
                        }
                    }
                }
                _ => {}
            }
        };
        // Cancellation also prevents an installation worker from activating after this window closes.
        cancel.store(true, Ordering::Relaxed);
        let _ = webview.remove_NavigationStarting(nav_token);
        let _ = webview.remove_NewWindowRequested(popup_token);
        let _ = webview.remove_PermissionRequested(permission_token);
        let _ = modern.remove_DownloadStarting(download_token);
        let completed_downloads =
            std::mem::replace(&mut *operations.borrow_mut(), Downloads(Vec::new()));
        drop(completed_downloads);
        result
    }
    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn download_requires_the_official_windows_archive_origin() {
            assert!(allowed_page(WINDOWS_DOWNLOAD));
            assert!(allowed_download(WINDOWS_DOWNLOAD));
            assert!(allowed_download(
                "https://soloist-builds.spotifycdn.com/releases/soloist_release_windows_x86_64.zip"
            ));
            for uri in [
                "https://evil.example/soloist_release_windows_x86_64.zip",
                "https://soloist-builds.spotifycdn.com.evil/soloist_release_windows_x86_64.zip",
                "http://soloist-builds.spotifycdn.com/soloist_release_windows_x86_64.zip",
                "https://soloist-builds.spotifycdn.com/soloist_release_arm64.tar.gz",
                "https://user@soloist-builds.spotifycdn.com/soloist_release_windows_x86_64.zip",
            ] {
                assert!(!allowed_download(uri));
            }
        }
        #[test]
        fn browser_navigation_is_limited_to_setup_and_identity_sites() {
            assert!(allowed_page(super::super::setup::KEY_PAGE));
            assert!(allowed_page("https://accounts.spotify.com/login"));
            assert!(!allowed_page("https://spotify.com.evil/login"));
            assert!(!allowed_page("file:///C:/secrets"));
            assert!(!allowed_page("javascript:alert(1)"));
        }
    }
}
static AUTH: Mutex<()> = Mutex::new(());
#[derive(Clone, Debug)]
pub(crate) enum AuthAction {
    Login,
    Logout,
    Refresh,
}
/// Window callers poll this receiver; browser/network work stays off the UI thread.
pub(crate) fn auth_async(
    action: AuthAction,
) -> Result<std::sync::mpsc::Receiver<Result<(), String>>, String> {
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("spotify-auth".into())
        .spawn(move || {
            let result = match action {
                AuthAction::Login => login_web(&AtomicBool::new(false)),
                AuthAction::Logout => logout(),
                AuthAction::Refresh => refresh(),
            };
            let _ = tx.send(result);
        })
        .map_err(|_| "Could not start Spotify authorization worker")?;
    Ok(rx)
}
/// Configuration is not proof that a saved grant is still authorized.
pub(crate) fn api_configured() -> bool {
    crate::session::provider_exists("spotify") || web::has_session()
}
pub(crate) const WARNING: &str = "Experimental Spotify playback uses a separately installed Spotify Soloist. Generate your own Soloist API key; never share it. Premium is required for initial setup. Pair once using the Spotify app on the same network. API authorization and web-session capture do not pair Soloist. Soloist builds expire after 90 days; install updates from Spotify. Soloist plays through Windows' default audio output, outside the Deezer EQ, crossfade and audio pipeline. Do not redistribute Soloist binaries.";

#[derive(Serialize, Deserialize)]
struct Credentials {
    client_id: String,
    access_token: String,
    refresh_token: String,
    expires_at: u64,
    sp_dc: String,
}
fn random() -> Result<String, String> {
    let mut bytes = [0u8; 32];
    let status = unsafe {
        windows_sys::Win32::Security::Cryptography::BCryptGenRandom(
            std::ptr::null_mut(),
            bytes.as_mut_ptr(),
            bytes.len() as u32,
            windows_sys::Win32::Security::Cryptography::BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        )
    };
    if status < 0 {
        return Err("Could not generate Spotify authorization challenge".into());
    }
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}
pub(crate) fn is_callback(uri: &str, redirect: &str) -> bool {
    let (Ok(actual), Ok(expected)) = (Url::parse(uri), Url::parse(redirect)) else {
        return false;
    };
    actual.scheme() == expected.scheme()
        && actual.host_str() == expected.host_str()
        && actual.port_or_known_default() == expected.port_or_known_default()
        && actual.path() == expected.path()
        && actual.username().is_empty()
        && actual.password().is_none()
        && actual.fragment().is_none()
}
fn authorization_code(callback: &str, state: &str) -> Result<String, String> {
    if !is_callback(callback, REDIRECT) {
        return Err("Unexpected Spotify callback".into());
    }
    let url = Url::parse(callback).map_err(|_| "Invalid Spotify callback")?;
    let values: Vec<_> = url.query_pairs().collect();
    let single = |key: &str| -> Result<String, String> {
        let found: Vec<_> = values.iter().filter(|(k, _)| k == key).collect();
        if found.len() != 1 {
            return Err("Missing or ambiguous Spotify callback parameter".into());
        }
        Ok(found[0].1.to_string())
    };
    if single("state")? != state {
        return Err("Spotify authorization state did not match".into());
    }
    if values.iter().any(|(k, _)| k == "error") {
        return Err("Spotify authorization was declined".into());
    }
    let code = single("code")?;
    if code.is_empty() {
        return Err("Spotify returned an empty authorization code".into());
    }
    Ok(code)
}
fn token_request(fields: &[(&str, &str)]) -> Result<serde_json::Value, String> {
    let body = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(fields.iter().copied())
        .finish();
    let response = crate::lyrics::http::response(
        "POST",
        "https://accounts.spotify.com/api/token",
        &[("Content-Type", "application/x-www-form-urlencoded")],
        body.as_bytes(),
    )
    .map_err(|_| "Spotify API token request failed or timed out")?;
    match response.status {
        200 => {}
        400 | 401 => {
            return Err("Spotify authorization expired or was revoked; sign in again".into())
        }
        429 => {
            return Err(format!(
                "Spotify token requests are rate limited; retry after {} seconds",
                response.retry_after.unwrap_or(5)
            ))
        }
        _ => return Err("Spotify token service is unavailable; retry later".into()),
    }
    let bytes = response.body;
    let value: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|_| "Invalid Spotify token response")?;
    if value["access_token"]
        .as_str()
        .filter(|s| !s.is_empty())
        .is_none()
    {
        return Err("Spotify did not grant an API token".into());
    }
    Ok(value)
}
fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn save(credentials: &Credentials) -> Result<(), String> {
    crate::session::provider_save(
        "spotify",
        &serde_json::to_string(credentials).map_err(|_| "Could not encode Spotify session")?,
    )
}
fn login() -> Result<(), String> {
    let client =
        configured_client_id().ok_or("Enter your Spotify Web API application client ID")?;
    if client.is_empty() {
        return Err("Spotify client ID is empty".into());
    }
    login_with(&client, &AtomicBool::new(false))
}
/// Run on a worker thread. The cancellation flag is also checked before storing either grant.
pub(crate) fn login_with(client: &str, cancel: &AtomicBool) -> Result<(), String> {
    if client.is_empty() {
        return Err("Spotify client ID is empty".into());
    }
    let state = random()?;
    let verifier = random()?;
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let mut url = Url::parse("https://accounts.spotify.com/authorize").unwrap();
    url.query_pairs_mut().extend_pairs([
        ("client_id", client),
        ("response_type", "code"),
        ("redirect_uri", REDIRECT),
        ("code_challenge_method", "S256"),
        ("code_challenge", challenge.as_str()),
        ("state", state.as_str()),
        (
            "scope",
            "user-read-private user-top-read user-read-playback-state user-modify-playback-state user-read-recently-played user-library-read user-library-modify user-follow-read user-follow-modify playlist-read-private playlist-read-collaborative playlist-modify-private playlist-modify-public",
        ),
    ]);
    let result = crate::spotify_webview::run(url.as_str(), REDIRECT, cancel)?;
    let captured: serde_json::Value =
        serde_json::from_str(&result).map_err(|_| "Invalid Spotify sign-in result")?;
    let code = authorization_code(
        captured["callback"]
            .as_str()
            .ok_or("Missing API authorization")?,
        &state,
    )?;
    let cookie = captured["sp_dc"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or("Missing Spotify web session")?;
    let token = token_request(&[
        ("grant_type", "authorization_code"),
        ("code", &code),
        ("redirect_uri", REDIRECT),
        ("client_id", &client),
        ("code_verifier", &verifier),
    ])?;
    let credentials = Credentials {
        client_id: client.to_owned(),
        access_token: token["access_token"].as_str().unwrap().into(),
        refresh_token: token["refresh_token"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or("Spotify did not grant a refresh token")?
            .into(),
        expires_at: token_expiry(&token, now())?,
        sp_dc: cookie.into(),
    };
    let _guard = AUTH
        .lock()
        .map_err(|_| "Spotify credential storage unavailable")?;
    if cancel.load(Ordering::Relaxed) {
        return Err("Spotify sign-in cancelled".into());
    }
    let result = save(&credentials);
    drop(_guard);
    if result.is_ok() {
        catalog::reset();
    }
    result
}
pub(crate) fn refresh() -> Result<(), String> {
    if web::has_session() {
        web::refresh()
    } else {
        access_token(true).map(|_| ())
    }
}
/// Cookie-only authorization; no developer application ID required.
pub(crate) fn login_web(cancel: &AtomicBool) -> Result<(), String> {
    let result = crate::spotify_webview::login_web(cancel)?;
    let value: serde_json::Value =
        serde_json::from_str(&result).map_err(|_| "Invalid Spotify sign-in result")?;
    let cookie = value["sp_dc"]
        .as_str()
        .ok_or("Spotify did not provide a web session")?;
    if cancel.load(Ordering::Relaxed) {
        return Err("Spotify sign-in cancelled".into());
    }
    save_web_cookie(cookie)?;
    web::refresh()?;
    // Session capture is independent from catalog request integration.
    let result = Ok(());
    catalog::reset();
    result
}
pub(crate) fn configured_client_id() -> Option<String> {
    std::env::var("DEEZNATS_SPOTIFY_CLIENT_ID")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .or_else(|| {
            crate::session::provider_saved("spotify")
                .ok()
                .and_then(|s| serde_json::from_str::<Credentials>(&s).ok())
                .map(|c| c.client_id)
        })
}
pub(crate) fn catalog_account() -> Result<serde_json::Value, String> {
    if web::has_session() {
        return web::account();
    }
    api::Client::saved().profile().map_err(|e| e.to_string())
}
/// Reload under one lock so concurrent clients cannot overwrite rotated refresh tokens.
fn access_token(force: bool) -> Result<String, String> {
    let _guard = AUTH
        .lock()
        .map_err(|_| "Spotify credential storage unavailable")?;
    let mut credentials: Credentials =
        serde_json::from_str(&crate::session::provider_saved("spotify")?)
            .map_err(|_| "Saved Spotify session is invalid")?;
    if force || credentials.expires_at <= now().saturating_add(60) {
        let _refresh = web::begin_refresh();
        let token = token_request(&[
            ("grant_type", "refresh_token"),
            ("client_id", &credentials.client_id),
            ("refresh_token", &credentials.refresh_token),
        ])?;
        update_token(&mut credentials, &token, now())?;
        save(&credentials)?;
    }
    Ok(credentials.access_token)
}
fn token_expiry(token: &serde_json::Value, now: u64) -> Result<u64, String> {
    let lifetime = token["expires_in"]
        .as_u64()
        .filter(|n| (1..=86400).contains(n))
        .ok_or("Invalid Spotify token expiry")?;
    now.checked_add(lifetime)
        .ok_or_else(|| "Invalid Spotify token expiry".into())
}
fn update_token(
    credentials: &mut Credentials,
    token: &serde_json::Value,
    now: u64,
) -> Result<(), String> {
    let expiry = token_expiry(token, now)?;
    let access = token["access_token"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or("Spotify did not grant an API token")?;
    credentials.access_token = access.into();
    credentials.expires_at = expiry;
    if let Some(value) = token["refresh_token"].as_str().filter(|s| !s.is_empty()) {
        credentials.refresh_token = value.into();
    }
    Ok(())
}
/// Preserve the same-window web grant without claiming API authorization or Soloist pairing.
pub(crate) fn save_web_cookie(cookie: &str) -> Result<(), String> {
    if !crate::spotify_webview::valid_spotify_cookie("sp_dc", ".spotify.com", true, cookie) {
        return Err("Invalid Spotify web session".into());
    }
    let _guard = AUTH
        .lock()
        .map_err(|_| "Spotify credential storage unavailable")?;
    crate::session::provider_save("spotify-web", cookie)?;
    if let Ok(saved) = crate::session::provider_saved("spotify") {
        if let Ok(mut credentials) = serde_json::from_str::<Credentials>(&saved) {
            credentials.sp_dc = cookie.into();
            save(&credentials)?;
        }
    }
    drop(_guard);
    web::reset();
    Ok(())
}
pub(crate) fn logout() -> Result<(), String> {
    web::reset();
    let _guard = AUTH
        .lock()
        .map_err(|_| "Spotify credential storage unavailable")?;
    let api = crate::session::provider_remove("spotify");
    let web = crate::session::provider_remove("spotify-web");
    let web_api = crate::session::provider_remove("spotify-web-api");
    drop(_guard);
    catalog::reset();
    api.and(web).and(web_api)
}
pub(crate) fn run_cli() -> Result<(), String> {
    let args: Vec<_> = std::env::args().collect();
    let action = args
        .iter()
        .position(|s| s == "--spotify")
        .and_then(|i| args.get(i + 1))
        .map(String::as_str)
        .unwrap_or("help");
    match action {
        "jam-doctor" => {
            let report = match jam::run(jam::Operation::Refresh) {
                Ok(session) => {
                    serde_json::json!({"ok":true,"active":session.as_ref().is_some_and(|s|s.active),
                    "owner":session.as_ref().is_some_and(|s|s.owner),"inviteAvailable":session.as_ref().is_some_and(|s|s.invite.is_some()),
                    "participants":session.as_ref().map_or(0,|s|s.members.len())})
                }
                Err(error) => serde_json::json!({"ok":false,"error":error}),
            };
            println!("{report}");
            return Ok(());
        }
        "jam-lifecycle-check" => {
            let result = (|| -> Result<serde_json::Value, String> {
                if jam::run(jam::Operation::Refresh)?
                    .as_ref()
                    .is_some_and(|s| s.active)
                {
                    return Err(
                        "An existing Jam is present; lifecycle check refused to change it.".into(),
                    );
                }
                let session = jam::run(jam::Operation::Start)
                    .map_err(|e| format!("Create: {e}"))?
                    .ok_or("No Jam was created")?;
                let invite = jam::run(jam::Operation::Invite(session.id.clone()));
                let ended = jam::run(jam::Operation::End(session.id.clone()))
                    .map_err(|e| format!("End: {e}"))?;
                Ok(
                    serde_json::json!({"created":session.active && session.owner,"inviteAvailable":invite.as_ref().ok().and_then(|s|s.as_ref()).is_some_and(|s|s.invite.is_some()),
                    "inviteError":invite.err(),"participants":session.members.len(),"ended":!ended.as_ref().is_some_and(|s|s.active)}),
                )
            })();
            println!(
                "{}",
                result.unwrap_or_else(|error| serde_json::json!({"error":error}))
            );
            return Ok(());
        }
        "catalog-layout-doctor" => {
            let mut report = serde_json::Map::new();
            if args.iter().any(|arg| arg == "--refresh-session") {
                report.insert(
                    "sessionRefresh".into(),
                    match web::refresh() {
                        Ok(()) => serde_json::json!({"ok":true}),
                        Err(error) => serde_json::json!({"error":error}),
                    },
                );
            }
            report.insert(
                "profileQuery".into(),
                match web::account() {
                    Ok(_) => serde_json::json!({"ok":true}),
                    Err(error) => serde_json::json!({"error":error}),
                },
            );
            if args.iter().any(|arg| arg == "--browse-shape") {
                report.insert(
                    "browseShape".into(),
                    web::browse().unwrap_or_else(|error| serde_json::json!({"error":error})),
                );
            }
            if args.iter().any(|arg| arg == "--directory-probe") {
                report.insert(
                    "browseOperations".into(),
                    serde_json::json!(web::browse_operations().unwrap_or_default()),
                );
                for (label, uri, operation) in [
                    (
                        "allMusicSection",
                        "spotify:section:0JQ5DAwD41iRZZRVB8eoeO",
                        "browseSection",
                    ),
                    (
                        "rockDirectory",
                        "spotify:page:0JQ5DAqbMKFDXXwE9BDJAr",
                        "browsePage",
                    ),
                ] {
                    report.insert(label.into(), web::query(operation, serde_json::json!({
                "uri":uri,"pagination":{"offset":0,"limit":100},
                "pagePagination":{"offset":0,"limit":100},"sectionPagination":{"offset":0,"limit":100},
                "browseEndUserIntegration":"INTEGRATION_WEB_PLAYER","includeEpisodeContentRatingsV2":false
            })).unwrap_or_else(|error| serde_json::json!({"error":error})));
                }
                report.insert("musicDirectory".into(), web::query("browsePage", serde_json::json!({
            "uri":"spotify:page:0JQ5DAqbMKFSi39LMRT0Cy", "pagePagination":{"offset":0,"limit":100},
            "sectionPagination":{"offset":0,"limit":100},"browseEndUserIntegration":"INTEGRATION_WEB_PLAYER",
            "includeEpisodeContentRatingsV2":false
        })).unwrap_or_else(|error| serde_json::json!({"error":error})));
                report.insert(
                    "publicCategories".into(),
                    web::rest_get("https://api.spotify.com/v1/browse/categories?limit=50&offset=0")
                        .unwrap_or_else(|error| serde_json::json!({"error":error})),
                );
            }
            if args.iter().any(|arg| arg == "--genre-probe") {
                for (label, uri) in [
                    ("dance", "spotify:page:0JQ5DAqbMKFHOzuVTgTizF"),
                    ("metal", "spotify:page:0JQ5DAqbMKFDkd668ypn6O"),
                    ("jazz", "spotify:page:0JQ5DAqbMKFAJ5xb0fwo9m"),
                    ("hiphop", "spotify:page:0JQ5DAqbMKFQ00XGBls6ym"),
                    ("pop", "spotify:page:0JQ5DAqbMKFEC4WFtoNRpw"),
                    ("latin", "spotify:page:0JQ5DAqbMKFxXaXKP7zcDp"),
                ] {
                    report.insert(label.into(), web::query("browsePage", serde_json::json!({
                "uri":uri,"pagePagination":{"offset":0,"limit":100},"sectionPagination":{"offset":0,"limit":100},
                "browseEndUserIntegration":"INTEGRATION_WEB_PLAYER","includeEpisodeContentRatingsV2":false
            })).unwrap_or_else(|error| serde_json::json!({"error":error})));
                }
            }
            if args.iter().any(|arg| arg == "--expanded-directory") {
                report.insert(
                    "expandedDirectory".into(),
                    crate::catalog::spotify_discovery_report()
                        .unwrap_or_else(|error| serde_json::json!({"error":error})),
                );
            }
            if args.iter().any(|arg| arg == "--lazy-probe") {
                let result = (|| -> Result<serde_json::Value, String> {
                    let value = catalog_invoke(
                        "SPCatalogChannelJSON",
                        &[serde_json::json!("spotify:page:0JQ5DAqbMKFDXXwE9BDJAr")],
                    )?;
                    let section = value["sections"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .find(|s| {
                            s["total"].as_u64().unwrap_or(0) > s["offset"].as_u64().unwrap_or(0)
                        })
                        .ok_or("No paginated section")?;
                    let more = catalog_invoke(
                        "SPCatalogSectionJSON",
                        &[
                            section["uri"].clone(),
                            section["offset"].clone(),
                            serde_json::json!(8),
                        ],
                    )?;
                    Ok(
                        serde_json::json!({"title":section["title"],"initial":section["items"].as_array().map_or(0,Vec::len),
                "nextBatch":more["items"].as_array().map_or(0,Vec::len),"offset":more["offset"],"total":more["total"]}),
                    )
                })();
                report.insert(
                    "lazySection".into(),
                    result.unwrap_or_else(|error| serde_json::json!({"error":error})),
                );
            }
            for name in ["SPCatalogHomeJSON", "SPCatalogDiscoveryJSON"] {
                let value = match web_catalog::invoke(name, &[]).unwrap() {
                    Ok(value) => {
                        serde_json::json!({"sections":value["sections"].as_array().into_iter().flatten().map(|s|
                serde_json::json!({"title":s["title"],"items":s["items"].as_array().into_iter().flatten().map(|item|
                    serde_json::json!({"type":item["type"],"name":item["name"],"uri":item["uri"],"images":item["images"]})).collect::<Vec<_>>()})).collect::<Vec<_>>()})
                    }
                    Err(error) => serde_json::json!({"error":error}),
                };
                report.insert(name.into(), value);
            }
            if args.iter().any(|arg| arg == "--library") {
                for module in 0..5 {
                    report.insert(format!("library:{module}"), match catalog_invoke("DZHomeModuleJSON", &[serde_json::json!(module)]) {
                Ok(value) => serde_json::json!({"items":value["items"].as_array().map_or(0,Vec::len)}),
                Err(error) => serde_json::json!({"error":error}),
            });
                }
                report.insert("genrePage".into(), match catalog_invoke("SPCatalogChannelJSON", &[serde_json::json!("spotify:page:0JQ5DAqbMKFEC4WFtoNRpw")]) {
            Ok(value) => serde_json::json!({"sections":value["sections"].as_array().map_or(0,Vec::len)}),
            Err(error) => serde_json::json!({"error":error}),
        });
                for (name, uri) in [
                    ("editorial", "spotify:page:0JQ5DAqbMKFSi39LMRT0Cy"),
                    ("newReleases", "spotify:page:0JQ5DAqbMKFz6FAsUtgAab"),
                    ("charts", "spotify:page:0JQ5DAudkNjCgYMM0TZXDw"),
                ] {
                    report.insert(name.into(), match catalog_invoke("SPCatalogChannelJSON", &[serde_json::json!(uri)]) {
                Ok(value) => serde_json::json!({"shelves":value["sections"].as_array().into_iter().flatten()
                    .map(|s| serde_json::json!({"title":s["title"],"types":s["items"].as_array().into_iter().flatten()
                        .map(|item| item["type"].clone()).collect::<Vec<_>>()})).collect::<Vec<_>>()}),
                Err(error) => serde_json::json!({"error":error}),
            });
                }
            }
            let path = args
                .iter()
                .position(|s| s == "--spotify")
                .and_then(|i| args.get(i + 2))
                .ok_or("Provide a local report path")?;
            std::fs::write(
                path,
                serde_json::to_vec_pretty(&report).map_err(|_| "Could not encode report")?,
            )
            .map_err(|_| "Could not write report")?;
            Ok(())
        }
        "login-web" => login_web(&AtomicBool::new(false)),
        "devices-doctor" => {
            let devices = web_devices::devices()?;
            println!(
                "Spotify web Connect devices: {}",
                devices.as_array().map_or(0, Vec::len)
            );
            println!(
                "Spotify web active playback state available: {}",
                web_devices::snapshot()?.is_some()
            );
            Ok(())
        }
        "lyrics-doctor" => {
            let saved = catalog_invoke("DZFavoritesJSON", &[])?;
            let tracks = crate::catalog::entries(&saved, "track");
            for track in tracks.into_iter().take(3) {
                match crate::lyrics::fetch_spotify_lyrics(&track, &track.id, 1) {
                    Ok((provider, lines)) if !lines.is_empty() => {
                        println!(
                            "Spotify native lyrics: provider={provider}, lines={}",
                            lines.len()
                        );
                        return Ok(());
                    }
                    Ok(_) => println!("Spotify native lyrics: no lines for sample"),
                    Err(error) => println!("Spotify native lyrics: {error}"),
                }
            }
            Err("No native lyrics available for the sampled tracks".into())
        }
        "web-doctor" => {
            println!("Spotify web saved session: {}", web::has_session());
            web::account()?;
            println!("Spotify web authenticated token/client-token ready; profile query succeeded");
            let result = web::query(
                "home",
                serde_json::json!({"homeEndUserIntegration":"INTEGRATION_WEB_PLAYER","timeZone":"Europe/Istanbul","sp_t":"","facet":"","sectionItemsLimit":10,"includeEpisodeContentRatingsV2":false}),
            )?;
            println!(
                "Spotify web home query succeeded; sections: {}",
                result
                    .pointer("/data/home/sectionContainer/sections/items")
                    .and_then(|v| v.as_array())
                    .map_or(0, |v| v.len())
            );
            let mut failures = 0;
            let mut sample_track = None;
            let mut sample_playlist = None;
            for name in [
                "SPCatalogDiscoveryJSON",
                "DZFavoritesJSON",
                "DZFavoriteAlbumsJSON",
                "DZPlaylistsJSON",
                "DZHistoryRecentJSON",
                "DZFlowConfigsJSON",
            ] {
                match web_catalog::invoke(name, &[]).unwrap_or_else(|| Err("No web adapter".into()))
                {
                    Ok(value) => {
                        let items = crate::catalog::entries(
                            &value,
                            if name == "DZPlaylistsJSON" {
                                "playlist"
                            } else {
                                "track"
                            },
                        );
                        if name == "DZFavoritesJSON" {
                            sample_track = items.into_iter().next();
                        } else if name == "DZPlaylistsJSON" {
                            sample_playlist = items.into_iter().next();
                        }
                        println!("Spotify web {name}: OK");
                    }
                    Err(error) => {
                        failures += 1;
                        println!("Spotify web {name}: {error}");
                    }
                }
            }
            let mut checks = Vec::new();
            if let Some(track) = sample_track {
                checks.push(("DZTrackMetadataJSON", vec![serde_json::json!(track.id)]));
                if !track.artist_id.is_empty() {
                    checks.push((
                        "DZArtistProfileJSON",
                        vec![serde_json::json!(track.artist_id)],
                    ));
                }
                if !track.album_id.is_empty() {
                    checks.push(("DZAlbumTracksJSON", vec![serde_json::json!(track.album_id)]));
                }
            }
            if let Some(playlist) = sample_playlist {
                checks.push(("DZPlaylistTracksJSON", vec![serde_json::json!(playlist.id)]));
            }
            for (name, args) in checks {
                match web_catalog::invoke(name, &args)
                    .unwrap_or_else(|| Err("No web adapter".into()))
                {
                    Ok(_) => println!("Spotify web {name}: OK"),
                    Err(error) => {
                        failures += 1;
                        println!("Spotify web {name}: {error}");
                    }
                }
            }
            if failures > 0 {
                return Err(format!("Spotify read-only web checks failed: {failures}"));
            }
            Ok(())
        }
        "login" => {
            login()?;
            println!("Spotify API authorization and web session saved. Soloist pairing remains separate.");
            Ok(())
        }
        "refresh" => refresh(),
        "logout" => logout(),
        "configure" => {
            let index = args.iter().position(|arg| arg == "--spotify").unwrap() + 2;
            let api_key = args
                .get(index)
                .cloned()
                .or_else(|| std::env::var("DEEZNATS_SOLOIST_API_KEY").ok())
                .or_else(|| soloist::Config::saved().ok().map(|c| c.api_key))
                .filter(|key| !key.trim().is_empty())
                .ok_or("Provide an API key: --spotify configure [API_KEY] [PATH], or set DEEZNATS_SOLOIST_API_KEY")?;
            let executable = match args.get(index + 1) {
                Some(p) => {
                    let path = std::path::PathBuf::from(p);
                    if !path.is_file() {
                        return Err(format!(
                            "Soloist executable not found at {}",
                            path.display()
                        ));
                    }
                    path
                }
                None => setup::preferred_executable()?,
            };
            soloist::Config {
                executable,
                api_key,
            }
            .save()?;
            println!("Soloist configuration saved with Windows user protection");
            Ok(())
        }
        "soloist-discover"
        | "soloist-import"
        | "soloist-check"
        | "soloist-activate"
        | "soloist-rollback"
        | "soloist-doctor"
        | "soloist-open-downloads"
        | "soloist-open-key" => setup_cli(action, &args),
        "forget-soloist-config" => crate::session::provider_remove("spotify-soloist"),
        "profile" | "search" | "playlists" | "saved-tracks" | "saved-albums" | "playlist"
        | "album" | "item" => catalog_cli(action, &args),
        "run" => {
            println!("{WARNING}\nSelect DeezNats in Spotify Connect to pair. Commands: play [spotify:URI], pause, next, prev, seek MS, volume 0-100, shuffle on/off, repeat off/context/track, status, now, queue [1-80], snapshot, add-to-queue URI, activate, deactivate, quit.");
            let mut player = soloist::Soloist::start(soloist::Config::load()?)?;
            for line in io::stdin().lock().lines() {
                let line = line.map_err(|_| "Could not read Spotify command")?;
                let args: Vec<_> = line.split_whitespace().collect();
                if args.as_slice() == ["quit"] {
                    break;
                }
                if args.as_slice() == ["snapshot"] {
                    match player.snapshot() {
                        Ok(snapshot) => println!(
                            "{}",
                            serde_json::to_string(&snapshot)
                                .map_err(|_| "Could not serialize Spotify state")?
                        ),
                        Err(error) => println!("{error}"),
                    }
                    continue;
                }
                match soloist::Control::parse(&args).and_then(|control| player.control(&control)) {
                    Ok(value) if !value.is_null() => println!("{value}"),
                    Ok(_) => {}
                    Err(error) => println!("{error}"),
                }
            }
            Ok(())
        }
        _ => {
            println!("{WARNING}\nUsage: --spotify login|refresh|logout|configure [API_KEY] [PATH]|forget-soloist-config|run|profile|search|playlists|saved-tracks|saved-albums|playlist|album|item|soloist-discover|soloist-import ZIP|soloist-check [EXE]|soloist-activate ID|soloist-rollback|soloist-doctor|soloist-open-downloads|soloist-open-key. See docs/SPOTIFY.md.");
            Ok(())
        }
    }
}
fn setup_cli(action: &str, args: &[String]) -> Result<(), String> {
    let index = args.iter().position(|arg| arg == "--spotify").unwrap() + 2;
    let argument = || {
        args.get(index)
            .map(String::as_str)
            .ok_or_else(|| "Missing Soloist setup argument".to_owned())
    };
    let value = match action {
        "soloist-discover" => serde_json::to_value(setup::discover()?),
        "soloist-doctor" => serde_json::to_value(setup::doctor()?),
        "soloist-import" => {
            serde_json::to_value(setup::import_archive(std::path::Path::new(argument()?))?)
        }
        "soloist-check" => {
            let path = match args.get(index) {
                Some(path) => std::path::PathBuf::from(path),
                None => setup::preferred_executable()?,
            };
            serde_json::to_value(setup::check(&path)?)
        }
        "soloist-activate" => serde_json::to_value(setup::activate(argument()?)?),
        "soloist-rollback" => serde_json::to_value(setup::rollback()?),
        "soloist-open-downloads" => return setup::open_page(false),
        "soloist-open-key" => return setup::open_page(true),
        _ => unreachable!(),
    }
    .map_err(|_| "Could not encode Soloist setup report")?;
    println!("{value}");
    Ok(())
}
fn catalog_cli(action: &str, args: &[String]) -> Result<(), String> {
    let index = args.iter().position(|arg| arg == "--spotify").unwrap() + 2;
    let value = |offset: usize| {
        args.get(index + offset)
            .map(String::as_str)
            .ok_or_else(|| "Missing Spotify catalog argument".to_owned())
    };
    let mut client = api::Client::saved();
    let result = match action {
        "profile" => client.profile(),
        "search" => client
            .search(
                value(0)?,
                args.get(index + 1).map(String::as_str).unwrap_or("track"),
                0,
            )
            .and_then(|v| serde_json::to_value(v).map_err(|_| api::Error::InvalidResponse)),
        "item" => client
            .item(value(0)?, value(1)?)
            .and_then(|v| serde_json::to_value(v).map_err(|_| api::Error::InvalidResponse)),
        _ => {
            let mut page = match action {
                "playlists" => client.playlists(0),
                "saved-tracks" => client.saved_tracks(0),
                "saved-albums" => client.saved_albums(0),
                "playlist" => client.playlist_items(value(0)?, 0),
                "album" => client.album_tracks(value(0)?, 0),
                _ => unreachable!(),
            }
            .map_err(|e| e.to_string())?;
            println!(
                "{}",
                serde_json::to_string(&page).map_err(|_| "Could not encode Spotify page")?
            );
            // Explicit paging avoids fetching an unbounded library or hiding rate limits.
            for line in io::stdin().lock().lines() {
                if line.map_err(|_| "Could not read paging command")?.trim() != "next" {
                    break;
                }
                match client.next(&page).map_err(|e| e.to_string())? {
                    Some(next) => {
                        page = next;
                        println!(
                            "{}",
                            serde_json::to_string(&page)
                                .map_err(|_| "Could not encode Spotify page")?
                        );
                    }
                    None => break,
                }
            }
            return Ok(());
        }
    }
    .map_err(|e| e.to_string())?;
    println!("{result}");
    Ok(())
}
#[cfg(test)]
fn valid_control(args: &[&str]) -> bool {
    soloist::Control::parse(args).is_ok()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn callback_requires_exact_origin_path_and_state() {
        assert_eq!(
            authorization_code(&format!("{REDIRECT}?state=expected&code=abc"), "expected").unwrap(),
            "abc"
        );
        for suffix in [
            "?state=wrong&code=abc",
            "?state=expected&code=a&code=b",
            "?state=expected&error=access_denied",
            "?state=expected&code=",
            "?state=expected&state=expected&code=a",
        ] {
            assert!(authorization_code(&format!("{REDIRECT}{suffix}"), "expected").is_err());
        }
        assert!(!is_callback(
            "http://127.0.0.1:43821/spotify/callback/evil?code=a",
            REDIRECT
        ));
        assert!(!is_callback(
            "http://127.0.0.1:43822/spotify/callback?code=a",
            REDIRECT
        ));
        assert!(!is_callback(
            "http://127.0.0.1.evil/spotify/callback",
            REDIRECT
        ));
    }
    #[test]
    fn playback_commands_are_bounded_and_do_not_accept_cli_flags() {
        assert!(valid_control(&[
            "play",
            "spotify:track:0123456789012345678901"
        ]));
        assert!(valid_control(&["volume", "100"]));
        assert!(!valid_control(&["volume", "101"]));
        assert!(!valid_control(&["play", "--api-key"]));
        assert!(!valid_control(&["seek", "-1"]));
        assert!(!valid_control(&[
            "play",
            "https://open.spotify.com/track/foo"
        ]));
    }
    #[test]
    fn refresh_rotation_preserves_cookie_and_validates_before_mutating() {
        let mut credentials = Credentials {
            client_id: "client".into(),
            access_token: "old-access".into(),
            refresh_token: "old-refresh".into(),
            expires_at: 1,
            sp_dc: "cookie".into(),
        };
        update_token(
            &mut credentials,
            &serde_json::json!({"access_token":"new-access","expires_in":3600}),
            100,
        )
        .unwrap();
        assert_eq!(credentials.refresh_token, "old-refresh");
        assert_eq!(credentials.sp_dc, "cookie");
        assert_eq!(credentials.expires_at, 3700);
        update_token(&mut credentials, &serde_json::json!({"access_token":"next-access","refresh_token":"rotated","expires_in":3600}),200).unwrap();
        assert_eq!(credentials.refresh_token, "rotated");
        assert!(update_token(
            &mut credentials,
            &serde_json::json!({"access_token":"bad","expires_in":u64::MAX}),
            200
        )
        .is_err());
        assert_eq!(credentials.access_token, "next-access");
    }
}

pub(crate) mod spotify_webview {
    //! Short-lived private WebView2 login for the Spotify sp_dc session.
    use std::{
        cell::{Cell, RefCell},
        rc::Rc,
        sync::atomic::{AtomicBool, Ordering},
        time::{Duration, Instant},
    };
    use webview2_com::{
        AddScriptToExecuteOnDocumentCreatedCompletedHandler, CoTaskMemPWSTR,
        CreateCoreWebView2ControllerCompletedHandler,
        CreateCoreWebView2EnvironmentCompletedHandler, DownloadStartingEventHandler,
        ExecuteScriptCompletedHandler, GetCookiesCompletedHandler,
        Microsoft::Web::WebView2::Win32::*, NavigationCompletedEventHandler,
        NavigationStartingEventHandler, NewWindowRequestedEventHandler,
        PermissionRequestedEventHandler,
    };
    use windows::core::{Interface, PCWSTR, PWSTR};
    use windows_sys::Win32::{
        Foundation::*,
        Graphics::Gdi::*,
        System::{Com::*, LibraryLoader::GetModuleHandleW},
        UI::WindowsAndMessaging::*,
    };
    thread_local! { static CONTROLLER:RefCell<Option<ICoreWebView2Controller>>=const {RefCell::new(None)}; static CLOSED:Cell<bool>=const {Cell::new(false)}; }
    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }
    fn allowed_navigation(uri: &str) -> bool {
        // window.open() may create a blank document before navigating to sign-in.
        if uri == "about:blank" {
            return true;
        }
        let Some((_, rest)) = uri
            .split_once("://")
            .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("https"))
        else {
            return false;
        };
        let host = rest
            .split(['/', '?', '#'])
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        // Identity providers may redirect through other HTTPS origins (for example,
        // an identity-provider domain). This is browser navigation, not credential export:
        // cookies retain browser origin scoping and capture still accepts only Spotify sp_dc.
        let host = host.strip_suffix(":443").unwrap_or(&host);
        !host.is_empty()
            && host.len() <= 253
            && host
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b".-".contains(&b))
    }
    unsafe fn text(
        get: impl FnOnce(*mut PWSTR) -> windows::core::Result<()>,
    ) -> windows::core::Result<String> {
        let mut value = PWSTR::null();
        get(&mut value)?;
        Ok(CoTaskMemPWSTR::from(value).to_string())
    }
    unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wp: usize, lp: isize) -> isize {
        match msg {
            WM_SIZE => {
                CONTROLLER.with(|slot| {
                    if let Some(controller) = slot.borrow().as_ref() {
                        let mut r = RECT::default();
                        GetClientRect(hwnd, &mut r);
                        let _ = controller.SetBounds(windows::Win32::Foundation::RECT {
                            left: 0,
                            top: 0,
                            right: r.right,
                            bottom: r.bottom,
                        });
                    }
                });
                0
            }
            WM_CLOSE => {
                CLOSED.set(true);
                0
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
    unsafe fn pump(hwnd: HWND, cancel: &AtomicBool, started: Instant) -> Result<(), String> {
        if cancel.load(Ordering::Relaxed) || CLOSED.get() {
            return Err("Spotify sign-in cancelled".into());
        }
        if started.elapsed() > Duration::from_secs(300) {
            return Err("Spotify sign-in timed out".into());
        }
        if IsWindow(hwnd) == 0 {
            return Err("Spotify sign-in window closed".into());
        }
        let mut message = MSG::default();
        while PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
            if message.message == WM_QUIT {
                return Err("Spotify sign-in window closed".into());
            }
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        std::thread::sleep(Duration::from_millis(10));
        Ok(())
    }
    unsafe fn wait<T>(
        slot: &Rc<RefCell<Option<Result<T, String>>>>,
        hwnd: HWND,
        cancel: &AtomicBool,
        started: Instant,
    ) -> Result<T, String> {
        loop {
            if let Some(value) = slot.borrow_mut().take() {
                return value;
            }
            if started.elapsed() > Duration::from_secs(30) {
                return Err("Spotify WebView2 initialization timed out. Check the WebView2 Runtime and try again.".into());
            }
            pump(hwnd, cancel, started)?;
        }
    }
    struct Window(HWND);
    impl Drop for Window {
        fn drop(&mut self) {
            unsafe {
                CONTROLLER.with(|slot| {
                    if let Some(controller) = slot.borrow_mut().take() {
                        let _ = controller.Close();
                    }
                });
                if IsWindow(self.0) != 0 {
                    DestroyWindow(self.0);
                }
            }
        }
    }
    pub(crate) fn valid_spotify_cookie(
        name: &str,
        domain: &str,
        secure: bool,
        value: &str,
    ) -> bool {
        name == "sp_dc"
            && matches!(
                domain.trim_start_matches('.'),
                "spotify.com" | "www.spotify.com" | "accounts.spotify.com" | "open.spotify.com"
            )
            && secure
            && (20..=4096).contains(&value.len())
            && value.bytes().all(|b| b.is_ascii_graphic() && b != b';')
    }
    pub(crate) fn run(url: &str, redirect: &str, cancel: &AtomicBool) -> Result<String, String> {
        unsafe {
            if CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32) < 0 {
                return Err("Could not initialize sign-in window".into());
            }
            CLOSED.set(false);
            let result = run_sta(url, redirect, cancel, false, None);
            CoUninitialize();
            result
        }
    }
    /// Cookie-only login; closing the private window or setting cancel aborts capture.
    pub(crate) fn login_web(cancel: &AtomicBool) -> Result<String, String> {
        run(
            "https://accounts.spotify.com/en/login?continue=https%3A%2F%2Fopen.spotify.com%2F",
            "",
            cancel,
        )
    }
    pub(crate) fn transport(transport: crate::spotify::web::Transport) -> Result<(), String> {
        unsafe {
            if CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32) < 0 {
                return Err("Could not initialize Spotify web transport".into());
            }
            CLOSED.set(false);
            let cancel = transport.cancel.clone();
            let result = run_sta(
                "https://open.spotify.com/",
                "",
                &cancel,
                false,
                Some(transport),
            );
            CoUninitialize();
            result.map(|_| ())
        }
    }
    unsafe fn run_sta(
        url: &str,
        redirect: &str,
        cancel: &AtomicBool,
        smoke: bool,
        transport: Option<crate::spotify::web::Transport>,
    ) -> Result<String, String> {
        if !url.starts_with("https://") || !allowed_navigation(url) {
            return Err("Invalid Spotify sign-in address".into());
        }
        let instance = GetModuleHandleW(std::ptr::null());
        let class_name = wide("DeezNatsSpotifyAuth");
        let class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            lpszClassName: class_name.as_ptr(),
            hCursor: LoadCursorW(std::ptr::null_mut(), IDC_ARROW),
            hbrBackground: GetStockObject(BLACK_BRUSH) as HBRUSH,
            ..Default::default()
        };
        RegisterClassW(&class);
        let hwnd = CreateWindowExW(
            0,
            class_name.as_ptr(),
            wide("Spotify sign-in - DeezNats").as_ptr(),
            WS_OVERLAPPEDWINDOW,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            620,
            780,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            instance,
            std::ptr::null(),
        );
        if hwnd.is_null() {
            return Err("Could not create Spotify sign-in window".into());
        }
        let _window = Window(hwnd);
        if !smoke {
            if transport.is_none() {
                ShowWindow(hwnd, SW_SHOWMAXIMIZED);
            }
        }
        let started = Instant::now();
        let folder = crate::storage::data_dir()?.join("SpotifyAuthWebView");
        std::fs::create_dir_all(&folder)
            .map_err(|_| "Could not prepare the isolated Spotify browser profile")?;
        let folder = wide(&folder.to_string_lossy());
        let environment = Rc::new(RefCell::new(None));
        let output = environment.clone();
        CreateCoreWebView2EnvironmentWithOptions(
            PCWSTR::null(),
            PCWSTR(folder.as_ptr()),
            None,
            &CreateCoreWebView2EnvironmentCompletedHandler::create(Box::new(
                move |result, environment| {
                    *output.borrow_mut() = Some(
                    result
                        .map_err(|_| {
                            "WebView2 Runtime could not start. Use normal browser sign-in instead."
                                .into()
                        })
                        .and_then(|_| {
                            environment.ok_or_else(|| "WebView2 environment is unavailable".into())
                        }),
                );
                    Ok(())
                },
            )),
        )
        .map_err(|_| "WebView2 Runtime is unavailable. Use normal browser sign-in instead.")?;
        let environment: ICoreWebView2Environment = wait(&environment, hwnd, cancel, started)?;
        let modern: ICoreWebView2Environment10 = environment
            .cast()
            .map_err(|_| "Update WebView2 Runtime to use private sign-in")?;
        let options = modern
            .CreateCoreWebView2ControllerOptions()
            .map_err(|_| "Private login options are unavailable")?;
        options
            .SetIsInPrivateModeEnabled(true)
            .map_err(|_| "Could not enable private Spotify sign-in")?;
        let controller = Rc::new(RefCell::new(None));
        let output = controller.clone();
        modern
            .CreateCoreWebView2ControllerWithOptions(
                windows::Win32::Foundation::HWND(hwnd),
                &options,
                &CreateCoreWebView2ControllerCompletedHandler::create(Box::new(
                    move |result, controller| {
                        *output.borrow_mut() = Some(
                            result
                                .map_err(|_| "Spotify WebView2 controller could not start".into())
                                .and_then(|_| {
                                    controller.ok_or_else(|| {
                                        "Spotify WebView2 controller is unavailable".into()
                                    })
                                }),
                        );
                        Ok(())
                    },
                )),
            )
            .map_err(|_| "Could not open private Spotify sign-in")?;
        let controller: ICoreWebView2Controller = wait(&controller, hwnd, cancel, started)?;
        CONTROLLER.with(|slot| *slot.borrow_mut() = Some(controller.clone()));
        let mut bounds = RECT::default();
        GetClientRect(hwnd, &mut bounds);
        controller
            .SetBounds(windows::Win32::Foundation::RECT {
                left: 0,
                top: 0,
                right: bounds.right,
                bottom: bounds.bottom,
            })
            .map_err(|_| "Could not size the Spotify window")?;
        controller
            .SetIsVisible(true)
            .map_err(|_| "Could not show Spotify sign-in")?;
        let webview = controller
            .CoreWebView2()
            .map_err(|_| "Spotify WebView2 is unavailable")?;
        let settings = webview
            .Settings()
            .map_err(|_| "Spotify WebView2 settings unavailable")?;
        settings
            .SetAreHostObjectsAllowed(false)
            .map_err(|_| "Could not isolate Spotify sign-in")?;
        settings
            .SetIsWebMessageEnabled(false)
            .map_err(|_| "Could not isolate Spotify messages")?;
        settings
            .SetAreDevToolsEnabled(false)
            .map_err(|_| "Could not configure Spotify sign-in")?;
        settings
            .SetAreDefaultContextMenusEnabled(false)
            .map_err(|_| "Could not configure Spotify context menus")?;
        let mut permission_token = 0;
        webview
            .add_PermissionRequested(
                &PermissionRequestedEventHandler::create(Box::new(|_, args| {
                    if let Some(args) = args {
                        args.SetState(COREWEBVIEW2_PERMISSION_STATE_DENY)?;
                    }
                    Ok(())
                })),
                &mut permission_token,
            )
            .map_err(|_| "Could not isolate Spotify permissions")?;
        let mut download_token = 0;
        let downloads: ICoreWebView2_4 = webview
            .cast()
            .map_err(|_| "Update WebView2 Runtime for isolated sign-in")?;
        downloads
            .add_DownloadStarting(
                &DownloadStartingEventHandler::create(Box::new(|_, args| {
                    if let Some(args) = args {
                        args.SetCancel(true)?;
                    }
                    Ok(())
                })),
                &mut download_token,
            )
            .map_err(|_| "Could not disable sign-in downloads")?;

        let captured = Rc::new(RefCell::new(None::<String>));
        let callback = Rc::new(RefCell::new(None::<String>));
        let navigation_error = Rc::new(RefCell::new(None::<String>));
        let error_output = navigation_error.clone();
        let mut completed_token = 0;
        webview.add_NavigationCompleted(
        &NavigationCompletedEventHandler::create(Box::new(move |_, args| {
            if let Some(args) = args {
                let mut success = windows::core::BOOL(0);
                args.IsSuccess(&mut success)?;
                if !success.as_bool() {
                    let mut status = COREWEBVIEW2_WEB_ERROR_STATUS::default();
                    args.WebErrorStatus(&mut status)?;
                    if status != COREWEBVIEW2_WEB_ERROR_STATUS_OPERATION_CANCELED {
                        *error_output.borrow_mut() = Some(format!("Spotify page failed to load (WebView2 error {}). Check your connection and try again.", status.0));
                    }
                }
            }
            Ok(())
        })),
        &mut completed_token,
    ).map_err(|_| "Could not monitor Spotify navigation")?;
        let cookie_only = redirect.is_empty();
        let callback_output = callback.clone();
        let redirect = redirect.to_owned();
        let mut nav_token = 0;
        webview
            .add_NavigationStarting(
                &NavigationStartingEventHandler::create(Box::new(move |_, args| {
                    if let Some(args) = args {
                        let uri = text(|out| args.Uri(out))?;
                        if crate::spotify::is_callback(&uri, &redirect) {
                            *callback_output.borrow_mut() = Some(uri);
                            args.SetCancel(true)?;
                        } else if !allowed_navigation(&uri) {
                            args.SetCancel(true)?;
                        }
                    }
                    Ok(())
                })),
                &mut nav_token,
            )
            .map_err(|_| "Could not configure sign-in navigation")?;
        let mut popup_token = 0;
        webview
            .add_NewWindowRequested(
                &NewWindowRequestedEventHandler::create(Box::new(|_, args| {
                    if let Some(args) = args {
                        let uri = text(|out| args.Uri(out))?;
                        args.SetHandled(!allowed_navigation(&uri))?;
                    }
                    Ok(())
                })),
                &mut popup_token,
            )
            .map_err(|_| "Could not configure sign-in popups")?;
        let modern: ICoreWebView2_2 = webview
            .cast()
            .map_err(|_| "Update WebView2 Runtime for sign-in")?;
        let cookies = modern
            .CookieManager()
            .map_err(|_| "Spotify cookie manager unavailable")?;
        if let Some(session) = &transport {
            controller
                .SetIsVisible(false)
                .map_err(|_| "Could not hide Spotify web session")?;
            let cookie = cookies
                .CreateCookie(
                    PCWSTR(wide("sp_dc").as_ptr()),
                    PCWSTR(wide(&session.cookie).as_ptr()),
                    PCWSTR(wide(".spotify.com").as_ptr()),
                    PCWSTR(wide("/").as_ptr()),
                )
                .map_err(|_| "Could not restore Spotify web session")?;
            cookie
                .SetIsSecure(true)
                .map_err(|_| "Could not secure Spotify web session")?;
            cookie
                .SetIsHttpOnly(true)
                .map_err(|_| "Could not isolate Spotify cookie")?;
            cookies
                .AddOrUpdateCookie(&cookie)
                .map_err(|_| "Could not restore Spotify cookie")?;
            let installed = Rc::new(RefCell::new(None));
            let output = installed.clone();
            webview
                .AddScriptToExecuteOnDocumentCreated(
                    PCWSTR(wide(crate::spotify::web::BOOTSTRAP).as_ptr()),
                    &AddScriptToExecuteOnDocumentCreatedCompletedHandler::create(Box::new(
                        move |result, _| {
                            *output.borrow_mut() =
                                Some(result.map(|_| ()).map_err(|_| {
                                    "Could not install Spotify session observer".into()
                                }));
                            Ok(())
                        },
                    )),
                )
                .map_err(|_| "Could not install Spotify session observer")?;
            wait(&installed, hwnd, cancel, started)?;
        }
        if smoke {
            let cookie = cookies
                .CreateCookie(
                    PCWSTR(wide("sp_dc").as_ptr()),
                    PCWSTR(wide(&"a".repeat(192)).as_ptr()),
                    PCWSTR(wide(".spotify.com").as_ptr()),
                    PCWSTR(wide("/").as_ptr()),
                )
                .map_err(|_| "Could not create synthetic cookie")?;
            cookie
                .SetIsSecure(true)
                .map_err(|_| "Could not secure synthetic cookie")?;
            cookies
                .AddOrUpdateCookie(&cookie)
                .map_err(|_| "Could not set synthetic cookie")?;
            let profile = webview
                .cast::<ICoreWebView2_13>()
                .map_err(|_| "Private profile unavailable")?
                .Profile()
                .map_err(|_| "Private profile unavailable")?;
            let mut private = windows::core::BOOL(0);
            profile
                .IsInPrivateModeEnabled(&mut private)
                .map_err(|_| "Could not verify private profile")?;
            if !private.as_bool() {
                return Err("Sign-in profile is not private".into());
            }
        }
        webview
            .Navigate(PCWSTR(
                wide(if smoke { "about:blank" } else { url }).as_ptr(),
            ))
            .map_err(|_| "Could not open Spotify sign-in")?;
        if let Some(session) = transport {
            return transport_loop(&webview, hwnd, session).map(|_| String::new());
        }
        let pending = Rc::new(Cell::new(false));
        let mut checked = Instant::now() - Duration::from_secs(1);
        let result = loop {
            if let Some(error) = navigation_error.borrow_mut().take() {
                break Err(error);
            }
            if let Some(uri) = callback.borrow().as_ref() {
                if url::Url::parse(uri)
                    .is_ok_and(|url| url.query_pairs().any(|(key, _)| key == "error"))
                {
                    break Err("Spotify authorization was declined".into());
                }
            }
            // Keep the same private window alive until BOTH grants are present.
            // OAuth may return before the next asynchronous cookie poll completes.
            if (cookie_only || callback.borrow().is_some()) && captured.borrow().is_some() {
                break Ok(serde_json::json!({
                    "callback": callback.borrow_mut().take().unwrap_or_default(),
                    "sp_dc": captured.borrow_mut().take().unwrap(),
                })
                .to_string());
            }
            if let Err(error) = pump(hwnd, cancel, started) {
                break Err(error);
            }
            if checked.elapsed() >= Duration::from_millis(750) && !pending.get() {
                checked = Instant::now();
                pending.set(true);
                let output = captured.clone();
                let busy = pending.clone();
                if cookies
                    .GetCookies(
                        PCWSTR(wide("https://open.spotify.com/").as_ptr()),
                        &GetCookiesCompletedHandler::create(Box::new(move |result, list| {
                            busy.set(false);
                            if result.is_ok() {
                                if let Some(list) = list {
                                    let mut count = 0;
                                    list.Count(&mut count)?;
                                    for i in 0..count.min(256) {
                                        let cookie = list.GetValueAtIndex(i)?;
                                        let name = text(|out| cookie.Name(out))?;
                                        if name != "sp_dc" {
                                            continue;
                                        }
                                        let domain = text(|out| cookie.Domain(out))?;
                                        let value = text(|out| cookie.Value(out))?;
                                        let mut secure = windows::core::BOOL(0);
                                        cookie.IsSecure(&mut secure)?;
                                        if valid_spotify_cookie(
                                            &name,
                                            &domain,
                                            secure.as_bool(),
                                            &value,
                                        ) {
                                            *output.borrow_mut() = Some(value);
                                            break;
                                        }
                                    }
                                }
                            }
                            Ok(())
                        })),
                    )
                    .is_err()
                {
                    pending.set(false);
                }
            }
        };
        let _ = webview.remove_PermissionRequested(permission_token);
        let _ = downloads.remove_DownloadStarting(download_token);
        let _ = webview.remove_NavigationStarting(nav_token);
        let _ = webview.remove_NavigationCompleted(completed_token);
        let _ = webview.remove_NewWindowRequested(popup_token);
        result
    }

    unsafe fn evaluate(
        webview: &ICoreWebView2,
        hwnd: HWND,
        cancel: &AtomicBool,
        script: &str,
    ) -> Result<serde_json::Value, String> {
        let slot = Rc::new(RefCell::new(None));
        let output = slot.clone();
        webview
            .ExecuteScript(
                PCWSTR(wide(script).as_ptr()),
                &ExecuteScriptCompletedHandler::create(Box::new(move |result, value| {
                    let parsed = if result.is_err() {
                        Err("Spotify WebView2 script failed".into())
                    } else {
                        let raw = value.to_string();
                        if raw.len() > 6 * 1024 * 1024 {
                            Err("Spotify web response is too large".into())
                        } else {
                            serde_json::from_str(&raw)
                                .map_err(|_| "Invalid Spotify web response".into())
                        }
                    };
                    *output.borrow_mut() = Some(parsed);
                    Ok(())
                })),
            )
            .map_err(|_| "Spotify WebView2 script unavailable")?;
        let started = Instant::now();
        loop {
            if let Some(value) = slot.borrow_mut().take() {
                return value;
            }
            if started.elapsed() > Duration::from_secs(30) {
                return Err(
                    "Spotify WebView2 is not responding. Try Refresh session again.".into(),
                );
            }
            pump(hwnd, cancel, started)?;
        }
    }
    unsafe fn transport_ready(
        webview: &ICoreWebView2,
        hwnd: HWND,
        cancel: &AtomicBool,
        deadline: Instant,
    ) -> Result<(), String> {
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(30) && Instant::now() < deadline {
            if evaluate(
                webview,
                hwnd,
                cancel,
                "Boolean(window.__deeznatsSpotifyWeb?.ready())",
            )?
            .as_bool()
                == Some(true)
            {
                return Ok(());
            }
            for _ in 0..15 {
                pump(hwnd, cancel, started)?;
            }
        }
        Err("Spotify web authorization expired or could not initialize; sign in again (WebView2 Runtime required)".into())
    }
    pub(crate) unsafe fn capture_session(
        webview: &ICoreWebView2,
        hwnd: HWND,
        cancel: &AtomicBool,
    ) -> Result<(), String> {
        crate::spotify::web::save_session(transport_request(
            webview,
            hwnd,
            cancel,
            &serde_json::json!({"kind":"export"}),
        )?)
    }
    unsafe fn transport_request(
        webview: &ICoreWebView2,
        hwnd: HWND,
        cancel: &AtomicBool,
        command: &serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let deadline = Instant::now() + Duration::from_secs(90);
        if (command["kind"] == "browse" || command["kind"] == "export")
            && evaluate(
                webview,
                hwnd,
                cancel,
                "location.pathname.split('/').includes('search')",
            )?
            .as_bool()
                != Some(true)
        {
            webview
                .Navigate(PCWSTR(wide("https://open.spotify.com/search").as_ptr()))
                .map_err(|_| "Could not open Spotify browse categories")?;
            let navigating = Instant::now();
            while evaluate(
                webview,
                hwnd,
                cancel,
                "location.pathname.split('/').includes('search')",
            )?
            .as_bool()
                != Some(true)
            {
                if navigating.elapsed() > Duration::from_secs(15) {
                    return Err("Spotify browse navigation timed out".into());
                }
                pump(hwnd, cancel, navigating)?;
            }
        }
        for attempt in 0..2 {
            transport_ready(webview, hwnd, cancel, deadline)?;
            evaluate(
                webview,
                hwnd,
                cancel,
                &format!("window.__deeznatsSpotifyWeb.run({command});null"),
            )?;
            loop {
                if Instant::now() >= deadline {
                    return Err("Spotify web request timed out".into());
                }
                let value = evaluate(
                    webview,
                    hwnd,
                    cancel,
                    "window.__deeznatsSpotifyWeb?.result ?? null",
                )?;
                if value["refresh"] == true {
                    if attempt != 0 {
                        return Err(
                            "Spotify web authorization expired or revoked; sign in again".into(),
                        );
                    }
                    webview
                        .Reload()
                        .map_err(|_| "Could not refresh Spotify Web Player")?;
                    let started = Instant::now();
                    while started.elapsed() < Duration::from_millis(750) {
                        pump(hwnd, cancel, started)?;
                    }
                    break;
                }
                if value["ok"] == true {
                    return Ok(value["value"].clone());
                }
                if value["ok"] == false {
                    return Err(value["error"]
                        .as_str()
                        .unwrap_or("Spotify web request failed")
                        .to_owned());
                }
                for _ in 0..10 {
                    pump(hwnd, cancel, Instant::now())?;
                }
            }
        }
        Err("Spotify web authorization expired; sign in again".into())
    }
    unsafe fn transport_loop(
        webview: &ICoreWebView2,
        hwnd: HWND,
        session: crate::spotify::web::Transport,
    ) -> Result<(), String> {
        loop {
            if session.cancel.load(Ordering::Relaxed) {
                return Ok(());
            }
            match session.requests.try_recv() {
                Ok(request) => {
                    let result =
                        transport_request(webview, hwnd, &session.cancel, &request.command);
                    let _ = request.reply.send(result);
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => return Ok(()),
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
            pump(hwnd, &session.cancel, Instant::now())?;
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn captures_only_secure_spotify_session_cookie() {
            let value = "a".repeat(64);
            assert!(valid_spotify_cookie("sp_dc", ".spotify.com", true, &value));
            assert!(!valid_spotify_cookie(
                "sp_dc",
                ".spotify.com.evil",
                true,
                &value
            ));
            assert!(!valid_spotify_cookie(
                "sp_dc",
                ".evilspotify.com",
                true,
                &value
            ));
            assert!(!valid_spotify_cookie(
                "sp_dc",
                ".spotify.com",
                false,
                &value
            ));
            assert!(!valid_spotify_cookie("other", ".spotify.com", true, &value));
            assert!(!valid_spotify_cookie(
                "sp_dc",
                ".spotify.com",
                true,
                &";".repeat(64)
            ));
        }
    }
}
