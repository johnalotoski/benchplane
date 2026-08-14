// SPDX-License-Identifier: Apache-2.0

#include "llama.h"
#include "ggml-backend.h"

#include <algorithm>
#include <atomic>
#include <charconv>
#include <chrono>
#include <cstdint>
#include <cstdlib>
#include <cstdio>
#include <cstring>
#include <limits>
#include <string>
#include <string_view>
#include <vector>

#ifdef BENCHPLANE_TARGET_NVIDIA_CUDA
#include <cuda_runtime_api.h>
#endif

#ifndef BENCHPLANE_MODEL_PATH
#error "BENCHPLANE_MODEL_PATH must name the immutable packaged GGUF model"
#endif

#ifndef BENCHPLANE_BACKEND_PATH
#error "BENCHPLANE_BACKEND_PATH must name the immutable packaged ggml backend directory"
#endif

namespace {

constexpr uint32_t kMaxRecords = 16;
constexpr uint32_t kMaxOutputTokens = 32;
constexpr uint32_t kMaxPromptTokens = 96;
constexpr uint64_t kMaxTotalTokens = 8192;
constexpr int kUsageExit = 2;
constexpr int kModelInitExit = 20;
constexpr int kInferenceExit = 21;
#ifdef BENCHPLANE_TARGET_NVIDIA_CUDA
constexpr const char * kNvidiaMetadataFormat = "benchplane-llama-cpp-nvidia/v1";
constexpr uint32_t kMinimumNvidiaDriverMajor = 575;
constexpr uint32_t kMinimumNvidiaDriverMinor = 57;
constexpr uint32_t kMinimumNvidiaDriverPatch = 8;
#endif

struct Args {
    uint32_t requests = 0;
    uint32_t warmup_runs = 0;
    uint32_t repetitions = 0;
    uint32_t output_tokens = 0;
};

struct RequestObservation {
    uint64_t latency_micros = 0;
    uint64_t ttft_micros = 0;
};

#ifdef BENCHPLANE_TARGET_NVIDIA_CUDA
struct OffloadObservation {
    std::atomic<uint32_t> offloaded{0};
    std::atomic<uint32_t> total{0};
};

void observe_llama_log(enum ggml_log_level, const char * text, void * user_data) {
    auto * observation = static_cast<OffloadObservation *>(user_data);
    const char * marker = std::strstr(text, "offloaded ");
    uint32_t offloaded = 0;
    uint32_t total = 0;
    if (marker != nullptr &&
        std::sscanf(marker, "offloaded %u/%u layers to GPU", &offloaded, &total) == 2) {
        observation->offloaded.store(offloaded);
        observation->total.store(total);
    }
    std::fputs(text, stderr);
}

std::string bounded_driver_version() {
    std::FILE * file = std::fopen("/proc/driver/nvidia/version", "rb");
    if (file == nullptr) {
        return {};
    }
    char buffer[4097] = {};
    const size_t count = std::fread(buffer, 1, sizeof(buffer) - 1, file);
    const bool complete = std::feof(file) != 0;
    std::fclose(file);
    if (!complete || count == 0) {
        return {};
    }
    const char * marker = std::strstr(buffer, "Kernel Module");
    if (marker == nullptr) {
        return {};
    }
    marker += std::strlen("Kernel Module");
    while (*marker != '\0' && *marker != '\r' && *marker != '\n') {
        while (*marker == ' ' || *marker == '\t') {
            ++marker;
        }
        const char * end = marker;
        bool has_dot = false;
        bool version_token = true;
        while (*end != '\0' && *end != ' ' && *end != '\t' && *end != '\r' && *end != '\n') {
            if (*end == '.') {
                has_dot = true;
            } else if (*end < '0' || *end > '9') {
                version_token = false;
            }
            ++end;
        }
        if (version_token && has_dot && end != marker && static_cast<size_t>(end - marker) <= 256) {
            return std::string(marker, end);
        }
        marker = end;
    }
    return {};
}

constexpr bool parse_driver_version_component(
    std::string_view value,
    size_t & position,
    uint32_t & component) {
    if (position >= value.size() || value[position] < '0' || value[position] > '9') {
        return false;
    }
    component = 0;
    while (position < value.size() && value[position] >= '0' && value[position] <= '9') {
        const uint32_t digit = static_cast<uint32_t>(value[position] - '0');
        if (component > (std::numeric_limits<uint32_t>::max() - digit) / 10) {
            return false;
        }
        component = component * 10 + digit;
        ++position;
    }
    return true;
}

constexpr bool nvidia_driver_version_supported(std::string_view value) {
    size_t position = 0;
    uint32_t major = 0;
    uint32_t minor = 0;
    uint32_t patch = 0;
    if (!parse_driver_version_component(value, position, major) || position >= value.size() ||
        value[position++] != '.' ||
        !parse_driver_version_component(value, position, minor)) {
        return false;
    }
    if (position < value.size()) {
        if (value[position++] != '.' ||
            !parse_driver_version_component(value, position, patch)) {
            return false;
        }
    }
    if (position != value.size()) {
        return false;
    }
    return major > kMinimumNvidiaDriverMajor ||
        (major == kMinimumNvidiaDriverMajor && minor > kMinimumNvidiaDriverMinor) ||
        (major == kMinimumNvidiaDriverMajor && minor == kMinimumNvidiaDriverMinor &&
         patch >= kMinimumNvidiaDriverPatch);
}

static_assert(nvidia_driver_version_supported("575.57.08"));
static_assert(nvidia_driver_version_supported("575.57.9"));
static_assert(nvidia_driver_version_supported("595.84"));
static_assert(!nvidia_driver_version_supported("575.57"));
static_assert(!nvidia_driver_version_supported("575.57.07"));
static_assert(!nvidia_driver_version_supported("575.57.08.1"));
static_assert(!nvidia_driver_version_supported("not-a-version"));

std::string cuda_version(int version) {
    if (version <= 0) {
        return {};
    }
    return std::to_string(version / 1000) + "." + std::to_string((version % 1000) / 10);
}

std::string json_string(std::string_view value) {
    if (value.empty() || value.size() > 256) {
        return {};
    }
    std::string escaped;
    escaped.reserve(value.size() + 2);
    escaped.push_back('"');
    for (const unsigned char character : value) {
        if (character < 0x20 || character == 0x7f) {
            return {};
        }
        if (character == '"' || character == '\\') {
            escaped.push_back('\\');
        }
        escaped.push_back(static_cast<char>(character));
    }
    escaped.push_back('"');
    return escaped;
}

bool emit_nvidia_metadata(
    const cudaDeviceProp & properties,
    int driver_version,
    int runtime_version,
    const std::string & nvidia_driver_version,
    uint32_t offloaded_layers,
    uint32_t total_layers) {
    const std::string device_name = json_string(properties.name);
    const std::string nvidia_driver = json_string(nvidia_driver_version);
    const std::string cuda_driver = json_string(cuda_version(driver_version));
    const std::string cuda_runtime = json_string(cuda_version(runtime_version));
    const std::string cuda_toolkit = json_string(cuda_version(CUDART_VERSION));
    const std::string compute_capability =
        json_string(std::to_string(properties.major) + "." + std::to_string(properties.minor));
    if (device_name.empty() || nvidia_driver.empty() || cuda_driver.empty() ||
        cuda_runtime.empty() || cuda_toolkit.empty() || compute_capability.empty() ||
        properties.totalGlobalMem == 0 || offloaded_layers == 0 || offloaded_layers != total_layers) {
        return false;
    }
    return std::printf(
        "{\"format\":\"%s\",\"nvidia\":{\"vendor\":\"NVIDIA\","
        "\"deviceName\":%s,\"logicalDeviceIndex\":0,\"totalVramBytes\":%llu,"
        "\"nvidiaDriverVersion\":%s,\"cudaDriverVersion\":%s,"
        "\"cudaRuntimeVersion\":%s,\"cudaToolkitVersion\":%s,"
        "\"computeCapability\":%s,\"offload\":{\"policy\":"
        "\"singleDeviceAllLayers\",\"offloadedLayers\":%u,\"totalLayers\":%u}}}\n",
        kNvidiaMetadataFormat,
        device_name.c_str(),
        static_cast<unsigned long long>(properties.totalGlobalMem),
        nvidia_driver.c_str(),
        cuda_driver.c_str(),
        cuda_runtime.c_str(),
        cuda_toolkit.c_str(),
        compute_capability.c_str(),
        offloaded_layers,
        total_layers) > 0 && std::fflush(stdout) == 0;
}
#endif

bool parse_u32(std::string_view input, uint32_t & output) {
    if (input.empty()) {
        return false;
    }
    const char * begin = input.data();
    const char * end = begin + input.size();
    const auto result = std::from_chars(begin, end, output);
    return result.ec == std::errc{} && result.ptr == end;
}

bool checked_mul(uint64_t left, uint64_t right, uint64_t & output) {
    if (right != 0 && left > std::numeric_limits<uint64_t>::max() / right) {
        return false;
    }
    output = left * right;
    return true;
}

bool parse_args(int argc, char ** argv, Args & args) {
    if (argc != 9) {
        return false;
    }
    for (int index = 1; index < argc; index += 2) {
        if (index + 1 >= argc) {
            return false;
        }
        const std::string_view name(argv[index]);
        uint32_t value = 0;
        if (!parse_u32(argv[index + 1], value)) {
            return false;
        }
        if (name == "--requests") {
            args.requests = value;
        } else if (name == "--warmup-runs") {
            args.warmup_runs = value;
        } else if (name == "--repetitions") {
            args.repetitions = value;
        } else if (name == "--output-tokens") {
            args.output_tokens = value;
        } else {
            return false;
        }
    }
    return true;
}

bool validate_args(const Args & args) {
    if (args.requests == 0 || args.repetitions == 0 || args.output_tokens == 0 ||
        args.output_tokens > kMaxOutputTokens) {
        return false;
    }
    if (args.warmup_runs > std::numeric_limits<uint32_t>::max() - args.repetitions) {
        return false;
    }
    const uint32_t records = args.warmup_runs + args.repetitions;
    if (records > kMaxRecords) {
        return false;
    }
    uint64_t work = 0;
    if (!checked_mul(records, args.requests, work) ||
        !checked_mul(work, kMaxPromptTokens + args.output_tokens, work)) {
        return false;
    }
    return work <= kMaxTotalTokens;
}

std::string prompt_for(uint32_t request_index) {
    return "<|im_start|>system\nAnswer with one short phrase.<|im_end|>\n"
           "<|im_start|>user\nFixed benchmark request " +
           std::to_string(request_index) +
           ": name a color.<|im_end|>\n<|im_start|>assistant\n";
}

constexpr uint64_t ceil_mean_micros(uint64_t total_nanos, uint32_t count) {
    if (count == 0) {
        return 1;
    }
    const uint64_t divisor = static_cast<uint64_t>(count) * 1000;
    const uint64_t quotient = total_nanos / divisor;
    const uint64_t rounded = quotient + (total_nanos % divisor != 0 ? 1 : 0);
    return std::max<uint64_t>(1, rounded);
}

constexpr uint64_t ceil_micros(uint64_t nanos) {
    return std::max<uint64_t>(1, nanos / 1000 + (nanos % 1000 != 0 ? 1 : 0));
}

static_assert(ceil_mean_micros(2000, 2) == 1);
static_assert(ceil_mean_micros(2001, 2) == 2);
static_assert(ceil_mean_micros(1, std::numeric_limits<uint32_t>::max()) == 1);
static_assert(ceil_micros(1) == 1 && ceil_micros(1001) == 2);

bool run_request(
    llama_model * model,
    const llama_vocab * vocab,
    uint32_t request_index,
    uint32_t output_tokens,
    std::chrono::nanoseconds & latency,
    std::chrono::nanoseconds & ttft) {
    const auto started = std::chrono::steady_clock::now();
    const std::string prompt = prompt_for(request_index);
    const int32_t required = -llama_tokenize(
        vocab, prompt.data(), static_cast<int32_t>(prompt.size()), nullptr, 0, true, true);
    if (required <= 0 || required > static_cast<int32_t>(kMaxPromptTokens)) {
        return false;
    }
    std::vector<llama_token> tokens(static_cast<size_t>(required));
    if (llama_tokenize(
            vocab,
            prompt.data(),
            static_cast<int32_t>(prompt.size()),
            tokens.data(),
            static_cast<int32_t>(tokens.size()),
            true,
            true) != required) {
        return false;
    }

    llama_context_params context_params = llama_context_default_params();
    context_params.n_ctx = static_cast<uint32_t>(required) + output_tokens + 8;
    context_params.n_batch = static_cast<uint32_t>(required);
    context_params.n_threads = 1;
    context_params.n_threads_batch = 1;
    context_params.no_perf = true;
    llama_context * context = llama_init_from_model(model, context_params);
    if (context == nullptr) {
        return false;
    }
    llama_sampler * sampler = llama_sampler_init_greedy();
    if (sampler == nullptr) {
        llama_free(context);
        return false;
    }

    llama_batch batch = llama_batch_get_one(tokens.data(), required);
    bool ok = true;
    llama_token next = 0;
    for (uint32_t generated = 0; generated < output_tokens; ++generated) {
        if (llama_decode(context, batch) != 0) {
            ok = false;
            break;
        }
        next = llama_sampler_sample(sampler, context, -1);
        llama_sampler_accept(sampler, next);
        if (generated == 0) {
            ttft = std::chrono::steady_clock::now() - started;
        }
        batch = llama_batch_get_one(&next, 1);
    }
    latency = std::chrono::steady_clock::now() - started;
    llama_sampler_free(sampler);
    llama_free(context);
    return ok && ttft.count() > 0 && latency.count() >= ttft.count();
}

bool emit_repetition(
    llama_model * model,
    const llama_vocab * vocab,
    const Args & args,
    const char * phase,
    uint32_t repetition_index) {
    const auto repetition_started = std::chrono::steady_clock::now();
    std::chrono::nanoseconds total_latency{0};
    std::chrono::nanoseconds total_ttft{0};
    std::vector<RequestObservation> observations;
    observations.reserve(args.requests);
    for (uint32_t request = 0; request < args.requests; ++request) {
        std::chrono::nanoseconds latency{0};
        std::chrono::nanoseconds ttft{0};
        if (!run_request(model, vocab, request, args.output_tokens, latency, ttft)) {
            return false;
        }
        total_latency += latency;
        total_ttft += ttft;
        const uint64_t latency_micros = ceil_micros(static_cast<uint64_t>(latency.count()));
        observations.push_back(RequestObservation{
            latency_micros,
            std::min(latency_micros, ceil_micros(static_cast<uint64_t>(ttft.count()))),
        });
    }
    const auto repetition_elapsed = std::chrono::steady_clock::now() - repetition_started;
    const uint64_t latency_micros = ceil_mean_micros(
        static_cast<uint64_t>(total_latency.count()), args.requests);
    const uint64_t ttft_micros = std::min(
        latency_micros,
        ceil_mean_micros(static_cast<uint64_t>(total_ttft.count()), args.requests));
    const uint64_t elapsed_nanos =
        std::max<uint64_t>(1, static_cast<uint64_t>(repetition_elapsed.count()));
    const uint64_t throughput = std::max<uint64_t>(
        1, static_cast<uint64_t>(args.requests) * 1000ULL * 1000000000ULL / elapsed_nanos);

    const int written = std::printf(
        "{\"generator\":\"benchplane-llama-cpp-smollm2/v2\","
        "\"attemptNumber\":1,\"phase\":\"%s\",\"repetitionIndex\":%u,"
        "\"sampleIndex\":1,\"latencyMicros\":%llu,"
        "\"timeToFirstTokenMicros\":%llu,"
        "\"throughputMilliRequestsPerSecond\":%llu,"
        "\"successfulRequests\":%u,\"failedRequests\":0,"
        "\"requestObservations\":[",
        phase,
        repetition_index,
        static_cast<unsigned long long>(latency_micros),
        static_cast<unsigned long long>(ttft_micros),
        static_cast<unsigned long long>(throughput),
        args.requests);
    if (written <= 0) {
        return false;
    }
    for (uint32_t request = 0; request < observations.size(); ++request) {
        const RequestObservation & observation = observations[request];
        if (std::printf(
                "%s{\"requestIndex\":%u,\"latencyMicros\":%llu,"
                "\"timeToFirstTokenMicros\":%llu}",
                request == 0 ? "" : ",",
                request + 1,
                static_cast<unsigned long long>(observation.latency_micros),
                static_cast<unsigned long long>(observation.ttft_micros)) <= 0) {
            return false;
        }
    }
    return std::printf("]}\n") > 0 && std::fflush(stdout) == 0;
}

} // namespace

int main(int argc, char ** argv) {
    Args args;
    if (!parse_args(argc, argv, args) || !validate_args(args)) {
        std::fputs("invalid bounded llama.cpp helper arguments\n", stderr);
        return kUsageExit;
    }

    // b10133's explicit-directory loader also honors GGML_BACKEND_PATH after
    // loading the known backends. Direct helper invocation must not permit that
    // ambient variable to escape the compiled package-owned backend directory.
    if (::unsetenv("GGML_BACKEND_PATH") != 0) {
        std::fputs("could not neutralize backend redirection\n", stderr);
        return kModelInitExit;
    }
#ifdef BENCHPLANE_TARGET_NVIDIA_CUDA
    // The supported parent supplies an empty environment. Preserve fixed
    // logical-device selection for direct helper invocation as well: these
    // CUDA/GGML inputs can hide, reorder, or remap devices before the first
    // CUDA/backend call.
    if (::unsetenv("CUDA_VISIBLE_DEVICES") != 0 || ::unsetenv("CUDA_DEVICE_ORDER") != 0 ||
        ::unsetenv("GGML_CUDA_DEVICES") != 0) {
        std::fputs("could not neutralize CUDA device redirection\n", stderr);
        return kModelInitExit;
    }
    const std::string nvidia_driver_version = bounded_driver_version();
    if (!nvidia_driver_version_supported(nvidia_driver_version)) {
        std::fputs("host NVIDIA driver must be 575.57.08 or newer\n", stderr);
        return kModelInitExit;
    }
#endif
    ggml_backend_load_all_from_path(BENCHPLANE_BACKEND_PATH);
    llama_model_params model_params = llama_model_default_params();
#ifdef BENCHPLANE_TARGET_NVIDIA_CUDA
    if (cudaSetDevice(0) != cudaSuccess) {
        std::fputs("could not select fixed CUDA logical device 0\n", stderr);
        return kModelInitExit;
    }
    int device_count = 0;
    cudaDeviceProp device_properties{};
    int cuda_driver_version = 0;
    int cuda_runtime_version = 0;
    if (cudaGetDeviceCount(&device_count) != cudaSuccess || device_count < 1 ||
        cudaGetDeviceProperties(&device_properties, 0) != cudaSuccess ||
        cudaDriverGetVersion(&cuda_driver_version) != cudaSuccess ||
        cudaRuntimeGetVersion(&cuda_runtime_version) != cudaSuccess) {
        std::fputs("could not inspect fixed CUDA logical device 0\n", stderr);
        return kModelInitExit;
    }
    ggml_backend_dev_t cuda_device = ggml_backend_dev_by_name("CUDA0");
    if (cuda_device == nullptr ||
        ggml_backend_dev_type(cuda_device) != GGML_BACKEND_DEVICE_TYPE_GPU ||
        std::string_view(ggml_backend_reg_name(ggml_backend_dev_backend_reg(cuda_device))) != "CUDA") {
        std::fputs("packaged CUDA backend did not expose logical device CUDA0\n", stderr);
        return kModelInitExit;
    }
    ggml_backend_dev_t devices[] = {cuda_device, nullptr};
    model_params.devices = devices;
    model_params.n_gpu_layers = std::numeric_limits<int32_t>::max();
    model_params.split_mode = LLAMA_SPLIT_MODE_NONE;
    model_params.main_gpu = 0;
    OffloadObservation offload;
    llama_log_set(observe_llama_log, &offload);
#else
    model_params.n_gpu_layers = 0;
#endif
    llama_model * model = llama_model_load_from_file(BENCHPLANE_MODEL_PATH, model_params);
    if (model == nullptr) {
        std::fputs("could not initialize fixed packaged model\n", stderr);
        return kModelInitExit;
    }
#ifdef BENCHPLANE_TARGET_NVIDIA_CUDA
    const uint32_t observed_offloaded = offload.offloaded.load();
    const uint32_t observed_total = offload.total.load();
    const uint32_t expected_total = static_cast<uint32_t>(llama_model_n_layer(model)) + 1;
    if (observed_offloaded != expected_total || observed_total != expected_total ||
        !emit_nvidia_metadata(
            device_properties,
            cuda_driver_version,
            cuda_runtime_version,
            nvidia_driver_version,
            observed_offloaded,
            observed_total)) {
        llama_model_free(model);
        std::fputs("fixed model was not fully offloaded to CUDA logical device 0\n", stderr);
        return kModelInitExit;
    }
#endif
    const llama_vocab * vocab = llama_model_get_vocab(model);
    if (vocab == nullptr) {
        llama_model_free(model);
        std::fputs("fixed packaged model has no vocabulary\n", stderr);
        return kModelInitExit;
    }

    for (uint32_t index = 1; index <= args.warmup_runs; ++index) {
        if (!emit_repetition(model, vocab, args, "warmup", index)) {
            llama_model_free(model);
            std::fputs("llama.cpp inference failed\n", stderr);
            return kInferenceExit;
        }
    }
    for (uint32_t index = 1; index <= args.repetitions; ++index) {
        if (!emit_repetition(model, vocab, args, "measured", index)) {
            llama_model_free(model);
            std::fputs("llama.cpp inference failed\n", stderr);
            return kInferenceExit;
        }
    }
    llama_model_free(model);
    return 0;
}
