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
<title>Temporarily unavailable</title>
<style>
:root { color-scheme: light dark; }
* { box-sizing: border-box; }
body {
  margin: 0;
  min-height: 100vh;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 24px;
  background: #f6f6f5;
  color: #1c1b19;
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
}
p { margin: 0; }
p + p { margin-top: 12px; }
.muted { color: #6b665f; font-size: 0.875rem; }
@media (prefers-color-scheme: dark) {
  body { background: #17161a; color: #ecebe8; }
  main { background: #201f24; border-color: #343239; }
  .muted { color: #a19c95; }
}
</style>
</head>
<body>
<main>
<h1>Temporarily unavailable</h1>
<p>{{MESSAGE}}</p>
<p class="muted">Please try again later.</p>
</main>
</body>
</html>
`;

const MESSAGE = 'yakfeather.com is isolated (Worker).';

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

/** Renders the shared template with the message substituted in. */
export function renderPage(message) {
  return TEMPLATE.replaceAll('{{MESSAGE}}', escapeHtml(message));
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
