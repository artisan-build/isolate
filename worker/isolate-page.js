/**
 * isolate-page -- the Cloudflare Worker arm of Isolated Mode.
 *
 * Answers every request with the same page, status and headers as the Go and
 * Rust isolate binaries, so that a timeline taken against this Worker is
 * comparable with one taken against either of them. The page below is a copy
 * of ../templates/isolated.html; the test pins the two together.
 */

const TEMPLATE = `<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="robots" content="noindex, nofollow">
<title>{{SITE_NAME}}</title>
<style>
:root { color-scheme: light; }
* { box-sizing: border-box; }
body {
  margin: 0;
  min-height: 100vh;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 24px;
  background: {{BACKGROUND_COLOR}};
  color: {{TEXT_COLOR}};
  font: 16px/1.6 ui-sans-serif, system-ui, -apple-system, "Segoe UI", Roboto, Helvetica, Arial, sans-serif;
}
main {
  width: 100%;
  max-width: 34rem;
  background: #ffffff;
  border: 1px solid #e3e1dd;
  border-radius: 12px;
  padding: 32px;
}
h1 {
  margin: 0 0 12px;
  font-size: 1.375rem;
  font-weight: 600;
  letter-spacing: -0.01em;
  color: {{PRIMARY_COLOR}};
}
.logo { display: block; max-width: 100%; max-height: 72px; margin: 0 0 20px; }
p { margin: 0; }
p + p { margin-top: 12px; }
.muted { color: #6b665f; font-size: 0.875rem; }
</style>
</head>
<body>
<main data-testid="isolate-page">
<header data-testid="isolate-brand">
{{LOGO}}
<h1>{{SITE_NAME}}</h1>
</header>
<section data-testid="isolate-message">
{{MESSAGE}}
</section>
<footer data-testid="isolate-footer">
<p class="muted">Please try again later.</p>
</footer>
</main>
</body>
</html>
`;

const MESSAGE = 'This site is temporarily unavailable. It will be back shortly.';

/** Escapes the five characters that matter in element text, exactly as page.go
 * and src/page.rs do, so the three runtimes emit the same bytes. */
export function escapeHtml(value) {
  let out = '';

  for (const character of value) {
    switch (character) {
      case '&':
        out += '&amp;';
        break;
      case '<':
        out += '&lt;';
        break;
      case '>':
        out += '&gt;';
        break;
      case '"':
        out += '&quot;';
        break;
      case "'":
        out += '&#39;';
        break;
      default:
        out += character;
    }
  }

  return out;
}

function renderParagraphs(message) {
  const paragraphs = [];
  let lines = [];

  const flush = () => {
    if (lines.length === 0) return;
    paragraphs.push(`<p>${escapeHtml(lines.join('\n'))}</p>`);
    lines = [];
  };

  for (const line of message.replaceAll('\r\n', '\n').replaceAll('\r', '\n').split('\n')) {
    if (line.trim() === '') {
      flush();
    } else {
      lines.push(line);
    }
  }
  flush();

  return paragraphs.join('\n');
}

/** Renders the shared template with the built-in branding. */
export function renderPage(message) {
  return TEMPLATE
    .replaceAll('{{SITE_NAME}}', 'Temporarily unavailable')
    .replaceAll('{{PRIMARY_COLOR}}', '#1c1b19')
    .replaceAll('{{BACKGROUND_COLOR}}', '#f6f6f5')
    .replaceAll('{{TEXT_COLOR}}', '#1c1b19')
    .replaceAll('{{LOGO}}', '')
    .replaceAll('{{MESSAGE}}', renderParagraphs(message));
}

export default {
  fetch() {
    return new Response(renderPage(MESSAGE), {
      status: 503,
      headers: {
        'Content-Type': 'text/html; charset=utf-8',
        'Cache-Control': 'no-store, no-transform',
        'Retry-After': '3600',
        'X-Robots-Tag': 'noindex',
      },
    });
  },
};
