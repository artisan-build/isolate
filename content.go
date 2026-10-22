package main

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"net"
	"regexp"
	"strings"
	"sync"
	"time"
	"unicode/utf8"
)

const (
	builtinDefaultMessage    = "This site is temporarily unavailable. It will be back shortly."
	builtinSiteName          = "Temporarily unavailable"
	builtinPrimaryColor      = "#1c1b19"
	builtinBackgroundColor   = "#f6f6f5"
	builtinTextColor         = "#1c1b19"
	maximumMessageCharacters = 2000
)

var (
	errObjectNotFound = errors.New("object not found")
	colorPattern      = regexp.MustCompile(`^#[0-9a-fA-F]{6}$`)
	logoPattern       = regexp.MustCompile(`^logo\.(png|webp|jpg)$`)
)

type objectStore interface {
	Get(context.Context, string) ([]byte, error)
}

type cacheEntry struct {
	fetchedAt   time.Time
	value       []byte
	missing     bool
	lastGood    []byte
	hasLastGood bool
}

// cachedObjectStore keeps both found and missing objects fresh for the same
// short window. Transport failures do not replace the last successful read.
type cachedObjectStore struct {
	source  objectStore
	now     func() time.Time
	ttl     time.Duration
	mu      sync.Mutex
	entries map[string]cacheEntry
}

func newCachedObjectStore(source objectStore, now func() time.Time, ttl time.Duration) *cachedObjectStore {
	return &cachedObjectStore{
		source:  source,
		now:     now,
		ttl:     ttl,
		entries: make(map[string]cacheEntry),
	}
}

func (s *cachedObjectStore) Get(ctx context.Context, key string) ([]byte, error) {
	now := s.now()

	s.mu.Lock()
	entry, ok := s.entries[key]
	if ok && now.Sub(entry.fetchedAt) < s.ttl {
		s.mu.Unlock()
		if entry.missing {
			return nil, errObjectNotFound
		}

		return append([]byte(nil), entry.value...), nil
	}
	s.mu.Unlock()

	value, err := s.source.Get(ctx, key)

	s.mu.Lock()
	defer s.mu.Unlock()
	current := s.entries[key]

	switch {
	case err == nil:
		copied := append([]byte(nil), value...)
		s.entries[key] = cacheEntry{
			fetchedAt:   now,
			value:       copied,
			lastGood:    append([]byte(nil), copied...),
			hasLastGood: true,
		}

		return append([]byte(nil), copied...), nil
	case errors.Is(err, errObjectNotFound):
		current.fetchedAt = now
		current.value = nil
		current.missing = true
		s.entries[key] = current

		return nil, errObjectNotFound
	case current.hasLastGood:
		return append([]byte(nil), current.lastGood...), nil
	default:
		return nil, err
	}
}

type brandingFile struct {
	SiteName        *string `json:"site_name"`
	PrimaryColor    *string `json:"primary_color"`
	BackgroundColor *string `json:"background_color"`
	TextColor       *string `json:"text_color"`
	Logo            *string `json:"logo"`
}

type resolvedLogo struct {
	Bytes     []byte
	MediaType string
}

type pageContent struct {
	Message         string
	SiteName        string
	PrimaryColor    string
	BackgroundColor string
	TextColor       string
	Logo            *resolvedLogo
}

type contentResolver struct {
	objects *cachedObjectStore
}

func newContentResolver(source objectStore, now func() time.Time, ttl time.Duration) *contentResolver {
	return &contentResolver{objects: newCachedObjectStore(source, now, ttl)}
}

func (r *contentResolver) Resolve(ctx context.Context, requestHost string) pageContent {
	host := normalizeHost(requestHost)
	sitePrefix := ""
	if host != "" {
		sitePrefix = "sites/" + host + "/"
	}

	siteBranding := r.branding(ctx, sitePrefix)
	defaultBranding := r.branding(ctx, "default/")

	content := pageContent{
		Message:         r.message(ctx, sitePrefix),
		SiteName:        firstString(siteBranding.SiteName, defaultBranding.SiteName, builtinSiteName),
		PrimaryColor:    firstColor(siteBranding.PrimaryColor, defaultBranding.PrimaryColor, builtinPrimaryColor),
		BackgroundColor: firstColor(siteBranding.BackgroundColor, defaultBranding.BackgroundColor, builtinBackgroundColor),
		TextColor:       firstColor(siteBranding.TextColor, defaultBranding.TextColor, builtinTextColor),
	}

	content.Logo = r.logo(ctx, sitePrefix, siteBranding.Logo, defaultBranding.Logo)

	return content
}

func (r *contentResolver) message(ctx context.Context, sitePrefix string) string {
	if sitePrefix != "" {
		if value, ok := r.validMessage(ctx, sitePrefix+"message.txt"); ok {
			return value
		}
	}

	if value, ok := r.validMessage(ctx, "default/message.txt"); ok {
		return value
	}

	return builtinDefaultMessage
}

func (r *contentResolver) validMessage(ctx context.Context, key string) (string, bool) {
	value, err := r.objects.Get(ctx, key)
	if err != nil || !utf8.Valid(value) {
		return "", false
	}

	message := string(value)
	if utf8.RuneCountInString(message) > maximumMessageCharacters {
		return "", false
	}

	return message, true
}

func (r *contentResolver) branding(ctx context.Context, prefix string) brandingFile {
	if prefix == "" {
		return brandingFile{}
	}

	value, err := r.objects.Get(ctx, prefix+"branding.json")
	if err != nil || !utf8.Valid(value) {
		return brandingFile{}
	}

	var branding brandingFile
	if err := json.Unmarshal(value, &branding); err != nil {
		return brandingFile{}
	}

	return branding
}

func (r *contentResolver) logo(
	ctx context.Context,
	sitePrefix string,
	siteName *string,
	defaultName *string,
) *resolvedLogo {
	candidates := []struct {
		prefix string
		name   *string
	}{
		{prefix: sitePrefix, name: siteName},
		{prefix: "default/", name: defaultName},
	}

	for _, candidate := range candidates {
		if candidate.prefix == "" || candidate.name == nil || !logoPattern.MatchString(*candidate.name) {
			continue
		}

		value, err := r.objects.Get(ctx, candidate.prefix+*candidate.name)
		if err != nil {
			continue
		}

		return &resolvedLogo{
			Bytes:     value,
			MediaType: logoMediaType(*candidate.name),
		}
	}

	return nil
}

func firstString(site, fallback *string, builtin string) string {
	if site != nil {
		return *site
	}
	if fallback != nil {
		return *fallback
	}

	return builtin
}

func firstColor(site, fallback *string, builtin string) string {
	if site != nil && colorPattern.MatchString(*site) {
		return *site
	}
	if fallback != nil && colorPattern.MatchString(*fallback) {
		return *fallback
	}

	return builtin
}

func logoMediaType(name string) string {
	switch {
	case strings.HasSuffix(name, ".png"):
		return "image/png"
	case strings.HasSuffix(name, ".webp"):
		return "image/webp"
	case strings.HasSuffix(name, ".jpg"):
		return "image/jpeg"
	default:
		panic(fmt.Sprintf("validated logo has unsupported extension: %q", name))
	}
}

// normalizeHost creates the bucket key host: lower-case, without a port,
// trailing root dot, or leading www label.
func normalizeHost(host string) string {
	host = strings.ToLower(strings.TrimSpace(host))

	if strings.HasPrefix(host, "[") {
		if end := strings.Index(host, "]"); end >= 0 {
			host = host[1:end]
		}
	} else if parsedHost, _, err := net.SplitHostPort(host); err == nil {
		host = parsedHost
	} else if strings.Count(host, ":") == 1 {
		host, _, _ = strings.Cut(host, ":")
	}

	host = strings.TrimRight(host, ".")
	host = strings.TrimPrefix(host, "www.")

	return host
}
