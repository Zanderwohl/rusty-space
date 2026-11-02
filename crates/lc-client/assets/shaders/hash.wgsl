// Integer hashes for procedural surfaces: pcg, after Jarzynski and Olano, and a combiner.

fn pcg(x: u32) -> u32 {
    let state = x * 747796405u + 2891336453u;
    let word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    return (word >> 22u) ^ word;
}

fn mix_hash(a: u32, b: u32) -> u32 {
    return pcg(a ^ (b + 0x9e3779b9u + (a << 6u) + (a >> 2u)));
}

/// `h` as a share of one, from its top 24 bits.
fn unit(h: u32) -> f32 {
    return f32(h >> 8u) / 16777216.0;
}
