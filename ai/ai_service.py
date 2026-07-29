#!/usr/bin/env python3
"""
Unified AI Service

Provides REST API endpoints for:
- Image and text embeddings using SigLIP2
- Image descriptions using Qwen2.5-VL-3B-Instruct (quality) or SmolVLM-500M (fast)
- Face detection using InsightFace

Endpoints:
- POST /embed/image  - Generate embedding from base64 encoded image
- POST /embed/text   - Generate embedding from text query
- POST /describe      - Generate image description (SmolVLM-500M, ~5.6s)
- POST /describe/qwen - Generate image description (Qwen2.5-VL-3B, ~29s, higher quality)
- POST /detect       - Detect faces in image
- GET  /health       - Health check endpoint
"""

import base64
import io
import logging
import sys

import cv2
import numpy as np
import torch
from flask import Flask, request, jsonify
from PIL import Image
from transformers import AutoModel, AutoProcessor, SmolVLMForConditionalGeneration
from insightface.app import FaceAnalysis

# Configure logging
logging.basicConfig(
    level=logging.INFO,
    format='%(asctime)s - %(name)s - %(levelname)s - %(message)s'
)
logger = logging.getLogger(__name__)

import os
from functools import wraps

app = Flask(__name__)
app.config['MAX_CONTENT_LENGTH'] = 50 * 1024 * 1024  # 50MB limit to prevent OOM (C11)

def require_api_key(f):
    @wraps(f)
    def decorated(*args, **kwargs):
        expected_key = os.environ.get('API_SECRET_KEY') or os.environ.get('REMINISCE_API_SECRET_KEY')
        if not expected_key:
            return jsonify({'error': 'Server misconfigured: API_SECRET_KEY not set'}), 500

        auth_header = request.headers.get('Authorization')
        api_key = None
        if auth_header and auth_header.startswith('Bearer '):
            api_key = auth_header.split(" ")[1]
        if not api_key:
            api_key = request.headers.get('X-API-Key')
        if not api_key:
            return jsonify({'error': 'Unauthorized: Missing API Key'}), 401
        if api_key != expected_key:
            return jsonify({'error': 'Unauthorized: Invalid API Key'}), 401
        return f(*args, **kwargs)
    return decorated

# Global model references
siglip_model = None
siglip_processor = None
vlm_model = None
vlm_processor = None
smolvlm_model = None
smolvlm_processor = None
face_app = None
device = None


def detect_device():
    """Detect best available device (GPU or CPU) - vendor agnostic"""

    # Try ROCm (AMD)
    if torch.cuda.is_available() and torch.version.hip:
        device_name = torch.cuda.get_device_name(0)
        logger.info(f"AMD ROCm GPU detected: {device_name}")
        return "cuda"

    # Try NVIDIA CUDA
    if torch.cuda.is_available():
        device_name = torch.cuda.get_device_name(0)
        logger.info(f"NVIDIA GPU detected: {device_name}")
        return "cuda"

    # Try Apple Silicon MPS (Metal Performance Shaders)
    if hasattr(torch.backends, 'mps') and torch.backends.mps.is_available():
        logger.info("Apple Silicon GPU (MPS) detected")
        return "mps"

    # Fallback to CPU
    logger.info("PyTorch using CPU")
    return "cpu"


def get_onnx_providers():
    """Get available ONNX execution providers, preferring GPU"""
    available_providers = ['CPUExecutionProvider']
    try:
        import onnxruntime as ort
        all_providers = ort.get_available_providers()
        logger.info(f"Available ONNX providers: {all_providers}")
        
        # Priority order for ROCm/AMD and NVIDIA
        # Skip MIGraphX — it doesn't support all gfx targets (e.g. gfx1150)
        gpu_priority = ['ROCMExecutionProvider', 'CUDAExecutionProvider']
        
        # Build prioritized list
        to_use = []
        for p in gpu_priority:
            if p in all_providers:
                to_use.append(p)
        
        # Add CPU at the end
        to_use.append('CPUExecutionProvider')
        
        logger.info(f"Prioritized ONNX providers: {to_use}")
        return to_use
    except Exception as e:
        logger.warning(f"Could not check ONNX providers: {e}")
    return available_providers


models_loaded = False

def load_models():
    """Load all AI models on startup"""
    global models_loaded, siglip_model
    if models_loaded:
        return
    models_loaded = True, siglip_processor, vlm_model, vlm_processor, smolvlm_model, smolvlm_processor, face_app, device

    # Compatibility patch for models with missing config attributes (SigLIP, Moondream2, etc.)
    try:
        from transformers import PretrainedConfig
        orig_init = PretrainedConfig.__init__
        def patched_init(self, *args, **kwargs):
            orig_init(self, *args, **kwargs)
            if not hasattr(self, 'forced_bos_token_id'):
                self.__dict__['forced_bos_token_id'] = None
            if not hasattr(self, 'pad_token_id'):
                self.__dict__['pad_token_id'] = None
        PretrainedConfig.__init__ = patched_init
        logger.info("Applied PretrainedConfig monkey-patch")
    except Exception as e:
        logger.warning(f"Failed to apply PretrainedConfig patch: {e}")

    # Only set CPU threading when not using GPU (avoids unnecessary overhead)
    if detect_device() == "cpu":
        try:
            torch.set_num_threads(4)
            torch.set_num_interop_threads(2)
        except Exception:
            pass

    device = detect_device()

    # Load SigLIP2 for embeddings (drop-in replacement with better multilingual + spatial understanding)
    logger.info("Loading SigLIP2 model...")
    siglip_model_name = os.environ.get("SIGLIP_MODEL_NAME", "google/siglip2-so400m-patch14-384")

    try:
        siglip_dtype = torch.float16 if device == "cuda" else torch.float32
        siglip_model = AutoModel.from_pretrained(siglip_model_name, torch_dtype=siglip_dtype).to(device)
        siglip_processor = AutoProcessor.from_pretrained(siglip_model_name)
        siglip_model.eval()
        logger.info(f"SigLIP2 model loaded successfully on {device} (dtype={siglip_dtype})")
    except Exception as e:
        logger.error(f"Failed to load SigLIP2 model: {e}")
        raise

    # Load Qwen2.5-VL-3B for descriptions
    # Visual tokens capped at 256 max (down from 16384 default) — biggest perf lever on iGPU
    # bfloat16 preferred over float16 for ROCm (matches training dtype, less conversion)
    logger.info("Loading Qwen2.5-VL-3B-Instruct model for descriptions...")
    vlm_model_name = os.environ.get("VLM_MODEL_NAME", "Qwen/Qwen2.5-VL-3B-Instruct")
    try:
        from transformers import Qwen2_5_VLForConditionalGeneration
        vlm_model = Qwen2_5_VLForConditionalGeneration.from_pretrained(
            vlm_model_name,
            torch_dtype=torch.bfloat16 if device == "cuda" else torch.float32,
            attn_implementation="sdpa",
        ).to(device)
        # Cap visual tokens aggressively: 256 tokens max (~200K pixels)
        # Halving from 512 → 256 cuts attention cost ~2x and decode overhead ~2x
        vlm_processor = AutoProcessor.from_pretrained(
            vlm_model_name,
            min_pixels=64 * 28 * 28,
            max_pixels=256 * 28 * 28,
        )
        vlm_model.eval()
        logger.info(f"Qwen2.5-VL-3B-Instruct loaded on {device} (bfloat16, sdpa, max_pixels=256)")
    except Exception as e:
        logger.error(f"Failed to load Qwen2.5-VL model: {e}")
        # Continue without VLM - descriptions won't work but embeddings will

    # Load SmolVLM-500M for fast descriptions (parallel to Qwen2.5-VL)
    # 500M params vs 3B — ~64 visual tokens per patch vs 256+, target <5s per image
    logger.info("Loading SmolVLM-500M-Instruct model for fast descriptions...")
    smolvlm_model_name = os.environ.get("SMOLVLM_MODEL_NAME", "HuggingFaceTB/SmolVLM-500M-Instruct")
    try:
        smolvlm_model = SmolVLMForConditionalGeneration.from_pretrained(
            smolvlm_model_name,
            torch_dtype=torch.bfloat16 if device == "cuda" else torch.float32,
        ).to(device)
        smolvlm_processor = AutoProcessor.from_pretrained(smolvlm_model_name)
        smolvlm_model.eval()
        logger.info(f"SmolVLM-500M-Instruct loaded on {device} (bfloat16)")
    except Exception as e:
        logger.error(f"Failed to load SmolVLM model: {e}")

    # Load InsightFace for face detection
    # Use CPU provider — ROCm ONNX crashes with SIGSEGV on gfx1150 (RDNA3 iGPU)
    # InsightFace models are small and fast enough on CPU (~200ms/image)
    logger.info("Loading InsightFace (buffalo_l) on CPU...")
    try:
        face_app = FaceAnalysis(name='buffalo_l', root='/app/.insightface', providers=['CPUExecutionProvider'])
        face_app.prepare(ctx_id=-1, det_size=(640, 640))
        logger.info("InsightFace initialized successfully on CPU")
    except Exception as e:
        logger.error(f"Failed to initialize InsightFace: {e}", exc_info=True)
        # Continue without face detection - other features will work


def decode_image_to_pil(base64_string):
    """Decode base64 string to PIL Image (RGB)"""
    if ',' in base64_string:
        base64_string = base64_string.split(',')[1]

    image_data = base64.b64decode(base64_string)
    pil_image = Image.open(io.BytesIO(image_data))

    if pil_image.mode != 'RGB':
        pil_image = pil_image.convert('RGB')

    return pil_image


def decode_image_to_bgr(base64_string):
    """Decode base64 string to numpy array (BGR for OpenCV/InsightFace)"""
    pil_image = decode_image_to_pil(base64_string)
    image_np = np.array(pil_image)
    image_bgr = cv2.cvtColor(image_np, cv2.COLOR_RGB2BGR)
    return image_bgr


QUALITY_TEXTS = [
    "a high quality, beautiful, sharp, well-exposed photograph",
    "a low quality, blurry, dark, bad photograph",
]

# ==================== HEALTH & INFO ENDPOINTS ====================

@app.route('/health', methods=['GET'])
def health():
    """Health check endpoint"""
    return jsonify({
        'status': 'healthy',
        'embedding_model': 'google/siglip2-so400m-patch14-384',
        'vlm_model': 'Qwen/Qwen2.5-VL-3B-Instruct',
        'face_model': 'insightface/buffalo_l',
        'device': device,
        'embedding_dimension': 1152,
        'face_embedding_dimension': 512,
        'models_loaded': {
            'siglip2': siglip_model is not None,
            'qwen25_vl': vlm_model is not None,
            'smolvlm': smolvlm_model is not None,
            'insightface': face_app is not None
        }
    })


@app.route('/', methods=['GET'])
def index():
    """API information endpoint"""
    return jsonify({
        'service': 'Unified AI Service (SigLIP2 + Qwen2.5-VL + InsightFace)',
        'version': '3.0.0',
        'note': 'Inference is served over gRPC on port 50051 (see proto/ai_service.proto). HTTP is kept only for health checks.',
        'endpoints': {
            '/health': 'GET - Health check'
        },
        'grpc': {
            'EmbedImage': 'Image embedding (1152-dim)',
            'EmbedText': 'Text embedding (1152-dim)',
            'DescribeImage': 'Image description (SmolVLM / Qwen2.5-VL)',
            'DetectFaces': 'Face detection + embeddings (512-dim)',
            'QualityScore': 'Aesthetic + sharpness scoring',
            'EnhanceImage': 'Fix exposure, denoise, restore old photos',
            'HealthCheck': 'Model-load & device status',
        }
    })


# Load models when module is imported (for Gunicorn)
# or when run directly
load_models()

# Start gRPC server in background thread
try:
    import ai_service_grpc
    grpc_port = int(os.environ.get("GRPC_PORT", "50051"))
    grpc_server = ai_service_grpc.serve_grpc(port=grpc_port)
    logger.info(f"gRPC server started on port {grpc_port}")
except Exception as e:
    logger.warning(f"Could not start gRPC server: {e}")

if __name__ == '__main__':
    logger.info("Starting Unified AI Service on 0.0.0.0:8081 (Flask HTTP) and 0.0.0.0:50051 (gRPC)")
    app.run(host='0.0.0.0', port=8081, threaded=True)

