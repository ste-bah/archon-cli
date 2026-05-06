# VLM Image Descriptions

Archon can enrich ingested images with a vision-language model (VLM) description. OCR still runs first through the existing local OCR path; VLM is additive. When enabled, Archon stores the image description in `doc_image_descriptions`, chunks it into normal `doc_chunks`, embeds those text chunks with the existing fastembed pipeline, and links the chunks back to the source image through document provenance.

## Providers

| Provider | Default model | Cost | Privacy | Notes |
| --- | --- | --- | --- | --- |
| Ollama | `gemma4:e4b` | $0 | Local | Default local path. Install Ollama and pull the configured model first. |
| Gemini | `gemini-3-flash-preview` | Free-tier friendly | Sent to Google | Uses `GOOGLE_API_KEY` or `archon auth login --provider google`. Rate limited to 15 RPM by default. |
| Anthropic | `claude-sonnet-4-6` | Paid | Sent to Anthropic | Reuses existing Anthropic API key/OAuth spoofing. No separate VLM login. |

## Enable Local VLM

```toml
[policy.workers]
vlm = "allow-local"

[policy.docs.vlm]
enabled = true
mode = "local"
provider = "ollama"

[policy.docs.vlm.ollama]
endpoint = "http://localhost:11434"
model = "gemma4:e4b"
timeout_secs = 120
```

Then run:

```bash
ollama pull gemma4:e4b
archon docs model-status
archon docs ingest ./charts
archon docs search "ascending triangle pattern" --mode hybrid
```

If Ollama is unavailable, ingest still succeeds and prints a warning; image descriptions are skipped rather than failing the document ingest.

## Enable Gemini

```bash
archon auth login --provider google
```

Or export:

```bash
export GOOGLE_API_KEY="..."
```

Policy must allow cloud VLM in both places:

```toml
[policy.network]
allow_cloud_vlm = true

[policy.workers]
vlm = "allow-cloud"

[policy.docs.vlm]
enabled = true
mode = "cloud"
provider = "gemini"
allow_cloud = true

[policy.docs.vlm.gemini]
api_key_env = "GOOGLE_API_KEY"
model = "gemini-3-flash-preview"
endpoint_base = "https://generativelanguage.googleapis.com/v1beta"
rpm_limit = 15
```

## Enable Anthropic

Anthropic VLM uses the same auth path as the main agent. Either run `archon auth login --provider anthropic`, set `ANTHROPIC_API_KEY`, or use an Anthropic OAuth token with the existing spoof identity support.

```toml
[policy.network]
allow_cloud_vlm = true

[policy.workers]
vlm = "allow-cloud"

[policy.docs.vlm]
enabled = true
mode = "cloud"
provider = "anthropic"
allow_cloud = true

[policy.docs.vlm.anthropic]
model = "claude-sonnet-4-6"
```

## Diagnostics

```bash
archon docs model-status
archon providers doctor --live
archon auth status
```

Expected success output includes the configured VLM provider/model and whether its health check is ready. `archon docs model-status` also reports embedding backend status, pending chunks, and HNSW availability.

## PDF Image Extraction

As of v0.1.47, PDF ingest sends images through the same VLM path as standalone
PNG/JPEG/TIFF files. Each PDF gets three passes:

| Pass | Tool | What lands in search |
| --- | --- | --- |
| Text layer | `pdftotext -layout` | Normal body-text chunks with page provenance |
| Embedded images | `pdfimages -list` + `pdfimages -png` | OCR chunks plus optional VLM description chunks for charts, diagrams, figures, and photos |
| Scanned fallback | `pdftoppm -png` | Rendered page OCR plus optional VLM descriptions when no text or embedded image survived filtering |

The PDF-specific policy is:

```toml
[policy.docs.pdf]
extract_embedded_images = true
min_image_dimension = 200
min_image_bytes = 4096
vlm_per_page_image = true
render_text_pdf_pages = false
```

`extract_embedded_images = false` reverts PDFs to text-layer/scanned fallback
behaviour. `vlm_per_page_image = true` still respects `[policy.docs.vlm]`;
it does not bypass cloud gates. `render_text_pdf_pages = false` avoids costly
duplicate processing for native-text PDFs. Turn it on for bad OCR overlays
where you want every page visually re-described.

For long books, cloud VLM providers can create one VLM call per extracted chart
or rendered page. Local Ollama keeps that private and free; Gemini/Anthropic
require the cloud policy gates and should be used deliberately on large corpora.

After ingest, verify the source of truth:

```bash
archon docs status
archon docs inspect <document-id>
archon docs search "visual concept from a chart" --mode hybrid --debug
```

`inspect` reports embedded images extracted/skipped, image OCR runs/failures,
VLM descriptions/failures, and rendered fallback pages. Description rows live
in `doc_image_descriptions`; their generated chunks live in `doc_chunks` and
provenance links them back to the source PDF page.

## Example Workflow

```bash
archon docs ingest ~/research/charts
archon docs search "chart with rising volume breakout" --mode hybrid
archon gametheory run "Assess the incentive structure shown by the market-maker behaviour in the chart pack" --kb charts
archon-research "Compare the visually detected chart patterns against the written trading notes"
```

Because VLM descriptions become normal chunks, downstream document search, research, and game-theory pipelines can ground on visual content even when the exact words do not appear in OCR.
