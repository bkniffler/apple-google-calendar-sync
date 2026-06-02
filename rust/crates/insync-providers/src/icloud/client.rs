use crate::{
    CalendarProvider, ProviderCalendar, ProviderChangeSet, ProviderError, ProviderSyncCursor,
    icloud::{
        CalendarObject, IcloudCalendar, canonical_to_ical, event_filename, ical_meta_from_object,
        ical_object_to_canonical, icloud_calendar_to_provider,
    },
};
use async_trait::async_trait;
use insync_core::{CanonicalEvent, ProviderEventMeta, ProviderName};
use quick_xml::{Reader, events::Event};
use reqwest::{Client, Method, Url};

const DEFAULT_CALDAV_URL: &str = "https://caldav.icloud.com";

#[derive(Debug, Clone, Default)]
pub struct IcloudProviderOptions {
    pub username: Option<String>,
    pub app_specific_password: Option<String>,
    pub server_url: Option<String>,
}

#[derive(Debug, Clone)]
pub struct IcloudCalDavProvider {
    options: IcloudProviderOptions,
    client: Client,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct DavResponse {
    href: Option<String>,
    display_name: Option<String>,
    timezone: Option<String>,
    sync_token: Option<String>,
    etag: Option<String>,
    calendar_data: Option<String>,
    is_calendar: bool,
    current_user_principal: Option<String>,
    calendar_home_set: Option<String>,
}

impl IcloudCalDavProvider {
    pub fn new(options: IcloudProviderOptions) -> Self {
        Self {
            options,
            client: Client::new(),
        }
    }

    pub fn with_client(options: IcloudProviderOptions, client: Client) -> Self {
        Self { options, client }
    }

    fn assert_configured(&self) -> Result<(), ProviderError> {
        if self.options.username.is_some() && self.options.app_specific_password.is_some() {
            Ok(())
        } else {
            Err(ProviderError::NotConfigured(ProviderName::Icloud))
        }
    }

    fn server_url(&self) -> Result<Url, ProviderError> {
        Url::parse(
            self.options
                .server_url
                .as_deref()
                .unwrap_or(DEFAULT_CALDAV_URL),
        )
        .map_err(|error| ProviderError::Request(error.to_string()))
    }

    async fn discover_calendar_home(&self) -> Result<Url, ProviderError> {
        let server_url = self.server_url()?;
        let principal_body = r#"<?xml version="1.0" encoding="utf-8"?>
<d:propfind xmlns:d="DAV:">
  <d:prop><d:current-user-principal /></d:prop>
</d:propfind>"#;
        let principal_response = self
            .dav_text(
                propfind_method(),
                server_url.clone(),
                Some("0"),
                principal_body.to_string(),
            )
            .await?;
        let principal_href = parse_multistatus(&principal_response)
            .into_iter()
            .find_map(|response| response.current_user_principal.or(response.href))
            .ok_or_else(|| ProviderError::Mapping("missing current-user-principal".to_string()))?;
        let principal_url = resolve_url(&server_url, &principal_href)?;

        let home_body = r#"<?xml version="1.0" encoding="utf-8"?>
<d:propfind xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav">
  <d:prop><c:calendar-home-set /></d:prop>
</d:propfind>"#;
        let home_response = self
            .dav_text(
                propfind_method(),
                principal_url.clone(),
                Some("0"),
                home_body.to_string(),
            )
            .await?;
        let home_href = parse_multistatus(&home_response)
            .into_iter()
            .find_map(|response| response.calendar_home_set)
            .ok_or_else(|| ProviderError::Mapping("missing calendar-home-set".to_string()))?;

        resolve_url(&principal_url, &home_href)
    }

    async fn fetch_calendars(&self) -> Result<Vec<IcloudCalendar>, ProviderError> {
        let home_url = self.discover_calendar_home().await?;
        let body = r#"<?xml version="1.0" encoding="utf-8"?>
<d:propfind xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav">
  <d:prop>
    <d:resourcetype />
    <d:displayname />
    <c:calendar-timezone />
    <d:sync-token />
  </d:prop>
</d:propfind>"#;
        let response = self
            .dav_text(
                propfind_method(),
                home_url.clone(),
                Some("1"),
                body.to_string(),
            )
            .await?;

        parse_multistatus(&response)
            .into_iter()
            .filter(|response| response.is_calendar)
            .map(|response| {
                let href = response.href.ok_or_else(|| {
                    ProviderError::Mapping("calendar response missing href".to_string())
                })?;
                Ok(IcloudCalendar {
                    url: resolve_url(&home_url, &href)?.to_string(),
                    display_name: response.display_name,
                    timezone: response.timezone,
                    sync_token: response.sync_token,
                })
            })
            .collect()
    }

    async fn fetch_objects(
        &self,
        calendar_url: &str,
    ) -> Result<Vec<CalendarObject>, ProviderError> {
        let calendar_url =
            Url::parse(calendar_url).map_err(|error| ProviderError::Request(error.to_string()))?;
        let body = r#"<?xml version="1.0" encoding="utf-8"?>
<c:calendar-query xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav">
  <d:prop>
    <d:getetag />
    <c:calendar-data />
  </d:prop>
  <c:filter>
    <c:comp-filter name="VCALENDAR">
      <c:comp-filter name="VEVENT" />
    </c:comp-filter>
  </c:filter>
</c:calendar-query>"#;
        let response = self
            .dav_text(
                report_method(),
                calendar_url.clone(),
                Some("1"),
                body.to_string(),
            )
            .await?;

        parse_multistatus(&response)
            .into_iter()
            .filter(|response| response.calendar_data.is_some())
            .map(|response| {
                let href = response.href.ok_or_else(|| {
                    ProviderError::Mapping("calendar object missing href".to_string())
                })?;
                Ok(CalendarObject {
                    url: resolve_url(&calendar_url, &href)?.to_string(),
                    etag: response.etag,
                    data: response.calendar_data,
                })
            })
            .collect()
    }

    async fn dav_text(
        &self,
        method: Method,
        url: Url,
        depth: Option<&str>,
        body: String,
    ) -> Result<String, ProviderError> {
        self.assert_configured()?;
        let mut request = self
            .client
            .request(method, url)
            .basic_auth(
                self.options.username.as_deref().unwrap_or_default(),
                self.options.app_specific_password.as_deref(),
            )
            .header("Content-Type", "application/xml; charset=utf-8")
            .body(body);
        if let Some(depth) = depth {
            request = request.header("Depth", depth);
        }

        let response = request
            .send()
            .await
            .map_err(|error| ProviderError::network(ProviderName::Icloud, error))?;
        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|error| ProviderError::network(ProviderName::Icloud, error))?;
        if !status.is_success() {
            return Err(ProviderError::http(
                ProviderName::Icloud,
                status.as_u16(),
                text,
            ));
        }

        Ok(text)
    }

    async fn put_object(
        &self,
        url: Url,
        body: String,
        etag: Option<&str>,
        if_none_match: bool,
    ) -> Result<Option<String>, ProviderError> {
        self.assert_configured()?;
        let mut request = self
            .client
            .put(url)
            .basic_auth(
                self.options.username.as_deref().unwrap_or_default(),
                self.options.app_specific_password.as_deref(),
            )
            .header("Content-Type", "text/calendar; charset=utf-8")
            .body(body);
        if let Some(etag) = etag {
            request = request.header("If-Match", etag);
        }
        if if_none_match {
            request = request.header("If-None-Match", "*");
        }

        let response = request
            .send()
            .await
            .map_err(|error| ProviderError::network(ProviderName::Icloud, error))?;
        let status = response.status();
        let etag = response
            .headers()
            .get("etag")
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        let text = response
            .text()
            .await
            .map_err(|error| ProviderError::network(ProviderName::Icloud, error))?;

        if status.as_u16() == 412 {
            return Err(ProviderError::PreconditionFailed(ProviderName::Icloud));
        }
        if !status.is_success() {
            return Err(ProviderError::http(
                ProviderName::Icloud,
                status.as_u16(),
                text,
            ));
        }

        Ok(etag)
    }
}

#[async_trait]
impl CalendarProvider for IcloudCalDavProvider {
    fn name(&self) -> ProviderName {
        ProviderName::Icloud
    }

    async fn list_calendars(&self) -> Result<Vec<ProviderCalendar>, ProviderError> {
        Ok(self
            .fetch_calendars()
            .await?
            .into_iter()
            .map(icloud_calendar_to_provider)
            .collect())
    }

    async fn get_changes(
        &self,
        calendar_id: &str,
        _cursor: ProviderSyncCursor,
    ) -> Result<ProviderChangeSet, ProviderError> {
        let objects = self.fetch_objects(calendar_id).await?;
        let mut events = Vec::new();
        for object in objects {
            events.extend(ical_object_to_canonical(calendar_id, object)?);
        }

        Ok(ProviderChangeSet {
            provider: ProviderName::Icloud,
            calendar_id: calendar_id.to_string(),
            sync_token: None,
            events,
        })
    }

    async fn create_event(
        &self,
        calendar_id: &str,
        event: &CanonicalEvent,
    ) -> Result<ProviderEventMeta, ProviderError> {
        let calendar_url =
            Url::parse(calendar_id).map_err(|error| ProviderError::Request(error.to_string()))?;
        let filename = event_filename(event);
        let href = calendar_url
            .join(&filename)
            .map_err(|error| ProviderError::Request(error.to_string()))?;
        let etag = self
            .put_object(href.clone(), canonical_to_ical(event), None, true)
            .await?;

        Ok(ical_meta_from_object(
            calendar_id,
            href.as_str(),
            etag.as_deref(),
        ))
    }

    async fn update_event(
        &self,
        calendar_id: &str,
        remote_event_id: &str,
        event: &CanonicalEvent,
        etag: Option<&str>,
    ) -> Result<ProviderEventMeta, ProviderError> {
        let href = Url::parse(remote_event_id)
            .map_err(|error| ProviderError::Request(error.to_string()))?;
        let next_etag = self
            .put_object(href.clone(), canonical_to_ical(event), etag, false)
            .await?;

        Ok(ical_meta_from_object(
            calendar_id,
            href.as_str(),
            next_etag.as_deref().or(etag),
        ))
    }

    async fn delete_event(
        &self,
        _calendar_id: &str,
        remote_event_id: &str,
        etag: Option<&str>,
    ) -> Result<(), ProviderError> {
        self.assert_configured()?;
        let href = Url::parse(remote_event_id)
            .map_err(|error| ProviderError::Request(error.to_string()))?;
        let mut request = self.client.delete(href).basic_auth(
            self.options.username.as_deref().unwrap_or_default(),
            self.options.app_specific_password.as_deref(),
        );
        if let Some(etag) = etag {
            request = request.header("If-Match", etag);
        }

        let response = request
            .send()
            .await
            .map_err(|error| ProviderError::network(ProviderName::Icloud, error))?;
        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|error| ProviderError::network(ProviderName::Icloud, error))?;

        if status.as_u16() == 412 {
            return Err(ProviderError::PreconditionFailed(ProviderName::Icloud));
        }
        if !status.is_success() {
            return Err(ProviderError::http(
                ProviderName::Icloud,
                status.as_u16(),
                text,
            ));
        }

        Ok(())
    }
}

fn parse_multistatus(xml: &str) -> Vec<DavResponse> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut responses = Vec::new();
    let mut current = None::<DavResponse>;
    let mut text_target = None::<String>;
    let mut nested_href_target = None::<String>;

    loop {
        match reader.read_event() {
            Ok(Event::Start(event)) => {
                let name = local_name(event.name().as_ref());
                match name.as_str() {
                    "response" => current = Some(DavResponse::default()),
                    "href" => {
                        text_target = Some(
                            nested_href_target
                                .clone()
                                .unwrap_or_else(|| "href".to_string()),
                        )
                    }
                    "displayname" | "calendar-timezone" | "sync-token" | "getetag"
                    | "calendar-data" => text_target = Some(name),
                    "current-user-principal" | "calendar-home-set" => {
                        nested_href_target = Some(name)
                    }
                    "calendar" => {
                        if let Some(current) = current.as_mut() {
                            current.is_calendar = true;
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::Empty(event)) => {
                let name = local_name(event.name().as_ref());
                if name == "calendar"
                    && let Some(current) = current.as_mut()
                {
                    current.is_calendar = true;
                }
            }
            Ok(Event::Text(text)) => {
                if let (Some(current), Some(target)) = (current.as_mut(), text_target.as_deref()) {
                    let value = text
                        .xml_content()
                        .map(|value| value.into_owned())
                        .unwrap_or_default();
                    assign_dav_text(current, target, value);
                }
            }
            Ok(Event::CData(text)) => {
                if let (Some(current), Some(target)) = (current.as_mut(), text_target.as_deref()) {
                    let value = text
                        .xml_content()
                        .map(|value| value.into_owned())
                        .unwrap_or_default();
                    assign_dav_text(current, target, value);
                }
            }
            Ok(Event::End(event)) => {
                let name = local_name(event.name().as_ref());
                match name.as_str() {
                    "response" => {
                        if let Some(response) = current.take() {
                            responses.push(response);
                        }
                    }
                    "href" | "displayname" | "calendar-timezone" | "sync-token" | "getetag"
                    | "calendar-data" => text_target = None,
                    "current-user-principal" | "calendar-home-set" => nested_href_target = None,
                    _ => {}
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
    }

    responses
}

fn assign_dav_text(current: &mut DavResponse, target: &str, value: String) {
    match target {
        "href" => append_value(&mut current.href, value),
        "displayname" => append_value(&mut current.display_name, value),
        "calendar-timezone" => append_value(&mut current.timezone, value),
        "sync-token" => append_value(&mut current.sync_token, value),
        "getetag" => append_value(&mut current.etag, value),
        "calendar-data" => append_value(&mut current.calendar_data, value),
        "current-user-principal" => append_value(&mut current.current_user_principal, value),
        "calendar-home-set" => append_value(&mut current.calendar_home_set, value),
        _ => {}
    }
}

fn append_value(target: &mut Option<String>, value: String) {
    match target {
        Some(target) => target.push_str(&value),
        None => *target = Some(value),
    }
}

fn local_name(name: &[u8]) -> String {
    let name = String::from_utf8_lossy(name);
    name.rsplit(':').next().unwrap_or(&name).to_string()
}

fn resolve_url(base: &Url, href: &str) -> Result<Url, ProviderError> {
    Url::parse(href)
        .or_else(|_| base.join(href))
        .map_err(|error| ProviderError::Request(error.to_string()))
}

fn propfind_method() -> Method {
    Method::from_bytes(b"PROPFIND").expect("valid PROPFIND method")
}

fn report_method() -> Method {
    Method::from_bytes(b"REPORT").expect("valid REPORT method")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_calendar_discovery_multistatus() {
        let rows = parse_multistatus(
            r#"
            <d:multistatus xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav">
              <d:response>
                <d:href>/calendars/user/home/</d:href>
                <d:propstat>
                  <d:prop>
                    <d:resourcetype><d:collection/><c:calendar/></d:resourcetype>
                    <d:displayname>Home</d:displayname>
                    <c:calendar-timezone>Europe/Berlin</c:calendar-timezone>
                    <d:sync-token>token-1</d:sync-token>
                  </d:prop>
                </d:propstat>
              </d:response>
            </d:multistatus>
            "#,
        );

        assert_eq!(rows.len(), 1);
        assert!(rows[0].is_calendar);
        assert_eq!(rows[0].href.as_deref(), Some("/calendars/user/home/"));
        assert_eq!(rows[0].display_name.as_deref(), Some("Home"));
        assert_eq!(rows[0].sync_token.as_deref(), Some("token-1"));
    }

    #[test]
    fn parses_calendar_object_multistatus() {
        let rows = parse_multistatus(
            r#"
            <d:multistatus xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav">
              <d:response>
                <d:href>/cal/event.ics</d:href>
                <d:propstat>
                  <d:prop>
                    <d:getetag>"etag-1"</d:getetag>
                    <c:calendar-data>BEGIN:VCALENDAR&#13;&#10;END:VCALENDAR</c:calendar-data>
                  </d:prop>
                </d:propstat>
              </d:response>
            </d:multistatus>
            "#,
        );

        assert_eq!(rows[0].etag.as_deref(), Some("\"etag-1\""));
        assert!(
            rows[0]
                .calendar_data
                .as_deref()
                .unwrap_or_default()
                .contains("BEGIN:VCALENDAR")
        );
    }

    #[test]
    fn resolves_relative_dav_urls() {
        let base = Url::parse("https://caldav.example/principal/").unwrap();
        assert_eq!(
            resolve_url(&base, "/calendars/user/home/")
                .unwrap()
                .as_str(),
            "https://caldav.example/calendars/user/home/"
        );
    }
}
