# Isolate content contract (FROZEN v1, 2026-10-23)

The contract between the Isolate binaries (artisan-build/isolate, both runtimes) and the Scalpels side that writes content
(scalpels.app Isolated Mode tools). Both coordinators copy this file verbatim into their repo (isolate: `docs/content-contract.md`;
scalpels.app: next to the Isolated Mode code). A change is a new version, agreed through brain, never a silent edit.

## Bucket layout

One object-storage bucket attached to BOTH isolate environments as the default disk. Cloud injects the AWS_* variables. Keys:

```
default/message.txt        org-wide fallback message (UTF-8 plain text, ≤ 2,000 chars)
default/branding.json      org-wide fallback branding
default/logo.<png|webp|jpg>
sites/<host>/message.txt    per-site message
sites/<host>/branding.json  per-site branding
sites/<host>/logo.<ext>
```

`<host>` is the lower-cased apex host with any leading `www.` stripped, e.g. requests for `www.example.com` and `example.com` both
read `sites/example.com/`. No port, no trailing dot.

## branding.json (all fields optional)

```json
{ "site_name": "Example", "primary_color": "#1d4ed8", "background_color": "#ffffff", "text_color": "#0f172a", "logo": "logo.webp" }
```

- Colours must match `^#[0-9a-fA-F]{6}$`. Anything else is ignored, not an error.
- `logo` is a file name in the same directory, and must match `^logo\.(png|webp|jpg)$`. The binary serves it at `/__isolate/logo` for
  that host (the one path that isn't the page). Other paths and query strings get the page.
- Unknown keys are ignored.

## Resolution, per request

For each field: `sites/<host>/…`, then `default/…`, then the built-in value compiled into the binary. A missing or malformed file
falls through to the next level. A malformed file must NEVER take the page down or change the status code.

## Caching

Objects are cached in memory for at most 10 s (a positive or negative result). A bucket outage serves the last good value, or else the
built-in defaults. The page never waits more than 2 s on the bucket.

## Rendering

The message is HTML-escaped and rendered as paragraphs (blank line = new paragraph). There's no other markup. The response is unchanged
from today: 503, `Retry-After: 3600`, `Cache-Control: no-store, no-transform`, `X-Robots-Tag: noindex`, `Content-Type: text/html;
charset=utf-8`, every method and path except `/__isolate/logo`. Both runtimes produce byte-identical bodies for identical inputs. A
shared golden-file test fixture set lives in each branch, and they must be identical.

## Writers

Only Scalpels (or a customer's own agent, following the docs) writes these keys. The binaries only read.
