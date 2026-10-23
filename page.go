package main

import "strings"

const (
	messagePlaceholder         = "{{MESSAGE}}"
	siteNamePlaceholder        = "{{SITE_NAME}}"
	primaryColorPlaceholder    = "{{PRIMARY_COLOR}}"
	backgroundColorPlaceholder = "{{BACKGROUND_COLOR}}"
	textColorPlaceholder       = "{{TEXT_COLOR}}"
	logoPlaceholder            = "{{LOGO}}"
	logoMarkup                 = `<img class="logo" src="/__isolate/logo" alt="">`
)

func escapeHTML(value string) string {
	var builder strings.Builder
	builder.Grow(len(value))

	for _, character := range value {
		switch character {
		case '&':
			builder.WriteString("&amp;")
		case '<':
			builder.WriteString("&lt;")
		case '>':
			builder.WriteString("&gt;")
		case '"':
			builder.WriteString("&quot;")
		case '\'':
			builder.WriteString("&#39;")
		default:
			builder.WriteRune(character)
		}
	}

	return builder.String()
}

func renderParagraphs(message string) string {
	message = strings.ReplaceAll(message, "\r\n", "\n")
	message = strings.ReplaceAll(message, "\r", "\n")

	var paragraphs []string
	var lines []string
	flush := func() {
		if len(lines) == 0 {
			return
		}
		paragraphs = append(paragraphs, "<p>"+escapeHTML(strings.Join(lines, "\n"))+"</p>")
		lines = nil
	}

	for _, line := range strings.Split(message, "\n") {
		if strings.TrimSpace(line) == "" {
			flush()
			continue
		}
		lines = append(lines, line)
	}
	flush()

	return strings.Join(paragraphs, "\n")
}

func renderPage(template string, content pageContent) string {
	logo := ""
	if content.Logo != nil {
		logo = logoMarkup
	}

	return strings.NewReplacer(
		siteNamePlaceholder, escapeHTML(content.SiteName),
		primaryColorPlaceholder, content.PrimaryColor,
		backgroundColorPlaceholder, content.BackgroundColor,
		textColorPlaceholder, content.TextColor,
		logoPlaceholder, logo,
		messagePlaceholder, renderParagraphs(content.Message),
	).Replace(template)
}
