//! Typed views of the Mastodon API entities that the UI works with.

use crate::json::Value;
use std::collections::HashSet;

fn s(v: &Value) -> String {
    v.str().to_string()
}

fn opt(v: &Value) -> Option<String> {
    v.nonempty_str().map(str::to_string)
}

#[derive(Clone, Debug, Default)]
pub struct Field {
    pub name: String,
    pub value_html: String,
    pub verified: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Account {
    pub id: String,
    pub username: String,
    pub acct: String,
    pub display_name: String,
    pub url: String,
    pub avatar: String,
    pub header: String,
    pub note_html: String,
    pub locked: bool,
    pub bot: bool,
    pub followers_count: i64,
    pub following_count: i64,
    pub statuses_count: i64,
    pub created_at: String,
    pub fields: Vec<Field>,
    /// Present only in verify_credentials responses.
    pub source_note: Option<String>,
    pub source_privacy: Option<String>,
    pub source_sensitive: bool,
    pub moved_to: Option<Box<Account>>,
}

impl Account {
    pub fn from_json(v: &Value) -> Account {
        Account {
            id: s(&v["id"]),
            username: s(&v["username"]),
            acct: s(&v["acct"]),
            display_name: s(&v["display_name"]),
            url: s(&v["url"]),
            avatar: s(&v["avatar"]),
            header: s(&v["header"]),
            note_html: s(&v["note"]),
            locked: v["locked"].bool(),
            bot: v["bot"].bool(),
            followers_count: v["followers_count"].i64(),
            following_count: v["following_count"].i64(),
            statuses_count: v["statuses_count"].i64(),
            created_at: s(&v["created_at"]),
            fields: v["fields"]
                .arr()
                .iter()
                .map(|f| Field {
                    name: s(&f["name"]),
                    value_html: s(&f["value"]),
                    verified: !f["verified_at"].is_null(),
                })
                .collect(),
            source_note: v["source"]["note"].opt_str().map(str::to_string),
            source_privacy: v["source"]["privacy"].opt_str().map(str::to_string),
            source_sensitive: v["source"]["sensitive"].bool(),
            moved_to: match &v["moved"] {
                Value::Obj(_) => Some(Box::new(Account::from_json(&v["moved"]))),
                _ => None,
            },
        }
    }

    /// Display name, falling back to the user name.
    pub fn name(&self) -> &str {
        if self.display_name.trim().is_empty() {
            &self.username
        } else {
            &self.display_name
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Media {
    pub id: String,
    pub kind: String,
    pub url: String,
    pub preview_url: String,
    pub remote_url: Option<String>,
    pub description: Option<String>,
    /// Width / height from the server's metadata; 0 when not given.
    pub aspect: f64,
}

/// Aspect ratio from attachment `meta`, preferring the preview's size.
fn aspect(meta: &Value) -> f64 {
    for key in ["small", "original"] {
        let m = &meta[key];
        let (w, h) = (m["width"].f64(), m["height"].f64());
        if w > 0.0 && h > 0.0 {
            return w / h;
        }
        if m["aspect"].f64() > 0.0 {
            return m["aspect"].f64();
        }
    }
    0.0
}

impl Media {
    pub fn from_json(v: &Value) -> Media {
        Media {
            id: s(&v["id"]),
            kind: s(&v["type"]),
            url: s(&v["url"]),
            preview_url: opt(&v["preview_url"]).unwrap_or_else(|| s(&v["url"])),
            remote_url: opt(&v["remote_url"]),
            description: opt(&v["description"]),
            aspect: aspect(&v["meta"]),
        }
    }

    pub fn is_image(&self) -> bool {
        self.kind == "image"
    }
}

#[derive(Clone, Debug, Default)]
pub struct Mention {
    pub id: String,
    pub acct: String,
    pub url: String,
}

#[derive(Clone, Debug, Default)]
pub struct PollOption {
    pub title: String,
    pub votes: i64,
}

#[derive(Clone, Debug, Default)]
pub struct Poll {
    pub id: String,
    pub expires_at: Option<String>,
    pub expired: bool,
    pub multiple: bool,
    pub votes_count: i64,
    pub voters_count: Option<i64>,
    pub voted: bool,
    pub own_votes: Vec<i64>,
    pub options: Vec<PollOption>,
}

impl Poll {
    pub fn from_json(v: &Value) -> Poll {
        Poll {
            id: s(&v["id"]),
            expires_at: opt(&v["expires_at"]),
            expired: v["expired"].bool(),
            multiple: v["multiple"].bool(),
            votes_count: v["votes_count"].i64(),
            voters_count: (!v["voters_count"].is_null()).then(|| v["voters_count"].i64()),
            voted: v["voted"].bool(),
            own_votes: v["own_votes"].arr().iter().map(Value::i64).collect(),
            options: v["options"]
                .arr()
                .iter()
                .map(|o| PollOption {
                    title: s(&o["title"]),
                    votes: o["votes_count"].i64(),
                })
                .collect(),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Card {
    pub url: String,
    pub title: String,
    pub description: String,
    pub image: Option<String>,
    pub provider: String,
}

#[derive(Clone, Debug, Default)]
pub struct Status {
    pub id: String,
    pub uri: String,
    pub url: Option<String>,
    pub created_at: String,
    pub edited_at: Option<String>,
    pub account: Account,
    pub content_html: String,
    pub spoiler_text: String,
    pub sensitive: bool,
    pub visibility: String,
    pub language: Option<String>,
    pub replies_count: i64,
    pub reblogs_count: i64,
    pub favourites_count: i64,
    pub favourited: bool,
    pub reblogged: bool,
    pub bookmarked: bool,
    pub pinned: bool,
    pub muted: bool,
    pub in_reply_to_id: Option<String>,
    pub in_reply_to_account_id: Option<String>,
    pub reblog: Option<Box<Status>>,
    pub media: Vec<Media>,
    pub mentions: Vec<Mention>,
    pub tags: Vec<String>,
    pub poll: Option<Poll>,
    pub card: Option<Card>,
    pub application: Option<String>,
    /// Set when one of the user's filters matched this post.
    pub filtered: Option<FilterMatch>,
}

/// What the user's filters say about a post (the API's `filtered`).
#[derive(Clone, Debug, PartialEq)]
pub enum FilterMatch {
    /// Show it collapsed, behind the filter's title.  Also used for "blur"
    /// and unknown actions, which the API says to treat like "warn".
    Warn(String),
    /// Don't show it at all.  Holds the filter's title.
    Hide(String),
}

impl FilterMatch {
    fn from_results(results: &[Value]) -> Option<FilterMatch> {
        let title = |r: &Value| s(&r["filter"]["title"]);
        if let Some(r) = results
            .iter()
            .find(|r| r["filter"]["filter_action"].str() == "hide")
        {
            return Some(FilterMatch::Hide(title(r)));
        }
        results.first().map(|r| FilterMatch::Warn(title(r)))
    }
}

impl Status {
    pub fn from_json(v: &Value) -> Status {
        let filtered = FilterMatch::from_results(v["filtered"].arr());
        Status {
            id: s(&v["id"]),
            uri: s(&v["uri"]),
            url: opt(&v["url"]),
            created_at: s(&v["created_at"]),
            edited_at: opt(&v["edited_at"]),
            account: Account::from_json(&v["account"]),
            content_html: s(&v["content"]),
            spoiler_text: s(&v["spoiler_text"]),
            sensitive: v["sensitive"].bool(),
            visibility: s(&v["visibility"]),
            language: opt(&v["language"]),
            replies_count: v["replies_count"].i64(),
            reblogs_count: v["reblogs_count"].i64(),
            favourites_count: v["favourites_count"].i64(),
            favourited: v["favourited"].bool(),
            reblogged: v["reblogged"].bool(),
            bookmarked: v["bookmarked"].bool(),
            pinned: v["pinned"].bool(),
            muted: v["muted"].bool(),
            in_reply_to_id: opt(&v["in_reply_to_id"]),
            in_reply_to_account_id: opt(&v["in_reply_to_account_id"]),
            reblog: match &v["reblog"] {
                Value::Obj(_) => Some(Box::new(Status::from_json(&v["reblog"]))),
                _ => None,
            },
            media: v["media_attachments"]
                .arr()
                .iter()
                .map(Media::from_json)
                .collect(),
            mentions: v["mentions"]
                .arr()
                .iter()
                .map(|m| Mention {
                    id: s(&m["id"]),
                    acct: s(&m["acct"]),
                    url: s(&m["url"]),
                })
                .collect(),
            tags: v["tags"].arr().iter().map(|t| s(&t["name"])).collect(),
            poll: match &v["poll"] {
                Value::Obj(_) => Some(Poll::from_json(&v["poll"])),
                _ => None,
            },
            card: match &v["card"] {
                Value::Obj(_) => Some(Card {
                    url: s(&v["card"]["url"]),
                    title: s(&v["card"]["title"]),
                    description: s(&v["card"]["description"]),
                    image: opt(&v["card"]["image"]),
                    provider: s(&v["card"]["provider_name"]),
                }),
                _ => None,
            },
            application: opt(&v["application"]["name"]),
            filtered,
        }
    }

    /// The filter verdict for this post or, for a boost, the boosted post;
    /// "hide" wins over "warn".
    pub fn filter_match(&self) -> Option<&FilterMatch> {
        let inner = self.reblog.as_deref().and_then(|r| r.filtered.as_ref());
        match (self.filtered.as_ref(), inner) {
            (Some(h @ FilterMatch::Hide(_)), _) | (_, Some(h @ FilterMatch::Hide(_))) => Some(h),
            (own, inner) => own.or(inner),
        }
    }

    /// A copy with every filter verdict cleared ("Show anyway").
    pub fn unfiltered(&self) -> Status {
        let mut st = self.clone();
        st.filtered = None;
        if let Some(r) = st.reblog.as_mut() {
            r.filtered = None;
        }
        st
    }

    /// The status whose content is shown: the boosted one for a boost.
    pub fn shown(&self) -> &Status {
        self.reblog.as_deref().unwrap_or(self)
    }

    pub fn link(&self) -> String {
        self.url.clone().unwrap_or_else(|| self.uri.clone())
    }
}

#[derive(Clone, Debug, Default)]
pub struct Notification {
    pub kind: String,
    pub created_at: String,
    pub account: Account,
    pub status: Option<Status>,
}

impl Notification {
    pub fn from_json(v: &Value) -> Notification {
        Notification {
            kind: s(&v["type"]),
            created_at: s(&v["created_at"]),
            account: Account::from_json(&v["account"]),
            status: match &v["status"] {
                Value::Obj(_) => Some(Status::from_json(&v["status"])),
                _ => None,
            },
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Relationship {
    pub following: bool,
    pub followed_by: bool,
    pub requested: bool,
    pub blocking: bool,
    pub muting: bool,
    pub domain_blocking: bool,
    pub showing_reblogs: bool,
    pub notifying: bool,
}

impl Relationship {
    pub fn from_json(v: &Value) -> Relationship {
        Relationship {
            following: v["following"].bool(),
            followed_by: v["followed_by"].bool(),
            requested: v["requested"].bool(),
            blocking: v["blocking"].bool(),
            muting: v["muting"].bool(),
            domain_blocking: v["domain_blocking"].bool(),
            showing_reblogs: v["showing_reblogs"].bool(),
            notifying: v["notifying"].bool(),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Tag {
    pub name: String,
    /// Accounts using the tag over the last few days, when reported.
    pub recent_uses: i64,
}

impl Tag {
    pub fn from_json(v: &Value) -> Tag {
        Tag {
            name: s(&v["name"]),
            recent_uses: v["history"]
                .arr()
                .iter()
                .take(2)
                .map(|h| h["accounts"].i64())
                .sum(),
        }
    }
}

/// Accounts and domains the user has blocked or muted while the app is
/// running, so views can drop their posts at once.  (The server filters what
/// it sends after that, but not what's already loaded.)
#[derive(Default)]
pub struct Hidden {
    accounts: HashSet<String>,
    domains: HashSet<String>,
}

impl Hidden {
    pub fn add_account(&mut self, id: &str) {
        self.accounts.insert(id.to_string());
    }

    pub fn remove_account(&mut self, id: &str) {
        self.accounts.remove(id);
    }

    pub fn add_domain(&mut self, domain: &str) {
        self.domains.insert(domain.to_lowercase());
    }

    pub fn remove_domain(&mut self, domain: &str) {
        self.domains.remove(&domain.to_lowercase());
    }

    /// Whether the account with `id` and `acct` (`user` or `user@domain`)
    /// is hidden.  Domain blocks cover subdomains too, as on the server.
    pub fn hides(&self, id: &str, acct: &str) -> bool {
        if self.accounts.contains(id) {
            return true;
        }
        let Some((_, domain)) = acct.split_once('@') else {
            return false;
        };
        let domain = domain.to_lowercase();
        self.domains.iter().any(|d| {
            domain == *d
                || domain
                    .strip_suffix(d.as_str())
                    .is_some_and(|rest| rest.ends_with('.'))
        })
    }

    pub fn hides_any(&self, who: &[(String, String)]) -> bool {
        who.iter().any(|(id, acct)| self.hides(id, acct))
    }
}

/// The (id, acct) of every account a post involves: its author, and for a
/// boost also the booster.
pub fn status_accounts(st: &Status) -> Vec<(String, String)> {
    let mut who = vec![(st.account.id.clone(), st.account.acct.clone())];
    let shown = &st.shown().account;
    if shown.id != st.account.id {
        who.push((shown.id.clone(), shown.acct.clone()));
    }
    who
}

/// The places a filter can apply, as (API value, label).
pub const FILTER_CONTEXTS: [(&str, &str); 5] = [
    ("home", "Home and lists"),
    ("notifications", "Notifications"),
    ("public", "Public timelines"),
    ("thread", "Threads"),
    ("account", "Profiles"),
];

/// A keyword or hashtag in a filter.  `id` is empty for one not yet saved.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FilterKeyword {
    pub id: String,
    pub keyword: String,
    pub whole_word: bool,
}

/// One of the user's filters (`/api/v2/filters`).
#[derive(Clone, Debug, Default)]
pub struct Filter {
    pub id: String,
    pub title: String,
    pub context: Vec<String>,
    /// "warn", "hide" or "blur".
    pub action: String,
    pub expires_at: Option<String>,
    pub keywords: Vec<FilterKeyword>,
}

impl Filter {
    pub fn from_json(v: &Value) -> Filter {
        Filter {
            id: s(&v["id"]),
            title: s(&v["title"]),
            context: v["context"].arr().iter().map(s).collect(),
            action: s(&v["filter_action"]),
            expires_at: opt(&v["expires_at"]),
            keywords: v["keywords"]
                .arr()
                .iter()
                .map(|k| FilterKeyword {
                    id: s(&k["id"]),
                    keyword: s(&k["keyword"]),
                    whole_word: k["whole_word"].bool(),
                })
                .collect(),
        }
    }
}

/// Expiry chosen in the filter editor.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Expiry {
    /// Leave an existing filter's expiry as it is.
    Unchanged,
    Never,
    /// This many seconds from now.
    In(i64),
}

/// The JSON body that creates a filter (`old` is None) or updates `old` to
/// match the edited fields.  Keywords keep their ids; ones removed from
/// `old` are sent with `_destroy`.
pub fn filter_body(
    title: &str,
    context: &[String],
    action: &str,
    expiry: Expiry,
    keywords: &[FilterKeyword],
    old: Option<&Filter>,
) -> Value {
    let mut attrs: Vec<Value> = keywords
        .iter()
        .filter(|k| !k.keyword.trim().is_empty())
        .map(|k| {
            let mut o = vec![
                ("keyword".to_string(), Value::from(k.keyword.trim())),
                ("whole_word".to_string(), k.whole_word.into()),
            ];
            if !k.id.is_empty() {
                o.push(("id".to_string(), k.id.as_str().into()));
            }
            Value::Obj(o)
        })
        .collect();
    if let Some(old) = old {
        for k in &old.keywords {
            if !keywords.iter().any(|n| n.id == k.id) {
                attrs.push(Value::Obj(vec![
                    ("id".to_string(), k.id.as_str().into()),
                    ("_destroy".to_string(), true.into()),
                ]));
            }
        }
    }
    let mut body = vec![
        ("title".to_string(), Value::from(title.trim())),
        (
            "context".to_string(),
            context
                .iter()
                .map(|c| Value::from(c.as_str()))
                .collect::<Vec<_>>()
                .into(),
        ),
        ("filter_action".to_string(), action.into()),
        ("keywords_attributes".to_string(), attrs.into()),
    ];
    match expiry {
        Expiry::Unchanged => {}
        // A new filter never expires by default; an existing one is cleared.
        Expiry::Never if old.is_none() => {}
        Expiry::Never => body.push(("expires_in".to_string(), Value::Null)),
        Expiry::In(secs) => body.push(("expires_in".to_string(), secs.into())),
    }
    Value::Obj(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filtered(actions: &[(&str, &str)]) -> String {
        let results: Vec<String> = actions
            .iter()
            .map(|(title, action)| {
                format!(r#"{{"filter":{{"title":"{title}","filter_action":"{action}"}}}}"#)
            })
            .collect();
        format!("[{}]", results.join(","))
    }

    fn status(own: &str, reblog: Option<&str>) -> Status {
        let reblog = reblog.map_or("null".to_string(), |r| {
            format!(r#"{{"id":"2","filtered":{r}}}"#)
        });
        Status::from_json(
            &Value::parse(&format!(
                r#"{{"id":"1","filtered":{own},"reblog":{reblog}}}"#
            ))
            .unwrap(),
        )
    }

    #[test]
    fn filter_matches() {
        use FilterMatch::*;
        assert_eq!(status("[]", None).filter_match(), None);
        assert_eq!(
            status(&filtered(&[("a", "warn")]), None).filter_match(),
            Some(&Warn("a".into()))
        );
        // "blur" and unknown actions are treated like "warn".
        assert_eq!(
            status(&filtered(&[("b", "blur")]), None).filter_match(),
            Some(&Warn("b".into()))
        );
        // "hide" wins over "warn", in the post or in the boosted post.
        assert_eq!(
            status(&filtered(&[("a", "warn"), ("h", "hide")]), None).filter_match(),
            Some(&Hide("h".into()))
        );
        let boost = status(
            &filtered(&[("a", "warn")]),
            Some(&filtered(&[("h", "hide")])),
        );
        assert_eq!(boost.filter_match(), Some(&Hide("h".into())));
        assert_eq!(boost.unfiltered().filter_match(), None);
    }

    fn kw(id: &str, keyword: &str, whole_word: bool) -> FilterKeyword {
        FilterKeyword {
            id: id.into(),
            keyword: keyword.into(),
            whole_word,
        }
    }

    #[test]
    fn filter_bodies() {
        let ctx = vec!["home".to_string(), "public".to_string()];
        // New filter: no ids, blank keywords dropped, no expiry sent for "never".
        let body = filter_body(
            " Spoilers ",
            &ctx,
            "warn",
            Expiry::Never,
            &[kw("", "#show", false), kw("", "  ", true)],
            None,
        );
        assert_eq!(
            body.to_string(),
            r##"{"title":"Spoilers","context":["home","public"],"filter_action":"warn","keywords_attributes":[{"keyword":"#show","whole_word":false}]}"##
        );

        let old = Filter {
            id: "9".into(),
            keywords: vec![kw("1", "keep", true), kw("2", "drop", false)],
            ..Default::default()
        };
        // Update: kept keyword keeps its id, removed one is destroyed,
        // "never" clears the expiry.
        let body = filter_body(
            "T",
            &ctx,
            "hide",
            Expiry::Never,
            &[kw("1", "kept", false), kw("", "new", true)],
            Some(&old),
        );
        assert_eq!(
            body.to_string(),
            r#"{"title":"T","context":["home","public"],"filter_action":"hide","keywords_attributes":[{"keyword":"kept","whole_word":false,"id":"1"},{"keyword":"new","whole_word":true},{"id":"2","_destroy":true}],"expires_in":null}"#
        );
        let expiry =
            |e| filter_body("T", &ctx, "warn", e, &[], Some(&old))["expires_in"].to_string();
        assert_eq!(expiry(Expiry::In(3600)), "3600");
        assert!(
            filter_body("T", &ctx, "warn", Expiry::Unchanged, &[], Some(&old))["expires_in"]
                .is_null()
        );
        assert!(
            !filter_body("T", &ctx, "warn", Expiry::Unchanged, &[], Some(&old))
                .to_string()
                .contains("expires_in")
        );
    }

    #[test]
    fn hidden_accounts_and_domains() {
        let mut h = Hidden::default();
        assert!(!h.hides("1", "alice"));
        h.add_account("1");
        assert!(h.hides("1", "alice"));
        assert!(!h.hides("2", "bob@example.com"));
        h.add_domain("Example.com");
        assert!(h.hides("2", "bob@example.com"));
        assert!(h.hides("3", "carol@social.EXAMPLE.com"));
        assert!(!h.hides("4", "dave@notexample.com"));
        assert!(!h.hides("5", "erin"));
        h.remove_domain("example.com");
        h.remove_account("1");
        assert!(!h.hides("1", "alice") && !h.hides("2", "bob@example.com"));
    }

    #[test]
    fn media_aspect() {
        let m = |meta: &str| {
            let v = Value::parse(&format!(r#"{{"type":"image","meta":{meta}}}"#)).unwrap();
            Media::from_json(&v).aspect
        };
        assert_eq!(m(r#"{"small":{"width":400,"height":200}}"#), 2.0);
        assert_eq!(m(r#"{"original":{"aspect":0.5}}"#), 0.5);
        assert_eq!(m("null"), 0.0);
    }
}
