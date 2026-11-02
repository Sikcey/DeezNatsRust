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
    let value: Value =
        serde_json::from_slice(&bytes).map_err(|_| "Invalid Spotify service resolver response")?;
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
                "Spotify denied Jam access. An eligible account and device are required.".into(),
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
                return Err("Spotify has not confirmed that you left the Jam. Refresh Jam.".into());
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
            if !identifier(&member) || !current.members.iter().any(|m| m.id == member && !m.owner) {
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
