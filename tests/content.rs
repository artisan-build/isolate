mod common;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use common::FakeStore;
use isolate::content::{
    normalize_host, ContentResolver, BUCKET_CACHE_TTL, BUILTIN_BACKGROUND_COLOR,
    BUILTIN_DEFAULT_MESSAGE, BUILTIN_PRIMARY_COLOR, BUILTIN_SITE_NAME, BUILTIN_TEXT_COLOR,
    MAXIMUM_MESSAGE_CHARACTERS,
};

fn test_resolver(store: &FakeStore, milliseconds: &Arc<AtomicU64>) -> ContentResolver {
    let clock = Arc::clone(milliseconds);
    ContentResolver::new(
        Arc::new(store.clone()),
        Arc::new(move || Duration::from_millis(clock.load(Ordering::SeqCst))),
        BUCKET_CACHE_TTL,
    )
}

#[test]
fn host_normalization_matches_bucket_keys() {
    for (input, expected) in [
        ("Example.COM", "example.com"),
        ("WWW.Example.COM", "example.com"),
        ("www.Example.COM:8443", "example.com"),
        ("example.com.", "example.com"),
        ("WWW.Example.COM.:443", "example.com"),
        ("www.www.example.com", "www.example.com"),
    ] {
        assert_eq!(normalize_host(input), expected, "input {input}");
    }
}

#[tokio::test]
async fn resolution_is_independent_per_field() {
    let store = FakeStore::default();
    store.object(
        "sites/example.com/message.txt",
        "界".repeat(MAXIMUM_MESSAGE_CHARACTERS + 1),
    );
    store.object("default/message.txt", "Default maintenance message");
    store.object(
        "sites/example.com/branding.json",
        br##"{
            "site_name":"",
            "primary_color":"red; background: url(evil)",
            "background_color":"#ABCDEF",
            "text_color":false,
            "logo":"../logo.png",
            "future_setting":"ignored"
        }"##
        .to_vec(),
    );
    store.object(
        "default/branding.json",
        br##"{
            "site_name":"Default name",
            "primary_color":"#123456",
            "background_color":"#ffffff",
            "text_color":"#654321",
            "logo":"logo.webp"
        }"##
        .to_vec(),
    );
    store.object("default/logo.webp", b"webp-test-bytes".to_vec());
    let now = Arc::new(AtomicU64::new(0));

    let content = test_resolver(&store, &now)
        .resolve("WWW.Example.COM.:443")
        .await;

    assert_eq!(content.message, "Default maintenance message");
    assert_eq!(content.site_name, "", "empty site_name remains provided");
    assert_eq!(content.primary_color, "#123456");
    assert_eq!(content.background_color, "#ABCDEF");
    assert_eq!(content.text_color, "#654321");
    assert_eq!(content.logo.expect("default logo").media_type, "image/webp");
}

#[tokio::test]
async fn malformed_missing_wrong_typed_and_null_values_fall_through() {
    for bad_message in [
        vec![0xff, 0xfe],
        "x".repeat(MAXIMUM_MESSAGE_CHARACTERS + 1).into_bytes(),
    ] {
        let store = FakeStore::default();
        store.object("sites/example.com/message.txt", bad_message);
        store.object("default/message.txt", "A valid fallback");
        store.object("sites/example.com/branding.json", br#"{"site_name":"#);
        store.object("default/branding.json", br#"{"site_name":"Fallback name"}"#);
        let content = test_resolver(&store, &Arc::new(AtomicU64::new(0)))
            .resolve("example.com")
            .await;
        assert_eq!(content.message, "A valid fallback");
        assert_eq!(content.site_name, "Fallback name");
    }

    let store = FakeStore::default();
    store.object(
        "sites/example.com/branding.json",
        br##"{
            "site_name":null,
            "primary_color":42,
            "background_color":"#abcdef",
            "text_color":false,
            "logo":["logo.png"]
        }"##
        .to_vec(),
    );
    store.object(
        "default/branding.json",
        br##"{
            "site_name":"Default site",
            "primary_color":"#123456",
            "text_color":"#654321",
            "logo":"logo.svg"
        }"##
        .to_vec(),
    );
    let content = test_resolver(&store, &Arc::new(AtomicU64::new(0)))
        .resolve("example.com")
        .await;
    assert_eq!(content.site_name, "Default site");
    assert_eq!(content.primary_color, "#123456");
    assert_eq!(content.background_color, "#abcdef");
    assert_eq!(content.text_color, "#654321");
    assert!(content.logo.is_none());

    let store = FakeStore::default();
    store.object("sites/example.com/branding.json", vec![0xff, 0xfe]);
    let content = test_resolver(&store, &Arc::new(AtomicU64::new(0)))
        .resolve("example.com")
        .await;
    assert_eq!(content, isolate::content::PageContent::built_in());
}

#[tokio::test]
async fn positive_and_negative_cache_refresh_at_exactly_ten_seconds() {
    let store = FakeStore::default();
    store.object("sites/example.com/message.txt", "first");
    store.object("default/message.txt", "default");
    let now = Arc::new(AtomicU64::new(0));
    let resolver = test_resolver(&store, &now);

    assert_eq!(resolver.resolve("example.com").await.message, "first");
    store.object("sites/example.com/message.txt", "second");
    now.store(9_999, Ordering::SeqCst);
    assert_eq!(resolver.resolve("example.com").await.message, "first");
    assert_eq!(store.calls("sites/example.com/message.txt"), 1);
    now.store(10_000, Ordering::SeqCst);
    assert_eq!(resolver.resolve("example.com").await.message, "second");
    assert_eq!(store.calls("sites/example.com/message.txt"), 2);

    let absent = FakeStore::default();
    absent.object("default/message.txt", "negative fallback");
    let now = Arc::new(AtomicU64::new(0));
    let resolver = test_resolver(&absent, &now);
    assert_eq!(
        resolver.resolve("missing.example").await.message,
        "negative fallback"
    );
    absent.object("sites/missing.example/message.txt", "appeared");
    now.store(9_999, Ordering::SeqCst);
    assert_eq!(
        resolver.resolve("missing.example").await.message,
        "negative fallback"
    );
    assert_eq!(absent.calls("sites/missing.example/message.txt"), 1);
    now.store(10_000, Ordering::SeqCst);
    assert_eq!(
        resolver.resolve("missing.example").await.message,
        "appeared"
    );
    assert_eq!(absent.calls("sites/missing.example/message.txt"), 2);
}

#[tokio::test]
async fn moving_the_clock_backward_expires_a_cached_entry() {
    let store = FakeStore::default();
    store.object("sites/example.com/message.txt", "before clock correction");
    let now = Arc::new(AtomicU64::new(10_000));
    let resolver = test_resolver(&store, &now);

    assert_eq!(
        resolver.resolve("example.com").await.message,
        "before clock correction"
    );
    store.object("sites/example.com/message.txt", "after clock correction");
    now.store(9_000, Ordering::SeqCst);

    assert_eq!(
        resolver.resolve("example.com").await.message,
        "after clock correction"
    );
    assert_eq!(store.calls("sites/example.com/message.txt"), 2);
}

#[tokio::test]
async fn validated_last_good_survives_malformed_missing_and_outage_states() {
    let store = FakeStore::default();
    let message_key = "sites/example.com/message.txt";
    let branding_key = "sites/example.com/branding.json";
    store.object(message_key, "Last valid message");
    store.object(
        branding_key,
        br##"{"site_name":"Last valid site","primary_color":"#123456"}"##,
    );
    let now = Arc::new(AtomicU64::new(0));
    let resolver = test_resolver(&store, &now);
    let initial = resolver.resolve("example.com").await;
    assert_eq!(initial.message, "Last valid message");
    assert_eq!(initial.site_name, "Last valid site");

    now.store(10_000, Ordering::SeqCst);
    store.object(message_key, "x".repeat(MAXIMUM_MESSAGE_CHARACTERS + 1));
    store.object(branding_key, br#"{"site_name":"#);
    let malformed = resolver.resolve("example.com").await;
    assert_eq!(malformed.message, BUILTIN_DEFAULT_MESSAGE);
    assert_eq!(malformed.site_name, BUILTIN_SITE_NAME);

    now.store(20_000, Ordering::SeqCst);
    store.error(message_key);
    store.error(branding_key);
    let outage = resolver.resolve("example.com").await;
    assert_eq!(outage.message, "Last valid message");
    assert_eq!(outage.site_name, "Last valid site");
    assert_eq!(outage.primary_color, "#123456");

    now.store(30_000, Ordering::SeqCst);
    store.remove(message_key);
    assert_eq!(
        resolver.resolve("example.com").await.message,
        BUILTIN_DEFAULT_MESSAGE
    );
    now.store(40_000, Ordering::SeqCst);
    store.error(message_key);
    assert_eq!(
        resolver.resolve("example.com").await.message,
        "Last valid message"
    );
}

#[tokio::test]
async fn outage_before_a_good_read_uses_all_built_ins() {
    let store = FakeStore::default();
    for key in [
        "sites/example.com/message.txt",
        "default/message.txt",
        "sites/example.com/branding.json",
        "default/branding.json",
    ] {
        store.error(key);
    }
    let content = test_resolver(&store, &Arc::new(AtomicU64::new(0)))
        .resolve("example.com")
        .await;
    assert_eq!(content.message, BUILTIN_DEFAULT_MESSAGE);
    assert_eq!(content.site_name, BUILTIN_SITE_NAME);
    assert_eq!(content.primary_color, BUILTIN_PRIMARY_COLOR);
    assert_eq!(content.background_color, BUILTIN_BACKGROUND_COLOR);
    assert_eq!(content.text_color, BUILTIN_TEXT_COLOR);
}

#[test]
fn production_cache_ttl_is_exactly_ten_seconds() {
    assert_eq!(BUCKET_CACHE_TTL, Duration::from_secs(10));
}

#[tokio::test]
async fn messages_accept_two_thousand_unicode_scalars_and_reject_one_more() {
    let store = FakeStore::default();
    store.object(
        "sites/example.com/message.txt",
        "界".repeat(MAXIMUM_MESSAGE_CHARACTERS),
    );
    store.object("default/message.txt", "default after oversized");
    let now = Arc::new(AtomicU64::new(0));
    let resolver = test_resolver(&store, &now);
    assert_eq!(
        resolver
            .resolve("example.com")
            .await
            .message
            .chars()
            .count(),
        MAXIMUM_MESSAGE_CHARACTERS
    );

    now.store(10_000, Ordering::SeqCst);
    store.object(
        "sites/example.com/message.txt",
        "界".repeat(MAXIMUM_MESSAGE_CHARACTERS + 1),
    );
    assert_eq!(
        resolver.resolve("example.com").await.message,
        "default after oversized"
    );
}
