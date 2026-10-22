package main

import (
	"crypto/sha256"
	"encoding/hex"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
)

// templateSHA256 pins templates/isolated.html. The same constant is asserted by
// the rust branch's tests, which is what keeps the page byte-identical across
// the two runtimes. Changing the template means changing it on both branches.
const templateSHA256 = "34cd895cbe4d1429fc0bbcc12f759d1d0331e34f7c48fca3fac604848d75e1eb"

func TestTemplateHashMatchesTheOtherRuntime(t *testing.T) {
	sum := sha256.Sum256([]byte(pageTemplate))

	if got := hex.EncodeToString(sum[:]); got != templateSHA256 {
		t.Fatalf("templates/isolated.html hash = %s, want %s (the rust branch pins the same value)", got, templateSHA256)
	}
}

func TestEveryMethodPathAndQueryGetsThePage(t *testing.T) {
	cfg := loadConfig(`{"example.com":"Example is isolated."}`, "")
	srv := httptest.NewServer(handler(cfg, pageTemplate))
	defer srv.Close()

	methods := []string{
		http.MethodGet, http.MethodPost, http.MethodPut, http.MethodPatch,
		http.MethodDelete, http.MethodHead, http.MethodOptions, "PROPFIND",
	}
	paths := []string{
		"/", "/wp-admin/x.php?a=1", "/api/v1/orders", "/.env",
		"/a/very/deep/path?q=1&q=2#frag", "/%2e%2e/etc/passwd",
	}

	for _, method := range methods {
		for _, path := range paths {
			request, err := http.NewRequest(method, srv.URL+path, strings.NewReader("a body that must never be read"))
			if err != nil {
				t.Fatalf("new request %s %s: %v", method, path, err)
			}
			request.Host = "example.com"

			response, err := http.DefaultClient.Do(request)
			if err != nil {
				t.Fatalf("%s %s: %v", method, path, err)
			}

			body, err := io.ReadAll(response.Body)
			response.Body.Close()
			if err != nil {
				t.Fatalf("%s %s: read body: %v", method, path, err)
			}

			if response.StatusCode != http.StatusServiceUnavailable {
				t.Errorf("%s %s: status = %d, want 503", method, path, response.StatusCode)
			}
			if got := response.Header.Get("Retry-After"); got != "3600" {
				t.Errorf("%s %s: Retry-After = %q, want %q", method, path, got, "3600")
			}
			if got := response.Header.Get("Cache-Control"); got != "no-store" {
				t.Errorf("%s %s: Cache-Control = %q, want %q", method, path, got, "no-store")
			}
			if got := response.Header.Get("Content-Type"); got != "text/html; charset=utf-8" {
				t.Errorf("%s %s: Content-Type = %q, want %q", method, path, got, "text/html; charset=utf-8")
			}
			if got := response.Header.Get("X-Robots-Tag"); got != "noindex" {
				t.Errorf("%s %s: X-Robots-Tag = %q, want %q", method, path, got, "noindex")
			}
			if got := response.Header.Get("X-Powered-By"); got != "" {
				t.Errorf("%s %s: X-Powered-By = %q, want it absent", method, path, got)
			}

			// HEAD carries the headers but no body, by the HTTP spec.
			if method == http.MethodHead {
				if len(body) != 0 {
					t.Errorf("HEAD %s: body = %d bytes, want 0", path, len(body))
				}

				continue
			}

			if want := renderPage(pageTemplate, "Example is isolated."); string(body) != want {
				t.Errorf("%s %s: body does not match the rendered page", method, path)
			}
		}
	}
}

func TestHostSelectsTheMessage(t *testing.T) {
	cfg := loadConfig(
		`{"one.example":"One is isolated.","two.example":"Two is isolated."}`,
		"Everything else is isolated.",
	)

	cases := map[string]string{
		"one.example":      "One is isolated.",
		"TWO.Example:8080": "Two is isolated.",
		"one.example.":     "One is isolated.",
		"three.example":    "Everything else is isolated.",
		"":                 "Everything else is isolated.",
	}

	for host, want := range cases {
		if got := cfg.messageFor(host); got != want {
			t.Errorf("messageFor(%q) = %q, want %q", host, got, want)
		}
	}
}

func TestResponseBodyCarriesTheHostsMessage(t *testing.T) {
	cfg := loadConfig(`{"one.example":"One is isolated."}`, "Everything else is isolated.")
	h := handler(cfg, pageTemplate)

	for host, want := range map[string]string{
		"one.example":   "One is isolated.",
		"three.example": "Everything else is isolated.",
	} {
		request := httptest.NewRequest(http.MethodGet, "http://"+host+"/anything?x=1", nil)
		request.Host = host
		recorder := httptest.NewRecorder()

		h.ServeHTTP(recorder, request)

		if body := recorder.Body.String(); !strings.Contains(body, "<p>"+want+"</p>") {
			t.Errorf("host %q: body does not contain %q", host, want)
		}
	}
}

func TestUnknownHostFallsBackToTheBuiltInDefault(t *testing.T) {
	cfg := loadConfig("", "")

	if got := cfg.messageFor("anything.example"); got != builtinDefaultMessage {
		t.Errorf("messageFor = %q, want the built-in default %q", got, builtinDefaultMessage)
	}
}

func TestMessagesAreHTMLEscaped(t *testing.T) {
	raw := `<script>alert("x" & 'y')</script>`
	cfg := loadConfig(`{"xss.example":"<script>alert(\"x\" & 'y')</script>"}`, "")

	if got := cfg.messageFor("xss.example"); got != raw {
		t.Fatalf("messageFor = %q, want the raw message %q", got, raw)
	}

	body := renderPage(pageTemplate, cfg.messageFor("xss.example"))
	want := "&lt;script&gt;alert(&quot;x&quot; &amp; &#39;y&#39;)&lt;/script&gt;"

	if !strings.Contains(body, "<p>"+want+"</p>") {
		t.Errorf("body does not contain the escaped message %q", want)
	}
	if strings.Contains(body, "<script>") {
		t.Error("body contains an unescaped <script> tag")
	}
}

func TestMalformedMessagesFallBackWithoutCrashing(t *testing.T) {
	malformed := []string{
		`{"example.com": }`,
		`not json at all`,
		`["example.com","a message"]`,
		`{"example.com": 42}`,
		`{`,
		`null`,
	}

	for _, value := range malformed {
		cfg := loadConfig(value, "The configured default.")

		if len(cfg.messages) != 0 {
			t.Errorf("ISOLATE_MESSAGES=%q produced %d host messages, want 0", value, len(cfg.messages))
		}
		if got := cfg.messageFor("example.com"); got != "The configured default." {
			t.Errorf("ISOLATE_MESSAGES=%q: messageFor = %q, want the default", value, got)
		}

		recorder := httptest.NewRecorder()
		handler(cfg, pageTemplate).ServeHTTP(recorder, httptest.NewRequest(http.MethodGet, "/", nil))

		if recorder.Code != http.StatusServiceUnavailable {
			t.Errorf("ISOLATE_MESSAGES=%q: status = %d, want 503", value, recorder.Code)
		}
	}
}

func TestEscapeHTML(t *testing.T) {
	cases := map[string]string{
		"":          "",
		"plain":     "plain",
		"&":         "&amp;",
		"<>":        "&lt;&gt;",
		`"'`:        "&quot;&#39;",
		"café — ok": "café — ok",
		"a&<b":      "a&amp;&lt;b",
	}

	for in, want := range cases {
		if got := escapeHTML(in); got != want {
			t.Errorf("escapeHTML(%q) = %q, want %q", in, got, want)
		}
	}
}

func TestRenderPageLeavesNoPlaceholder(t *testing.T) {
	body := renderPage(pageTemplate, "A message.")

	if strings.Contains(body, messagePlaceholder) {
		t.Errorf("rendered page still contains %q", messagePlaceholder)
	}
	if !strings.Contains(body, "<p>A message.</p>") {
		t.Error("rendered page does not contain the message")
	}
}
