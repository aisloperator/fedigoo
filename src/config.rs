//! Persistent local data, kept in `$XDG_CONFIG_HOME/fedigoo/` (normally
//! `~/.config/fedigoo/`).

use crate::json::{Value, obj};
use gtk::glib;
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::PathBuf;

const CONFIG_FILE: &str = "config.json";

pub fn dir() -> PathBuf {
    glib::user_config_dir().join("fedigoo")
}

/// A logged-in user on some instance.
#[derive(Clone, Debug, Default)]
pub struct Account {
    /// Instance host name, e.g. "mastodon.social".
    pub instance: String,
    pub token: String,
    pub id: String,
    pub acct: String,
    pub display_name: String,
    pub avatar: String,
}

impl Account {
    pub fn full_acct(&self) -> String {
        format!("@{}@{}", self.acct, self.instance)
    }
}

/// OAuth client credentials obtained by registering FediGoo on an instance.
#[derive(Clone, Debug, Default)]
pub struct AppReg {
    pub instance: String,
    pub client_id: String,
    pub client_secret: String,
}

#[derive(Clone, Debug)]
pub struct Settings {
    pub show_sensitive_media: bool,
    pub expand_content_warnings: bool,
    pub stream_updates: bool,
    /// Sidebar shows icons only (labels as tooltips).
    pub compact_sidebar: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            show_sensitive_media: false,
            expand_content_warnings: false,
            stream_updates: true,
            compact_sidebar: false,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Config {
    pub accounts: Vec<Account>,
    pub active: usize,
    pub apps: Vec<AppReg>,
    pub settings: Settings,
}

impl Config {
    pub fn load() -> Config {
        let path = dir().join(CONFIG_FILE);
        let Ok(text) = std::fs::read_to_string(&path) else {
            return Config::default();
        };
        let v = match Value::parse(&text) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("fedigoo: ignoring unreadable {}: {e}", path.display());
                return Config::default();
            }
        };
        let accounts = v["accounts"]
            .arr()
            .iter()
            .map(|a| Account {
                instance: a["instance"].str().to_string(),
                token: a["token"].str().to_string(),
                id: a["id"].str().to_string(),
                acct: a["acct"].str().to_string(),
                display_name: a["display_name"].str().to_string(),
                avatar: a["avatar"].str().to_string(),
            })
            .collect::<Vec<_>>();
        let apps = v["apps"]
            .arr()
            .iter()
            .map(|a| AppReg {
                instance: a["instance"].str().to_string(),
                client_id: a["client_id"].str().to_string(),
                client_secret: a["client_secret"].str().to_string(),
            })
            .collect();
        let s = &v["settings"];
        let d = Settings::default();
        let flag = |k: &str, def: bool| match &s[k] {
            Value::Bool(b) => *b,
            _ => def,
        };
        Config {
            active: (v["active"].i64().max(0) as usize).min(accounts.len().saturating_sub(1)),
            accounts,
            apps,
            settings: Settings {
                show_sensitive_media: flag("show_sensitive_media", d.show_sensitive_media),
                expand_content_warnings: flag("expand_content_warnings", d.expand_content_warnings),
                stream_updates: flag("stream_updates", d.stream_updates),
                compact_sidebar: flag("compact_sidebar", d.compact_sidebar),
            },
        }
    }

    pub fn save(&self) {
        if let Err(e) = self.try_save() {
            eprintln!("fedigoo: could not save configuration: {e}");
        }
    }

    fn try_save(&self) -> std::io::Result<()> {
        let dir = dir();
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&dir)?;
        let accounts = self
            .accounts
            .iter()
            .map(|a| {
                obj([
                    ("instance", a.instance.as_str().into()),
                    ("token", a.token.as_str().into()),
                    ("id", a.id.as_str().into()),
                    ("acct", a.acct.as_str().into()),
                    ("display_name", a.display_name.as_str().into()),
                    ("avatar", a.avatar.as_str().into()),
                ])
            })
            .collect::<Vec<_>>();
        let apps = self
            .apps
            .iter()
            .map(|a| {
                obj([
                    ("instance", a.instance.as_str().into()),
                    ("client_id", a.client_id.as_str().into()),
                    ("client_secret", a.client_secret.as_str().into()),
                ])
            })
            .collect::<Vec<_>>();
        let s = &self.settings;
        let v = obj([
            ("active", (self.active as i64).into()),
            ("accounts", accounts.into()),
            ("apps", apps.into()),
            (
                "settings",
                obj([
                    ("show_sensitive_media", s.show_sensitive_media.into()),
                    ("expand_content_warnings", s.expand_content_warnings.into()),
                    ("stream_updates", s.stream_updates.into()),
                    ("compact_sidebar", s.compact_sidebar.into()),
                ]),
            ),
        ]);
        // Write to a temp file then rename, so a crash never leaves a
        // truncated config.  The file holds access tokens: owner-only.
        let tmp = dir.join(format!("{CONFIG_FILE}.tmp"));
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)?;
        f.write_all(v.to_string().as_bytes())?;
        f.sync_all()?;
        std::fs::rename(&tmp, dir.join(CONFIG_FILE))
    }

    pub fn active_account(&self) -> Option<&Account> {
        self.accounts.get(self.active)
    }

    pub fn app_for(&self, instance: &str) -> Option<&AppReg> {
        self.apps.iter().find(|a| a.instance == instance)
    }
}
