use crate::{
    CalendarProvider, ProviderCalendar, ProviderChangeSet, ProviderError, ProviderSyncCursor,
    google::{
        GoogleCalendarListResponse, GoogleEvent, GoogleEventsResponse, canonical_to_google,
        google_calendar_to_provider, google_meta_from_response, google_to_canonical,
    },
};
use async_trait::async_trait;
use insync_core::{CanonicalEvent, ProviderEventMeta, ProviderName};
use reqwest::{Client, Method, Url};
use serde::Deserialize;
use std::{
    sync::Mutex,
    time::{Duration, Instant},
};
use tokio::time;

const GOOGLE_API_BASE: &str = "https://www.googleapis.com/calendar/v3";
const GOOGLE_AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const GOOGLE_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const GOOGLE_SCOPES: &[&str] = &[
    "https://www.googleapis.com/auth/calendar.events",
    "https://www.googleapis.com/auth/calendar.calendarlist.readonly",
];

#[derive(Debug, Clone, Default)]
pub struct GoogleProviderOptions {
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
    pub refresh_token: Option<String>,
}

#[derive(Debug)]
pub struct GoogleCalendarProvider {
    options: GoogleProviderOptions,
    client: Client,
    token_cache: Mutex<Option<TokenCache>>,
}

#[derive(Debug, Clone)]
struct TokenCache {
    access_token: String,
    expires_at: Instant,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct GoogleAuthCodeExchange {
    pub client_id: String,
    pub client_secret: String,
    pub redirect_uri: String,
    pub code: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoogleAuthToken {
    pub refresh_token: Option<String>,
    pub access_token: String,
    pub expires_in: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct AuthCodeTokenResponse {
    refresh_token: Option<String>,
    access_token: String,
    expires_in: Option<u64>,
}

impl GoogleCalendarProvider {
    pub fn new(options: GoogleProviderOptions) -> Self {
        Self {
            options,
            client: Client::new(),
            token_cache: Mutex::new(None),
        }
    }

    pub fn with_client(options: GoogleProviderOptions, client: Client) -> Self {
        Self {
            options,
            client,
            token_cache: Mutex::new(None),
        }
    }

    fn assert_configured(&self) -> Result<(), ProviderError> {
        if self.options.client_id.is_some()
            && self.options.client_secret.is_some()
            && self.options.refresh_token.is_some()
        {
            Ok(())
        } else {
            Err(ProviderError::NotConfigured(ProviderName::Google))
        }
    }

    async fn request<T: for<'de> Deserialize<'de>>(
        &self,
        method: Method,
        path: &str,
        query: &[(&str, Option<String>)],
        body: Option<serde_json::Value>,
        etag: Option<&str>,
    ) -> Result<T, ProviderError> {
        self.assert_configured()?;
        let mut url = Url::parse(&format!("{GOOGLE_API_BASE}{path}"))
            .map_err(|error| ProviderError::Request(error.to_string()))?;
        {
            let mut pairs = url.query_pairs_mut();
            for (key, value) in query {
                if let Some(value) = value {
                    pairs.append_pair(key, value);
                }
            }
        }

        for attempt in 0..6 {
            let token = self.access_token().await?;
            let mut request = self
                .client
                .request(method.clone(), url.clone())
                .bearer_auth(token);
            if let Some(etag) = etag {
                request = request.header("If-Match", etag);
            }
            if let Some(body) = body.as_ref() {
                request = request.json(body);
            }

            let response = request
                .send()
                .await
                .map_err(|error| ProviderError::network(ProviderName::Google, error))?;
            let status = response.status();
            let text = response
                .text()
                .await
                .map_err(|error| ProviderError::network(ProviderName::Google, error))?;

            if status.as_u16() == 410 {
                return Err(ProviderError::SyncTokenExpired(ProviderName::Google));
            }
            if status.as_u16() == 412 {
                return Err(ProviderError::PreconditionFailed(ProviderName::Google));
            }
            if !status.is_success() {
                let error = ProviderError::http(ProviderName::Google, status.as_u16(), text);
                if error.is_rate_limited() && attempt < 5 {
                    time::sleep(rate_limit_delay(attempt)).await;
                    continue;
                }
                return Err(error);
            }
            if text.is_empty() {
                return serde_json::from_value(serde_json::Value::Null)
                    .map_err(|error| ProviderError::Mapping(error.to_string()));
            }

            return serde_json::from_str(&text)
                .map_err(|error| ProviderError::Mapping(error.to_string()));
        }

        Err(ProviderError::RateLimited {
            provider: ProviderName::Google,
            status: 429,
            body: "rate limit retries exhausted".to_string(),
        })
    }

    async fn request_empty(
        &self,
        method: Method,
        path: &str,
        query: &[(&str, Option<String>)],
        etag: Option<&str>,
    ) -> Result<(), ProviderError> {
        self.assert_configured()?;
        let mut url = Url::parse(&format!("{GOOGLE_API_BASE}{path}"))
            .map_err(|error| ProviderError::Request(error.to_string()))?;
        {
            let mut pairs = url.query_pairs_mut();
            for (key, value) in query {
                if let Some(value) = value {
                    pairs.append_pair(key, value);
                }
            }
        }

        for attempt in 0..6 {
            let token = self.access_token().await?;
            let mut request = self
                .client
                .request(method.clone(), url.clone())
                .bearer_auth(token);
            if let Some(etag) = etag {
                request = request.header("If-Match", etag);
            }

            let response = request
                .send()
                .await
                .map_err(|error| ProviderError::network(ProviderName::Google, error))?;
            let status = response.status();
            let text = response
                .text()
                .await
                .map_err(|error| ProviderError::network(ProviderName::Google, error))?;

            if status.as_u16() == 410 {
                return Err(ProviderError::SyncTokenExpired(ProviderName::Google));
            }
            if status.as_u16() == 412 {
                return Err(ProviderError::PreconditionFailed(ProviderName::Google));
            }
            if !status.is_success() {
                let error = ProviderError::http(ProviderName::Google, status.as_u16(), text);
                if error.is_rate_limited() && attempt < 5 {
                    time::sleep(rate_limit_delay(attempt)).await;
                    continue;
                }
                return Err(error);
            }

            return Ok(());
        }

        Err(ProviderError::RateLimited {
            provider: ProviderName::Google,
            status: 429,
            body: "rate limit retries exhausted".to_string(),
        })
    }

    async fn access_token(&self) -> Result<String, ProviderError> {
        self.assert_configured()?;
        if let Some(cache) = self
            .token_cache
            .lock()
            .map_err(|error| ProviderError::Request(error.to_string()))?
            .clone()
            && Instant::now() < cache.expires_at
        {
            return Ok(cache.access_token);
        }

        let params = [
            (
                "client_id",
                self.options.client_id.as_deref().unwrap_or_default(),
            ),
            (
                "client_secret",
                self.options.client_secret.as_deref().unwrap_or_default(),
            ),
            (
                "refresh_token",
                self.options.refresh_token.as_deref().unwrap_or_default(),
            ),
            ("grant_type", "refresh_token"),
        ];
        let response = self
            .client
            .post(GOOGLE_TOKEN_URL)
            .form(&params)
            .send()
            .await
            .map_err(|error| ProviderError::network(ProviderName::Google, error))?;
        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|error| ProviderError::network(ProviderName::Google, error))?;

        if !status.is_success() {
            return Err(ProviderError::http(
                ProviderName::Google,
                status.as_u16(),
                text,
            ));
        }

        let token: TokenResponse = serde_json::from_str(&text)
            .map_err(|error| ProviderError::Mapping(error.to_string()))?;
        let cache = TokenCache {
            access_token: token.access_token,
            expires_at: Instant::now()
                + Duration::from_secs(token.expires_in.unwrap_or(3600).saturating_sub(60)),
        };
        let access_token = cache.access_token.clone();
        *self
            .token_cache
            .lock()
            .map_err(|error| ProviderError::Request(error.to_string()))? = Some(cache);

        Ok(access_token)
    }
}

#[async_trait]
impl CalendarProvider for GoogleCalendarProvider {
    fn name(&self) -> ProviderName {
        ProviderName::Google
    }

    async fn list_calendars(&self) -> Result<Vec<ProviderCalendar>, ProviderError> {
        let mut calendars = Vec::new();
        let mut page_token = None;

        loop {
            let response: GoogleCalendarListResponse = self
                .request(
                    Method::GET,
                    "/users/me/calendarList",
                    &[("pageToken", page_token.clone())],
                    None,
                    None,
                )
                .await?;
            calendars.extend(
                response
                    .items
                    .unwrap_or_default()
                    .into_iter()
                    .map(google_calendar_to_provider),
            );
            page_token = response.next_page_token;
            if page_token.is_none() {
                break;
            }
        }

        Ok(calendars)
    }

    async fn get_changes(
        &self,
        calendar_id: &str,
        cursor: ProviderSyncCursor,
    ) -> Result<ProviderChangeSet, ProviderError> {
        let mut events = Vec::new();
        let mut page_token = None;
        let mut next_sync_token = None;

        loop {
            let response: GoogleEventsResponse = self
                .request(
                    Method::GET,
                    &format!("/calendars/{}/events", url_path(calendar_id)),
                    &[
                        ("maxResults", Some("2500".to_string())),
                        ("pageToken", page_token.clone()),
                        ("showDeleted", Some("true".to_string())),
                        ("singleEvents", Some("false".to_string())),
                        (
                            "syncToken",
                            if cursor.full_sync {
                                None
                            } else {
                                cursor.sync_token.clone()
                            },
                        ),
                    ],
                    None,
                    None,
                )
                .await?;

            for event in response.items.unwrap_or_default() {
                events.push(google_to_canonical(calendar_id, event)?);
            }
            page_token = response.next_page_token;
            next_sync_token = response.next_sync_token.or(next_sync_token);
            if page_token.is_none() {
                break;
            }
        }

        Ok(ProviderChangeSet {
            provider: ProviderName::Google,
            calendar_id: calendar_id.to_string(),
            sync_token: next_sync_token,
            events,
        })
    }

    async fn create_event(
        &self,
        calendar_id: &str,
        event: &CanonicalEvent,
    ) -> Result<ProviderEventMeta, ProviderError> {
        let body = serde_json::to_value(canonical_to_google(event, ProviderName::Icloud))
            .map_err(|error| ProviderError::Mapping(error.to_string()))?;
        let created: GoogleEvent = self
            .request(
                Method::POST,
                &format!("/calendars/{}/events", url_path(calendar_id)),
                &[("sendUpdates", Some("none".to_string()))],
                Some(body),
                None,
            )
            .await?;
        Ok(google_meta_from_response(calendar_id, &created))
    }

    async fn update_event(
        &self,
        calendar_id: &str,
        remote_event_id: &str,
        event: &CanonicalEvent,
        etag: Option<&str>,
    ) -> Result<ProviderEventMeta, ProviderError> {
        let body = serde_json::to_value(canonical_to_google(event, ProviderName::Icloud))
            .map_err(|error| ProviderError::Mapping(error.to_string()))?;
        let updated: GoogleEvent = self
            .request(
                Method::PATCH,
                &format!(
                    "/calendars/{}/events/{}",
                    url_path(calendar_id),
                    url_path(remote_event_id)
                ),
                &[("sendUpdates", Some("none".to_string()))],
                Some(body),
                etag,
            )
            .await?;
        Ok(google_meta_from_response(calendar_id, &updated))
    }

    async fn delete_event(
        &self,
        calendar_id: &str,
        remote_event_id: &str,
        etag: Option<&str>,
    ) -> Result<(), ProviderError> {
        self.request_empty(
            Method::DELETE,
            &format!(
                "/calendars/{}/events/{}",
                url_path(calendar_id),
                url_path(remote_event_id)
            ),
            &[("sendUpdates", Some("none".to_string()))],
            etag,
        )
        .await
    }
}

pub fn google_auth_url(client_id: &str, redirect_uri: &str, state: &str) -> String {
    let mut url = Url::parse(GOOGLE_AUTH_URL).expect("valid Google auth URL");
    url.query_pairs_mut()
        .append_pair("client_id", client_id)
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("response_type", "code")
        .append_pair("scope", &GOOGLE_SCOPES.join(" "))
        .append_pair("access_type", "offline")
        .append_pair("prompt", "consent")
        .append_pair("state", state);
    url.to_string()
}

pub async fn exchange_google_auth_code(
    input: GoogleAuthCodeExchange,
) -> Result<GoogleAuthToken, ProviderError> {
    exchange_google_auth_code_with_client(&Client::new(), input).await
}

pub async fn exchange_google_auth_code_with_client(
    client: &Client,
    input: GoogleAuthCodeExchange,
) -> Result<GoogleAuthToken, ProviderError> {
    let params = [
        ("client_id", input.client_id.as_str()),
        ("client_secret", input.client_secret.as_str()),
        ("redirect_uri", input.redirect_uri.as_str()),
        ("code", input.code.as_str()),
        ("grant_type", "authorization_code"),
    ];
    let response = client
        .post(GOOGLE_TOKEN_URL)
        .form(&params)
        .send()
        .await
        .map_err(|error| ProviderError::network(ProviderName::Google, error))?;
    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|error| ProviderError::network(ProviderName::Google, error))?;

    if !status.is_success() {
        return Err(ProviderError::http(
            ProviderName::Google,
            status.as_u16(),
            text,
        ));
    }

    let parsed: AuthCodeTokenResponse =
        serde_json::from_str(&text).map_err(|error| ProviderError::Mapping(error.to_string()))?;
    Ok(GoogleAuthToken {
        refresh_token: parsed.refresh_token,
        access_token: parsed.access_token,
        expires_in: parsed.expires_in,
    })
}

fn rate_limit_delay(attempt: u32) -> Duration {
    Duration::from_millis(1_000 * 2_u64.saturating_pow(attempt))
}

fn url_path(value: &str) -> String {
    value
        .bytes()
        .flat_map(|byte| {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
                vec![byte as char]
            } else {
                format!("%{byte:02X}").chars().collect()
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_url_requests_offline_calendar_access() {
        let url = google_auth_url("client", "http://localhost/callback", "state-1");

        assert!(url.starts_with(GOOGLE_AUTH_URL));
        assert!(url.contains("access_type=offline"));
        assert!(url.contains("prompt=consent"));
        assert!(url.contains("state=state-1"));
        assert!(url.contains("calendar.events"));
    }

    #[test]
    fn detects_google_rate_limit_responses() {
        assert!(
            ProviderError::http(ProviderName::Google, 429, "").is_rate_limited(),
            "HTTP 429 should be typed as rate limited"
        );
        assert!(
            ProviderError::http(ProviderName::Google, 403, "userRateLimitExceeded")
                .is_rate_limited(),
            "Google userRateLimitExceeded should be typed as rate limited"
        );
        assert!(
            !ProviderError::http(ProviderName::Google, 403, "forbidden").is_rate_limited(),
            "ordinary Google 403s are not necessarily rate limits"
        );
    }

    #[test]
    fn encodes_path_segments() {
        assert_eq!(url_path("primary"), "primary");
        assert_eq!(url_path("a/b c"), "a%2Fb%20c");
    }
}
