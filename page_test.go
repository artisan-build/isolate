package main

import (
	"bytes"
	"net/http"
	"net/http/httptest"
	"os"
	"strings"
	"testing"
	"time"
)

func TestRenderPageEscapesContentAndSplitsBlankLineParagraphs(t *testing.T) {
	content := pageContent{
		Message:         "First <unsafe> paragraph\n\nSecond & final paragraph",
		SiteName:        `A "quoted" <site>`,
		PrimaryColor:    "#123456",
		BackgroundColor: "#abcdef",
		TextColor:       "#654321",
	}

	body := renderPage(pageTemplate, content)

	for _, marker := range []string{
		`data-testid="isolate-page"`,
		`data-testid="isolate-brand"`,
		`data-testid="isolate-message"`,
		`data-testid="isolate-footer"`,
	} {
		if !strings.Contains(body, marker) {
			t.Errorf("body is missing structural marker %q", marker)
		}
	}
	if !strings.Contains(body, `<h1>A &quot;quoted&quot; &lt;site&gt;</h1>`) {
		t.Error("body does not contain the escaped test site name")
	}
	if !strings.Contains(body, "<p>First &lt;unsafe&gt; paragraph</p>\n<p>Second &amp; final paragraph</p>") {
		t.Error("body does not contain two escaped test paragraphs")
	}
	if strings.Contains(body, "<unsafe>") || strings.Contains(body, "{{") {
		t.Error("body contains unsafe input or an unresolved placeholder")
	}
}

func TestHandlerKeepsPageShapeForMethodsPathsAndQueries(t *testing.T) {
	store := newFakeObjectStore()
	store.objects["sites/example.com/message.txt"] = []byte("Method and route test message")
	store.objects["sites/example.com/branding.json"] = []byte(`{"logo":"logo.png"}`)
	store.objects["sites/example.com/logo.png"] = []byte("png-test-bytes")
	resolver := newContentResolver(store, time.Now, 10*time.Second)
	h := handler(resolver, pageTemplate, 50*time.Millisecond)

	cases := []struct {
		method string
		target string
	}{
		{http.MethodGet, "/"},
		{http.MethodHead, "/status"},
		{http.MethodPost, "/api/orders?attempt=2"},
		{http.MethodDelete, "/.env"},
		{"PROPFIND", "/deep/path"},
		{http.MethodPost, "/__isolate/logo"},
		{http.MethodGet, "/__isolate/logo?"},
		{http.MethodGet, "/__isolate/logo?download=1"},
		{http.MethodGet, "/__isolate%2Flogo"},
		{http.MethodGet, "/__isolate/logo/"},
	}

	for _, testCase := range cases {
		request := httptest.NewRequest(testCase.method, "http://example.com"+testCase.target, strings.NewReader("unread body"))
		recorder := httptest.NewRecorder()
		h.ServeHTTP(recorder, request)

		if recorder.Code != http.StatusServiceUnavailable {
			t.Errorf("%s %s: status = %d, want 503", testCase.method, testCase.target, recorder.Code)
		}
		assertPageHeaders(t, recorder.Header())
		if !strings.Contains(recorder.Body.String(), `data-testid="isolate-page"`) {
			t.Errorf("%s %s: response does not contain the page marker", testCase.method, testCase.target)
		}
		if !strings.Contains(recorder.Body.String(), "Method and route test message") {
			t.Errorf("%s %s: response does not contain test-created content", testCase.method, testCase.target)
		}
		if strings.Contains(recorder.Body.String(), "png-test-bytes") {
			t.Errorf("%s %s: response contains logo bytes", testCase.method, testCase.target)
		}
	}
}

func TestExactLogoPathServesBytesAndMediaType(t *testing.T) {
	store := newFakeObjectStore()
	store.objects["sites/example.com/branding.json"] = []byte(`{"logo":"logo.jpg","unused":true}`)
	store.objects["sites/example.com/logo.jpg"] = []byte("jpeg-test-bytes")
	h := handler(newContentResolver(store, time.Now, 10*time.Second), pageTemplate, 50*time.Millisecond)

	for _, method := range []string{http.MethodGet, http.MethodHead} {
		t.Run(method, func(t *testing.T) {
			request := httptest.NewRequest(method, "http://example.com/__isolate/logo", nil)
			recorder := httptest.NewRecorder()
			h.ServeHTTP(recorder, request)

			if recorder.Code != http.StatusOK {
				t.Fatalf("status = %d, want 200", recorder.Code)
			}
			if got := recorder.Header().Get("Content-Type"); got != "image/jpeg" {
				t.Errorf("Content-Type = %q, want image/jpeg", got)
			}
			wantBody := "jpeg-test-bytes"
			if method == http.MethodHead {
				wantBody = ""
			}
			if got := recorder.Body.String(); got != wantBody {
				t.Errorf("body = %q, want %q", got, wantBody)
			}
		})
	}
}

func TestMissingLogoAtExactPathReturnsThePage(t *testing.T) {
	store := newFakeObjectStore()
	store.objects["sites/example.com/message.txt"] = []byte("Missing logo fallback message")
	store.objects["sites/example.com/branding.json"] = []byte(`{"logo":"logo.png"}`)
	h := handler(newContentResolver(store, time.Now, 10*time.Second), pageTemplate, 50*time.Millisecond)

	request := httptest.NewRequest(http.MethodGet, "http://example.com/__isolate/logo", nil)
	recorder := httptest.NewRecorder()
	h.ServeHTTP(recorder, request)

	if recorder.Code != http.StatusServiceUnavailable {
		t.Errorf("status = %d, want 503 page fallback", recorder.Code)
	}
	assertPageHeaders(t, recorder.Header())
	if !strings.Contains(recorder.Body.String(), "Missing logo fallback message") {
		t.Error("page fallback does not contain test-created content")
	}
}

func TestMalformedBrandingKeeps503AndBuiltInBranding(t *testing.T) {
	store := newFakeObjectStore()
	store.objects["sites/example.com/message.txt"] = []byte("Malformed branding test message")
	store.objects["sites/example.com/branding.json"] = []byte(`{"site_name":`)
	h := handler(newContentResolver(store, time.Now, 10*time.Second), pageTemplate, 50*time.Millisecond)

	request := httptest.NewRequest(http.MethodGet, "http://example.com/maintenance", nil)
	recorder := httptest.NewRecorder()
	h.ServeHTTP(recorder, request)

	if recorder.Code != http.StatusServiceUnavailable {
		t.Errorf("status = %d, want 503", recorder.Code)
	}
	assertPageHeaders(t, recorder.Header())
	if !strings.Contains(recorder.Body.String(), "Malformed branding test message") {
		t.Error("page does not contain test-created message")
	}
	if !strings.Contains(recorder.Body.String(), "<h1>"+builtinSiteName+"</h1>") {
		t.Error("page does not contain built-in branding")
	}
}

func TestHandlerCancelsBlockedBucketAndServesBuiltIns(t *testing.T) {
	store := newFakeObjectStore()
	store.block = true
	injectedTimeout := 25 * time.Millisecond
	h := handler(newContentResolver(store, time.Now, 10*time.Second), pageTemplate, injectedTimeout)
	request := httptest.NewRequest(http.MethodGet, "http://blocked.example/", nil)
	recorder := httptest.NewRecorder()

	started := time.Now()
	h.ServeHTTP(recorder, request)
	elapsed := time.Since(started)

	if elapsed >= 3*injectedTimeout {
		t.Errorf("blocked request took %s, want injected deadline to cancel promptly", elapsed)
	}
	if recorder.Code != http.StatusServiceUnavailable {
		t.Errorf("status = %d, want 503", recorder.Code)
	}
	if !strings.Contains(recorder.Body.String(), builtinDefaultMessage) {
		t.Error("response does not contain the built-in message")
	}
}

func TestProductionBucketRequestTimeoutIsExactlyTwoSeconds(t *testing.T) {
	if bucketRequestTimeout != 2*time.Second {
		t.Errorf("bucketRequestTimeout = %s, want exactly 2s", bucketRequestTimeout)
	}
}

func TestRenderedBodiesMatchGoldenFixtures(t *testing.T) {
	builtin := pageContent{
		Message:         builtinDefaultMessage,
		SiteName:        builtinSiteName,
		PrimaryColor:    builtinPrimaryColor,
		BackgroundColor: builtinBackgroundColor,
		TextColor:       builtinTextColor,
	}
	custom := pageContent{
		Message:         "Scheduled maintenance is underway.\n\nStatus updates are posted separately.",
		SiteName:        "Example & Company",
		PrimaryColor:    "#1d4ed8",
		BackgroundColor: "#fff7ed",
		TextColor:       "#0f172a",
		Logo:            &resolvedLogo{Bytes: []byte("fixture-logo"), MediaType: "image/webp"},
	}

	assertGoldenBody(t, "testdata/builtin.html", renderPage(pageTemplate, builtin))
	assertGoldenBody(t, "testdata/customized.html", renderPage(pageTemplate, custom))
}

func assertPageHeaders(t *testing.T, header http.Header) {
	t.Helper()

	wants := map[string]string{
		"Content-Type":  "text/html; charset=utf-8",
		"Cache-Control": "no-store, no-transform",
		"Retry-After":   "3600",
		"X-Robots-Tag":  "noindex",
	}
	for name, want := range wants {
		if got := header.Get(name); got != want {
			t.Errorf("%s = %q, want %q", name, got, want)
		}
	}
}

func assertGoldenBody(t *testing.T, path, got string) {
	t.Helper()

	want, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("read golden fixture %s: %v", path, err)
	}
	if !bytes.Equal([]byte(got), want) {
		t.Errorf("rendered body differs from %s", path)
	}
}
