# Security policy

PixelGrab is a local-first Windows desktop capture utility. Its job is to
take a screenshot of what is visible on your desktop and let you annotate
it. Because the product touches the screen, it is critical that it be
trustworthy.

## Privacy by design

- All capture processing happens on the local machine. There is no
  network telemetry, no cloud upload, and no remote logging.
- The cache stores captures under the user's
  `%LOCALAPPDATA%\com.pixelgrab.app\` directory. Captures are pruned by
  age, entry count, and disk usage.
- PixelGrab never logs captured pixels, annotation text, clipboard
  contents, or paths outside the application cache root.
- The CI pipeline never runs a test that captures real desktop content.

## Supported versions

The latest minor release receives security fixes. Earlier minor releases
do not.

## Reporting a vulnerability

Please email security@pixelgrab.example.com rather than opening a public
issue. You should receive a response within two business days. Include:

- A description of the vulnerability.
- A reproducer (if available).
- The expected impact.

We follow responsible disclosure. We will not take legal action against
researchers who follow this policy.

## Hardening checklist

- The application sets `Per-Monitor V2` DPI awareness on Windows.
- The overlay window is `TopMost` and never accepts focus while the
  capture is in progress.
- Global shortcuts are validated and rolled back on registration failure.
- The single-instance plugin prevents a second primary process from
  competing for the tray.
- All structured errors redact file paths from outside the cache.
