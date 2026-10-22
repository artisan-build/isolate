package main

import (
	"context"
	"errors"
	"strings"
	"sync"
	"testing"
	"time"
)

type fakeObjectStore struct {
	mu      sync.Mutex
	objects map[string][]byte
	errors  map[string]error
	calls   map[string]int
	block   bool
}

func newFakeObjectStore() *fakeObjectStore {
	return &fakeObjectStore{
		objects: make(map[string][]byte),
		errors:  make(map[string]error),
		calls:   make(map[string]int),
	}
}

func (s *fakeObjectStore) Get(ctx context.Context, key string) ([]byte, error) {
	s.mu.Lock()
	s.calls[key]++
	block := s.block
	value, found := s.objects[key]
	err := s.errors[key]
	s.mu.Unlock()

	if block {
		<-ctx.Done()
		return nil, ctx.Err()
	}
	if err != nil {
		return nil, err
	}
	if !found {
		return nil, errObjectNotFound
	}

	return append([]byte(nil), value...), nil
}

func (s *fakeObjectStore) callCount(key string) int {
	s.mu.Lock()
	defer s.mu.Unlock()

	return s.calls[key]
}

func TestNormalizeHostForBucketLookup(t *testing.T) {
	t.Parallel()

	cases := map[string]string{
		"Example.COM":          "example.com",
		"WWW.Example.COM":      "example.com",
		"www.Example.COM:8443": "example.com",
		"example.com.":         "example.com",
		"WWW.Example.COM.:443": "example.com",
	}

	for input, want := range cases {
		if got := normalizeHost(input); got != want {
			t.Errorf("normalizeHost(%q) = %q, want %q", input, got, want)
		}
	}
}

func TestResolveUsesSiteDefaultAndBuiltInPerField(t *testing.T) {
	store := newFakeObjectStore()
	store.objects["sites/example.com/message.txt"] = []byte(strings.Repeat("x", maximumMessageCharacters+1))
	store.objects["default/message.txt"] = []byte("Default maintenance message")
	store.objects["sites/example.com/branding.json"] = []byte(`{
		"site_name":"Site name <unsafe>",
		"primary_color":"red; background: url(evil)",
		"background_color":"#ABCDEF",
		"logo":"../logo.png",
		"future_setting":"ignored"
	}`)
	store.objects["default/branding.json"] = []byte(`{
		"site_name":"Default name",
		"primary_color":"#123456",
		"background_color":"#ffffff",
		"text_color":"#654321",
		"logo":"logo.webp"
	}`)
	store.objects["default/logo.webp"] = []byte("webp-test-bytes")

	resolver := newContentResolver(store, time.Now, 10*time.Second)
	content := resolver.Resolve(context.Background(), "WWW.Example.COM.:443")

	if content.Message != "Default maintenance message" {
		t.Errorf("message = %q, want default message", content.Message)
	}
	if content.SiteName != "Site name <unsafe>" {
		t.Errorf("site name = %q, want site value", content.SiteName)
	}
	if content.PrimaryColor != "#123456" {
		t.Errorf("primary color = %q, want valid default", content.PrimaryColor)
	}
	if content.BackgroundColor != "#ABCDEF" {
		t.Errorf("background color = %q, want site value", content.BackgroundColor)
	}
	if content.TextColor != "#654321" {
		t.Errorf("text color = %q, want default value", content.TextColor)
	}
	if content.Logo == nil {
		t.Fatal("logo is nil, want default logo")
	}
	if string(content.Logo.Bytes) != "webp-test-bytes" || content.Logo.MediaType != "image/webp" {
		t.Errorf("logo = (%q, %q), want default WebP", content.Logo.Bytes, content.Logo.MediaType)
	}
}

func TestResolveFallsThroughMissingAndMalformedFiles(t *testing.T) {
	cases := map[string][]byte{
		"invalid UTF-8 message": {0xff, 0xfe},
		"oversized message":     []byte(strings.Repeat("界", maximumMessageCharacters+1)),
	}

	for name, siteMessage := range cases {
		t.Run(name, func(t *testing.T) {
			store := newFakeObjectStore()
			store.objects["sites/example.com/message.txt"] = siteMessage
			store.objects["default/message.txt"] = []byte("A valid fallback")
			store.objects["sites/example.com/branding.json"] = []byte(`{"site_name":`)
			store.objects["default/branding.json"] = []byte(`{"site_name":"Fallback name"}`)

			content := newContentResolver(store, time.Now, 10*time.Second).
				Resolve(context.Background(), "example.com")

			if content.Message != "A valid fallback" {
				t.Errorf("message = %q, want valid fallback", content.Message)
			}
			if content.SiteName != "Fallback name" {
				t.Errorf("site name = %q, want valid fallback", content.SiteName)
			}
		})
	}

	store := newFakeObjectStore()
	content := newContentResolver(store, time.Now, 10*time.Second).
		Resolve(context.Background(), "missing.example")
	if content.Message != builtinDefaultMessage || content.SiteName != builtinSiteName {
		t.Errorf("missing objects resolved to %#v, want built-ins", content)
	}

	invalidBrandingStore := newFakeObjectStore()
	invalidBrandingStore.objects["sites/example.com/branding.json"] = []byte{0xff, 0xfe}
	invalidBrandingStore.objects["default/branding.json"] = []byte(`{"site_name":"UTF-8 fallback"}`)
	content = newContentResolver(invalidBrandingStore, time.Now, 10*time.Second).
		Resolve(context.Background(), "example.com")
	if content.SiteName != "UTF-8 fallback" {
		t.Errorf("invalid UTF-8 branding resolved site name %q, want default", content.SiteName)
	}
}

func TestObjectCacheReusesPositiveAndNegativeResultsForLessThanTenSeconds(t *testing.T) {
	now := time.Date(2026, 10, 23, 12, 0, 0, 0, time.UTC)
	store := newFakeObjectStore()
	store.objects["present"] = []byte("first")
	cache := newCachedObjectStore(store, func() time.Time { return now }, 10*time.Second)

	if got, err := cache.Get(context.Background(), "present"); err != nil || string(got) != "first" {
		t.Fatalf("first positive read = %q, %v", got, err)
	}
	if got, err := cache.Get(context.Background(), "absent"); got != nil || !errors.Is(err, errObjectNotFound) {
		t.Fatalf("first negative read = %q, %v", got, err)
	}
	store.objects["present"] = []byte("second")
	store.objects["absent"] = []byte("appeared")
	now = now.Add(9*time.Second + 999*time.Millisecond)
	if got, err := cache.Get(context.Background(), "present"); err != nil || string(got) != "first" {
		t.Fatalf("cached positive read = %q, %v", got, err)
	}
	if got, err := cache.Get(context.Background(), "absent"); got != nil || !errors.Is(err, errObjectNotFound) {
		t.Fatalf("cached negative read = %q, %v", got, err)
	}

	now = now.Add(time.Millisecond)
	if got, err := cache.Get(context.Background(), "present"); err != nil || string(got) != "second" {
		t.Fatalf("positive refresh at ten seconds = %q, %v", got, err)
	}
	if got, err := cache.Get(context.Background(), "absent"); err != nil || string(got) != "appeared" {
		t.Fatalf("negative refresh at ten seconds = %q, %v", got, err)
	}
	if store.callCount("present") != 2 || store.callCount("absent") != 2 {
		t.Errorf("source calls = present %d, absent %d; want two each", store.callCount("present"), store.callCount("absent"))
	}
}

func TestObjectCacheUsesLastGoodValueOnlyForOutages(t *testing.T) {
	now := time.Date(2026, 10, 23, 12, 0, 0, 0, time.UTC)
	store := newFakeObjectStore()
	store.objects["message"] = []byte("last known value")
	cache := newCachedObjectStore(store, func() time.Time { return now }, 10*time.Second)

	if _, err := cache.Get(context.Background(), "message"); err != nil {
		t.Fatalf("prime cache: %v", err)
	}
	now = now.Add(10 * time.Second)
	delete(store.objects, "message")
	store.errors["message"] = errors.New("bucket unavailable")
	if got, err := cache.Get(context.Background(), "message"); err != nil || string(got) != "last known value" {
		t.Errorf("read during outage = %q, %v; want last good", got, err)
	}

	freshCache := newCachedObjectStore(store, func() time.Time { return now }, 10*time.Second)
	if _, err := freshCache.Get(context.Background(), "message"); err == nil {
		t.Error("outage before a good read returned nil error")
	}
}

func TestOutageBeforeAnyGoodReadUsesBuiltIns(t *testing.T) {
	store := newFakeObjectStore()
	store.errors["default/message.txt"] = errors.New("bucket unavailable")
	store.errors["default/branding.json"] = errors.New("bucket unavailable")

	content := newContentResolver(store, time.Now, 10*time.Second).
		Resolve(context.Background(), "unknown.example")

	if content.Message != builtinDefaultMessage || content.SiteName != builtinSiteName {
		t.Errorf("content = %#v, want built-ins", content)
	}
}

func TestMissingOrInvalidLogoFallsBackWithoutServingIt(t *testing.T) {
	store := newFakeObjectStore()
	store.objects["sites/example.com/branding.json"] = []byte(`{"logo":"logo.svg"}`)
	store.objects["default/branding.json"] = []byte(`{"logo":"logo.jpg"}`)

	content := newContentResolver(store, time.Now, 10*time.Second).
		Resolve(context.Background(), "example.com")

	if content.Logo != nil {
		t.Errorf("logo = %#v, want no resolved logo", content.Logo)
	}
}

func TestValidatedLogoExtensionsSelectTheirImageMediaTypes(t *testing.T) {
	cases := map[string]string{
		"logo.png":  "image/png",
		"logo.webp": "image/webp",
		"logo.jpg":  "image/jpeg",
	}

	for logoName, wantMediaType := range cases {
		t.Run(logoName, func(t *testing.T) {
			store := newFakeObjectStore()
			store.objects["sites/example.com/branding.json"] = []byte(`{"logo":"` + logoName + `"}`)
			store.objects["sites/example.com/"+logoName] = []byte("test image bytes")

			content := newContentResolver(store, time.Now, 10*time.Second).
				Resolve(context.Background(), "example.com")

			if content.Logo == nil || content.Logo.MediaType != wantMediaType {
				t.Errorf("logo = %#v, want media type %q", content.Logo, wantMediaType)
			}
		})
	}
}
