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
        let mut url = Url::parse("https://api.spotify.com/v1/me/player/recently-played").unwrap();
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
            Url::parse("https://api.spotify.com/v1/me/following?type=artist&limit=50").unwrap();
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
        if device_id.is_empty() || device_id.len() > 256 || device_id.chars().any(char::is_control)
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
    pub fn add_playlist_items(&mut self, id: &str, uris: &[String]) -> Result<Value, Error> {
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
    pub fn upload_playlist_cover(&mut self, id: &str, path: &std::path::Path) -> Result<(), Error> {
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
                "track" | "episode" | "album" | "artist" | "playlist" | "show" | "audiobook"
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
            .remove_playlist_items("1234567890123456789012", &["deezer:track:1".into()], None)
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
