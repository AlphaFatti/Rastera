struct Clock { seconds: f32, pad0: f32, pad1: f32, pad2: f32 }
@group(0) @binding(0) var<uniform> clock: Clock;
@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {
    var p = array<vec2f,3>(vec2f(-1,-1), vec2f(3,-1), vec2f(-1,3));
    return vec4f(p[i],0,1);
}
@fragment fn fs(@builtin(position) p: vec4f) -> @location(0) vec4f {
    var color = select(vec3f(0.025,0.18,0.45), vec3f(0.5,0.09,0.015), p.x >= 1920.0);
    if (u32(p.x) % 120u < 2u || u32(p.y) % 120u < 2u) { color = vec3f(0.65); }
    // Distinct single-pixel seam columns, at pixel centers 1919.5 and 1920.5.
    if (u32(p.x) == 1919u) { color = vec3f(0,1,0); }
    if (u32(p.x) == 1920u) { color = vec3f(1,0,1); }
    let local_x = p.x % 1920.0;
    let marker = 80.0 + (clock.seconds * 300.0) % 1760.0;
    if (abs(local_x-marker)<18.0 && abs(p.y-540.0)<100.0) { color = vec3f(1,1,0); }
    // One white block on left, two on right, baked into the canvas.
    if (p.y>40.0 && p.y<90.0 && (local_x>40.0 && local_x<90.0 || p.x>=1920.0 && local_x>110.0 && local_x<160.0)) { color=vec3f(1); }
    return vec4f(color,1);
}
