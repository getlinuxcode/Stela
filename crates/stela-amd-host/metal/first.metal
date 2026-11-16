// SPDX-License-Identifier: GPL-2.0-or-later
// The first Metal work stela-amd-host --device runs on the GPU: a buffer sum
// and a triangle. Compiled on a Mac with
//   xcrun -sdk macosx metal -o first.metallib first.metal
#include <metal_stdlib>
using namespace metal;

kernel void add(device const float *a [[buffer(0)]],
                device const float *b [[buffer(1)]],
                device float *sum [[buffer(2)]],
                uint index [[thread_position_in_grid]])
{
    sum[index] = a[index] + b[index];
}

struct Varyings {
    float4 position [[position]];
    float4 color;
};

vertex Varyings triangle_vertex(uint vertex_id [[vertex_id]])
{
    const float2 positions[3] = {float2(0.0, 0.75), float2(-0.75, -0.75), float2(0.75, -0.75)};
    const float4 colors[3] = {float4(1, 0, 0, 1), float4(0, 1, 0, 1), float4(0, 0, 1, 1)};
    Varyings out;
    out.position = float4(positions[vertex_id], 0.0, 1.0);
    out.color = colors[vertex_id];
    return out;
}

fragment float4 triangle_fragment(Varyings in [[stage_in]])
{
    return in.color;
}
