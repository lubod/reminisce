# AI Inference Service (`ai/`)

## Purpose
Python sidecar providing machine learning inference for Reminisce: image embedding, natural language description generation, face detection/embedding, quality scoring, orientation detection, and photo enhancement.

## Service Architecture
The service runs **two servers** in one process, both sharing the models loaded at startup:
- **HTTP (Flask, `:8081`)** — `/health`, `/enhance`, `/quality`, `/orientation`. Still used by the backend for health checks and media processing endpoints.
- **gRPC (`:50051`)** — binary protobuf RPCs for the high-throughput inference calls (embed, describe, face detection), replacing the old HTTP Base64 JSON path. Schema in `proto/ai_service.proto`; implemented in `ai_service_grpc.py` (imported and started in a background thread by `ai_service.py`).

```
Backend (Rust) ──HTTP POST / :8081 (X-API-Key)──▶ Flask (ai_service.py)     [health/enhance/quality/orientation]
             └─gRPC / :50051 (x-api-key metadata)─▶ gRPC (ai_service_grpc.py) [embed/describe/detect]
                                                              │
                     ┌────────────────────────────────────────┼────────────────────────────────────────┐
                     ▼                                        ▼                                        ▼
             SigLIP2 Model                            SmolVLM / Qwen                          InsightFace
     (Image/Text Vector Embedding)               (Visual Description Gen)               (Face Detection & Re-ID)
```

## Security & Authentication
- All endpoints require authentication via **either** the `Authorization: Bearer <key>` header **or** the `X-API-Key` header (HTTP) / `x-api-key` metadata (gRPC). Requests with missing or non-matching keys return HTTP 401 / gRPC UNAUTHENTICATED.
- If `API_SECRET_KEY` (or `REMINISCE_API_SECRET_KEY`) is not set on the service, requests are allowed (development fallback) — always set it in production.

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
| `/health` | GET | Health & hardware acceleration status | - |
| `/quality` | POST | Aesthetic quality & sharpness scoring | SigLIP2 + OpenCV |
| `/orientation` | POST | Image orientation detection | Qwen2.5-VL |
| `/enhance` | POST | Photo enhancement pipeline | OpenCV |

### gRPC (`:50051`) — `proto/ai_service.proto`
| RPC | Purpose | Model / Engine |
|-----|---------|----------------|
| `EmbedImage` | Image vector embedding (1152-dim) | SigLIP2 |
| `EmbedText` | Text query vector embedding (1152-dim) | SigLIP2 |
| `DescribeImage` | Visual scene description (`use_qwen` selects model) | SmolVLM / Qwen2.5-VL |
| `DetectFaces` | Face detection, bounding box & embeddings | InsightFace |
| `HealthCheck` | Model-load & device status | - |

The HTTP embed/describe/detect routes (`/embed/*`, `/describe*`, `/detect`) still exist for backward compatibility but the backend uses the gRPC path for these.

## Key Files
- [ai_service.py](file:///Users/ldr/work/reminisce/ai/ai_service.py): Model loading, Flask HTTP routes, and gRPC server startup.
- [ai_service_grpc.py](file:///Users/ldr/work/reminisce/ai/ai_service_grpc.py): gRPC servicer implementing embed/describe/detect/health RPCs.
- [proto/ai_service.proto](file:///Users/ldr/work/reminisce/proto/ai_service.proto): gRPC service & message definitions (source of truth for the wire contract).
- [requirements.txt](file:///Users/ldr/work/reminisce/ai/requirements.txt): Python package dependencies.
- [Dockerfile](file:///Users/ldr/work/reminisce/ai/Dockerfile): Container setup with CUDA/ROCm dependencies; regenerates gRPC stubs from the proto at build time.

## Invariants & Performance Gotchas
- **Model Warmup**: Models are loaded into memory on service startup. Ensure sufficient VRAM/RAM (minimum 4 GB free) before starting container.
- **Inference Speed Expectation**: Visual description (`DescribeImage`) via SmolVLM/Qwen requires significantly more compute time per item (~1-3s GPU, ~10-30s CPU) compared to vector embedding generation (~50-100ms).
- **gRPC Message Size**: Max message size is raised to 64 MB on both server and client. The backend pre-resizes images (≤768 px) before sending, so payload is normally well under 1 MB. Do NOT remove the server-side limit bump without also keeping client-side pre-resizing.
- **Port Publishing**: The gRPC port (`50051`) must be published/reachable wherever the backend runs — dev compose publishes `50051:50051`; production compose uses the compose-internal network (`ai-server:50051`).
