/*
 * hipblas _v2 symbol bridge for onnxruntime-rocm on ROCm 7.1.1
 *
 * ROCm 7.1.1 removed the _v2 suffix from hipblasGemmEx and
 * hipblasGemmStridedBatchedEx, but onnxruntime-rocm 1.22.2
 * still links against the _v2 variants.
 *
 * This shim simply forwards _v2 calls to the non-_v2 functions.
 * All hipblas enum types (hipblasOperation_t, hipDataType, etc.)
 * are just ints at ABI level, so we use int to avoid pulling in
 * the full HIP/hipblas header dependency chain.
 */

#include <dlfcn.h>

/* hipblasStatus_t is an enum = int */
typedef int hipblasStatus_t;

/* Function pointer types matching the real hipblas signatures */
typedef hipblasStatus_t (*gemmex_fn)(
    void*, int, int, int, int, int,
    const void*, const void*, int, int,
    const void*, int, int,
    const void*, void*, int, int,
    int, int);

typedef hipblasStatus_t (*gemm_strided_fn)(
    void*, int, int, int, int, int,
    const void*, const void*, int, int, long long,
    const void*, int, int, long long,
    const void*, void*, int, int, long long,
    int, int, int);

hipblasStatus_t hipblasGemmEx_v2(
    void* handle, int transA, int transB,
    int m, int n, int k,
    const void* alpha,
    const void* A, int aType, int lda,
    const void* B, int bType, int ldb,
    const void* beta,
    void* C, int cType, int ldc,
    int computeType, int algo)
{
    static gemmex_fn fn = 0;
    if (!fn) fn = (gemmex_fn)dlsym(RTLD_NEXT, "hipblasGemmEx");
    return fn(handle, transA, transB, m, n, k,
              alpha, A, aType, lda, B, bType, ldb,
              beta, C, cType, ldc, computeType, algo);
}

hipblasStatus_t hipblasGemmStridedBatchedEx_v2(
    void* handle, int transA, int transB,
    int m, int n, int k,
    const void* alpha,
    const void* A, int aType, int lda, long long strideA,
    const void* B, int bType, int ldb, long long strideB,
    const void* beta,
    void* C, int cType, int ldc, long long strideC,
    int batchCount, int computeType, int algo)
{
    static gemm_strided_fn fn = 0;
    if (!fn) fn = (gemm_strided_fn)dlsym(RTLD_NEXT, "hipblasGemmStridedBatchedEx");
    return fn(handle, transA, transB, m, n, k,
              alpha, A, aType, lda, strideA,
              B, bType, ldb, strideB,
              beta, C, cType, ldc, strideC,
              batchCount, computeType, algo);
}
