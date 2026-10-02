# tc chat

A streaming chat app built with [Topcoat](https://github.com/tokio-rs/topcoat) 0.9. A page keeps its transcript in memory until refresh. Replies are pushed over the runtime WebSocket (`live!` + `connected`) and rendered as text, not HTML.

Two model backends sit behind the same chat:

| `PROVIDER` | Backend | What it does |
| --- | --- | --- |
| `inprocess` | [mistral.rs](https://github.com/EricLBuehler/mistral.rs) | Loads a model in this process and streams tokens from it. |
| `ollama` | [rig](https://github.com/0xPlaygrounds/rig) | Talks to an Ollama daemon. Other rig providers can be added beside it. |

## Run

```sh
cp .env.example .env
# edit PROVIDER, MODEL, and PROVIDER_BASE
topcoat dev
```

The server listens on `127.0.0.1:3000` unless `HOST` or `PORT` is set. It loads the asset bundle already next to the binary. `topcoat dev` writes that bundle on each rebuild, and a Docker build should do the same before the process starts.

Ollama, with a model you have already pulled:

```sh
PROVIDER=ollama MODEL=llama3.2 topcoat dev
```

In-process, from a Hugging Face id or a local `.gguf` file. The first run downloads weights. On Apple Silicon, build with `--features metal` and keep the asset bundle from that same build next to the binary:

```sh
PROVIDER=inprocess MODEL=Qwen/Qwen3-4B ISQ=4 cargo run --features metal
```

`ISQ` quantizes a full model as it loads (`2`, `3`, `4`, `5`, `6`, or `8`). Leave it unset to use the published weights. A path ending in `.gguf`, or `MODEL` plus `MODEL_FILE`, uses the GGUF loader. Those files are already quantized, so leave `ISQ` unset for them.

## Configuration

| Variable | Required | Meaning |
| --- | --- | --- |
| `PROVIDER` | yes | `inprocess` or `ollama`. |
| `MODEL` | yes | Hugging Face id, local path, or `.gguf` file for in-process. Ollama model tag otherwise. |
| `PROVIDER_BASE` | no | Ollama URL. Defaults to `http://127.0.0.1:11434`. Ignored for in-process. |
| `API_KEY` | no | Bearer token for a proxied Ollama, or a Hugging Face token for a gated in-process model. |
| `MODEL_FILE` | no | GGUF filename when `MODEL` is a directory or repository. |
| `ISQ` | no | Bit width when loading a full in-process model. Leave unset for GGUF and Ollama. |
| `SYSTEM_PROMPT` | no | Instructions prepended to each request. |
| `TEMPERATURE` | no | `0` through `2`. Defaults to `0.7`. `0` is deterministic. |
| `MAX_TOKENS` | no | Cap on a single reply. |

`.env` is loaded when present and does not override variables already set in the environment.

## Layout

- `src/chat.rs` is the page, the live transcript, and the send procedure. Messages are rendered as Markdown.
- `src/markdown.rs` turns that Markdown into HTML and strips anything that is not content.
- `src/hub.rs` keeps each page's messages in memory and runs generation outside the live loop, so a reconnect does not start the model again.
- `src/provider` is the in-process and Ollama backends. Both yield a stream of text deltas.
