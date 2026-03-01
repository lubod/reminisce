FROM rocm/pytorch:rocm7.2_ubuntu24.04_py3.12_pytorch_release_2.9.1

LABEL maintainer="reminisce"
LABEL description="Unified AI service: SigLIP2 embeddings, Qwen2.5-VL descriptions, InsightFace detection with ROCm 7.2"

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

# ROCm 7.1.1 ships hipblas.so.3 but onnxruntime-rocm needs .so.2
# Create compatibility symlinks BEFORE installing onnxruntime-rocm
RUN ROCM_LIB=$(readlink -f /opt/rocm/lib) && \
    ln -sf ${ROCM_LIB}/libhipblas.so.3 ${ROCM_LIB}/libhipblas.so.2 && \
    ln -sf ${ROCM_LIB}/libamdhip64.so.7 ${ROCM_LIB}/libamdhip64.so.6 && \
    ln -sf ${ROCM_LIB}/libhipblaslt.so.1 ${ROCM_LIB}/libhipblaslt.so.2 && \
    ln -sf ${ROCM_LIB}/librocm_smi64.so.1 ${ROCM_LIB}/librocm_smi64.so.7 2>/dev/null || true && \
    ldconfig

# Copy requirements file
COPY requirements.txt .

# Install Python dependencies
# onnxruntime-rocm for AMD GPU support in InsightFace
RUN pip install --no-cache-dir -r requirements.txt onnxruntime-rocm

# Create _v2 symbol bridges for hipblas functions
# ROCm 7.1.1 removed the _v2 suffix but onnxruntime-rocm 1.22.2 still expects them
COPY hipblas_v2_shim.c /app/hipblas_v2_shim.c
RUN gcc -shared -fPIC -o /app/libhipblas_fix.so /app/hipblas_v2_shim.c -ldl

# Create InsightFace model directory and pre-download models
# This prevents downloading at runtime which can timeout or fail
RUN mkdir -p /root/.insightface/models && \
    python -c "import insightface; \
    from insightface.app import FaceAnalysis; \
    app = FaceAnalysis(name='buffalo_l', root='/root/.insightface'); \
    app.prepare(ctx_id=-1)" || true

# Copy service file
COPY ai_service.py .

# Expose port (single port for all services)
EXPOSE 8081

# Health check
HEALTHCHECK --interval=30s --timeout=10s --start-period=120s --retries=3 \
    CMD python -c "import requests; requests.get('http://localhost:8081/health').raise_for_status()"

# ROCm performance tuning for iGPU (gfx1150 / RDNA3)
# hipBLASLt has faster GEMM kernels than rocBLAS on RDNA3 iGPUs (~2x prompt throughput)
ENV ROCBLAS_USE_HIPBLASLT=1
# Cache Triton-compiled kernels across container restarts
ENV TRITON_CACHE_DIR=/tmp/triton_cache

# Run the service with gunicorn and the hipblas fix injected via LD_PRELOAD
CMD ["sh", "-c", "LD_PRELOAD=/app/libhipblas_fix.so gunicorn --bind 0.0.0.0:8081 --workers 1 --timeout 300 --threads 4 ai_service:app"]
