# AI Inference Service (`ai/`)

## Purpose
Python Flask sidecar service providing machine learning inference for Reminisce: image embedding, natural language description generation, face detection/embedding, and duplicate media analysis.

## Service Architecture
```
Backend (Rust) ──HTTP POST / :8081 (Header: X-API-Key)──▶ Flask (ai_service.py)
                                                              │
                     ┌────────────────────────────────────────┼────────────────────────────────────────┐
                     ▼                                        ▼                                        ▼
             SigLIP2 Model                            SmolVLM / Qwen                          InsightFace
     (Image/Text Vector Embedding)               (Visual Description Gen)               (Face Detection & Re-ID)
```

## Security & Authentication
- All endpoints require authentication via **either** the `Authorization: Bearer <key>` header **or** the `X-API-Key` header. Requests with missing or non-matching keys return HTTP 401 Unauthorized.

## Hardware & Acceleration Auto-Detection
The service automatically detects and configures available hardware accelerators upon startup:
1. NVIDIA GPU via PyTorch CUDA.
2. AMD GPU via ROCm (`hipblas_v2_shim.c`).
3. Apple Silicon GPU via Metal Performance Shaders (MPS).
4. CPU fallback with multi-threading.

## API Endpoint Inventory

| Endpoint | Method | Purpose | Model / Engine |
|----------|--------|---------|----------------|
| `/health` | GET | Health & hardware acceleration status | - |
| `/embed/image` | POST | Image vector embedding generation | SigLIP2 |
| `/embed/text` | POST | Text query vector embedding generation | SigLIP2 |
| `/describe` | POST | Fast visual scene description generation | SmolVLM |
| `/describe/qwen` | POST | High-quality visual scene description generation | Qwen2.5-VL |
| `/quality` | POST | Aesthetic quality & sharpness scoring | SigLIP2 + OpenCV |
| `/orientation` | POST | Image orientation detection | Qwen2.5-VL |
| `/detect` | POST | Face detection, bounding box & embeddings | InsightFace |
| `/enhance` | POST | Photo enhancement pipeline | OpenCV |

## Key Files
- [ai_service.py](file:///Users/ldr/work/reminisce/ai/ai_service.py): Flask application entry point, model loading, and route handlers.
- [requirements.txt](file:///Users/ldr/work/reminisce/ai/requirements.txt): Python package dependencies.
- [Dockerfile](file:///Users/ldr/work/reminisce/ai/Dockerfile): Container setup with CUDA/ROCm dependencies.

## Invariants & Performance Gotchas
- **Model Warmup**: Models are loaded into memory on service startup. Ensure sufficient VRAM/RAM (minimum 4 GB free) before starting container.
- **Inference Speed Expectation**: Visual description (`/describe`) via SmolVLM/Qwen requires significantly more compute time per item (~1-3s GPU, ~10-30s CPU) compared to vector embedding generation (~50-100ms).
