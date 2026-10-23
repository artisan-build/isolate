//! Bucket-backed content resolution and its short in-memory cache.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::store::{ObjectStore, StoreError};

pub const BUILTIN_DEFAULT_MESSAGE: &str =
    "This site is temporarily unavailable. It will be back shortly.";
pub const BUILTIN_SITE_NAME: &str = "Temporarily unavailable";
pub const BUILTIN_PRIMARY_COLOR: &str = "#1c1b19";
pub const BUILTIN_BACKGROUND_COLOR: &str = "#f6f6f5";
pub const BUILTIN_TEXT_COLOR: &str = "#1c1b19";
pub const MAXIMUM_MESSAGE_CHARACTERS: usize = 2_000;
pub const BUCKET_CACHE_TTL: Duration = Duration::from_secs(10);

type Clock = Arc<dyn Fn() -> Duration + Send + Sync>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedLogo {
    pub bytes: Vec<u8>,
    pub media_type: &'static str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PageContent {
    pub message: String,
    pub site_name: String,
    pub primary_color: String,
    pub background_color: String,
    pub text_color: String,
    pub logo: Option<ResolvedLogo>,
}

impl PageContent {
    pub fn built_in() -> Self {
        Self {
            message: BUILTIN_DEFAULT_MESSAGE.to_owned(),
            site_name: BUILTIN_SITE_NAME.to_owned(),
            primary_color: BUILTIN_PRIMARY_COLOR.to_owned(),
            background_color: BUILTIN_BACKGROUND_COLOR.to_owned(),
            text_color: BUILTIN_TEXT_COLOR.to_owned(),
            logo: None,
        }
    }
}

#[derive(Clone, Default)]
struct BrandingFile {
    site_name: Option<String>,
    primary_color: Option<String>,
    background_color: Option<String>,
    text_color: Option<String>,
    logo: Option<String>,
}

#[derive(Clone, Default)]
struct CacheEntry {
    fetched_at: Duration,
    value: Vec<u8>,
    missing: bool,
    fetched: bool,
    last_good: Vec<u8>,
    has_last_good: bool,
}

struct CachedObjectStore {
    source: Arc<dyn ObjectStore>,
    now: Clock,
    ttl: Duration,
    entries: Mutex<HashMap<String, CacheEntry>>,
}

impl CachedObjectStore {
    fn new(source: Arc<dyn ObjectStore>, now: Clock, ttl: Duration) -> Self {
        Self {
            source,
            now,
            ttl,
            entries: Mutex::new(HashMap::new()),
        }
    }

    async fn get(&self, key: &str) -> Result<Vec<u8>, StoreError> {
        self.get_validated(key, |_| true).await
    }

    async fn get_validated(
        &self,
        key: &str,
        valid: fn(&[u8]) -> bool,
    ) -> Result<Vec<u8>, StoreError> {
        let now = (self.now)();
        let cached = {
            let entries = self.entries.lock().expect("content cache lock");
            entries.get(key).cloned()
        };
        if let Some(entry) = cached {
            let age = now.checked_sub(entry.fetched_at);
            if entry.fetched && age.is_some_and(|age| age < self.ttl) {
                return if entry.missing {
                    Err(StoreError::Missing)
                } else {
                    Ok(entry.value)
                };
            }
        }

        let result = self.source.get(key).await;
        let mut entries = self.entries.lock().expect("content cache lock");
        let entry = entries.entry(key.to_owned()).or_default();
        match result {
            Ok(value) => {
                entry.fetched_at = now;
                entry.fetched = true;
                entry.missing = false;
                entry.value.clone_from(&value);
                if valid(&value) {
                    entry.last_good.clone_from(&value);
                    entry.has_last_good = true;
                }
                Ok(value)
            }
            Err(StoreError::Missing) => {
                entry.fetched_at = now;
                entry.fetched = true;
                entry.missing = true;
                entry.value.clear();
                Err(StoreError::Missing)
            }
            Err(StoreError::Unavailable) if entry.has_last_good => Ok(entry.last_good.clone()),
            Err(StoreError::Unavailable) => Err(StoreError::Unavailable),
        }
    }

    fn last_good(&self, key: &str) -> Option<Vec<u8>> {
        self.entries
            .lock()
            .expect("content cache lock")
            .get(key)
            .filter(|entry| entry.has_last_good)
            .map(|entry| entry.last_good.clone())
    }
}

#[derive(Clone)]
pub struct ContentResolver {
    objects: Arc<CachedObjectStore>,
}

impl ContentResolver {
    pub fn production(source: Arc<dyn ObjectStore>) -> Self {
        let started = Instant::now();
        Self::new(
            source,
            Arc::new(move || started.elapsed()),
            BUCKET_CACHE_TTL,
        )
    }

    pub fn new(source: Arc<dyn ObjectStore>, now: Clock, ttl: Duration) -> Self {
        Self {
            objects: Arc::new(CachedObjectStore::new(source, now, ttl)),
        }
    }

    pub async fn resolve(&self, request_host: &str) -> PageContent {
        let site_prefix = site_prefix(request_host);

        let site_branding = self.branding(&site_prefix).await;
        let default_branding = self.branding("default/").await;
        let mut content = PageContent {
            message: self.message(&site_prefix).await,
            site_name: first_string(
                site_branding.site_name.as_ref(),
                default_branding.site_name.as_ref(),
                BUILTIN_SITE_NAME,
            ),
            primary_color: first_string(
                site_branding.primary_color.as_ref(),
                default_branding.primary_color.as_ref(),
                BUILTIN_PRIMARY_COLOR,
            ),
            background_color: first_string(
                site_branding.background_color.as_ref(),
                default_branding.background_color.as_ref(),
                BUILTIN_BACKGROUND_COLOR,
            ),
            text_color: first_string(
                site_branding.text_color.as_ref(),
                default_branding.text_color.as_ref(),
                BUILTIN_TEXT_COLOR,
            ),
            logo: None,
        };
        content.logo = self
            .logo(
                &site_prefix,
                site_branding.logo.as_deref(),
                default_branding.logo.as_deref(),
            )
            .await;
        content
    }

    /// Resolves only from values that previously passed validation. This is
    /// used if the shared request deadline expires while an object read hangs.
    pub fn last_good_or_built_in(&self, request_host: &str) -> PageContent {
        let site_prefix = site_prefix(request_host);
        let site_branding = self.last_good_branding(&site_prefix);
        let default_branding = self.last_good_branding("default/");
        let mut content = PageContent {
            message: self
                .last_good_message(&format!("{site_prefix}message.txt"))
                .or_else(|| self.last_good_message("default/message.txt"))
                .unwrap_or_else(|| BUILTIN_DEFAULT_MESSAGE.to_owned()),
            site_name: first_string(
                site_branding.site_name.as_ref(),
                default_branding.site_name.as_ref(),
                BUILTIN_SITE_NAME,
            ),
            primary_color: first_string(
                site_branding.primary_color.as_ref(),
                default_branding.primary_color.as_ref(),
                BUILTIN_PRIMARY_COLOR,
            ),
            background_color: first_string(
                site_branding.background_color.as_ref(),
                default_branding.background_color.as_ref(),
                BUILTIN_BACKGROUND_COLOR,
            ),
            text_color: first_string(
                site_branding.text_color.as_ref(),
                default_branding.text_color.as_ref(),
                BUILTIN_TEXT_COLOR,
            ),
            logo: None,
        };
        content.logo = self.last_good_logo(
            &site_prefix,
            site_branding.logo.as_deref(),
            default_branding.logo.as_deref(),
        );
        content
    }

    async fn message(&self, site_prefix: &str) -> String {
        if !site_prefix.is_empty() {
            if let Some(value) = self
                .valid_message(&format!("{site_prefix}message.txt"))
                .await
            {
                return value;
            }
        }
        if let Some(value) = self.valid_message("default/message.txt").await {
            return value;
        }
        BUILTIN_DEFAULT_MESSAGE.to_owned()
    }

    async fn valid_message(&self, key: &str) -> Option<String> {
        let value = self
            .objects
            .get_validated(key, valid_message_bytes)
            .await
            .ok()?;
        valid_message_bytes(&value).then(|| String::from_utf8(value).expect("validated UTF-8"))
    }

    fn last_good_message(&self, key: &str) -> Option<String> {
        let value = self.objects.last_good(key)?;
        valid_message_bytes(&value).then(|| String::from_utf8(value).expect("validated UTF-8"))
    }

    async fn branding(&self, prefix: &str) -> BrandingFile {
        if prefix.is_empty() {
            return BrandingFile::default();
        }
        let value = match self
            .objects
            .get_validated(&format!("{prefix}branding.json"), valid_branding_bytes)
            .await
        {
            Ok(value) => value,
            Err(_) => return BrandingFile::default(),
        };
        decode_branding(&value).unwrap_or_default()
    }

    fn last_good_branding(&self, prefix: &str) -> BrandingFile {
        if prefix.is_empty() {
            return BrandingFile::default();
        }
        self.objects
            .last_good(&format!("{prefix}branding.json"))
            .and_then(|value| decode_branding(&value))
            .unwrap_or_default()
    }

    async fn logo(
        &self,
        site_prefix: &str,
        site_name: Option<&str>,
        default_name: Option<&str>,
    ) -> Option<ResolvedLogo> {
        for (prefix, name) in [(site_prefix, site_name), ("default/", default_name)] {
            let Some(name) = name.filter(|name| valid_logo_name(name)) else {
                continue;
            };
            if prefix.is_empty() {
                continue;
            }
            if let Ok(bytes) = self.objects.get(&format!("{prefix}{name}")).await {
                return Some(ResolvedLogo {
                    bytes,
                    media_type: logo_media_type(name),
                });
            }
        }
        None
    }

    fn last_good_logo(
        &self,
        site_prefix: &str,
        site_name: Option<&str>,
        default_name: Option<&str>,
    ) -> Option<ResolvedLogo> {
        for (prefix, name) in [(site_prefix, site_name), ("default/", default_name)] {
            let Some(name) = name.filter(|name| valid_logo_name(name)) else {
                continue;
            };
            if prefix.is_empty() {
                continue;
            }
            if let Some(bytes) = self.objects.last_good(&format!("{prefix}{name}")) {
                return Some(ResolvedLogo {
                    bytes,
                    media_type: logo_media_type(name),
                });
            }
        }
        None
    }
}

pub fn valid_message_bytes(value: &[u8]) -> bool {
    std::str::from_utf8(value)
        .map(|message| message.chars().count() <= MAXIMUM_MESSAGE_CHARACTERS)
        .unwrap_or(false)
}

fn valid_branding_bytes(value: &[u8]) -> bool {
    decode_branding(value).is_some()
}

fn decode_branding(value: &[u8]) -> Option<BrandingFile> {
    let object = serde_json::from_slice::<Value>(value)
        .ok()?
        .as_object()?
        .clone();
    let string = |name: &str| object.get(name).and_then(Value::as_str).map(str::to_owned);
    let color = |name: &str| string(name).filter(|value| valid_color(value));

    Some(BrandingFile {
        site_name: string("site_name"),
        primary_color: color("primary_color"),
        background_color: color("background_color"),
        text_color: color("text_color"),
        logo: string("logo").filter(|value| valid_logo_name(value)),
    })
}

pub fn valid_color(value: &str) -> bool {
    value.len() == 7
        && value.starts_with('#')
        && value.as_bytes()[1..].iter().all(u8::is_ascii_hexdigit)
}

pub fn valid_logo_name(value: &str) -> bool {
    matches!(value, "logo.png" | "logo.webp" | "logo.jpg")
}

fn logo_media_type(name: &str) -> &'static str {
    match name {
        "logo.png" => "image/png",
        "logo.webp" => "image/webp",
        "logo.jpg" => "image/jpeg",
        _ => unreachable!("logo name was validated"),
    }
}

fn first_string(site: Option<&String>, fallback: Option<&String>, built_in: &str) -> String {
    site.or(fallback)
        .map_or_else(|| built_in.to_owned(), Clone::clone)
}

fn site_prefix(request_host: &str) -> String {
    let host = normalize_host(request_host);
    if host.is_empty() {
        String::new()
    } else {
        format!("sites/{host}/")
    }
}

pub fn normalize_host(host: &str) -> String {
    let mut host = host.trim().to_lowercase();
    if let Some(rest) = host.strip_prefix('[') {
        if let Some(end) = rest.find(']') {
            host = rest[..end].to_owned();
        }
    } else if host.matches(':').count() == 1 {
        host.truncate(host.find(':').expect("one colon exists"));
    }
    host = host.trim_end_matches('.').to_owned();
    if let Some(apex) = host.strip_prefix("www.") {
        apex.to_owned()
    } else {
        host
    }
}
