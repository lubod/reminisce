#include <stdio.h>
#include <dlfcn.h>

// Function pointer type matching hipblasGemmStridedBatchedEx
typedef int (*hipblasGemmStridedBatchedEx_ptr)(void*, int, int, int, int, int, const void*, const void*, int, long long, const void*, int, long long, const void*, void*, int, long long, int, int);

static hipblasGemmStridedBatchedEx_ptr original_func = NULL;

// This is the symbol ONNX Runtime is looking for
int hipblasGemmStridedBatchedEx_v2(void* handle, int transa, int transb, int m, int n, int k, const void* alpha, const void* A, int lda, long long strideA, const void* B, int ldb, long long strideB, const void* beta, void* C, int ldc, long long strideC, int batchCount, int computeType) {
    if (!original_func) {
        void* lib = dlopen("libhipblas.so.3", RTLD_LAZY | RTLD_GLOBAL);
        if (!lib) {
            fprintf(stderr, "Bridge: Failed to load libhipblas.so.3: %s
", dlerror());
            return -1;
        }
        original_func = (hipblasGemmStridedBatchedEx_ptr)dlsym(lib, "hipblasGemmStridedBatchedEx");
        if (!original_func) {
            fprintf(stderr, "Bridge: Failed to find hipblasGemmStridedBatchedEx in libhipblas.so.3
");
            return -1;
        }
    }
    return original_func(handle, transa, transb, m, n, k, alpha, A, lda, strideA, B, ldb, strideB, beta, C, ldc, strideC, batchCount, computeType);
}
