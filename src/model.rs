//! Typed views of the Mastodon API entities that the UI works with.

use crate::json::Value;

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
    /// Set when a server-side filter matched with the "warn" action.
    pub filtered: Option<String>,
}

impl Status {
    pub fn from_json(v: &Value) -> Status {
        let filtered = v["filtered"]
            .arr()
            .iter()
            .find(|r| r["filter"]["filter_action"].str() == "warn")
            .map(|r| s(&r["filter"]["title"]));
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

#[cfg(test)]
mod tests {
    use super::*;

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
