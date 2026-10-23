package main

import (
	"context"
	_ "embed"
	"errors"
	"log"
	"net/http"
	"os"
	"os/signal"
	"syscall"
	"time"
)

const (
	bucketCacheTTL       = 10 * time.Second
	bucketRequestTimeout = 2 * time.Second
)

// pageTemplate is the shared isolation page, byte-identical to the copy on the
// rust branch. Golden fixtures pin complete rendered responses on each branch.
//
//go:embed templates/isolated.html
var pageTemplate string

func main() {
	log.SetFlags(0)

	store := newObjectStore(os.Getenv)
	resolver := newContentResolver(store, time.Now, bucketCacheTTL)

	port := os.Getenv("PORT")
	if port == "" {
		port = "3000"
	}

	// A bare ":port" listens on every IPv4 and IPv6 interface. Cloud's startup
	// probe arrives over IPv6, so never narrow this to tcp4 or loopback.
	serve(":"+port, handler(resolver, pageTemplate, bucketRequestTimeout))
}

// handler answers every request with the isolation page, except an exact GET
// or HEAD for a configured logo. The request body is never read.
func handler(resolver *contentResolver, template string, bucketTimeout time.Duration) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		ctx, cancel := context.WithTimeout(r.Context(), bucketTimeout)
		defer cancel()

		content := resolver.Resolve(ctx, r.Host)
		if isLogoRequest(r) && content.Logo != nil {
			header := w.Header()
			header.Set("Content-Type", content.Logo.MediaType)
			header.Set("Cache-Control", "no-store, no-transform")
			header.Set("X-Content-Type-Options", "nosniff")
			w.WriteHeader(http.StatusOK)
			if r.Method != http.MethodHead {
				_, _ = w.Write(content.Logo.Bytes)
			}
			return
		}

		body := renderPage(template, content)

		header := w.Header()
		header.Set("Content-Type", "text/html; charset=utf-8")
		header.Set("Cache-Control", "no-store, no-transform")
		header.Set("Retry-After", "3600")
		header.Set("X-Robots-Tag", "noindex")

		w.WriteHeader(http.StatusServiceUnavailable)

		_, _ = w.Write([]byte(body))
	})
}

func isLogoRequest(r *http.Request) bool {
	return (r.Method == http.MethodGet || r.Method == http.MethodHead) &&
		r.URL.Path == "/__isolate/logo" && r.URL.RawQuery == ""
}

// serve runs an HTTP server on addr until SIGTERM or SIGINT, then drains the
// requests already in flight before returning.
func serve(addr string, h http.Handler) {
	ctx, stop := signal.NotifyContext(context.Background(), syscall.SIGTERM, os.Interrupt)
	defer stop()

	srv := &http.Server{
		Addr:              addr,
		Handler:           h,
		ReadHeaderTimeout: 10 * time.Second,
	}

	go func() {
		log.Printf("isolate: listening on %s", addr)

		if err := srv.ListenAndServe(); err != nil && !errors.Is(err, http.ErrServerClosed) {
			log.Fatalf("isolate: listen: %v", err)
		}
	}()

	<-ctx.Done()
	log.Print("isolate: received shutdown signal")

	shutdownCtx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()

	if err := srv.Shutdown(shutdownCtx); err != nil {
		log.Printf("isolate: shutdown: %v", err)
	}
}
