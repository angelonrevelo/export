# Security audit & public-deployment roadmap for `export`

> Writing this so a security-minded friend can review the codebase before we
> consider making `export` accessible from the public internet (e.g. as
> `bygelo.com/export`). This doc tells you (a) where it stands today, (b) what
> moves are needed to ship safely, and (c) what we're explicitly *not* trying
> to defend against.

## TL;DR

`export` is a single-user personal tool in its current form. It's safe on
localhost, safe on a private LAN, and **not** safe on the open internet without
~90 minutes of additional hardening. Nothing about the architecture is broken —
the gaps are the deliberate kind you'd skip when building for "me, alone."
Closing them is straightforward.

## Threat model

| Tier | Adversary | Defended? |
|---|---|---|
| **T0** | Curious browser tab on the same machine | ✓ Yes |
| **T1** | Network observer on the same WiFi | Partial — needs HTTPS |
| **T2** | Random internet scanner hitting the password page | Partial — needs HTTPS + rate limit |
| **T3** | Trusted friend with the password who tries to abuse it | Partial — single password = no audit, no per-user limits |
| **T4** | Untrusted user with the password (leaked) | **Not defended** — they can DoS, SSRF internal endpoints, possibly crash the server with crafted inputs |
| **T5** | Determined attacker (zero-day in ffmpeg/Pillow/Pandoc) | **Out of scope** — would require sandboxing every converter, weeks of work |

Public deployment must defend at least through T2 honestly, T3 reasonably, T4
opportunistically. T5 is a research project we explicitly punt on.

## Current security posture (audit)

### Authentication & transport

| Item | State | Risk | Notes |
|---|---|---|---|
| Single shared password | ⚠️ | T3, T4 | No accounts, no audit trail, no revocation. Acceptable for T0-T2. |
| Password sent as `X-Password` header | ✓ | — | Header-based, not in URL for JSON endpoints. |
| **Password sent as `?pw=` query param for downloads** | ✓ | — | Replaced by a per-job download `token` on `/api/files`. `?pw=` returns 401. |
| Password stored in browser localStorage | ⚠️ | XSS | We render no user content as HTML, so XSS surface is currently nil. Future-proof would be httpOnly cookie. |
| HTTPS | ⚠️ | T1, T2 | In-repo: `scripts/nginx-export.conf` + `scripts/Caddyfile`. Live certbot/DNS is deploy-time. |
| CSRF tokens | ⚠️ | T2 | None. Mitigated by same-origin and X-Password header requirement, but multipart uploads from a malicious origin would still go through if password leaks. |

### Input handling & resource limits

| Item | State | Risk | Notes |
|---|---|---|---|
| Per-file upload cap | ⚠️ | T3, T4 | **Default: unlimited** (correct for personal use). Set `EXPORT_MAX_UPLOAD_MB=N` to enforce. **Required before any public exposure** — Phase 2 should ship a sane cap (e.g. 500 MB). |
| Multi-file total cap | ⚠️ | T3, T4 | Same — `EXPORT_MAX_COMPRESS_MB` defaults to unlimited; cap before exposing. |
| **No request rate limiting** | ✓ | — | 30 mutating POSTs / minute / IP (`EXPORT_RATE_LIMIT_PER_MINUTE`). 429 + one-line detail. |
| No per-user job concurrency limit | ⚠️ | T3 | Whisper is single-model; concurrent transcribes serialize but other ops compete for CPU. |
| **SSRF via URL extraction** | ✓ | — | `refuse_ssrf` before yt-dlp: RFC1918, `169.254/16`, `127/8`, IPv6 unique-local → 400. |
| **No image-bomb protection** | ✓ | — | `Image.MAX_IMAGE_PIXELS = 64_000_000` (~8K×8K). |
| File extension trusted from upload | ⚠️ | T4 | We dispatch by suffix. A `.pdf` that's actually JS bytecode just fails the relevant parser — but the parsers themselves have CVEs (Ghostscript especially). |

### Subprocess & code execution

| Item | State | Risk | Notes |
|---|---|---|---|
| All subprocess calls use list-args (no `shell=True`) | ✓ | — | No shell injection from filenames or env vars. |
| `EXPORT_PIPER_VOICE` path goes to subprocess | ⚠️ | T3 | If an admin sets it to a malicious path, piper would crash but not RCE. Low risk. |
| No sandboxing of converters | ⚠️ | T5 | A weaponized PDF could exploit a Ghostscript CVE → arbitrary file read at minimum. Out of scope unless we go production. |
| Job dirs with UUIDs | ✓ | — | No cross-job pollution. Path traversal mitigated by the dict-gated lookup. |

### Operational

| Item | State | Risk | Notes |
|---|---|---|---|
| 6-hour TTL cleanup loop (configurable) | ✓ | — | Disk doesn't accumulate. Tunable via `EXPORT_CLEANUP_AGE_SECONDS`. |
| Stream-and-delete download mode | ⚙️ opt-in | — | `EXPORT_DOWNLOAD_AUTODELETE=1` gives true ephemeral storage (file deleted instant download finishes). Recommended for any public exposure. |
| Lifespan-managed background tasks | ✓ | — | Clean shutdown on SIGTERM. |
| Single-process FastAPI | ⚠️ | T3 | One CPU-bound job (Whisper, demucs) can stall API responses. |
| No structured logging | ⚠️ | — | uvicorn logs only. No audit trail of who did what when. |
| No metrics / health endpoint | ⚠️ | — | `/api/capabilities` works as a poor-man's liveness check. |
| Optional Groq STT backend (cloud) | ⚙️ opt-in | — | When `EXPORT_GROQ_API_KEY` is set, audio for transcription is uploaded to api.groq.com. Per Groq's TOS, audio is not retained for training. Breaks the "data never leaves your machine" promise — only set the env var if you accept that. Without the env var, all STT runs locally. |

## Deployment scope phases

We define four scopes. Each phase has a hard checklist of fixes that gate
moving up. **Don't move up a phase until every item in the checklist below it
is actually done.** Each box is unchecked here as a working list.

### Phase 1: personal use (current state) ✓

Where we are. Run on localhost, your laptop, or a VPN-only host. Nothing more
needed. Ship.

### Phase 2: small trusted group (≤5 people who know each other)

Minimum to be defensible on the open internet for a few friends. Roughly 1
afternoon of work.

- [x] **HTTPS terminating reverse proxy** — in-repo `scripts/nginx-export.conf`
      and `scripts/Caddyfile`. Certbot / live DNS is deploy-time (not claimed).
- [x] **Move download auth from `?pw=` query param to short-lived
      tokens** — job `token` minted at ready; `/api/files/{job_id}?token=…`.
      `?pw=` is 401. Token lives with the job TTL (shrink-more can re-fetch).
- [x] **Rate limiting** — in-process 30 mutating POSTs / minute / IP
      (`EXPORT_RATE_LIMIT_PER_MINUTE`). 429 with a one-line body.
- [x] **SSRF guard in URL extractor** — `refuse_ssrf` before yt-dlp/gallery-dl:
      RFC1918, `169.254/16`, `127/8`, IPv6 unique-local → 400.
- [x] **Image-bomb defense** — `PIL.Image.MAX_IMAGE_PIXELS = 64_000_000`.
- [x] **Stronger password** — documented: public 32+ chars
      (`openssl rand -base64 24`); localhost may stay `1234`.
- [x] **Set upload caps** — documented public values `EXPORT_MAX_UPLOAD_MB=500`
      and `EXPORT_MAX_COMPRESS_MB=2000`. Localhost default remains unlimited.
- [x] **Enable stream-and-delete** — documented `EXPORT_DOWNLOAD_AUTODELETE=1`
      for public; localhost opt-in.
- [x] **Shorten cleanup TTL** — documented public `EXPORT_CLEANUP_AGE_SECONDS=900`
      + `EXPORT_CLEANUP_INTERVAL_SECONDS=60`. Localhost default 6 h / 15 min.
- [x] **Mount /tmp/export as tmpfs** (Linux only) — notes in
      `scripts/install-linux.sh`. Live mount is deploy-time.
- [x] **Document the threat model clearly to the trusted users** — this file.

After Phase 2 everything in the audit table above marked 🔴 is resolved.

### Phase 3: public single-tenant (anyone can sign up, you operate it alone)

This is "real product" territory. ~1-2 weeks of work, much of it operational.

- [ ] **Real auth**: per-user accounts. Magic-link email, OAuth (Google/GitHub),
      or username+passkey. The single shared password goes away.
- [ ] **Per-user quotas**: jobs/day, total storage MB, max concurrent CPU
      seconds. Surface limits in the UI, return 402/429 with a clear message
      when hit.
- [ ] **Move job state to Redis** + `arq` worker pool — research recommended
      this in the first audit. Necessary the moment you have >1 active user.
- [ ] **Move uploads to streaming + background** — current code reads
      `await file.read()` chunks but holds the whole file in tmp. Fine for
      one user, fragile under concurrency. Stream straight to S3 / local
      object storage with `aiofiles` + a presigned URL pattern.
- [ ] **Audit log** — every job, who, what URL/file, what operation, time,
      result. Append-only file or a `jobs` table in Postgres. Required for
      abuse investigation.
- [ ] **Per-IP and per-account rate limits, separately** — at the nginx layer
      *and* in the app (defense in depth).
- [ ] **Sandboxed converters** — at minimum, run ffmpeg, Ghostscript,
      Pandoc, LibreOffice in a `firejail` / `bubblewrap` profile so a CVE
      can't escape into the rest of the system. Or lift each into its own
      container per-job (heavier).
- [ ] **Antivirus scan on uploaded files** — ClamAV daemon, scan in the
      background task before any further processing. Reject infected uploads
      with a clear error.
- [ ] **DMCA takedown contact and process** — if URL extraction is part of
      the offering, you become liable for what users extract. Need a working
      `dmca@bygelo.com` and a documented response procedure.
- [ ] **Monitoring**: Prometheus metrics, log aggregation, alerts on:
      sustained high CPU, error spikes, disk filling, Redis queue length.
- [ ] **Daily backups of the user/auth database**.
- [ ] **Incident response runbook**: documented "if Y happens, do X."

### Phase 4: public multi-tenant at scale (paid product)

Out of scope for this doc. Adds: payments, refunds, GDPR/CCPA compliance,
SOC2 (for B2B customers), 24/7 oncall, multi-region failover, formal pen
test, bug bounty program, etc.

## Architectural considerations beyond security

Things to be aware of when planning:

- **Single Whisper model = serial transcription.** Two users transcribing a
  podcast at the same time will queue. Multi-process workers are required at
  Phase 3.
- **LibreOffice memory leak** — known issue (`unoserver` daemon mitigates by
  recycling). Necessary at Phase 3.
- **Disk pressure**: `/tmp/export/` grows mid-day. The 1-hour TTL keeps it
  manageable for personal use; under load, you want a real disk monitoring
  alert.
- **Container build time**: a Dockerfile that includes ffmpeg + LibreOffice +
  Pandoc + Ghostscript + Tesseract + Pango/Cairo + Whisper models is going
  to be ~3-4 GB. Plan for a CI/registry strategy. Slim runtime images
  (build-stage + runtime-stage split) cut this significantly.
- **License hygiene**: see `README.md#license`. AGPL (Ghostscript) means if
  you ever ship a hosted SaaS, you must offer source. CC-BY-NC weights
  (BiRefNet, Marker) cannot be used commercially without a separate license.

## Things we deliberately *don't* defend against

Being explicit so reviewers don't burn time on these:

- **Determined nation-state attacker** — out of scope.
- **Malicious users with valid accounts attacking each other** — currently
  there's only one user; at Phase 3 we'd need data isolation, but the threat
  model is "individuals who got bored," not "competitors."
- **Side-channel attacks on the host** (timing, power) — not a meaningful
  threat for a tiny converter service.
- **Supply chain attacks on our dependencies** — we trust the major libs
  (yt-dlp, Pillow, ffmpeg, etc.). A long-term pinned-dependency / SBOM
  strategy belongs to Phase 4.
- **DDoS at the network layer** — sit behind Cloudflare or similar.

## Verification checklist before going public

If your friend agrees the audit is complete and Phase 2 is done, do this dry
run before flipping DNS:

- [ ] Run [SecurityHeaders.com](https://securityheaders.com) on the deployed
      URL. Aim for at least B grade. Add CSP, HSTS, X-Content-Type-Options,
      Referrer-Policy in nginx.
- [ ] Run [Mozilla Observatory](https://observatory.mozilla.org/). Same.
- [ ] `nikto` or `nuclei` baseline scan against the URL. Expect noise but no
      genuine findings.
- [ ] Manually try: an SSRF (`http://169.254.169.254/`), an oversized upload
      (200 MB file), 1000 requests in 10 seconds (rate limit), a malformed
      PDF (zip bomb in pdf wrapper).
- [ ] Verify `EXPORT_PASSWORD` is not in any logs (uvicorn, nginx, journald).
- [ ] Check Caddy/nginx access logs for `?pw=` strings — should be **zero**
      after the signed-token migration.

## Final word for the friend

The threat model is "self + small trusted circle on the open internet."
Phase 2 app items (token, rate limit, SSRF, image-bomb) are in; HTTPS is
the reverse-proxy deploy. The Phase 3 list is not a hint
that we should go there — it's a list to read *before deciding whether* to.
The current architecture is built for personal use; promoting it to a public
product is a real shift in scope, not a checkbox.

If a fix in this list looks wrong or insufficient to you, that's exactly
what we want to know. Open an issue with what you'd change.
