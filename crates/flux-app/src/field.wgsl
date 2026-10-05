// Rendu du champ dans le fragment shader, sans géométrie : lignes de champ
// (isolignes de Az), carte de |B| en échelle logarithmique, LIC et limaille de fer.

struct U {
    center: vec2<f32>,   // centre de la vue (m)
    half: vec2<f32>,     // demi-étendue de la vue (m)
    size: f32,           // côté du domaine (m)
    n: u32,              // cellules par côté
    delta_a: f32,        // pas entre deux lignes (T·m)
    b_max: f32,          // haut de l'échelle de couleurs (T)
    flags: u32,          // 1 lignes, 2 carte, 4 cible sRGB, 8 LIC, 16 limaille, 32 vue scindée, 64 différence, 128 LIC animée
    line_px: f32,
    px: f32,             // taille d'un pixel physique (m)
    time: f32,           // phase d'animation dans [0, 1)
    grain: f32,          // maille de la limaille (m)
    split: f32,          // abscisse de la séparation avant/après (m)
    pad: vec2<f32>,
}

@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var<storage, read> az: array<f32>;
// Champ de référence figé, pour la comparaison avant/après.
@group(0) @binding(2) var<storage, read> az_ref: array<f32>;

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

// (Az, Bx, By) en un point, par interpolation bilinéaire du potentiel courant ou de référence.
fn sample_raw(world: vec2<f32>, use_ref: bool) -> vec3<f32> {
    let nf = f32(u.n);
    let h = u.size / nf;
    let g = clamp((world + 0.5 * u.size) / h, vec2<f32>(0.0), vec2<f32>(nf));
    let ci = min(u32(g.x), u.n - 1u);
    let cj = min(u32(g.y), u.n - 1u);
    let f = g - vec2<f32>(f32(ci), f32(cj));
    let k = cj * (u.n + 1u) + ci;
    var a00: f32;
    var a10: f32;
    var a01: f32;
    var a11: f32;
    if (use_ref) {
        a00 = az_ref[k];
        a10 = az_ref[k + 1u];
        a01 = az_ref[k + u.n + 1u];
        a11 = az_ref[k + u.n + 2u];
    } else {
        a00 = az[k];
        a10 = az[k + 1u];
        a01 = az[k + u.n + 1u];
        a11 = az[k + u.n + 2u];
    }
    let a = mix(mix(a00, a10, f.x), mix(a01, a11, f.x), f.y);
    let b = vec2<f32>(mix(a01 - a00, a11 - a10, f.x), -mix(a10 - a00, a11 - a01, f.y)) / h;
    return vec3<f32>(a, b);
}

// Champ affiché : courant, référence à gauche de la séparation, ou différence des deux.
fn sample_shown(world: vec2<f32>) -> vec3<f32> {
    if ((u.flags & 64u) != 0u) {
        return sample_raw(world, false) - sample_raw(world, true);
    }
    return sample_raw(world, (u.flags & 32u) != 0u && world.x < u.split);
}

fn hash21(p: vec2<f32>) -> f32 {
    let q = fract(p * vec2<f32>(0.1031, 0.1030));
    let r = q + dot(q, q.yx + 33.33);
    return fract((r.x + r.y) * r.x);
}

// Convolution intégrale linéaire : un bruit blanc moyenné le long de la ligne de champ.
// En mode animé, le noyau porte une ondulation qui glisse dans le sens de B.
fn lic_value(world: vec2<f32>) -> f32 {
    let steps = 14;
    let ds = u.px * 1.5;
    var acc = 0.0;
    var wsum = 0.0;
    for (var dir = -1.0; dir <= 1.0; dir += 2.0) {
        var p = world;
        for (var k = 0; k < steps; k++) {
            let b = sample_shown(p).yz;
            let l = length(b);
            if (l < 1e-20) {
                break;
            }
            p += dir * ds * b / l;
            let s = dir * f32(k + 1) / f32(steps);
            var w = 1.0 - abs(s);
            if ((u.flags & 128u) != 0u) {
                w *= 0.5 + 0.5 * cos(6.2831853 * (s - u.time));
            }
            acc += w * hash21(floor(p / ds));
            wsum += w;
        }
    }
    return acc / max(wsum, 1e-6);
}

// Limaille de fer : un grain par maille au plus, orienté selon B, présent avec une
// probabilité ∝ |B|^0,55. Renvoie la couverture du pixel par un grain.
fn filings_cover(world: vec2<f32>) -> f32 {
    let base = floor(world / u.grain);
    var cover = 0.0;
    for (var dy = -1; dy <= 1; dy++) {
        for (var dx = -1; dx <= 1; dx++) {
            let c = base + vec2<f32>(f32(dx), f32(dy));
            let center = (c + vec2<f32>(hash21(c), hash21(c + 17.3))) * u.grain;
            let b = sample_shown(center).yz;
            let l = length(b);
            if (l < 1e-20 || hash21(c + 41.7) > pow(min(l / u.b_max, 1.0), 0.55)) {
                continue;
            }
            let d = b / l;
            let rel = world - center;
            let along = clamp(dot(rel, d), -0.5 * u.grain, 0.5 * u.grain);
            let dist = length(rel - d * along);
            cover = max(cover, 1.0 - smoothstep(0.5 * u.px, 1.3 * u.px, dist));
        }
    }
    return cover;
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
    let bg = vec3<f32>(0.039, 0.043, 0.051);
    let g_raw = (in.world + 0.5 * u.size) / u.size;
    let inside = all(g_raw >= vec2<f32>(0.0)) && all(g_raw <= vec2<f32>(1.0));
    let s = sample_shown(in.world);
    let a = s.x;
    let b = s.yz;

    var color = bg;
    if ((u.flags & 2u) != 0u) {
        // Trois décades sous b_max.
        let t = 1.0 + log(max(length(b), 1e-12) / u.b_max) / log(1000.0);
        color = magma(clamp(t, 0.0, 1.0));
    }
    if ((u.flags & 8u) != 0u) {
        let v = clamp(0.5 + (lic_value(in.world) - 0.5) * 4.0, 0.0, 1.0);
        if ((u.flags & 2u) != 0u) {
            color = color * (0.45 + 1.1 * v);
        } else {
            color = mix(bg, vec3<f32>(0.62, 0.68, 0.78), v);
        }
    }
    if ((u.flags & 16u) != 0u) {
        color = mix(color, vec3<f32>(0.80, 0.82, 0.86), filings_cover(in.world));
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
