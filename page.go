package main

import (
	"encoding/json"
	"log"
	"strings"
)

// builtinDefaultMessage is the last fallback: used when neither
// ISOLATE_MESSAGES nor ISOLATE_DEFAULT_MESSAGE supplies a message.
const builtinDefaultMessage = "This site is temporarily unavailable. It will be back shortly."

// messagePlaceholder is the only substitution the shared template carries.
const messagePlaceholder = "{{MESSAGE}}"

// config is the boot-time configuration: a message per host plus a fallback.
type config struct {
	messages map[string]string
	fallback string
}

// loadConfig resolves the configuration from raw environment values. A
// malformed ISOLATE_MESSAGES value is logged and ignored, never fatal.
func loadConfig(messagesJSON, defaultMessage string) config {
	cfg := config{messages: map[string]string{}, fallback: builtinDefaultMessage}

	if trimmed := strings.TrimSpace(defaultMessage); trimmed != "" {
		cfg.fallback = trimmed
	}

	raw := strings.TrimSpace(messagesJSON)
	if raw == "" {
		return cfg
	}

	var parsed map[string]string
	if err := json.Unmarshal([]byte(raw), &parsed); err != nil {
		log.Printf("isolate: ignoring malformed ISOLATE_MESSAGES: %v", err)

		return cfg
	}

	for host, message := range parsed {
		cfg.messages[normalizeHost(host)] = message
	}

	return cfg
}

// messageFor picks the message for a Host header value.
func (c config) messageFor(host string) string {
	if message, ok := c.messages[normalizeHost(host)]; ok {
		return message
	}

	return c.fallback
}

// normalizeHost lowercases a host, drops any port and any trailing root dot so
// that "Example.COM:8080" and "example.com." share one key.
func normalizeHost(host string) string {
	host = strings.ToLower(strings.TrimSpace(host))

	if strings.HasPrefix(host, "[") {
		if end := strings.Index(host, "]"); end >= 0 {
			host = host[1:end]
		}
	} else if colon := strings.LastIndex(host, ":"); colon >= 0 {
		host = host[:colon]
	}

	return strings.TrimSuffix(host, ".")
}

// escapeHTML escapes the five characters that matter in element text. It is
// hand-rolled rather than taken from html/template so that the Go and Rust
// runtimes emit byte-identical bodies.
func escapeHTML(s string) string {
	var b strings.Builder
	b.Grow(len(s))

	for _, r := range s {
		switch r {
		case '&':
			b.WriteString("&amp;")
		case '<':
			b.WriteString("&lt;")
		case '>':
			b.WriteString("&gt;")
		case '"':
			b.WriteString("&quot;")
		case '\'':
			b.WriteString("&#39;")
		default:
			b.WriteRune(r)
		}
	}

	return b.String()
}

// renderPage substitutes the escaped message into the shared template.
func renderPage(template, message string) string {
	return strings.ReplaceAll(template, messagePlaceholder, escapeHTML(message))
}
