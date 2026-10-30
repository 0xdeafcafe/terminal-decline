#include <metal_stdlib>
using namespace metal;

struct Uniforms {
    float2 viewport; // drawable size in pixels
    float2 origin;   // the pane's top-left corner in the drawable, pixels
};

// The instance structs below must match `RectInstance` and `GlyphInstance`
// in frame.rs; const asserts there check their sizes and field offsets.

// Solid rectangles: cell backgrounds (merged into runs), underlines,
// strikethrough, cursor.
struct Rect {
    short2 pos;
    ushort2 size;
    uint color; // RGBA8, red in the low byte
};

// A theme transition (transition.rs, `Uniforms`): the old and new
// theme's colours, how far along it is and how it moves. `count` is 0
// while none plays.
#define SLOTS 24
struct Blend {
    float progress;
    uint kind; // 0 none, 1 fade, 2 wave, 3 pulse, 4 glimmer
    float2 viewport;
    uint count;
    uint pad[3];
    uint old_slots[SLOTS];
    uint new_slots[SLOTS];
};

// An instance colour as the transition draws it: from the old theme's
// colour in the slot whose new colour it equals (RGB only; the first
// match), else unchanged. `matched` says whether it moves at all.
struct Blended {
    float4 old_colour;
    float4 new_colour;
    float matched;
};

static Blended blended(uint packed, constant Blend &b) {
    float4 c = unpack_unorm4x8_to_float(packed);
    Blended o = { c, c, 0.0 };
    uint rgb = packed & 0x00FFFFFFu;
    for (uint k = 0; k < b.count; k++) {
        if ((b.new_slots[k] & 0x00FFFFFFu) == rgb) {
            float4 old = unpack_unorm4x8_to_float(b.old_slots[k]);
            o.old_colour = float4(old.rgb, c.a);
            o.matched = 1.0;
            break;
        }
    }
    return o;
}

// Mirrors `ease`, `noise`, `local` and `lift` in transition.rs.
static float ease(float t) {
    t = clamp(t, 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

static float block_noise(uint bx, uint by) {
    uint h = bx * 374761393u + by * 668265263u;
    h = (h ^ (h >> 13)) * 1274126177u;
    h ^= h >> 16;
    return float(h & 0x00FFFFFFu) / 16777216.0;
}

static float4 transition_colour(float4 old_colour, float4 new_colour, float matched, float2 pixel, constant Blend &b) {
    if (b.kind == 0u || matched == 0.0) {
        return new_colour;
    }
    float p = b.progress;
    float2 at = pixel / max(b.viewport, float2(1.0));
    float d = (at.x + at.y) * 0.5;
    float t;
    float lift = 0.0;
    if (p >= 1.0) {
        t = 1.0;
    } else if (p <= 0.0) {
        t = 0.0;
    } else if (b.kind == 2u) {
        const float soft = 0.35;
        t = ease((p * (1.0 + soft) - d) / soft);
    } else if (b.kind == 4u) {
        const float turn = 0.4;
        uint2 block = uint2(pixel / 8.0);
        t = ease((p - block_noise(block.x, block.y) * (1.0 - turn)) / turn);
    } else {
        t = ease(p);
    }
    if (b.kind == 3u) {
        float k = (p - 0.5) / 0.18;
        lift = 0.22 * exp(-k * k);
    } else if (b.kind == 4u) {
        lift = 0.16 * sin(M_PI_F * p) * max(1.0 - abs(d - p) * 5.0, 0.0);
    }
    float3 rgb = mix(old_colour.rgb, new_colour.rgb, t);
    rgb += (1.0 - rgb) * lift;
    return float4(rgb, new_colour.a);
}

struct RectOut {
    float4 pos [[position]];
    float4 old_colour [[flat]];
    float4 new_colour [[flat]];
    float matched [[flat]];
};

static float4 to_clip(float2 p, float2 viewport) {
    return float4(p / viewport * float2(2.0, -2.0) + float2(-1.0, 1.0), 0.0, 1.0);
}

vertex RectOut rect_vertex(uint vid [[vertex_id]], uint iid [[instance_id]],
                           constant Rect *rects [[buffer(0)]],
                           constant Uniforms &u [[buffer(1)]],
                           constant Blend &b [[buffer(2)]]) {
    Rect r = rects[iid];
    float2 corner = float2(vid & 1, vid >> 1);
    RectOut o;
    o.pos = to_clip(u.origin + float2(r.pos) + corner * float2(r.size), u.viewport);
    Blended c = blended(r.color, b);
    o.old_colour = c.old_colour;
    o.new_colour = c.new_colour;
    o.matched = c.matched;
    return o;
}

fragment float4 rect_fragment(RectOut in [[stage_in]], constant Blend &b [[buffer(0)]]) {
    float4 c = transition_colour(in.old_colour, in.new_colour, in.matched, in.pos.xy, b);
    return float4(c.rgb * c.a, c.a);
}

struct Glyph {
    short2 pos;     // top-left of the bitmap, pixels
    ushort2 size;   // bitmap size
    ushort2 uv;     // top-left in the glyph texture
    uint color;     // RGBA8 text colour
    uint is_color;  // bit 0: sample the colour (emoji) texture; the rest is the CPU's
};

struct GlyphOut {
    float4 pos [[position]];
    float2 uv;
    float4 old_colour [[flat]];
    float4 new_colour [[flat]];
    float matched [[flat]];
    uint is_color [[flat]];
};

vertex GlyphOut glyph_vertex(uint vid [[vertex_id]], uint iid [[instance_id]],
                             constant Glyph *glyphs [[buffer(0)]],
                             constant Uniforms &u [[buffer(1)]],
                             constant Blend &b [[buffer(2)]]) {
    Glyph g = glyphs[iid];
    float2 corner = float2(vid & 1, vid >> 1);
    GlyphOut o;
    o.pos = to_clip(u.origin + float2(g.pos) + corner * float2(g.size), u.viewport);
    o.uv = float2(g.uv) + corner * float2(g.size);
    Blended c = blended(g.color, b);
    o.old_colour = c.old_colour;
    o.new_colour = c.new_colour;
    o.matched = c.matched;
    o.is_color = g.is_color & 1u;
    return o;
}

fragment float4 glyph_fragment(GlyphOut in [[stage_in]],
                               texture2d<float> mask [[texture(0)]],
                               texture2d<float> colour [[texture(1)]],
                               constant Blend &b [[buffer(0)]]) {
    constexpr sampler s(coord::pixel, filter::nearest, address::clamp_to_zero);
    if (in.is_color) {
        return colour.sample(s, in.uv) * in.new_colour.a; // already premultiplied
    }
    float4 c = transition_colour(in.old_colour, in.new_colour, in.matched, in.pos.xy, b);
    float a = mask.sample(s, in.uv).r * c.a;
    return float4(c.rgb * a, a);
}

// Pieces of images (images.md §6.3): `ImageInstance` in images.rs.
struct Image {
    short2 pos;
    short2 size;
    packed_float4 uv;    // texels at the left, top, right, bottom edges
    packed_float4 clamp; // samples stay inside these texels
    float scale;  // texels per pixel
    uint pad;
};

struct ImageOut {
    float4 pos [[position]];
    float2 uv;
    float4 clamp [[flat]];
    float2 step [[flat]];
};

vertex ImageOut image_vertex(uint vid [[vertex_id]], uint iid [[instance_id]],
                             constant Image *images [[buffer(0)]],
                             constant Uniforms &u [[buffer(1)]]) {
    Image im = images[iid];
    float2 corner = float2(vid & 1, vid >> 1);
    ImageOut o;
    o.pos = to_clip(u.origin + float2(im.pos) + corner * float2(im.size), u.viewport);
    o.uv = mix(im.uv.xy, im.uv.zw, corner);
    o.clamp = im.clamp;
    // A quarter of a pixel's footprint in texels, for the 2x2 box when the
    // image is drawn at less than half size; 0 otherwise.
    float2 texels = abs(im.uv.zw - im.uv.xy) / max(float2(im.size), float2(1.0));
    o.step = im.scale > 2.0 ? texels * 0.25 : float2(0.0);
    return o;
}

fragment float4 image_fragment(ImageOut in [[stage_in]], texture2d<float> image [[texture(0)]]) {
    constexpr sampler s(coord::pixel, filter::linear, address::clamp_to_edge);
    float2 lo = in.clamp.xy, hi = in.clamp.zw;
    if (in.step.x == 0.0 && in.step.y == 0.0) {
        return image.sample(s, clamp(in.uv, lo, hi)); // premultiplied BGRA
    }
    float4 c = image.sample(s, clamp(in.uv + float2(-in.step.x, -in.step.y), lo, hi));
    c += image.sample(s, clamp(in.uv + float2(in.step.x, -in.step.y), lo, hi));
    c += image.sample(s, clamp(in.uv + float2(-in.step.x, in.step.y), lo, hi));
    c += image.sample(s, clamp(in.uv + float2(in.step.x, in.step.y), lo, hi));
    return c * 0.25;
}
