#!/usr/bin/env python3
"""
gRPC AI Inference Server for Reminisce

Serves gRPC endpoints defined in proto/ai_service.proto:
- EmbedImage
- EmbedText
- DescribeImage
- DetectFaces
- HealthCheck
"""

import io
import logging
import os
import sys
from concurrent import futures
import cv2
import grpc
import numpy as np
import torch
from PIL import Image

# Import generated gRPC stubs from relative/current directory
sys.path.insert(0, os.path.dirname(__file__))
import ai_service_pb2
import ai_service_pb2_grpc

# Import model loader and state from ai_service
import ai_service

logger = logging.getLogger(__name__)

def check_api_key(context):
    metadata = dict(context.invocation_metadata())
    expected_key = os.environ.get('API_SECRET_KEY') or os.environ.get('REMINISCE_API_SECRET_KEY')
    if not expected_key:
        return True # If not configured, allow (development fallback)
    
    auth_val = metadata.get('x-api-key') or metadata.get('authorization')
    if auth_val and auth_val.startswith('Bearer '):
        auth_val = auth_val.split(' ')[1]
    
    if auth_val != expected_key:
        context.abort(grpc.StatusCode.UNAUTHENTICATED, "Invalid or missing API key")
        return False
    return True


class AIServiceServicer(ai_service_pb2_grpc.AIServiceServicer):
    
    def EmbedImage(self, request, context):
        check_api_key(context)
        if ai_service.siglip_model is None:
            context.abort(grpc.StatusCode.UNAVAILABLE, "SigLIP model not loaded")

        if not request.image_data:
            context.abort(grpc.StatusCode.INVALID_ARGUMENT, "Missing image_data")

        try:
            image = Image.open(io.BytesIO(request.image_data))
            if image.mode != 'RGB':
                image = image.convert('RGB')
            
            if image.width < 3 or image.height < 3:
                context.abort(grpc.StatusCode.INVALID_ARGUMENT, f"Image too small ({image.width}x{image.height})")

            inputs = ai_service.siglip_processor(images=image, return_tensors="pt").to(ai_service.device)
            model_dtype = next(ai_service.siglip_model.parameters()).dtype
            inputs = {k: v.to(model_dtype) if v.is_floating_point() else v for k, v in inputs.items()}

            with torch.no_grad():
                image_features = ai_service.siglip_model.get_image_features(**inputs)
                if hasattr(image_features, "pooler_output"):
                    image_features = image_features.pooler_output
                elif not torch.is_tensor(image_features) and hasattr(image_features, "last_hidden_state"):
                    image_features = image_features.last_hidden_state[:, 0, :]
                image_features = image_features / image_features.norm(dim=-1, keepdim=True)

            embedding = image_features.cpu().numpy().flatten().tolist()
            return ai_service_pb2.EmbedImageResponse(embedding=embedding)
        except Exception as e:
            logger.error(f"Error in gRPC EmbedImage: {e}", exc_info=True)
            context.abort(grpc.StatusCode.INTERNAL, str(e))

    def EmbedText(self, request, context):
        check_api_key(context)
        if ai_service.siglip_model is None:
            context.abort(grpc.StatusCode.UNAVAILABLE, "SigLIP model not loaded")

        if not request.text:
            context.abort(grpc.StatusCode.INVALID_ARGUMENT, "Missing text")

        try:
            inputs = ai_service.siglip_processor(text=[request.text], return_tensors="pt", padding="max_length").to(ai_service.device)
            model_dtype = next(ai_service.siglip_model.parameters()).dtype
            inputs = {k: v.to(model_dtype) if v.is_floating_point() else v for k, v in inputs.items()}

            with torch.no_grad():
                text_features = ai_service.siglip_model.get_text_features(**inputs)
                if hasattr(text_features, "pooler_output"):
                    text_features = text_features.pooler_output
                elif not torch.is_tensor(text_features) and hasattr(text_features, "last_hidden_state"):
                    text_features = text_features.last_hidden_state[:, 0, :]
                text_features = text_features / text_features.norm(dim=-1, keepdim=True)

            embedding = text_features.cpu().numpy().flatten().tolist()
            return ai_service_pb2.EmbedTextResponse(embedding=embedding)
        except Exception as e:
            logger.error(f"Error in gRPC EmbedText: {e}", exc_info=True)
            context.abort(grpc.StatusCode.INTERNAL, str(e))

    def DescribeImage(self, request, context):
        check_api_key(context)
        if not request.image_data:
            context.abort(grpc.StatusCode.INVALID_ARGUMENT, "Missing image_data")

        try:
            image = Image.open(io.BytesIO(request.image_data))
            if image.mode != 'RGB':
                image = image.convert('RGB')

            if request.use_qwen:
                if ai_service.vlm_model is None:
                    context.abort(grpc.StatusCode.UNAVAILABLE, "Qwen VLM model not loaded")
                
                from qwen_vl_utils import process_vision_info
                messages = [
                    {
                        "role": "user",
                        "content": [
                            {"type": "image", "image": image},
                            {"type": "text", "text": "Describe this image concisely for search indexing."},
                        ],
                    }
                ]
                text = ai_service.vlm_processor.apply_chat_template(messages, tokenize=False, add_generation_prompt=True)
                image_inputs, video_inputs = process_vision_info(messages)
                inputs = ai_service.vlm_processor(
                    text=[text],
                    images=image_inputs,
                    videos=video_inputs,
                    padding=True,
                    return_tensors="pt"
                ).to(ai_service.device)

                with torch.no_grad():
                    generated_ids = ai_service.vlm_model.generate(**inputs, max_new_tokens=128)
                    generated_ids_trimmed = [
                        out_ids[len(in_ids):] for in_ids, out_ids in zip(inputs.input_ids, generated_ids)
                    ]
                    output_text = ai_service.vlm_processor.batch_decode(
                        generated_ids_trimmed, skip_special_tokens=True, clean_up_tokenization_spaces=False
                    )[0]
                model_used = "Qwen2.5-VL-3B"
            else:
                if ai_service.smolvlm_model is None:
                    context.abort(grpc.StatusCode.UNAVAILABLE, "SmolVLM model not loaded")

                messages = [
                    {
                        "role": "user",
                        "content": [
                            {"type": "image"},
                            {"type": "text", "text": "Describe this photo concisely in 1-2 sentences."}
                        ]
                    }
                ]
                prompt = ai_service.smolvlm_processor.apply_chat_template(messages, add_generation_prompt=True)
                inputs = ai_service.smolvlm_processor(text=prompt, images=image, return_tensors="pt").to(ai_service.device)

                with torch.no_grad():
                    generated_ids = ai_service.smolvlm_model.generate(**inputs, max_new_tokens=128)
                    output_text = ai_service.smolvlm_processor.batch_decode(
                        generated_ids, skip_special_tokens=True
                    )[0]
                    if "Assistant:" in output_text:
                        output_text = output_text.split("Assistant:")[-1].strip()
                model_used = "SmolVLM-500M"

            return ai_service_pb2.DescribeImageResponse(description=output_text.strip(), model_used=model_used)
        except Exception as e:
            logger.error(f"Error in gRPC DescribeImage: {e}", exc_info=True)
            context.abort(grpc.StatusCode.INTERNAL, str(e))

    def DetectFaces(self, request, context):
        check_api_key(context)
        if ai_service.face_app is None:
            context.abort(grpc.StatusCode.UNAVAILABLE, "InsightFace model not loaded")

        if not request.image_data:
            context.abort(grpc.StatusCode.INVALID_ARGUMENT, "Missing image_data")

        try:
            image_pil = Image.open(io.BytesIO(request.image_data))
            if image_pil.mode != 'RGB':
                image_pil = image_pil.convert('RGB')
            image_np = np.array(image_pil)
            image_bgr = cv2.cvtColor(image_np, cv2.COLOR_RGB2BGR)

            faces = ai_service.face_app.get(image_bgr)
            pb_faces = []
            for face in faces:
                bbox = face.bbox.astype(int).tolist()
                embedding = face.embedding.flatten().tolist()
                det_score = float(face.det_score) if hasattr(face, 'det_score') else 1.0

                pb_faces.append(ai_service_pb2.FaceBoundingBox(
                    x=int(bbox[0]),
                    y=int(bbox[1]),
                    width=int(bbox[2] - bbox[0]),
                    height=int(bbox[3] - bbox[1]),
                    embedding=embedding,
                    det_score=det_score,
                ))

            return ai_service_pb2.DetectFacesResponse(faces=pb_faces)
        except Exception as e:
            logger.error(f"Error in gRPC DetectFaces: {e}", exc_info=True)
            context.abort(grpc.StatusCode.INTERNAL, str(e))

    def HealthCheck(self, request, context):
        device_str = str(ai_service.device) if ai_service.device else "unknown"
        return ai_service_pb2.HealthCheckResponse(
            status="healthy",
            device=device_str,
            siglip_loaded=ai_service.siglip_model is not None,
            vlm_loaded=ai_service.vlm_model is not None,
            smolvlm_loaded=ai_service.smolvlm_model is not None,
            face_loaded=ai_service.face_app is not None,
        )


def serve_grpc(port=50051, max_workers=4):
    options = [
        ('grpc.max_send_message_length', 64 * 1024 * 1024),
        ('grpc.max_receive_message_length', 64 * 1024 * 1024),
    ]
    server = grpc.server(futures.ThreadPoolExecutor(max_workers=max_workers), options=options)
    ai_service_pb2_grpc.add_AIServiceServicer_to_server(AIServiceServicer(), server)
    server.add_insecure_port(f"[::]:{port}")
    server.start()
    logger.info(f"gRPC AI Server listening on port {port}")
    return server


if __name__ == '__main__':
    logging.basicConfig(level=logging.INFO, format='%(asctime)s - %(name)s - %(levelname)s - %(message)s')
    logger.info("Starting AI Service models initialization...")
    ai_service.load_models()

    port = int(os.environ.get("GRPC_PORT", "50051"))
    server = serve_grpc(port=port)
    server.wait_for_termination()
