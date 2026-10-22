# Inference server setup

AI Town speaks one protocol: non-streaming OpenAI-compatible Chat Completions.
Run an inference server or API translation proxy of your choice, then point the
backend at it. That server owns provider selection, upstream credentials,
provider-specific options, retries, and model routing. The frontend never needs
model credentials.

For example, [LiteLLM Proxy](https://docs.litellm.ai/docs/proxy/docker_quick_start)
can expose an OpenAI-compatible endpoint for upstream APIs. Follow its setup
instructions, configure your chosen provider there, and expose a model alias such
as `town-model`. Any server implementing the contract below can be used;
AI Town has no proxy package or provider SDK dependency.

## Backend configuration

Set these variables in the shell running `cargo run -p ai-town-server`:

```bash
export LLM_BASE_URL=http://127.0.0.1:4000/v1
export LLM_MODEL=town-model
# Optional token for authenticating to your inference server:
export LLM_API_KEY=your-server-access-token
```

| Setting | Meaning | Default |
| --- | --- | --- |
| `LLM_BASE_URL` | Inference server API root, including any prefix such as `/v1` | Unset |
| `LLM_MODEL` | Model name or routing alias exposed by that server | Unset |
| `LLM_API_KEY` | Optional bearer token for that server | No Authorization header |
| `LLM_MAX_TOKENS` | Positive integer completion budget, shared by dialogue and memory | `1024` |

Use the API root, rather than the full `/chat/completions` URL. Trailing slashes
are accepted. Both `LLM_BASE_URL` and `LLM_MODEL` are required to enable inference.
When all settings are absent or blank, the deterministic stub runs without network
requests. Partial or invalid configuration fails at startup. Settings are read
once, so restart the backend after changing them. The backend does not load `.env`
files automatically.

Local servers without authentication can leave `LLM_API_KEY` unset. Credentials
for NVIDIA, Gemini, or other upstream APIs belong in your inference server's
environment or configuration. AI Town never reads provider-specific variables.

## API contract

Both dialogue and memory send `POST <LLM_BASE_URL>/chat/completions` with JSON:

```json
{
  "model": "town-model",
  "messages": [
    {"role": "system", "content": "..."},
    {"role": "user", "content": "..."}
  ],
  "max_tokens": 1024,
  "temperature": 0.7,
  "stream": false
}
```

Dialogue uses temperature `0.7`; memory summaries use `0.2`. If configured,
authentication is `Authorization: Bearer <LLM_API_KEY>`. The server must return
a successful HTTP status and text at `choices[0].message.content`:

```json
{"choices": [{"message": {"content": "Hello, town!"}}]}
```

Models that require different parameters should have those translated or supplied
by the inference server. Increase `LLM_MAX_TOKENS` if a model uses its completion
budget for internal reasoning before producing text. Returned text is trimmed
and clipped to the kernel's byte limits at a UTF-8 boundary.

Requests have a 20-second timeout. HTTP errors, transport failures, malformed
responses, and empty text use the deterministic stub for that operation. There
are no application retries. Logs report status and timing without credentials
or raw request/response bodies.

## Verification

The normal test suite uses local mock servers and needs no upstream API keys:

```bash
cargo test -p ai-town-server
```

With your inference server running and `LLM_*` configured, explicitly run the live
dialogue and memory smoke test (this makes two inference requests):

```bash
cargo test -p ai-town-server live_inference_server -- --ignored --nocapture
```

Live checks on 2026-10-22 used a temporary local proxy that translated the
`town-model` alias and supplied upstream credentials. Provider variables were
removed from the Rust test process; it received only generic `LLM_*` settings.

| Upstream | Model | Observed result |
| --- | --- | --- |
| NVIDIA | `google/diffusiongemma-26b-a4b-it` | Dialogue and memory passed, HTTP 200 in 1.98 s and 2.26 s |
| Gemini | `gemini-flash-latest` | First dialogue succeeded, HTTP 200 in 6.61 s; memory returned 503; later attempts returned 503 or timed out |
| Gemini | `gemini-2.5-flash-lite` | Returned HTTP 404 despite appearing in the model list |
| Gemini | `gemini-3.8-flash` | Timed out at the temporary proxy's 19 s upstream deadline |
| Gemini | `gemini-3.5-flash-lite` | Timed out at the temporary proxy's 19 s upstream deadline |
| Gemini | `gemini-3.1-flash-lite` | Dialogue and memory passed, HTTP 200 in 7.34 s and 2.95 s |

The native Gemini `generateContent` endpoint also returned HTTP 503 with an
`UNAVAILABLE` error reporting high demand for `gemini-flash-latest`. Switching the
proxy's model mapping to `gemini-3.1-flash-lite` completed the full Gemini smoke
test, with no changes to AI Town's transport or generic connection settings.
Earlier failed operations correctly used the stub. These results describe the
observed requests, rather than a guarantee of future provider availability.
The proxy was used only for verification and is not a repository dependency.

## Migration from NVIDIA configuration

`NVIDIA_API_KEY`, `NVIDIA_MODEL`, and `NVIDIA_BASE_URL` no longer configure AI Town.
Move upstream NVIDIA settings to your inference server, expose the desired model
through its OpenAI-compatible endpoint, and set `LLM_BASE_URL` and `LLM_MODEL`
in the backend. Set `LLM_API_KEY` only if that server requires an access token.
There is no hardcoded provider endpoint or default model in AI Town.
