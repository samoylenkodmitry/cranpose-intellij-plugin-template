// The editor effects: every live effect in one pass. Positions are physical
// pixels from the surface's top-left; each effect adds premultiplied colour.
//
// Uniforms: 0 effect count, 1 density, 4-6 tint, then 8 floats per effect
// from 8: at.xy, start.xy, age (0..1), kind, seed, reach (logical pixels).

const TAU: f32 = 6.2831853;

fn hash(n: f32) -> f32 {
    return fract(sin(n) * 43758.5453);
}

fn soft(d: f32, radius: f32) -> f32 {
    return exp(-(d * d) / max(radius * radius, 0.0001));
}

fn hue(t: f32) -> vec3<f32> {
    return 0.5 + 0.5 * cos(TAU * (vec3<f32>(t) + vec3<f32>(0.0, 0.33, 0.67)));
}

fn segment_distance(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> f32 {
    let ab = b - a;
    let t = clamp(dot(p - a, ab) / max(dot(ab, ab), 0.0001), 0.0, 1.0);
    return length(p - (a + ab * t));
}

// A four-pointed glint of radius r at the origin of q.
fn glint(q: vec2<f32>, r: f32) -> f32 {
    let a = abs(q);
    return soft(length(a), r) + soft(a.x, r * 0.22) * soft(a.y, r * 2.4) + soft(a.y, r * 0.22) * soft(a.x, r * 2.4);
}

// ---- Sparks ----------------------------------------------------------------

fn burst(p: vec2<f32>, c: vec2<f32>, age: f32, seed: f32, density: f32, sparks: i32, reach: f32) -> vec4<f32> {
    let fade = 1.0 - age;
    let ease = 1.0 - fade * fade;
    var color = vec3<f32>(0.0);
    var alpha = 0.0;
    for (var i = 0; i < sparks; i = i + 1) {
        let fi = f32(i);
        let angle = fi / f32(sparks) * TAU + hash(seed + fi) * 0.9;
        let speed = (0.45 + 0.55 * hash(seed * 1.7 + fi)) * reach * density;
        let fall = vec2<f32>(0.0, 34.0 * density * age * age);
        let at = c + vec2<f32>(cos(angle), sin(angle)) * speed * ease + fall;
        let radius = (1.6 + 1.6 * hash(seed + fi * 3.1)) * density * fade + 0.3 * density;
        let glow = soft(length(p - at), radius) * fade;
        let heat = hash(seed + fi * 5.3);
        color = color + mix(vec3<f32>(1.0, 0.92, 0.55), vec3<f32>(1.0, 0.38, 0.12), heat) * glow;
        alpha = alpha + glow;
    }
    let flash = exp(-length(p - c) / (9.0 * density)) * (1.0 - smoothstep(0.0, 0.3, age)) * 0.7;
    return vec4<f32>(color + vec3<f32>(1.0, 0.9, 0.7) * flash, alpha + flash);
}

fn shockwave(p: vec2<f32>, c: vec2<f32>, age: f32, density: f32, tint: vec3<f32>) -> vec4<f32> {
    let radius = age * 44.0 * density;
    let width = (3.5 * (1.0 - age) + 0.8) * density;
    let offset = (length(p - c) - radius) / width;
    let ring = exp(-offset * offset) * (1.0 - age);
    return vec4<f32>(tint * ring, ring);
}

fn lightning(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>, age: f32, seed: f32, density: f32, tint: vec3<f32>) -> vec4<f32> {
    let span = b - a;
    let length_px = max(length(span), 1.0);
    let along = span / length_px;
    let across = vec2<f32>(-along.y, along.x);
    let t = clamp(dot(p - a, along) / length_px, 0.0, 1.0);
    let taper = sin(t * 3.14159);
    let jag = (sin(t * 23.0 + seed * 7.0) * 0.6 + sin(t * 57.0 + seed * 13.0) * 0.3
        + sin(t * 131.0 + seed * 3.0) * 0.15) * 16.0 * density * taper;
    let flicker = 0.65 + 0.35 * sin(age * 70.0 + seed * 11.0);
    let d = length(p - (a + along * t * length_px + across * jag));
    let core = exp(-d / (1.1 * density));
    let glow = exp(-d / (10.0 * density)) * 0.55;
    let strength = (core + glow) * (1.0 - age) * flicker;
    return vec4<f32>(mix(tint, vec3<f32>(1.0), core) * strength, strength);
}

// ---- Water -----------------------------------------------------------------

fn droplet(p: vec2<f32>, at: vec2<f32>, r: f32) -> vec4<f32> {
    let body = 1.0 - smoothstep(r * 0.6, r, length(p - at));
    let shine = soft(length(p - at + vec2<f32>(r * 0.35, r * 0.35)), r * 0.3);
    let color = mix(vec3<f32>(0.25, 0.6, 1.0), vec3<f32>(0.9, 0.97, 1.0), clamp(shine + 0.25, 0.0, 1.0));
    return vec4<f32>(color * body, body);
}

fn splash(p: vec2<f32>, c: vec2<f32>, age: f32, seed: f32, density: f32) -> vec4<f32> {
    let fade = 1.0 - age;
    var total = vec4<f32>(0.0);
    for (var i = 0; i < 9; i = i + 1) {
        let fi = f32(i);
        let side = hash(seed + fi) * 2.0 - 1.0;
        let lift = 0.55 + 0.45 * hash(seed * 1.3 + fi);
        let at = c + vec2<f32>(side * 46.0 * age, -110.0 * lift * age + 170.0 * age * age) * density;
        let r = (2.0 + 1.6 * hash(seed + fi * 2.3)) * density;
        total = total + droplet(p, at, r) * fade;
    }
    let q = (p - c) * vec2<f32>(1.0, 2.6);
    let wave = (length(q) - age * 34.0 * density) / (1.6 * density);
    let ring = exp(-wave * wave) * fade * 0.8;
    return total + vec4<f32>(vec3<f32>(0.55, 0.85, 1.0) * ring, ring);
}

fn ripple(p: vec2<f32>, c: vec2<f32>, age: f32, density: f32) -> vec4<f32> {
    let dist = length((p - c) * vec2<f32>(1.0, 1.9));
    let front = age * 70.0 * density;
    var crest = 0.0;
    var trough = 0.0;
    for (var k = 0; k < 4; k = k + 1) {
        let r = front - f32(k) * 10.0 * density;
        if (r > 0.0) {
            let weight = 1.0 - f32(k) * 0.22;
            crest = crest + soft(dist - r, 2.0 * density) * weight;
            trough = trough + soft(dist - r - 3.5 * density, 2.0 * density) * weight;
        }
    }
    let fade = 1.0 - age;
    let light = crest * fade;
    // The troughs darken what lies beneath: alpha without light.
    let shadow = trough * fade * 0.35;
    return vec4<f32>(vec3<f32>(0.6, 0.88, 1.0) * light, light + shadow);
}

fn bubble(p: vec2<f32>, at: vec2<f32>, r: f32, density: f32) -> vec4<f32> {
    let d = length(p - at);
    let rim = soft(d - r, 0.7 * density);
    let fill = (1.0 - smoothstep(r * 0.9, r, d)) * 0.12;
    let shine = soft(length(p - at + vec2<f32>(r * 0.4, r * 0.4)), r * 0.28);
    let s = rim * 0.75 + shine + fill;
    return vec4<f32>(vec3<f32>(0.75, 0.92, 1.0) * s, s);
}

// Bubbles rising from along a..b (a point when a == b).
fn bubbles(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>, age: f32, seed: f32, density: f32, count: i32) -> vec4<f32> {
    let fade = 1.0 - smoothstep(0.6, 1.0, age);
    var total = vec4<f32>(0.0);
    for (var i = 0; i < count; i = i + 1) {
        let fi = f32(i);
        let along = (fi + hash(seed + fi * 0.7)) / f32(count);
        let start = mix(a, b, along) + vec2<f32>((hash(seed + fi) - 0.5) * 10.0 * density, 0.0);
        let t = max(age - hash(seed * 1.9 + fi) * 0.3, 0.0);
        let rise = (26.0 + 30.0 * hash(seed * 2.1 + fi)) * density * t;
        let wobble = sin(t * 14.0 + fi * 2.3) * 2.5 * density;
        let r = (1.6 + 2.2 * hash(seed + fi * 1.7)) * density * (0.5 + t);
        total = total + bubble(p, start + vec2<f32>(wobble, -rise), r, density) * fade * step(0.0001, t);
    }
    return total;
}

// ---- Ice -------------------------------------------------------------------

// One arm of a frost crystal along +x, with branches at 60 degrees.
fn crystal_arm(local: vec2<f32>, size: f32, density: f32) -> f32 {
    let width = 0.75 * density;
    var s = soft(local.y, width) * step(0.0, local.x) * step(local.x, size);
    for (var k = 1; k <= 3; k = k + 1) {
        let fk = f32(k);
        let rel = local - vec2<f32>(size * (0.2 + 0.2 * fk), 0.0);
        let along = dot(rel, vec2<f32>(0.5, 0.8660254));
        let across = dot(rel, vec2<f32>(-0.8660254, 0.5));
        s = s + soft(across, width * 0.8) * step(0.0, along) * step(along, size * (0.42 - 0.1 * fk));
    }
    return s;
}

fn frost(p: vec2<f32>, c: vec2<f32>, age: f32, seed: f32, density: f32) -> vec4<f32> {
    let q = p - c;
    let grow = 1.0 - pow(1.0 - min(age * 2.5, 1.0), 3.0);
    let size = (11.0 + 7.0 * hash(seed)) * density * grow;
    let fade = 1.0 - smoothstep(0.55, 1.0, age);
    let sector = TAU / 6.0;
    let angle = atan2(q.y, q.x) + hash(seed * 3.1) * sector;
    let folded = abs(fract(angle / sector + 0.5) - 0.5) * sector;
    let r = length(q);
    let arms = crystal_arm(vec2<f32>(cos(folded), sin(folded)) * r, size, density);
    let core = soft(r, 2.2 * density);
    let halo = soft(r, size * 1.1 + density) * 0.22 * fade;
    let s = clamp(arms + core, 0.0, 1.2) * fade;
    let color = mix(vec3<f32>(0.62, 0.85, 1.0), vec3<f32>(1.0), clamp(core + 0.3, 0.0, 1.0));
    return vec4<f32>(color * s + vec3<f32>(0.45, 0.72, 1.0) * halo, s + halo);
}

fn shatter(p: vec2<f32>, c: vec2<f32>, age: f32, seed: f32, density: f32) -> vec4<f32> {
    let fade = 1.0 - age;
    let ease = 1.0 - fade * fade;
    var total = vec4<f32>(0.0);
    for (var i = 0; i < 10; i = i + 1) {
        let fi = f32(i);
        let heading = fi / 10.0 * TAU + hash(seed + fi) * 0.6;
        let speed = (22.0 + 30.0 * hash(seed * 1.7 + fi)) * density;
        let at = c + vec2<f32>(cos(heading), sin(heading)) * speed * ease + vec2<f32>(0.0, 40.0 * density * age * age);
        let spin = heading + age * (hash(seed + fi * 3.3) - 0.5) * 14.0;
        let dir = vec2<f32>(cos(spin), sin(spin));
        let rel = p - at;
        let x = dot(rel, dir);
        let y = dot(rel, vec2<f32>(-dir.y, dir.x));
        let half_length = (2.5 + 3.0 * hash(seed + fi * 5.1)) * density;
        let half_width = (0.9 + 0.8 * hash(seed + fi * 7.7)) * density;
        let body = 1.0 - smoothstep(0.75, 1.0, abs(x) / half_length + abs(y) / half_width);
        let facet = select(0.55, 1.0, y > 0.0);
        total = total + vec4<f32>(mix(vec3<f32>(0.6, 0.85, 1.0), vec3<f32>(1.0), facet) * body, body) * fade;
    }
    let flash = soft(length(p - c), 10.0 * density) * (1.0 - smoothstep(0.0, 0.25, age)) * 0.8;
    return total + vec4<f32>(vec3<f32>(0.8, 0.93, 1.0) * flash, flash);
}

fn frost_trail(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>, age: f32, seed: f32, density: f32) -> vec4<f32> {
    let span = b - a;
    let length_px = max(length(span), 1.0);
    let dir = span / length_px;
    let t = dot(p - a, dir);
    let off = dot(p - a, vec2<f32>(-dir.y, dir.x));
    let reach = min(age * 3.0, 1.0) * length_px;
    let fade = 1.0 - smoothstep(0.5, 1.0, age);
    let inside = step(0.0, t) * step(t, reach);
    let cell = 8.0 * density;
    let k = floor(t / cell);
    let rel = vec2<f32>(t - k * cell, abs(off));
    let along = dot(rel, vec2<f32>(0.5, 0.8660254));
    let across = dot(rel, vec2<f32>(-0.8660254, 0.5));
    let tick = soft(across, 0.6 * density) * step(0.0, along) * step(along, (2.5 + 4.0 * hash(seed + k)) * density);
    let tip = soft(length(p - (a + dir * reach)), 4.0 * density) * (1.0 - step(1.0, age * 3.0));
    let s = ((soft(off, 0.9 * density) + tick * 0.85) * inside + tip) * fade;
    let halo = soft(off, 6.0 * density) * inside * 0.2 * fade;
    return vec4<f32>(vec3<f32>(0.78, 0.93, 1.0) * s + vec3<f32>(0.4, 0.7, 1.0) * halo, s + halo);
}

fn snow(p: vec2<f32>, c: vec2<f32>, age: f32, seed: f32, density: f32) -> vec4<f32> {
    let fade = 1.0 - age;
    var total = vec4<f32>(0.0);
    for (var i = 0; i < 6; i = i + 1) {
        let fi = f32(i);
        let start = c + vec2<f32>((hash(seed * 3.0 + fi) - 0.5) * 16.0, -6.0 * hash(seed + fi * 4.0)) * density;
        let drift = vec2<f32>(
            (hash(seed + fi) - 0.5) * 30.0 * age + sin(age * 6.0 + fi) * 4.0,
            26.0 * age * (0.6 + 0.6 * hash(seed * 2.0 + fi)),
        ) * density;
        let s = glint(p - start - drift, (1.2 + hash(seed + fi * 9.0)) * density) * fade;
        total = total + vec4<f32>(vec3<f32>(0.85, 0.95, 1.0) * s, s);
    }
    return total;
}

// ---- Cosmic ----------------------------------------------------------------

fn stars(p: vec2<f32>, c: vec2<f32>, age: f32, seed: f32, density: f32, count: i32, reach: f32) -> vec4<f32> {
    let fade = 1.0 - age;
    let ease = 1.0 - fade * fade * fade;
    var total = vec4<f32>(0.0);
    for (var i = 0; i < count; i = i + 1) {
        let fi = f32(i);
        let angle = fi / f32(count) * TAU + hash(seed + fi) * 0.8 + age * 1.5;
        let dist = (0.35 + 0.65 * hash(seed * 1.9 + fi)) * reach * density * ease;
        let at = c + vec2<f32>(cos(angle), sin(angle)) * dist;
        let twinkle = 0.6 + 0.4 * sin(age * 30.0 + fi * 4.0);
        let s = glint(p - at, (1.1 + 1.2 * hash(seed + fi * 2.9)) * density) * fade * twinkle;
        let color = mix(vec3<f32>(0.75, 0.55, 1.0), vec3<f32>(0.5, 0.95, 1.0), hash(seed + fi * 6.1));
        total = total + vec4<f32>(color * s, s);
    }
    return total;
}

fn black_hole(p: vec2<f32>, c: vec2<f32>, age: f32, seed: f32, density: f32) -> vec4<f32> {
    let q = p - c;
    let r = length(q);
    let life = sin(age * 3.14159);
    let core = 7.0 * density * life;
    let dark = (1.0 - smoothstep(core * 0.8, core, r)) * 0.85;
    let swirl = 0.5 + 0.5 * sin(atan2(q.y, q.x) * 3.0 - log(max(r, 1.0)) * 6.0 + age * 18.0 + seed);
    let disk = soft(r - core * 1.35, 2.5 * density) * (0.5 + 0.8 * swirl) * life;
    let glow = soft(r, core * 2.6 + density) * 0.25 * life;
    let light = mix(vec3<f32>(1.0, 0.55, 0.2), vec3<f32>(0.7, 0.5, 1.0), swirl) * (disk + glow);
    // The core swallows what lies beneath: alpha without light.
    return vec4<f32>(light * (1.0 - dark), disk + glow + dark);
}

fn comet(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>, age: f32, density: f32, tint: vec3<f32>) -> vec4<f32> {
    let travel = 1.0 - pow(1.0 - min(age * 1.6, 1.0), 2.0);
    let span = b - a;
    let length_px = max(length(span), 1.0);
    let dir = span / length_px;
    let rel = p - mix(a, b, travel);
    let behind = -dot(rel, dir);
    let off = dot(rel, vec2<f32>(-dir.y, dir.x));
    let tail_length = max(min(travel * length_px, 90.0 * density), 1.0);
    let taper = 1.0 - behind / tail_length;
    let tail = soft(off, (1.0 + 3.5 * taper) * density) * taper * step(0.0, behind) * step(behind, tail_length);
    let head = soft(length(rel), 3.0 * density) + soft(length(rel), 9.0 * density) * 0.4;
    let s = (tail * 0.9 + head) * (1.0 - smoothstep(0.6, 1.0, age));
    return vec4<f32>(mix(tint, vec3<f32>(1.0), clamp(head, 0.0, 1.0)) * s, s);
}

// ---- Party -----------------------------------------------------------------

fn confetti(p: vec2<f32>, c: vec2<f32>, age: f32, seed: f32, density: f32, count: i32, reach: f32) -> vec4<f32> {
    let fade = 1.0 - smoothstep(0.65, 1.0, age);
    var total = vec4<f32>(0.0);
    for (var i = 0; i < count; i = i + 1) {
        let fi = f32(i);
        let heading = -1.5708 + (hash(seed + fi) - 0.5) * 2.4;
        let speed = (0.5 + 0.5 * hash(seed * 1.3 + fi)) * reach * density;
        let at = c + vec2<f32>(cos(heading), sin(heading)) * speed * age
            + vec2<f32>(sin(age * 9.0 + fi) * 3.0, 120.0 * age * age) * density;
        let spin = age * (6.0 + 8.0 * hash(seed + fi * 2.0)) + fi;
        let dir = vec2<f32>(cos(spin), sin(spin));
        let rel = p - at;
        let x = abs(dot(rel, dir));
        let y = abs(dot(rel, vec2<f32>(-dir.y, dir.x)));
        // The piece flutters: it turns edge-on and back.
        let turn = abs(cos(age * 11.0 + fi * 1.7));
        let half_width = 2.4 * density;
        let half_height = 1.2 * density * (0.25 + 0.75 * turn);
        let body = (1.0 - smoothstep(half_width - 0.6 * density, half_width, x))
            * (1.0 - smoothstep(half_height - 0.5 * density, half_height, y));
        total = total + vec4<f32>(hue(hash(seed + fi * 4.3)) * body, body) * fade;
    }
    return total;
}

fn poof(p: vec2<f32>, c: vec2<f32>, age: f32, seed: f32, density: f32) -> vec4<f32> {
    let fade = 1.0 - age;
    var cloud = 0.0;
    for (var i = 0; i < 6; i = i + 1) {
        let fi = f32(i);
        let angle = fi / 6.0 * TAU + hash(seed + fi);
        let at = c + vec2<f32>(cos(angle), sin(angle) - 0.6 * age) * (6.0 + 16.0 * age) * density;
        cloud = cloud + soft(length(p - at), (5.0 + 7.0 * age) * density);
    }
    let s = clamp(cloud, 0.0, 1.0) * fade * fade * 0.7;
    return vec4<f32>(vec3<f32>(0.85, 0.85, 0.9) * s, s);
}

// A rainbow arching from a to b over the side of the path that faces up.
fn rainbow(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>, age: f32, density: f32) -> vec4<f32> {
    let center = (a + b) * 0.5;
    let span = b - a;
    let radius = max(length(span) * 0.5, 6.0 * density);
    let dir = span / max(length(span), 0.0001);
    let normal = vec2<f32>(-dir.y, dir.x);
    let up = select(normal, -normal, normal.y > 0.0);
    let rel = p - center;
    // 0 at a, 1 at b, along the upper half circle.
    let progress = 1.0 - atan2(dot(rel, up), dot(rel, dir)) / 3.14159;
    let shown = step(0.0, dot(rel, up)) * step(progress, min(age * 2.2, 1.0));
    let band = (length(rel) - radius) / (2.2 * density);
    let bands = (1.0 - smoothstep(2.6, 3.2, abs(band))) * shown;
    let s = bands * 0.6 * (1.0 - smoothstep(0.65, 1.0, age));
    return vec4<f32>(hue((3.0 - band) / 7.0 * 0.8) * s, s);
}

// ---- Dispatch --------------------------------------------------------------

fn effect(p: vec2<f32>, kind: i32, at: vec2<f32>, start: vec2<f32>, age: f32, seed: f32, density: f32, tint: vec3<f32>) -> vec4<f32> {
    var color = vec4<f32>(0.0);
    switch kind {
        case 0: { color = burst(p, at, age, seed, density, 12, 70.0); }
        case 1: { color = shockwave(p, at, age, density, tint); }
        case 2: { color = lightning(p, start, at, age, seed, density, tint); }
        case 3: { color = burst(p, at, age, seed, density, 6, 28.0) * 0.7; }
        case 4: { color = splash(p, at, age, seed, density); }
        case 5: { color = ripple(p, at, age, density); }
        case 6: { color = bubbles(p, start, at, age, seed, density, 12); }
        case 7: { color = bubbles(p, at, at, age, seed, density, 4); }
        case 8: { color = frost(p, at, age, seed, density); }
        case 9: { color = shatter(p, at, age, seed, density); }
        case 10: { color = frost_trail(p, start, at, age, seed, density); }
        case 11: { color = snow(p, at, age, seed, density); }
        case 12: { color = stars(p, at, age, seed, density, 10, 60.0); }
        case 13: { color = black_hole(p, at, age, seed, density); }
        case 14: { color = comet(p, start, at, age, density, tint); }
        case 15: { color = stars(p, at, age, seed, density, 4, 22.0) * 0.8; }
        case 16: { color = confetti(p, at, age, seed, density, 14, 55.0); }
        case 17: { color = poof(p, at, age, seed, density); }
        case 18: { color = rainbow(p, start, at, age, density); }
        case 19: { color = confetti(p, at, age, seed, density, 5, 35.0); }
        default: {}
    }
    return color;
}

@fragment
fn effect_fs(input: VertexOutput) -> @location(0) vec4<f32> {
    let p = effect_local(input.uv);
    let count = u32(get_float(0u));
    let density = get_float(1u);
    let tint = vec3<f32>(get_float(4u), get_float(5u), get_float(6u));

    var total = vec4<f32>(0.0);
    for (var i = 0u; i < count; i = i + 1u) {
        let base = 8u + i * 8u;
        let at = vec2<f32>(get_float(base), get_float(base + 1u)) * density;
        let start = vec2<f32>(get_float(base + 2u), get_float(base + 3u)) * density;
        if (segment_distance(p, start, at) > get_float(base + 7u) * density) {
            continue;
        }
        let age = get_float(base + 4u);
        let kind = i32(get_float(base + 5u) + 0.5);
        total = total + effect(p, kind, at, start, age, get_float(base + 6u), density, tint);
    }
    let alpha = clamp(total.a, 0.0, 1.0);
    let rgb = clamp(total.rgb, vec3<f32>(0.0), vec3<f32>(alpha));
    return vec4<f32>(rgb, alpha);
}
