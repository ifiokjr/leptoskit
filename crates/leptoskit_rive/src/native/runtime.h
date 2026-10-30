#pragma once
#include <cstddef>
#include <cstdint>

// All pointers in draw commands are borrowed only for the callback.
struct LkPath {
    uint64_t id;
    uint64_t revision;
    const uint8_t* verbs;
    size_t verb_count;
    const float* points;
    size_t point_count;
    uint32_t fill_rule;
};

struct LkPaint {
    uint32_t color;
    uint32_t style;
    float thickness;
    uint32_t join;
    uint32_t cap;
    uint32_t blend;
    uint32_t gradient;
    float coordinates[5];
    const uint32_t* colors;
    const float* stops;
    size_t stop_count;
    float shader_matrix[6];
};

struct LkCommand {
    uint32_t operation;
    float matrix[6];
    const LkPath* path;
    const LkPaint* paint;
    float opacity;
    uint64_t released_path;
};

using LkEmit = void (*)(void*, const LkCommand*);
struct LkRive;
extern "C" {
LkRive* lk_rive_new(const uint8_t*, size_t, const char*, size_t, uint32_t,
                    const char*, size_t, void*, LkEmit, uint32_t*) noexcept;
void lk_rive_delete(LkRive*) noexcept;
uint32_t lk_rive_draw(LkRive*, float, uint32_t, uint32_t, uint32_t*) noexcept;
uint32_t lk_rive_input(LkRive*, const char*, size_t, uint32_t, float, bool) noexcept;
}
