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

app = Flask(__name__)

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


def load_models():
    """Load all AI models on startup"""
    global siglip_model, siglip_processor, vlm_model, vlm_processor, smolvlm_model, smolvlm_processor, face_app, device

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
        torch.set_num_threads(4)
        torch.set_num_interop_threads(2)

    device = detect_device()

    # Load SigLIP2 for embeddings (drop-in replacement with better multilingual + spatial understanding)
    logger.info("Loading SigLIP2 model...")
    siglip_model_name = "google/siglip2-so400m-patch14-384"

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
    vlm_model_name = "Qwen/Qwen2.5-VL-3B-Instruct"
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
    smolvlm_model_name = "HuggingFaceTB/SmolVLM-500M-Instruct"
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
        face_app = FaceAnalysis(name='buffalo_l', root='/root/.insightface', providers=['CPUExecutionProvider'])
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


# ==================== EMBEDDING ENDPOINTS ====================

@app.route('/embed/image', methods=['POST'])
def embed_image():
    """Generate image embedding from base64 encoded image."""
    if siglip_model is None:
        return jsonify({'error': 'Embedding model not loaded'}), 503

    try:
        data = request.json
        if not data or 'image' not in data:
            return jsonify({'error': 'Missing image field'}), 400

        try:
            image = decode_image_to_pil(data['image'])
        except Exception as e:
            return jsonify({'error': 'Invalid image data'}), 400

        # Ensure image is valid RGB and minimum size to avoid processor crash
        # Images smaller than 3x3 cause channel dimension ambiguity
        if image.mode != 'RGB':
            image = image.convert('RGB')
        if image.width < 3 or image.height < 3:
            return jsonify({'error': f'Image too small ({image.width}x{image.height}), minimum 3x3'}), 400

        inputs = siglip_processor(images=image, return_tensors="pt").to(device)
        # Cast inputs to model dtype (FP16 on GPU) to avoid type mismatch
        model_dtype = next(siglip_model.parameters()).dtype
        inputs = {k: v.to(model_dtype) if v.is_floating_point() else v for k, v in inputs.items()}

        with torch.no_grad():
            image_features = siglip_model.get_image_features(**inputs)
            # Handle cases where output is an object instead of a tensor
            if hasattr(image_features, "pooler_output"):
                image_features = image_features.pooler_output
            elif not torch.is_tensor(image_features) and hasattr(image_features, "last_hidden_state"):
                image_features = image_features.last_hidden_state[:, 0, :]
            
            image_features = image_features / image_features.norm(dim=-1, keepdim=True)

        embedding = image_features.cpu().numpy().flatten().tolist()
        logger.info(f"Image embedding generated: {len(embedding)}d, image={image.width}x{image.height}")
        return jsonify({'embedding': embedding, 'dimension': len(embedding)})

    except Exception as e:
        logger.error(f"Error: {e}", exc_info=True)
        return jsonify({'error': str(e)}), 500


@app.route('/embed/text', methods=['POST'])
def embed_text():
    """Generate text embedding from search query."""
    if siglip_model is None:
        return jsonify({'error': 'Embedding model not loaded'}), 503

    try:
        data = request.json
        if not data or 'text' not in data:
            return jsonify({'error': 'Missing text field'}), 400

        text = data['text']
        inputs = siglip_processor(text=[text], return_tensors="pt", padding="max_length").to(device)
        # Cast inputs to model dtype (FP16 on GPU) to avoid type mismatch
        model_dtype = next(siglip_model.parameters()).dtype
        inputs = {k: v.to(model_dtype) if v.is_floating_point() else v for k, v in inputs.items()}

        with torch.no_grad():
            text_features = siglip_model.get_text_features(**inputs)
            # Handle cases where output is an object instead of a tensor
            if hasattr(text_features, "pooler_output"):
                text_features = text_features.pooler_output
            elif not torch.is_tensor(text_features) and hasattr(text_features, "last_hidden_state"):
                text_features = text_features.last_hidden_state[:, 0, :]
            
            text_features = text_features / text_features.norm(dim=-1, keepdim=True)

        embedding = text_features.cpu().numpy().flatten().tolist()
        logger.info(f"Text embedding generated: {len(embedding)}d, text=\"{text}\"")
        return jsonify({'embedding': embedding, 'dimension': len(embedding), 'text': text})

    except Exception as e:
        logger.error(f"Error: {e}", exc_info=True)
        return jsonify({'error': str(e)}), 500


# ==================== QUALITY SCORING ENDPOINT ====================

QUALITY_TEXTS = [
    "a high quality, beautiful, sharp, well-exposed photograph",
    "a low quality, blurry, dark, bad photograph",
]

@app.route('/quality', methods=['POST'])
def quality_score():
    """Compute quality signals for an image: aesthetic score, sharpness, resolution."""
    if siglip_model is None:
        return jsonify({'error': 'Embedding model not loaded'}), 503

    try:
        data = request.json
        if not data or 'image' not in data:
            return jsonify({'error': 'Missing image field'}), 400

        try:
            img = decode_image_to_pil(data['image'])
        except Exception:
            return jsonify({'error': 'Invalid image data'}), 400

        w, h = img.size
        if w < 3 or h < 3:
            return jsonify({'error': 'Image too small'}), 400

        # Sharpness: Laplacian variance on grayscale
        gray = np.array(img.convert('L'))
        sharpness = float(cv2.Laplacian(gray, cv2.CV_64F).var())

        # Aesthetic: zero-shot SigLIP text comparison
        inputs = siglip_processor(text=QUALITY_TEXTS, images=img, return_tensors="pt", padding="max_length")
        inputs = {k: v.to(device) for k, v in inputs.items()}
        model_dtype = next(siglip_model.parameters()).dtype
        inputs = {k: v.to(model_dtype) if v.is_floating_point() else v for k, v in inputs.items()}

        with torch.no_grad():
            outputs = siglip_model(**inputs)

        probs = outputs.logits_per_image.softmax(dim=-1)[0]
        aesthetic_score = float(probs[0].item()) * 10.0  # 0–10

        logger.info(f"Quality score: aesthetic={aesthetic_score:.2f}, sharpness={sharpness:.1f}, size={w}x{h}")
        return jsonify({
            "aesthetic_score": aesthetic_score,
            "sharpness_score": sharpness,
            "width": w,
            "height": h,
        })

    except Exception as e:
        logger.error(f"Error in quality_score: {e}", exc_info=True)
        return jsonify({'error': str(e)}), 500


# ==================== DESCRIPTION ENDPOINTS ====================

@app.route('/describe/qwen', methods=['POST'])
def describe_image():
    """Generate image description using Qwen2.5-VL (high quality, ~29s)."""
    if vlm_model is None:
        return jsonify({'error': 'VLM model not loaded'}), 503

    try:
        data = request.json
        if not data or 'image' not in data:
            return jsonify({'error': 'Missing image field'}), 400

        try:
            image = decode_image_to_pil(data['image'])
        except Exception as e:
            return jsonify({'error': 'Invalid image data'}), 400

        # Ensure valid RGB and minimum size
        if image.mode != 'RGB':
            image = image.convert('RGB')
        if image.width < 3 or image.height < 3:
            return jsonify({'error': f'Image too small ({image.width}x{image.height}), minimum 3x3'}), 400

        # Use Qwen2.5-VL chat format for description
        # Visual token count is capped by max_pixels on the processor (512*28*28)
        from qwen_vl_utils import process_vision_info

        messages = [
            {"role": "user", "content": [
                {"type": "image", "image": image},
                {"type": "text", "text": "Describe this image in detail."},
            ]}
        ]
        text = vlm_processor.apply_chat_template(messages, tokenize=False, add_generation_prompt=True)
        image_inputs, video_inputs = process_vision_info(messages)
        inputs = vlm_processor(
            text=[text], images=image_inputs, videos=video_inputs, return_tensors="pt"
        ).to(device)

        num_visual_tokens = inputs.input_ids.shape[1]
        logger.info(f"Describing image {image.width}x{image.height}, visual tokens={num_visual_tokens}")
        with torch.no_grad():
            generated_ids = vlm_model.generate(
                **inputs,
                max_new_tokens=250,
                do_sample=False,
                use_cache=True,
            )

        # Trim input tokens from output
        trimmed = [out[len(inp):] for inp, out in zip(inputs.input_ids, generated_ids)]
        description = vlm_processor.batch_decode(trimmed, skip_special_tokens=True)[0]

        logger.info(f"Generated description: {description}")
        return jsonify({'description': description})

    except Exception as e:
        logger.error(f"Error describing image: {e}", exc_info=True)
        return jsonify({'error': str(e)}), 500


@app.route('/describe', methods=['POST'])
def describe_image_fast():
    """Generate image description using SmolVLM-500M (default, ~5.6s)."""
    if smolvlm_model is None:
        return jsonify({'error': 'SmolVLM model not loaded'}), 503

    try:
        data = request.json
        if not data or 'image' not in data:
            return jsonify({'error': 'Missing image field'}), 400

        try:
            image = decode_image_to_pil(data['image'])
        except Exception:
            return jsonify({'error': 'Invalid image data'}), 400

        if image.mode != 'RGB':
            image = image.convert('RGB')
        if image.width < 3 or image.height < 3:
            return jsonify({'error': f'Image too small ({image.width}x{image.height}), minimum 3x3'}), 400

        # SmolVLM uses AutoModelForVision2Seq with a messages-style prompt
        messages = [
            {
                "role": "user",
                "content": [
                    {"type": "image"},
                    {"type": "text", "text": "Describe this image in 3-4 sentences."},
                ],
            }
        ]
        prompt = smolvlm_processor.apply_chat_template(messages, add_generation_prompt=True)
        inputs = smolvlm_processor(text=prompt, images=[image], return_tensors="pt").to(device)

        num_tokens = inputs.input_ids.shape[1]
        logger.info(f"Fast-describing image {image.width}x{image.height}, input tokens={num_tokens}")

        with torch.no_grad():
            generated_ids = smolvlm_model.generate(
                **inputs,
                max_new_tokens=600,
                do_sample=False,
                use_cache=True,
                repetition_penalty=1.3,
                no_repeat_ngram_size=4,
            )

        # Trim input tokens from output
        trimmed = generated_ids[:, inputs.input_ids.shape[1]:]
        description = smolvlm_processor.batch_decode(trimmed, skip_special_tokens=True)[0].strip()

        logger.info(f"Fast description generated: {description}")
        return jsonify({'description': description, 'model': 'SmolVLM-500M'})

    except Exception as e:
        logger.error(f"Error in fast description: {e}", exc_info=True)
        return jsonify({'error': str(e)}), 500


# ==================== FACE DETECTION ENDPOINTS ====================

@app.route('/detect', methods=['POST'])
def detect_faces():
    """Detect faces in image and return bounding boxes + embeddings + confidence."""
    if face_app is None:
        return jsonify({'error': 'Face detection model not loaded'}), 503

    try:
        data = request.json
        if not data or 'image' not in data:
            return jsonify({'error': 'No image provided'}), 400

        try:
            image_bgr = decode_image_to_bgr(data['image'])
        except Exception as e:
            logger.error(f"Error decoding image: {e}")
            return jsonify({'error': 'Invalid image data'}), 400

        # Downscale very large images to prevent OOM (InsightFace det_size is 640x640 anyway)
        h, w = image_bgr.shape[:2]
        max_dim = 2048
        if max(h, w) > max_dim:
            scale = max_dim / max(h, w)
            image_bgr = cv2.resize(image_bgr, (int(w * scale), int(h * scale)))
            logger.info(f"Downscaled image from {w}x{h} to {image_bgr.shape[1]}x{image_bgr.shape[0]} for face detection")

        # Detect faces (InsightFace expects BGR image)
        faces = face_app.get(image_bgr)

        results = []
        for face in faces:
            # InsightFace bbox is [x1, y1, x2, y2]
            # We need [x, y, width, height]
            x1, y1, x2, y2 = face.bbox.astype(int)
            x, y = x1, y1
            w, h = x2 - x1, y2 - y1

            # Ensure valid dimensions
            if w <= 0 or h <= 0:
                continue

            results.append({
                'bbox': [int(x), int(y), int(w), int(h)],
                'embedding': face.embedding.tolist(),  # 512-dim list
                'confidence': float(face.det_score)    # Detection score
            })

        logger.info(f"Detected {len(results)} faces")

        return jsonify({
            'status': 'success',
            'count': len(results),
            'faces': results
        })

    except Exception as e:
        logger.error(f"Error detecting faces: {e}", exc_info=True)
        return jsonify({'error': str(e)}), 500


# ==================== ENHANCE ENDPOINT ====================

@app.route('/enhance', methods=['POST'])
def enhance_image_endpoint():
    """Enhance an image: fix exposure, denoise, sharpen, restore faded colors.

    Request JSON:
      image  - base64-encoded image
      mode   - "auto" (default), "exposure", "restore", "all"

    Response JSON:
      image      - base64-encoded JPEG of the enhanced result
      operations - list of applied operations
      analysis   - detected brightness / sharpness before enhancement
    """
    try:
        data = request.json
        if not data or 'image' not in data:
            return jsonify({'error': 'Missing image field'}), 400

        mode = data.get('mode', 'auto')
        if mode not in ('auto', 'exposure', 'restore', 'all'):
            return jsonify({'error': f'Invalid mode "{mode}". Use auto, exposure, restore, or all'}), 400

        try:
            img = decode_image_to_pil(data['image'])
        except Exception:
            return jsonify({'error': 'Invalid image data'}), 400

        if img.mode != 'RGB':
            img = img.convert('RGB')
        if img.width < 3 or img.height < 3:
            return jsonify({'error': f'Image too small ({img.width}x{img.height})'}), 400

        result = np.array(img)
        operations = []

        # ── Analysis ──────────────────────────────────────────────────────────
        gray = cv2.cvtColor(result, cv2.COLOR_RGB2GRAY)
        mean_brightness = float(gray.mean())
        sharpness = float(cv2.Laplacian(gray, cv2.CV_64F).var())

        do_exposure = mode in ('exposure', 'all') or (
            mode == 'auto' and (mean_brightness < 80 or mean_brightness > 190)
        )
        do_denoise = mode in ('restore', 'all') or (mode == 'auto' and sharpness < 50)
        do_restore = mode in ('restore', 'all')
        do_sharpen = mode in ('restore', 'exposure', 'all') or (mode == 'auto' and sharpness < 150)

        logger.info(
            f"Enhance request: {img.width}x{img.height} mode={mode} "
            f"brightness={mean_brightness:.1f} sharpness={sharpness:.1f} "
            f"→ exposure={do_exposure} denoise={do_denoise} restore={do_restore} sharpen={do_sharpen}"
        )

        # ── 1. Denoise (before sharpening) ────────────────────────────────────
        # fastNlMeansDenoisingColored expects BGR; h/hColor tuned for moderate noise removal
        if do_denoise:
            bgr = cv2.cvtColor(result, cv2.COLOR_RGB2BGR)
            bgr = cv2.fastNlMeansDenoisingColored(
                bgr, None, h=8, hColor=8, templateWindowSize=7, searchWindowSize=21
            )
            result = cv2.cvtColor(bgr, cv2.COLOR_BGR2RGB)
            operations.append('denoised')

        # ── 2. Exposure correction via CLAHE on L channel ─────────────────────
        # Works in LAB so luminance is adjusted without shifting hue/saturation
        if do_exposure:
            lab = cv2.cvtColor(result, cv2.COLOR_RGB2LAB)
            l_ch, a_ch, b_ch = cv2.split(lab)
            clahe = cv2.createCLAHE(clipLimit=3.0, tileGridSize=(8, 8))
            l_ch = clahe.apply(l_ch)
            result = cv2.cvtColor(cv2.merge([l_ch, a_ch, b_ch]), cv2.COLOR_LAB2RGB)
            operations.append('exposure_corrected')

        # ── 3. Per-channel histogram stretch for faded / old photos ───────────
        # Stretches each channel independently between its 2nd and 98th percentile
        if do_restore:
            for i in range(3):
                lo = float(np.percentile(result[:, :, i], 2))
                hi = float(np.percentile(result[:, :, i], 98))
                if hi > lo:
                    result[:, :, i] = np.clip(
                        (result[:, :, i].astype(np.float32) - lo) * 255.0 / (hi - lo),
                        0, 255
                    ).astype(np.uint8)
            operations.append('color_restored')

        # ── 4. Unsharp mask ───────────────────────────────────────────────────
        if do_sharpen:
            blurred = cv2.GaussianBlur(result, (0, 0), sigmaX=2.0)
            result = cv2.addWeighted(result, 1.4, blurred, -0.4, 0)
            result = np.clip(result, 0, 255).astype(np.uint8)
            operations.append('sharpened')

        # ── Encode result ─────────────────────────────────────────────────────
        buf = io.BytesIO()
        Image.fromarray(result).save(buf, format='JPEG', quality=92)
        enhanced_b64 = base64.b64encode(buf.getvalue()).decode('utf-8')

        logger.info(f"Enhancement done: ops={operations}")
        return jsonify({
            'image': enhanced_b64,
            'operations': operations,
            'analysis': {'brightness': mean_brightness, 'sharpness': sharpness},
        })

    except Exception as e:
        logger.error(f"Error in enhance: {e}", exc_info=True)
        return jsonify({'error': str(e)}), 500


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
        'endpoints': {
            '/embed/image': 'POST - Generate image embedding (1152-dim)',
            '/embed/text': 'POST - Generate text embedding (1152-dim)',
            '/describe': 'POST - Generate image description (SmolVLM-500M, ~5.6s)',
            '/describe/qwen': 'POST - Generate image description (Qwen2.5-VL-3B, ~29s, higher quality)',
            '/detect': 'POST - Detect faces and get embeddings (512-dim)',
            '/enhance': 'POST - Enhance image: fix exposure, denoise, restore old photos',
            '/health': 'GET - Health check'
        }
    })


# Load models when module is imported (for Gunicorn)
# or when run directly
load_models()

if __name__ == '__main__':
    logger.info("Starting Unified AI Service on 0.0.0.0:8081")
    app.run(host='0.0.0.0', port=8081, threaded=True)
