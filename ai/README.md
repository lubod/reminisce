# AI Inference Service (`ai/`)

## Purpose
Python sidecar providing machine learning inference for Reminisce: image embedding, natural language description generation, face detection/embedding, quality scoring, orientation detection, and photo enhancement.

## Service Architecture
The service runs **two servers** in one process, both sharing the models loaded at startup:
- **HTTP (Flask, `:8081`)** — `/health` and `/` only. Kept solely for container healthchecks; all inference is via gRPC.
- **gRPC (`:50051`)** — binary protobuf RPCs for the high-throughput inference calls (embed, describe, face detection, quality, enhance), replacing the old HTTP Base64 JSON path. Schema in `proto/ai_service.proto`; implemented in `ai_service_grpc.py` (imported and started in a background thread by `ai_service.py`).

```
Backend (Rust) ──HTTP POST / :8081 (X-API-Key)──▶ Flask (ai_service.py)     [health only]
             └─gRPC / :50051 (x-api-key metadata)─▶ gRPC (ai_service_grpc.py) [embed/describe/detect/quality/enhance/orientation]
                                                              │
            ┌──────────────────────────────────────────────┬──┴──────────────────┬─────────────────────────────┐
            ▼                                              ▼                     ▼                             ▼
     SigLIP2 Model                                 SmolVLM / Qwen           InsightFace                    BEiT-Base
(Image/Text Vector Embedding)               (Visual Description Gen)   (Face Detection & Re-ID)   (Rotation → EXIF Orientation)
```

## Security & Authentication
- All endpoints require authentication via **either** the `Authorization: Bearer <key>` header **or** the `X-API-Key` header (HTTP) / `x-api-key` metadata (gRPC). Requests with missing or non-matching keys return HTTP 401 / gRPC UNAUTHENTICATED.
- If `API_SECRET_KEY` (or `REMINISCE_API_SECRET_KEY`) is not set on the service, requests are **rejected** (fails closed — gRPC `UNAVAILABLE`, HTTP 500). Always set the master secret in both backend and AI service configs.

## Hardware & Acceleration Auto-Detection
The service automatically detects and configures available hardware accelerators upon startup:
1. NVIDIA GPU via PyTorch CUDA.
2. AMD GPU via ROCm (`hipblas_v2_shim.c`).
3. Apple Silicon GPU via Metal Performance Shaders (MPS).
4. CPU fallback with multi-threading.

## API Endpoint Inventory

### HTTP (Flask, `:8081`)
| Endpoint | Method | Purpose | Model / Engine |
|----------|--------|---------|----------------|
| `/health` | GET | Health & model-load status | - |
| `/` | GET | Service info / endpoint list | - |

HTTP is kept **only** for container healthchecks. All inference moved to gRPC.

### gRPC (`:50051`) — `proto/ai_service.proto`
| RPC | Purpose | Model / Engine |
|-----|---------|----------------|
| `EmbedImage` | Image vector embedding (1152-dim) | SigLIP2 |
| `EmbedText` | Text query vector embedding (1152-dim) | SigLIP2 |
| `DescribeImage` | Visual scene description (`use_qwen` selects model) | SmolVLM / Qwen2.5-VL |
| `DetectFaces` | Face detection, bounding box & embeddings | InsightFace |
| `QualityScore` | Aesthetic quality & sharpness scoring | SigLIP2 + OpenCV |
| `EnhanceImage` | Photo enhancement (exposure, denoise, restore, sharpen) | OpenCV |
| `DetectOrientation` | Photo rotation detection → EXIF orientation value | BEiT-Base classifier |
| `HealthCheck` | Model-load & device status | - |

**Orientation detection** uses a dedicated lightweight classifier (`amaye15/Beit-Base-Image-Orientation-Fixer`, Apache-2.0) instead of prompting a VLM — it is deterministic, ~2-3 orders of magnitude faster than Qwen2.5-VL generation, and maps directly to EXIF values 1/3/6/8. EXIF-first detection happens in Rust at ingest time (`services/ingest.rs`); this RPC is the **AI fallback** for images that carry no EXIF orientation (the Rust `ai_worker` calls it for `exif IS NULL AND orientation IS NULL` images and stores the result, which `media.rs` then injects at serve time).

## Key Files
- [ai_service.py](file:///Users/ldr/work/reminisce/ai/ai_service.py): Model loading, HTTP `/health` + `/`, and gRPC server startup.
- [ai_service_grpc.py](file:///Users/ldr/work/reminisce/ai/ai_service_grpc.py): gRPC servicer implementing all inference + health RPCs.
- [proto/ai_service.proto](file:///Users/ldr/work/reminisce/proto/ai_service.proto): gRPC service & message definitions (source of truth for the wire contract).
- [requirements.txt](file:///Users/ldr/work/reminisce/ai/requirements.txt): Python package dependencies.
- [Dockerfile](file:///Users/ldr/work/reminisce/ai/Dockerfile): Container setup with CUDA/ROCm dependencies; regenerates gRPC stubs from the proto at build time.

## Invariants & Performance Gotchas
- **Model Warmup**: Models are loaded into memory on service startup. Ensure sufficient VRAM/RAM (minimum 4 GB free) before starting container. The orientation classifier adds ~350 MB (BEiT-Base, fp32) on top of SigLIP2 + VLM + InsightFace.
- **Orientation model**: overridable via the `ORIENTATION_MODEL_NAME` env var (default `amaye15/Beit-Base-Image-Orientation-Fixer`). Load failure is non-fatal — orientation detection simply degrades to EXIF-only.
- **Inference Speed Expectation**: Visual description (`DescribeImage`) via SmolVLM/Qwen requires significantly more compute time per item (~1-3s GPU, ~10-30s CPU) compared to vector embedding generation (~50-100ms).
- **gRPC Message Size**: Max message size is raised to 64 MB on both server and client. The backend pre-resizes images (≤768 px) before sending, so payload is normally well under 1 MB. Do NOT remove the server-side limit bump without also keeping client-side pre-resizing.
- **Port Publishing**: The gRPC port (`50051`) must be published/reachable wherever the backend runs — dev compose publishes `50051:50051`; production compose uses the compose-internal network (`ai-server:50051`).
