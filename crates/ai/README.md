# pdfcraft-ai · API Translation Edition

User-selected PDF text or a bounded rendered crop is translated only after an explicit click.
This L4 crate has no UI or document mutator. The native transport supports user-supplied
OpenAI-compatible Chat Completions and Responses endpoints; provider presets are editable,
not a guarantee that each named model supports image input.

Image mode uses **two requests**: a vision model transcribes the crop, then the text model
translates the recognized text with only matched local glossary records. No complete PDF
file or unrelated glossary records are sent. The UI displays the exact crop, OCR source,
translation, and model warnings. Each request may incur fees.

Settings are under PdfCraftAPI, separately from upstream preferences. API keys are omitted from
JSON and are session-only by default; explicit persistence uses macOS Keychain / Windows
Credential Manager. No real credentials or personal glossary data belong in the repository.
Native request files live in a private temporary directory, redirects are refused, input and
response sizes are bounded, errors are sanitized, cancellation stops local waiting, and requests
are never automatically retried. Providers may still process requests after cancellation.

Tests use ephemeral loopback HTTP servers and synthetic text/images only. No live paid model
has been qualified. OCR accuracy, translation quality, and a provider's retention policies remain
the user's responsibility. Official OpenAI documentation was HTTP 403 from this environment
during development; mock compatibility tests are not live-provider certification.
