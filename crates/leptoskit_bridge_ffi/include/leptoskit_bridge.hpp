#ifndef LEPTOSKIT_BRIDGE_HPP
#define LEPTOSKIT_BRIDGE_HPP

#include "leptoskit_bridge.h"
#include <span>
#include <utility>

namespace leptoskit {
/* C++20/NDK view over Rust-owned memory. Move-only ownership prevents double
 * release; data stays borrowed until this object is destroyed. */
class buffer final {
    lk_buffer value_{};
public:
    explicit buffer(lk_buffer value) noexcept : value_(value) {}
    buffer(const buffer&) = delete;
    buffer& operator=(const buffer&) = delete;
    buffer(buffer&& other) noexcept : value_(std::exchange(other.value_, {})) {}
    buffer& operator=(buffer&& other) noexcept {
        if (this != &other) {
            lk_buffer_release(value_);
            value_ = std::exchange(other.value_, {});
        }

        return *this;
    }
    ~buffer() { lk_buffer_release(value_); }

    [[nodiscard]] std::span<const uint8_t> bytes() const noexcept {
        return {value_.data, value_.len};
    }
    /* Consumes this owner on all return paths. Any previous span is invalid. */
    [[nodiscard]] int32_t complete(lk_bridge* bridge, uint64_t id) && noexcept {
        return lk_bridge_complete_owned(bridge, id, std::exchange(value_, {}));
    }
};
}
#endif
