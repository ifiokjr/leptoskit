#include "runtime.h"
#include "rive/artboard.hpp"
#include "rive/file.hpp"
#include "rive/factory.hpp"
#include "rive/animation/linear_animation_instance.hpp"
#include "rive/animation/state_machine_instance.hpp"
#include "rive/animation/state_machine_input_instance.hpp"
#include "rive/animation/state_machine_bool.hpp"

#include "rive/animation/state_machine_number.hpp"
#include "rive/animation/state_machine_trigger.hpp"
#include "rive/layout_component.hpp"
#include "rive/text/text.hpp"
#include "rive/scripted/scripted_object.hpp"
#include <cmath>
#include <cstring>
#include <memory>
#include <string>
#include <unordered_map>
#include <vector>

namespace {
using namespace rive;
struct Context {
    void* rust;
    LkEmit emit;
    uint64_t next_path = 1;
    bool unsupported = false;
    void send(uint32_t op, const LkPath* path = nullptr,
              const LkPaint* paint = nullptr, const Mat2D* matrix = nullptr,
              float opacity = 1, uint64_t released = 0) {
        LkCommand command{};
        command.operation = op;
        command.path = path;
        command.paint = paint;
        command.opacity = opacity;
        command.released_path = released;

        if (matrix) std::memcpy(command.matrix, matrix->values(), sizeof(command.matrix));
        emit(rust, &command);
    }
};

class Path final : public RenderPath {
    Context& context;
    const uint64_t id;
    uint64_t revision = 0;
    uint32_t rule = 0;
    RawPath path;
public:
    explicit Path(Context& c) : context(c), id(c.next_path++) {}
    ~Path() override { context.send(6, nullptr, nullptr, nullptr, 1, id); }
    void rewind() override { path.rewind(); ++revision; }
    void fillRule(FillRule r) override { rule = static_cast<uint32_t>(r); }
    void moveTo(float x, float y) override { path.moveTo(x, y); ++revision; }
    void lineTo(float x, float y) override { path.lineTo(x, y); ++revision; }
    void cubicTo(float a, float b, float c, float d, float e, float f) override {
        path.cubicTo(a, b, c, d, e, f); ++revision;
    }
    void close() override { path.close(); ++revision; }
    void addRenderPath(const RenderPath* other, const Mat2D& matrix) override {
        path.addPath(static_cast<const Path*>(other)->path, &matrix); ++revision;
    }
    void addRenderPathBackwards(const RenderPath* other, const Mat2D& matrix) override {
        path.addPathBackwards(static_cast<const Path*>(other)->path, &matrix); ++revision;
    }
    void addRawPath(const RawPath& other) override { path.addPath(other); ++revision; }
    LkPath view() const {
        auto verbs = path.verbsU8();
        auto points = path.points();
        static_assert(sizeof(Vec2D) == sizeof(float) * 2);

        return {id, revision, verbs.data(), verbs.size(),
                reinterpret_cast<const float*>(points.data()), points.size() * 2, rule};
    }
};

class Gradient final : public RenderShader {
public:
    uint32_t kind;
    float coordinates[5];
    std::vector<ColorInt> colors;
    std::vector<float> stops;
    Gradient(uint32_t k, float a, float b, float c, float d, float e,
             const ColorInt* color, const float* stop, size_t count)
        : kind(k), coordinates{a,b,c,d,e}, colors(color, color + count), stops(stop, stop + count) {}
};

class Paint final : public RenderPaint {
    Context& context;
    rcp<RenderShader> gradient;
    Mat2D matrix;
public:
    LkPaint values{};
    explicit Paint(Context& c) : context(c) {
        values.blend = 3; values.thickness = 1;
    }
    void style(RenderPaintStyle s) override { values.style = static_cast<uint32_t>(s); }
    void color(ColorInt c) override { values.color = c; }
    void thickness(float v) override { values.thickness = v; }
    void join(StrokeJoin v) override { values.join = static_cast<uint32_t>(v); }
    void cap(StrokeCap v) override { values.cap = static_cast<uint32_t>(v); }
    void blendMode(BlendMode v) override { values.blend = static_cast<uint32_t>(v); }

    void feather(float v) override { if (v != 0) context.unsupported = true; }
    void strokePosition(StrokePosition v) override {
        if (v != StrokePosition::center) context.unsupported = true;
    }

    void additiveness(float v) override { if (v != 0) context.unsupported = true; }
    void shader(rcp<RenderShader> v) override { gradient = std::move(v); }
    void shaderTransform(const Mat2D& v) override { matrix = v; }
    void modulatedImage(const RenderImage*, ImageSampler, const Mat2D&) override {
        context.unsupported = true;
    }
    void invalidateStroke() override {}
    LkPaint view() const {
        LkPaint result = values;
        std::memcpy(result.shader_matrix, matrix.values(), sizeof(result.shader_matrix));

        if (gradient) {
            auto* g = static_cast<Gradient*>(gradient.get());
            result.gradient = g->kind;
            std::memcpy(result.coordinates, g->coordinates, sizeof(result.coordinates));
            result.colors = g->colors.data(); result.stops = g->stops.data();
            result.stop_count = g->colors.size();
        }

        return result;
    }
};

class NativeFactory final : public rive::Factory {
    Context& context;
public:
    explicit NativeFactory(Context& c) : context(c) {}
    rcp<RenderBuffer> makeRenderBuffer(RenderBufferType, RenderBufferFlags, size_t) override {
        context.unsupported = true; return nullptr;
    }
    rcp<RenderShader> makeLinearGradient(float a,float b,float c,float d,
        const ColorInt* colors,const float* stops,size_t n) override {
        if (n == 0 || n > 4096) { context.unsupported = true; return nullptr; }
        return make_rcp<Gradient>(1,a,b,c,d,0,colors,stops,n);
    }
    rcp<RenderShader> makeRadialGradient(float a,float b,float r,
        const ColorInt* colors,const float* stops,size_t n) override {
        if (n == 0 || n > 4096) { context.unsupported = true; return nullptr; }
        return make_rcp<Gradient>(2,a,b,0,0,r,colors,stops,n);
    }
    rcp<RenderShader> makeAngularGradient(const ColorInt*,const float*,size_t) override {
        context.unsupported = true; return nullptr;
    }
    rcp<RenderPath> makeRenderPath(RawPath& raw, FillRule rule) override {
        auto path = make_rcp<Path>(context); path->addRawPath(raw); path->fillRule(rule);

        return path;
    }

    rcp<RenderPath> makeEmptyRenderPath() override { return make_rcp<Path>(context); }
    rcp<RenderPaint> makeRenderPaint() override { return make_rcp<Paint>(context); }
    rcp<RenderImage> decodeImage(Span<const uint8_t>) override {
        context.unsupported = true; return nullptr;
    }
};

class NativeRenderer final : public rive::Renderer {
    Context& context;
public:
    explicit NativeRenderer(Context& c) : context(c) {}
    void save() override { context.send(0); }
    void restore() override { context.send(1); }
    void transform(const Mat2D& m) override { context.send(2,nullptr,nullptr,&m); }
    void drawPath(RenderPath* raw, RenderPaint* paint) override {
        auto path = static_cast<Path*>(raw)->view();
        auto values = static_cast<Paint*>(paint)->view();
        context.send(4,&path,&values);
    }
    void clipPath(RenderPath* raw) override {
        auto path = static_cast<Path*>(raw)->view(); context.send(3,&path);
    }
    void clipStroke(RenderPath*, const StrokeParams&) override { context.unsupported = true; }
    void applyLayerMask(const RenderImage*, ImageSampler, LayerMaskMode) override {
        context.unsupported = true;
    }
    void modulateOpacity(float v) override { context.send(5,nullptr,nullptr,nullptr,v); }
    void modulateColor(ColorInt, bool) override { context.unsupported = true; }
    void drawImage(const RenderImage*,ImageSampler,BlendMode,float) override {
        context.unsupported = true;
    }
    void drawImageMesh(const RenderImage*,ImageSampler,rcp<RenderBuffer>,rcp<RenderBuffer>,
                      rcp<RenderBuffer>,uint32_t,uint32_t,BlendMode,float) override {
        context.unsupported = true;
    }
};
}

struct LkRive {
    // Destruction is reverse declaration order: context/factory outlive every path.
    Context context;
    NativeFactory factory;
    rive::rcp<rive::File> file;
    std::unique_ptr<rive::ArtboardInstance> artboard;
    std::unique_ptr<rive::Scene> scene;
    rive::StateMachineInstance* machine = nullptr;
    std::unordered_map<std::string, rive::SMIInput*> inputs;
    explicit LkRive(void* rust, LkEmit emit) : context{rust,emit}, factory(context) {}
};

extern "C" LkRive* lk_rive_new(const uint8_t* bytes,size_t size,const char* board,size_t board_len,
    uint32_t scene_kind,const char* scene,size_t scene_len,void* rust,LkEmit emit,uint32_t* error) noexcept {
    *error = 1;
    try {
        auto player = std::make_unique<LkRive>(rust,emit);
        player->file = rive::File::import({bytes,size},&player->factory,nullptr,nullptr,nullptr,true);

        if (!player->file) return nullptr;

        if (!player->file->assets().empty() || player->file->viewModelCount() != 0) {
            *error = 3; return nullptr;
        }

        player->artboard = board_len ? player->file->artboardNamed(std::string(board,board_len))
                                      : player->file->artboardDefault();

        if (!player->artboard) { *error = 2; return nullptr; }
        // Inspect every source artboard: a nested artboard is instantiated later.
        for (size_t index = 0; index < player->file->artboardCount(); ++index) {

          for (auto* object : player->file->artboard(index)->objects()) {
            if (object && (object->is<rive::Text>() || rive::ScriptedObject::from(object) ||
                (object->is<rive::LayoutComponent>() && !object->is<rive::Artboard>()))) {
                *error = 3; return nullptr;
            }
          }
        }

        if (scene_kind == 2) {
            auto machine = player->artboard->stateMachineNamed(std::string(scene,scene_len));
            player->machine = machine.get(); player->scene = std::move(machine);
        } else if (scene_kind == 1) {
            player->scene = player->artboard->animationNamed(std::string(scene,scene_len));
        } else {
            player->scene = player->artboard->animationAt(0);
        }

        if (scene_kind && !player->scene) { *error = 2; return nullptr; }

        if (player->machine) {
            for (size_t i=0; i<player->machine->inputCount(); ++i) {
                auto* input = player->machine->input(i);

                if (!input) { *error = 3; return nullptr; }

                if (!player->inputs.emplace(input->name(),input).second) {
                    *error = 4; return nullptr;
                }
            }
        }

        if (player->context.unsupported) { *error = 3; return nullptr; }
        *error = 0;

        return player.release();
    } catch (...) { *error = 6; return nullptr; }
}

extern "C" void lk_rive_delete(LkRive* player) noexcept { delete player; }

extern "C" uint32_t lk_rive_draw(LkRive* player,float elapsed,uint32_t width,uint32_t height,uint32_t* more) noexcept {
    try {
        *more = player->scene ? player->scene->advanceAndApply(elapsed)
                              : player->artboard->advance(elapsed);
        auto bounds = player->artboard->bounds();
        if (!std::isfinite(bounds.width()) || !std::isfinite(bounds.height()) ||
            bounds.width() <= 0 || bounds.height() <= 0) return 1;
        NativeRenderer renderer(player->context);
        renderer.align(rive::Fit::contain,rive::Alignment::center,
                       rive::AABB(0,0,float(width),float(height)),bounds);
        player->artboard->draw(&renderer);

        return player->context.unsupported ? 3 : 0;
    } catch (...) { return 6; }
}

extern "C" uint32_t lk_rive_input(LkRive* player,const char* name,size_t size,uint32_t kind,float value,bool apply) noexcept {
    try {
        if (!player->machine) return 4;
        std::string key(name,size);
        auto found = player->inputs.find(key);

        if (found == player->inputs.end()) return 4;

        if (kind == 0) {
            if (!found->second->input()->is<rive::StateMachineBool>()) return 4;
            auto* input = static_cast<rive::SMIBool*>(found->second);

            if (apply) input->value(value != 0);
        } else if (kind == 1) {
            if (!found->second->input()->is<rive::StateMachineNumber>()) return 4;
            auto* input = static_cast<rive::SMINumber*>(found->second);

            if (apply) input->value(value);
        } else {
            if (!found->second->input()->is<rive::StateMachineTrigger>()) return 4;
            auto* input = static_cast<rive::SMITrigger*>(found->second);

            if (apply) input->fire();
        }

        return 0;
    } catch (...) { return 6; }
}
