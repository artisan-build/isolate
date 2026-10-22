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

// pageTemplate is the shared isolation page, byte-identical to the copy on the
// rust branch. templateSHA256 in page_test.go pins it.
//
//go:embed templates/isolated.html
var pageTemplate string

func main() {
	log.SetFlags(0)

	cfg := loadConfig(os.Getenv("ISOLATE_MESSAGES"), os.Getenv("ISOLATE_DEFAULT_MESSAGE"))

	port := os.Getenv("PORT")
	if port == "" {
		port = "3000"
	}

	// A bare ":port" listens on every IPv4 and IPv6 interface. Cloud's startup
	// probe arrives over IPv6, so never narrow this to tcp4 or loopback.
	serve(":"+port, handler(cfg, pageTemplate))
}

// handler answers every request -- any method, path or query string -- with the
// isolation page for the requested host. The request body is never read.
func handler(cfg config, template string) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		body := renderPage(template, cfg.messageFor(r.Host))

		header := w.Header()
		header.Set("Content-Type", "text/html; charset=utf-8")
		header.Set("Cache-Control", "no-store")
		header.Set("Retry-After", "3600")
		header.Set("X-Robots-Tag", "noindex")

		w.WriteHeader(http.StatusServiceUnavailable)

		_, _ = w.Write([]byte(body))
	})
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
