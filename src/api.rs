//! HTTP access to the Mastodon REST API (via libsoup) plus a small cache of
//! downloaded images.

use crate::json::Value;
use gtk::glib::translate::IntoGlib;
use gtk::{gdk, glib, prelude::*};
use soup::prelude::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

pub const CLIENT_NAME: &str = "FediGoo";
pub const REDIRECT_URI: &str = "urn:ietf:wg:oauth:2.0:oob";
pub const SCOPES: &str = "read write follow push";

thread_local! {
    static SESSION: soup::Session = soup::Session::builder()
        .user_agent(concat!("FediGoo/", env!("CARGO_PKG_VERSION")))
        .timeout(60)
        .max_conns_per_host(6)
        .build();
}

pub fn session() -> soup::Session {
    SESSION.with(Clone::clone)
}

pub fn escape(s: &str) -> String {
    glib::Uri::escape_string(s, None, false).to_string()
}

pub fn form_encode(pairs: &[(&str, &str)]) -> String {
    pairs
        .iter()
        .map(|(k, v)| format!("{}={}", escape(k), escape(v)))
        .collect::<Vec<_>>()
        .join("&")
}

/// Normalises what the user typed as an instance ("https://Foo.social/",
/// "@me@foo.social", "foo.social") to a bare host name.
pub fn normalize_instance(input: &str) -> String {
    let mut s = input.trim();
    for p in ["https://", "http://"] {
        if let Some(rest) = s.strip_prefix(p) {
            s = rest;
        }
    }
    if let Some((_, host)) = s.trim_start_matches('@').rsplit_once('@') {
        s = host;
    }
    s.split('/').next().unwrap_or("").to_lowercase()
}

pub enum Body {
    Empty,
    Json(Value),
    Form(Vec<(String, String)>),
    Multipart(soup::Multipart),
}

/// A decoded response plus the pagination links from the `Link` header.
pub struct Response {
    pub json: Value,
    pub next: Option<String>,
}

/// Returns the (next, prev) URLs of a `Link` header.
fn parse_link_header(h: &str) -> (Option<String>, Option<String>) {
    let (mut next, mut prev) = (None, None);
    for part in h.split(',') {
        let Some((url, rest)) = part.split_once(';') else {
            continue;
        };
        let url = url
            .trim()
            .trim_start_matches('<')
            .trim_end_matches('>')
            .to_string();
        if rest.contains("rel=\"next\"") {
            next = Some(url);
        } else if rest.contains("rel=\"prev\"") {
            prev = Some(url);
        }
    }
    (next, prev)
}

/// Connection to one instance, optionally authenticated.
#[derive(Clone, Debug)]
pub struct Api {
    pub host: String,
    pub token: Option<String>,
}

impl Api {
    pub fn new(host: &str, token: Option<&str>) -> Api {
        Api {
            host: host.to_string(),
            token: token.map(str::to_string),
        }
    }

    pub fn url(&self, path: &str) -> String {
        if path.starts_with("https://") || path.starts_with("http://") {
            path.to_string()
        } else {
            format!("https://{}{}", self.host, path)
        }
    }

    pub async fn request(
        &self,
        method: &str,
        path: &str,
        query: &[(&str, &str)],
        body: Body,
    ) -> Result<Response, String> {
        let mut url = self.url(path);
        if !query.is_empty() {
            url.push(if url.contains('?') { '&' } else { '?' });
            url.push_str(&form_encode(query));
        }
        let new_msg =
            || soup::Message::new(method, &url).map_err(|_| format!("Invalid URL: {url}"));
        let msg = match body {
            Body::Empty => new_msg()?,
            Body::Multipart(mut mp) => {
                let m = soup::Message::from_multipart(&url, &mut mp).map_err(|e| e.to_string())?;
                m.set_method(method);
                m
            }
            Body::Json(v) => {
                let m = new_msg()?;
                m.set_request_body_from_bytes(
                    Some("application/json"),
                    Some(&glib::Bytes::from_owned(v.to_string().into_bytes())),
                );
                m
            }
            Body::Form(pairs) => {
                let pairs: Vec<(&str, &str)> = pairs
                    .iter()
                    .map(|(k, v)| (k.as_str(), v.as_str()))
                    .collect();
                let m = new_msg()?;
                m.set_request_body_from_bytes(
                    Some("application/x-www-form-urlencoded"),
                    Some(&glib::Bytes::from_owned(form_encode(&pairs).into_bytes())),
                );
                m
            }
        };
        let headers = msg.request_headers().ok_or("no request headers")?;
        if let Some(t) = &self.token {
            headers.append("Authorization", &format!("Bearer {t}"));
        }
        headers.append("Accept", "application/json");
        if method == "POST" && path.ends_with("/statuses") {
            // Lets the server de-duplicate if a post is retried.
            headers.append("Idempotency-Key", &glib::uuid_string_random());
        }
        let bytes = session()
            .send_and_read_future(&msg, glib::Priority::DEFAULT)
            .await
            .map_err(|e| format!("{}: {}", self.host, e.message()))?;
        let code = msg.status().into_glib();
        let text = String::from_utf8_lossy(&bytes);
        let json = if text.trim().is_empty() {
            Value::Null
        } else {
            Value::parse(&text).unwrap_or(Value::Null)
        };
        if !(200..300).contains(&code) {
            let detail = json["error"]
                .nonempty_str()
                .map(str::to_string)
                .or_else(|| msg.reason_phrase().map(|r| r.to_string()))
                .unwrap_or_default();
            return Err(format!("HTTP {code}: {detail}"));
        }
        let (next, _prev) = msg
            .response_headers()
            .and_then(|h| h.one("Link"))
            .map(|h| parse_link_header(&h))
            .unwrap_or((None, None));
        Ok(Response { json, next })
    }

    pub async fn get(&self, path: &str, query: &[(&str, &str)]) -> Result<Value, String> {
        Ok(self.request("GET", path, query, Body::Empty).await?.json)
    }

    pub async fn post(&self, path: &str, body: Value) -> Result<Value, String> {
        let body = if body.is_null() {
            Body::Empty
        } else {
            Body::Json(body)
        };
        Ok(self.request("POST", path, &[], body).await?.json)
    }

    pub async fn put(&self, path: &str, body: Value) -> Result<Value, String> {
        Ok(self.request("PUT", path, &[], Body::Json(body)).await?.json)
    }

    pub async fn delete(&self, path: &str) -> Result<Value, String> {
        Ok(self.request("DELETE", path, &[], Body::Empty).await?.json)
    }

    pub async fn post_form(&self, path: &str, pairs: &[(&str, &str)]) -> Result<Value, String> {
        let pairs = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        Ok(self
            .request("POST", path, &[], Body::Form(pairs))
            .await?
            .json)
    }

    /// Uploads a file as a media attachment; returns the attachment JSON.
    /// Waits until the server has finished processing it.
    pub async fn upload_media(
        &self,
        file_name: &str,
        content_type: &str,
        data: &glib::Bytes,
        description: &str,
    ) -> Result<Value, String> {
        let mut mp = soup::Multipart::new("multipart/form-data");
        mp.append_form_file("file", Some(file_name), Some(content_type), data);
        if !description.is_empty() {
            mp.append_form_string("description", description);
        }
        let mut v = self
            .request("POST", "/api/v2/media", &[], Body::Multipart(mp))
            .await?
            .json;
        // 202 Accepted means still processing: poll until a URL appears.
        let mut tries = 0;
        while v["url"].is_null() && tries < 60 {
            glib::timeout_future(std::time::Duration::from_secs(1)).await;
            v = self
                .get(&format!("/api/v1/media/{}", v["id"].str()), &[])
                .await?;
            tries += 1;
        }
        Ok(v)
    }
}

// ----- Image cache -----

type Waiter = Box<dyn FnOnce(&gdk::Texture)>;

#[derive(Default)]
struct ImageCache {
    done: HashMap<String, gdk::Texture>,
    pending: HashMap<String, Vec<Waiter>>,
}

thread_local! {
    static IMAGES: Rc<RefCell<ImageCache>> = Rc::default();
}

const IMAGE_CACHE_LIMIT: usize = 600;

/// Fetches the image at `url` (once, then cached) and hands it to `f`.
pub fn with_texture(url: &str, f: impl FnOnce(&gdk::Texture) + 'static) {
    if url.is_empty() {
        return;
    }
    let cache = IMAGES.with(Clone::clone);
    if let Some(t) = cache.borrow().done.get(url).cloned() {
        f(&t);
        return;
    }
    {
        let mut c = cache.borrow_mut();
        if let Some(waiters) = c.pending.get_mut(url) {
            waiters.push(Box::new(f));
            return;
        }
        c.pending.insert(url.to_string(), vec![Box::new(f)]);
    }
    let url = url.to_string();
    glib::spawn_future_local(async move {
        let texture = async {
            let msg = soup::Message::new("GET", &url).ok()?;
            let bytes = session()
                .send_and_read_future(&msg, glib::Priority::LOW)
                .await
                .ok()?;
            if !(200..300).contains(&msg.status().into_glib()) {
                return None;
            }
            gdk::Texture::from_bytes(&bytes).ok()
        }
        .await;
        let waiters = cache.borrow_mut().pending.remove(&url).unwrap_or_default();
        if let Some(t) = texture {
            {
                let mut c = cache.borrow_mut();
                if c.done.len() >= IMAGE_CACHE_LIMIT {
                    c.done.clear();
                }
                c.done.insert(url, t.clone());
            }
            for w in waiters {
                w(&t);
            }
        }
    });
}

/// Loads `url` into `image` asynchronously.
pub fn load_image(image: &gtk::Image, url: &str) {
    let weak = image.downgrade();
    with_texture(url, move |t| {
        if let Some(i) = weak.upgrade() {
            i.set_paintable(Some(t));
        }
    });
}

pub fn load_picture(picture: &gtk::Picture, url: &str) {
    let weak = picture.downgrade();
    with_texture(url, move |t| {
        if let Some(p) = weak.upgrade() {
            p.set_paintable(Some(t));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn link_header() {
        let (n, p) = parse_link_header(
            r#"<https://x.y/api/v1/timelines/home?max_id=5>; rel="next", <https://x.y/api/v1/timelines/home?min_id=9>; rel="prev""#,
        );
        assert_eq!(
            n.as_deref(),
            Some("https://x.y/api/v1/timelines/home?max_id=5")
        );
        assert_eq!(
            p.as_deref(),
            Some("https://x.y/api/v1/timelines/home?min_id=9")
        );
    }

    #[test]
    fn instance_names() {
        assert_eq!(
            normalize_instance(" https://Mastodon.Social/about "),
            "mastodon.social"
        );
        assert_eq!(normalize_instance("@me@fosstodon.org"), "fosstodon.org");
        assert_eq!(normalize_instance("hachyderm.io"), "hachyderm.io");
    }
}

/// Exercises HTTP, TLS, JSON and model parsing against a real server.
/// Run with `cargo test -- --ignored`.
#[cfg(test)]
mod network_tests {
    use super::*;
    use crate::model::{Account, Status};

    #[test]
    #[ignore]
    fn public_api() {
        let api = Api::new("mastodon.social", None);
        glib::MainContext::default().block_on(async {
            let acc = api
                .get("/api/v1/accounts/lookup", &[("acct", "Mastodon")])
                .await
                .unwrap();
            let acc = Account::from_json(&acc);
            assert!(!acc.id.is_empty());
            let resp = api
                .request(
                    "GET",
                    &format!("/api/v1/accounts/{}/statuses", acc.id),
                    &[("limit", "5")],
                    Body::Empty,
                )
                .await
                .unwrap();
            let statuses: Vec<Status> = resp.json.arr().iter().map(Status::from_json).collect();
            assert!(!statuses.is_empty());
            assert!(resp.next.is_some(), "expected a Link: rel=next header");
            for s in &statuses {
                assert!(!s.shown().id.is_empty());
                let _ = crate::html::to_markup(&s.shown().content_html);
            }
            println!(
                "{} ({} followers): {}",
                acc.acct,
                acc.followers_count,
                crate::html::to_text(&statuses[0].shown().content_html)
                    .chars()
                    .take(80)
                    .collect::<String>()
            );
            let err = api
                .get("/api/v1/accounts/verify_credentials", &[])
                .await
                .unwrap_err();
            assert!(err.starts_with("HTTP 401"), "{err}");
        });
    }
}
