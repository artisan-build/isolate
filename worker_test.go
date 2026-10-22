package main

import (
	"os"
	"strings"
	"testing"
)

// The Cloudflare Worker in worker/ is the third arm of Isolated Mode and has to
// answer with the same page and the same headers as this binary, or a timeline
// taken against it is not comparable. The Worker carries its own copy of the
// template because a Worker is a single module with no filesystem; these tests
// are what keep that copy honest.

// workerTemplate returns the template literal the Worker renders.
func workerTemplate(t *testing.T) string {
	t.Helper()

	source, err := os.ReadFile("worker/isolate-page.js")
	if err != nil {
		t.Fatalf("read worker source: %v", err)
	}

	const marker = "const TEMPLATE = `"

	start := strings.Index(string(source), marker)
	if start < 0 {
		t.Fatalf("worker source has no %q", marker)
	}

	rest := string(source)[start+len(marker):]

	end := strings.Index(rest, "`;")
	if end < 0 {
		t.Fatal("worker source has an unterminated TEMPLATE literal")
	}

	return rest[:end]
}

func TestWorkerCarriesTheSharedTemplate(t *testing.T) {
	want, err := os.ReadFile("templates/isolated.html")
	if err != nil {
		t.Fatalf("read template: %v", err)
	}

	if got := workerTemplate(t); got != string(want) {
		t.Errorf("worker template differs from templates/isolated.html\nworker:\n%s\nfile:\n%s", got, want)
	}
}

func TestWorkerSendsTheSameResponseShape(t *testing.T) {
	source, err := os.ReadFile("worker/isolate-page.js")
	if err != nil {
		t.Fatalf("read worker source: %v", err)
	}

	for _, want := range []string{
		"status: 503",
		"'Content-Type': 'text/html; charset=utf-8'",
		"'Cache-Control': 'no-store, no-transform'",
		"'Retry-After': '3600'",
		"'X-Robots-Tag': 'noindex'",
	} {
		if !strings.Contains(string(source), want) {
			t.Errorf("worker source is missing %q", want)
		}
	}
}
