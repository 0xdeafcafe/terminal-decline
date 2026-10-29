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

struct RectOut {
    float4 pos [[position]];
    float4 color;
};

static float4 to_clip(float2 p, float2 viewport) {
    return float4(p / viewport * float2(2.0, -2.0) + float2(-1.0, 1.0), 0.0, 1.0);
}

vertex RectOut rect_vertex(uint vid [[vertex_id]], uint iid [[instance_id]],
                           constant Rect *rects [[buffer(0)]],
                           constant Uniforms &u [[buffer(1)]]) {
    Rect r = rects[iid];
    float2 corner = float2(vid & 1, vid >> 1);
    RectOut o;
    o.pos = to_clip(u.origin + float2(r.pos) + corner * float2(r.size), u.viewport);
    float4 c = unpack_unorm4x8_to_float(r.color);
    o.color = float4(c.rgb * c.a, c.a);
    return o;
}

fragment float4 rect_fragment(RectOut in [[stage_in]]) {
    return in.color;
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
    float4 color;
    uint is_color [[flat]];
};

vertex GlyphOut glyph_vertex(uint vid [[vertex_id]], uint iid [[instance_id]],
                             constant Glyph *glyphs [[buffer(0)]],
                             constant Uniforms &u [[buffer(1)]]) {
    Glyph g = glyphs[iid];
    float2 corner = float2(vid & 1, vid >> 1);
    GlyphOut o;
    o.pos = to_clip(u.origin + float2(g.pos) + corner * float2(g.size), u.viewport);
    o.uv = float2(g.uv) + corner * float2(g.size);
    o.color = unpack_unorm4x8_to_float(g.color);
    o.is_color = g.is_color & 1u;
    return o;
}

fragment float4 glyph_fragment(GlyphOut in [[stage_in]],
                               texture2d<float> mask [[texture(0)]],
                               texture2d<float> colour [[texture(1)]]) {
    constexpr sampler s(coord::pixel, filter::nearest, address::clamp_to_zero);
    if (in.is_color) {
        return colour.sample(s, in.uv) * in.color.a; // already premultiplied
    }
    float a = mask.sample(s, in.uv).r * in.color.a;
    return float4(in.color.rgb * a, a);
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
