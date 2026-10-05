// Rendu du champ dans le fragment shader, sans géométrie : lignes de champ
// (isolignes de Az) et carte de |B| en échelle logarithmique.

struct U {
    center: vec2<f32>,   // centre de la vue (m)
    half: vec2<f32>,     // demi-étendue de la vue (m)
    size: f32,           // côté du domaine (m)
    n: u32,              // cellules par côté
    delta_a: f32,        // pas entre deux lignes (T·m)
    b_max: f32,          // haut de l'échelle de couleurs (T)
    flags: u32,          // 1 lignes, 2 carte, 4 cible sRGB
    line_px: f32,
    pad: vec2<f32>,
}

@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var<storage, read> az: array<f32>;

struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) world: vec2<f32>,
}

@vertex
fn vs(@builtin(vertex_index) vi: u32) -> VOut {
    let p = vec2<f32>(f32((vi << 1u) & 2u), f32(vi & 2u)) * 2.0 - 1.0;
    var o: VOut;
    o.pos = vec4<f32>(p, 0.0, 1.0);
    o.world = u.center + p * u.half;
    return o;
}

fn node(i: u32, j: u32) -> f32 {
    return az[j * (u.n + 1u) + i];
}

// Palette magma (ajustement polynomial de la palette matplotlib).
fn magma(t: f32) -> vec3<f32> {
    let c0 = vec3<f32>(-0.002136485, -0.000749655, -0.005386128);
    let c1 = vec3<f32>(0.2516605, 0.6775232, 2.494027);
    let c2 = vec3<f32>(8.353717, -3.577720, 0.3144679);
    let c3 = vec3<f32>(-27.66873, 14.26473, -13.64921);
    let c4 = vec3<f32>(52.17614, -27.94361, 12.94417);
    let c5 = vec3<f32>(-50.76853, 29.04658, 4.234153);
    let c6 = vec3<f32>(18.65571, -11.48977, -5.601962);
    return clamp(c0 + t * (c1 + t * (c2 + t * (c3 + t * (c4 + t * (c5 + t * c6))))), vec3<f32>(0.0), vec3<f32>(1.0));
}

@fragment
fn fs(in: VOut) -> @location(0) vec4<f32> {
    let bg = vec3<f32>(0.055, 0.067, 0.086);
    let nf = f32(u.n);
    let h = u.size / nf;
    let g_raw = (in.world + 0.5 * u.size) / h;
    let inside = all(g_raw >= vec2<f32>(0.0)) && all(g_raw <= vec2<f32>(nf));
    let g = clamp(g_raw, vec2<f32>(0.0), vec2<f32>(nf));
    let ci = min(u32(g.x), u.n - 1u);
    let cj = min(u32(g.y), u.n - 1u);
    let f = g - vec2<f32>(f32(ci), f32(cj));
    let a00 = node(ci, cj);
    let a10 = node(ci + 1u, cj);
    let a01 = node(ci, cj + 1u);
    let a11 = node(ci + 1u, cj + 1u);
    let a = mix(mix(a00, a10, f.x), mix(a01, a11, f.x), f.y);
    let b = vec2<f32>(mix(a01 - a00, a11 - a10, f.x), -mix(a10 - a00, a11 - a01, f.y)) / h;

    var color = bg;
    if ((u.flags & 2u) != 0u) {
        // Trois décades sous b_max.
        let t = 1.0 + log(max(length(b), 1e-12) / u.b_max) / log(1000.0);
        color = magma(clamp(t, 0.0, 1.0));
    }
    if ((u.flags & 1u) != 0u) {
        // delta_a constant : la densité des lignes est proportionnelle à |B|.
        let t = a / u.delta_a;
        let dist_px = abs(fract(t + 0.5) - 0.5) / max(fwidth(t), 1e-9);
        let line = 1.0 - smoothstep(0.0, u.line_px, dist_px);
        color = mix(color, vec3<f32>(0.863, 0.922, 1.0), line * 0.8);
    }
    color = select(bg * 0.55, color, inside);
    if ((u.flags & 4u) != 0u) {
        color = pow(color, vec3<f32>(2.2));
    }
    return vec4<f32>(color, 1.0);
}
