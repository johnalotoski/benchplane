// SPDX-License-Identifier: Apache-2.0

#include <unistd.h>

__attribute__((constructor)) static void fail_if_loaded(void) {
    _exit(86);
}
