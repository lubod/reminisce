import io
import os
import sys
import types
import unittest
from unittest.mock import patch

import grpc
import torch
from PIL import Image

sys.path.insert(0, os.path.dirname(__file__))
import ai_service_grpc  # noqa: E402
import ai_service_pb2  # noqa: E402


class AbortError(Exception):
    def __init__(self, code, details):
        super().__init__(f"{code}: {details}")
        self.code = code
        self.details = details


class FakeContext:
    def __init__(self, metadata=()):
        self._metadata = list(metadata)

    def invocation_metadata(self):
        return self._metadata

    def abort(self, code, details):
        raise AbortError(code, details)


class FakeBatch(dict):
    def to(self, device):
        return FakeBatch({k: v.to(device) if torch.is_tensor(v) else v for k, v in self.items()})


def make_image_bytes(width=16, height=16, fmt="PNG"):
    img = Image.new("RGB", (width, height), color=(120, 160, 200))
    buf = io.BytesIO()
    img.save(buf, format=fmt)
    return buf.getvalue()


def make_fake_svc(logits_list, id2label):
    class FakeOrientationModel:
        def __init__(self):
            self.config = types.SimpleNamespace(id2label=id2label)
            self._param = torch.nn.Parameter(torch.zeros(1, dtype=torch.float32))

        def parameters(self):
            return iter([self._param])

        def __call__(self, **kwargs):
            return types.SimpleNamespace(logits=torch.tensor([logits_list], dtype=torch.float32))

    def fake_processor(images=None, return_tensors="pt"):
        return FakeBatch({"pixel_values": torch.zeros((1, 3, 8, 8), dtype=torch.float32)})

    return types.SimpleNamespace(
        device=torch.device("cpu"),
        orientation_model=FakeOrientationModel(),
        orientation_processor=fake_processor,
    )


class TestCheckApiKey(unittest.TestCase):
    def test_missing_secret_aborts_unavailable(self):
        with patch.dict(os.environ, {}, clear=True):
            ctx = FakeContext()
            with self.assertRaises(AbortError) as cm:
                ai_service_grpc.check_api_key(ctx)
            self.assertEqual(cm.exception.code, grpc.StatusCode.UNAVAILABLE)

    def test_wrong_key_aborts_unauthenticated(self):
        with patch.dict(os.environ, {"API_SECRET_KEY": "secret-123"}, clear=True):
            ctx = FakeContext([("x-api-key", "wrong")])
            with self.assertRaises(AbortError) as cm:
                ai_service_grpc.check_api_key(ctx)
            self.assertEqual(cm.exception.code, grpc.StatusCode.UNAUTHENTICATED)

    def test_valid_x_api_key_and_bearer_succeed(self):
        with patch.dict(os.environ, {"API_SECRET_KEY": "secret-123"}, clear=True):
            self.assertTrue(
                ai_service_grpc.check_api_key(FakeContext([("x-api-key", "secret-123")]))
            )
            self.assertTrue(
                ai_service_grpc.check_api_key(FakeContext([("authorization", "Bearer secret-123")]))
            )


class TestValidateImageInput(unittest.TestCase):
    def test_empty_and_corrupt_bytes_abort_invalid_argument(self):
        for payload in (b"", b"not-an-image"):
            with self.assertRaises(AbortError) as cm:
                ai_service_grpc._validate_image_input(payload, FakeContext())
            self.assertEqual(cm.exception.code, grpc.StatusCode.INVALID_ARGUMENT)

    def test_valid_png_and_jpeg_return_pil_image(self):
        for fmt in ("PNG", "JPEG"):
            data = make_image_bytes(24, 16, fmt=fmt)
            img = ai_service_grpc._validate_image_input(data, FakeContext())
            self.assertEqual(img.size, (24, 16))


class TestDetectOrientation(unittest.TestCase):
    def setUp(self):
        self.servicer = ai_service_grpc.AIServiceServicer()
        self.auth_ctx = FakeContext([("x-api-key", "secret-123")])
        self.env_patcher = patch.dict(os.environ, {"API_SECRET_KEY": "secret-123"}, clear=True)
        self.env_patcher.start()

    def tearDown(self):
        self.env_patcher.stop()

    def test_label_to_exif_mapping_int_and_str_keys(self):
        expected = [
            (0, "0", 1),
            (1, "90", 6),
            (2, "180", 3),
            (3, "270", 8),
        ]
        id2label_variants = [
            {0: "0", 1: "90", 2: "180", 3: "270"},
            {"0": "0", "1": "90", "2": "180", "3": "270"},
        ]
        req = ai_service_pb2.DetectOrientationRequest(image_data=make_image_bytes(32, 32))

        for id2label in id2label_variants:
            for idx, label, exif_val in expected:
                logits = [-10.0, -10.0, -10.0, -10.0]
                logits[idx] = 10.0
                svc = make_fake_svc(logits, id2label)
                with patch("ai_service_grpc.get_ai_service", return_value=svc):
                    resp = self.servicer.DetectOrientation(req, self.auth_ctx)
                self.assertEqual(resp.orientation_value, exif_val)
                self.assertEqual(resp.label, label)
                self.assertGreater(resp.confidence, 0.9)

    def test_tiny_image_aborts_invalid_argument(self):
        svc = make_fake_svc([1.0, 0.0, 0.0, 0.0], {0: "0", 1: "90", 2: "180", 3: "270"})
        req = ai_service_pb2.DetectOrientationRequest(image_data=make_image_bytes(2, 2))
        with patch("ai_service_grpc.get_ai_service", return_value=svc):
            with self.assertRaises(AbortError) as cm:
                self.servicer.DetectOrientation(req, self.auth_ctx)
        self.assertEqual(cm.exception.code, grpc.StatusCode.INVALID_ARGUMENT)

    def test_model_not_loaded_aborts_unavailable(self):
        svc = types.SimpleNamespace(orientation_model=None)
        req = ai_service_pb2.DetectOrientationRequest(image_data=make_image_bytes(32, 32))
        with patch("ai_service_grpc.get_ai_service", return_value=svc):
            with self.assertRaises(AbortError) as cm:
                self.servicer.DetectOrientation(req, self.auth_ctx)
        self.assertEqual(cm.exception.code, grpc.StatusCode.UNAVAILABLE)


if __name__ == "__main__":
    unittest.main()
