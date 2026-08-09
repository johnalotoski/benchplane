// SPDX-License-Identifier: Apache-2.0

#include "llama.h"

#include <algorithm>
#include <charconv>
#include <chrono>
#include <cstdint>
#include <cstdlib>
#include <cstdio>
#include <limits>
#include <string>
#include <string_view>
#include <vector>

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
    ggml_backend_load_all_from_path(BENCHPLANE_BACKEND_PATH);
    llama_model_params model_params = llama_model_default_params();
    model_params.n_gpu_layers = 0;
    llama_model * model = llama_model_load_from_file(BENCHPLANE_MODEL_PATH, model_params);
    if (model == nullptr) {
        std::fputs("could not initialize fixed packaged model\n", stderr);
        return kModelInitExit;
    }
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
