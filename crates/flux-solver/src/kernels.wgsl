// Noyaux de calcul du MG-PCG. Tous partagent le même groupe de liaisons ;
// `a`, `b`, `c` changent de rôle selon le noyau et `P.mode`.

struct Params {
    n: u32,      // cellules par côté de la grille traitée
    mode: u32,
    omega: f32,
    pad: f32,
}

@group(0) @binding(0) var<uniform> P: Params;
@group(0) @binding(1) var<storage, read> nu: array<f32>;
@group(0) @binding(2) var<storage, read_write> a: array<f32>;
@group(0) @binding(3) var<storage, read_write> b: array<f32>;
@group(0) @binding(4) var<storage, read_write> c: array<f32>;
// Scalaires du gradient conjugué : 0 r·z, 1 alpha, 2 beta, 3 r·r, 4 première itération.
@group(0) @binding(5) var<storage, read_write> s: array<f32>;

// Condition de Robin asymptotique concentrée aux nœuds du bord.
fn robin(i: u32, j: u32) -> f32 {
    let n = P.n;
    if (i == 0u || j == 0u || i == n || j == n) {
        let h = f32(n) * 0.5;
        let d = vec2<f32>(f32(i) - h, f32(j) - h);
        return h / dot(d, d);
    }
    return 0.0;
}

// Stencil Q1 à 9 points appliqué à `a` : renvoie ((A·a)(i,j), diagonale).
fn stencil(i: u32, j: u32) -> vec2<f32> {
    let n = P.n;
    let m = n + 1u;
    let vc = a[j * m + i];
    var av = 0.0;
    var dg = 0.0;
    for (var q = 0u; q < 4u; q++) {
        let ox = q & 1u;
        let oy = q >> 1u;
        if ((ox == 0u && i == 0u) || (ox == 1u && i == n) || (oy == 0u && j == 0u) || (oy == 1u && j == n)) {
            continue;
        }
        let ci = i + ox - 1u;
        let cj = j + oy - 1u;
        let ii = i + 2u * ox - 1u;
        let jj = j + 2u * oy - 1u;
        let k = nu[cj * n + ci] / 6.0;
        av += k * (4.0 * vc - a[j * m + ii] - a[jj * m + i] - 2.0 * a[jj * m + ii]);
        dg += 4.0 * k;
    }
    let rb = robin(i, j);
    return vec2<f32>(av + rb * vc, dg + rb);
}

// mode 0 : c = b − A·a   (résidu)
// mode 1 : c = A·a
// mode 2 : c = a + ω·(b − A·a)/D   (Jacobi pondéré)
// mode 3 : c = ω·b/D               (Jacobi depuis une estimation nulle)
@compute @workgroup_size(16, 16)
fn k_stencil(@builtin(global_invocation_id) id: vec3<u32>) {
    let m = P.n + 1u;
    if (id.x >= m || id.y >= m) {
        return;
    }
    let idx = id.y * m + id.x;
    let st = stencil(id.x, id.y);
    var val: f32;
    switch P.mode {
        case 0u: { val = b[idx] - st.x; }
        case 1u: { val = st.x; }
        case 2u: { val = a[idx] + P.omega * (b[idx] - st.x) / st.y; }
        default: { val = P.omega * b[idx] / st.y; }
    }
    c[idx] = val;
}

// c (grille de P.n cellules) = Pᵀ·a (grille deux fois plus fine).
@compute @workgroup_size(16, 16)
fn k_restrict(@builtin(global_invocation_id) id: vec3<u32>) {
    let m = P.n + 1u;
    if (id.x >= m || id.y >= m) {
        return;
    }
    let mf = i32(2u * P.n + 1u);
    var sum = 0.0;
    for (var dj = -1; dj <= 1; dj++) {
        for (var di = -1; di <= 1; di++) {
            let x = i32(2u * id.x) + di;
            let y = i32(2u * id.y) + dj;
            if (x < 0 || y < 0 || x >= mf || y >= mf) {
                continue;
            }
            let w = (1.0 - 0.5 * f32(abs(di))) * (1.0 - 0.5 * f32(abs(dj)));
            sum += w * a[y * mf + x];
        }
    }
    c[id.y * m + id.x] = sum;
}

// c (grille de P.n cellules) += P·a (grille deux fois plus grossière).
@compute @workgroup_size(16, 16)
fn k_prolong(@builtin(global_invocation_id) id: vec3<u32>) {
    let m = P.n + 1u;
    if (id.x >= m || id.y >= m) {
        return;
    }
    let mc = P.n / 2u + 1u;
    let i0 = id.x / 2u;
    let i1 = (id.x + 1u) / 2u;
    let j0 = id.y / 2u;
    let j1 = (id.y + 1u) / 2u;
    c[id.y * m + id.x] += 0.25 * (a[j0 * mc + i0] + a[j0 * mc + i1] + a[j1 * mc + i0] + a[j1 * mc + i1]);
}

// mode 0 : c += alpha·a ; mode 1 : c −= alpha·a ; mode 2 : c = a + beta·c.
@compute @workgroup_size(16, 16)
fn k_axpy(@builtin(global_invocation_id) id: vec3<u32>) {
    let m = P.n + 1u;
    if (id.x >= m || id.y >= m) {
        return;
    }
    let idx = id.y * m + id.x;
    switch P.mode {
        case 0u: { c[idx] += s[1] * a[idx]; }
        case 1u: { c[idx] -= s[1] * a[idx]; }
        default: { c[idx] = a[idx] + s[2] * c[idx]; }
    }
}

// Produit scalaire, première passe : c[ligne] = Σ a·b (mode 0) ou Σ a·a (mode 1).
@compute @workgroup_size(64)
fn k_dot_rows(@builtin(global_invocation_id) id: vec3<u32>) {
    let m = P.n + 1u;
    if (id.x >= m) {
        return;
    }
    let base = id.x * m;
    var sum = 0.0;
    if (P.mode == 1u) {
        for (var i = 0u; i < m; i++) { sum += a[base + i] * a[base + i]; }
    } else {
        for (var i = 0u; i < m; i++) { sum += a[base + i] * b[base + i]; }
    }
    c[id.x] = sum;
}

// Produit scalaire, seconde passe : somme des lignes puis mise à jour des scalaires.
// mode 0 : r·z → beta ; mode 1 : p·Ap → alpha ; mode 2 : r·r.
@compute @workgroup_size(1)
fn k_dot_final() {
    let m = P.n + 1u;
    var sum = 0.0;
    for (var i = 0u; i < m; i++) { sum += a[i]; }
    switch P.mode {
        case 0u: {
            if (s[4] > 0.5) { s[2] = 0.0; } else { s[2] = sum / s[0]; }
            s[0] = sum;
            s[4] = 0.0;
        }
        case 1u: {
            if (sum > 0.0) { s[1] = s[0] / sum; } else { s[1] = 0.0; }
        }
        default: { s[3] = sum; }
    }
}
