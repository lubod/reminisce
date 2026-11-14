FROM rocm/pytorch:rocm7.2_ubuntu24.04_py3.12_pytorch_release_2.9.1@sha256:a3b65813621095e3389269417e963725b59310184588c9d2490d44e6e83fa01c

LABEL maintainer="reminisce"
LABEL description="Unified AI service: SigLIP2 embeddings, Qwen3-VL / SmolVLM descriptions, InsightFace detection, BEiT orientation with ROCm 7.2"

WORKDIR /app

# Install system dependencies
# InsightFace (via OpenCV) requires libgl1, libglib2.0
RUN apt-get update && apt-get install -y \
    curl \
    libgl1 \
    libglib2.0-0 \
    libgomp1 \
    --no-install-recommends \
    && rm -rf /var/lib/apt/lists/*

# Copy requirements file
COPY ai/requirements.txt .

# Install Python dependencies (InsightFace runs on CPUExecutionProvider via standard onnxruntime)
RUN pip install --no-cache-dir -r requirements.txt

# Create InsightFace model directory and pre-download models
# This prevents downloading at runtime which can timeout or fail
RUN mkdir -p /app/.insightface/models && \
    python -c "import insightface; \
    from insightface.app import FaceAnalysis; \
    app = FaceAnalysis(name='buffalo_l', root='/app/.insightface'); \
    app.prepare(ctx_id=-1)" || true

# Copy service files and proto schema
COPY proto/ /app/proto/
COPY ai/ai_service.py .
COPY ai/ai_service_grpc.py .
RUN python -m grpc_tools.protoc -I/app/proto --python_out=/app --grpc_python_out=/app /app/proto/ai_service.proto

# Create a non-privileged user and group, change ownership
RUN groupadd -g 10002 ai_user && \
    useradd -u 10002 -g ai_user -m -s /sbin/nologin ai_user && \
    chown -R ai_user:ai_user /app && \
    mkdir -p /home/ai_user/.cache /home/ai_user/.config && \
    chown -R ai_user:ai_user /home/ai_user/.cache /home/ai_user/.config

# Switch to the non-privileged user
USER ai_user:ai_user

# Expose ports (HTTP 8081, gRPC 50051)
EXPOSE 8081 50051

# Health check
HEALTHCHECK --interval=30s --timeout=10s --start-period=120s --retries=3 \
    CMD python -c "import requests; requests.get('http://localhost:8081/health').raise_for_status()"

# ROCm performance tuning for iGPU (gfx1150 / RDNA3.5)
# hipBLASLt has faster GEMM kernels than rocBLAS on RDNA3 iGPUs (~2x prompt throughput)
ENV ROCBLAS_USE_HIPBLASLT=1
# Persist Triton-compiled kernels across container restarts inside ai_model_cache
ENV TRITON_CACHE_DIR=/home/ai_user/.cache/huggingface/triton_cache

# Run the service with gunicorn
CMD ["gunicorn", "--bind", "0.0.0.0:8081", "--workers", "1", "--timeout", "300", "--threads", "1", "ai_service:app"]
