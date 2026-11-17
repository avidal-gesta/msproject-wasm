//! Descifrado de PDF con el manejador de seguridad estandar: RC4 de 40 a 128 bits, AES-128 y
//! AES-256 (revisiones 5 y 6).
//!
//! Cubre el caso habitual de documentos "protegidos" contra edicion o copia pero que se
//! abren sin pedir contrasena (contrasena de usuario vacia). No rompe ninguna proteccion: usa
//! la misma clave que cualquier visor deriva para mostrar el documento.

const PAD: [u8; 32] = [
    0x28, 0xBF, 0x4E, 0x5E, 0x4E, 0x75, 0x8A, 0x41, 0x64, 0x00, 0x4E, 0x56, 0xFF, 0xFA, 0x01, 0x08, 0x2E, 0x2E, 0x00, 0xB6, 0xD0, 0x68, 0x3E, 0x80,
    0x2F, 0x0C, 0xA9, 0xFE, 0x64, 0x53, 0x69, 0x7A,
];

// ---------------------------------------------------------------------------------------------
// MD5 (RFC 1321)
// ---------------------------------------------------------------------------------------------

const MD5_K: [u32; 64] = [
    0xd76aa478, 0xe8c7b756, 0x242070db, 0xc1bdceee, 0xf57c0faf, 0x4787c62a, 0xa8304613, 0xfd469501, 0x698098d8, 0x8b44f7af, 0xffff5bb1, 0x895cd7be,
    0x6b901122, 0xfd987193, 0xa679438e, 0x49b40821, 0xf61e2562, 0xc040b340, 0x265e5a51, 0xe9b6c7aa, 0xd62f105d, 0x02441453, 0xd8a1e681, 0xe7d3fbc8,
    0x21e1cde6, 0xc33707d6, 0xf4d50d87, 0x455a14ed, 0xa9e3e905, 0xfcefa3f8, 0x676f02d9, 0x8d2a4c8a, 0xfffa3942, 0x8771f681, 0x6d9d6122, 0xfde5380c,
    0xa4beea44, 0x4bdecfa9, 0xf6bb4b60, 0xbebfbc70, 0x289b7ec6, 0xeaa127fa, 0xd4ef3085, 0x04881d05, 0xd9d4d039, 0xe6db99e5, 0x1fa27cf8, 0xc4ac5665,
    0xf4292244, 0x432aff97, 0xab9423a7, 0xfc93a039, 0x655b59c3, 0x8f0ccc92, 0xffeff47d, 0x85845dd1, 0x6fa87e4f, 0xfe2ce6e0, 0xa3014314, 0x4e0811a1,
    0xf7537e82, 0xbd3af235, 0x2ad7d2bb, 0xeb86d391,
];
const MD5_S: [u32; 16] = [7, 12, 17, 22, 5, 9, 14, 20, 4, 11, 16, 23, 6, 10, 15, 21];

pub fn md5(data: &[u8]) -> [u8; 16] {
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&((data.len() as u64).wrapping_mul(8)).to_le_bytes());
    let mut h: [u32; 4] = [0x67452301, 0xefcdab89, 0x98badcfe, 0x10325476];
    for chunk in msg.chunks_exact(64) {
        let mut m = [0u32; 16];
        for (i, w) in chunk.chunks_exact(4).enumerate() {
            m[i] = u32::from_le_bytes([w[0], w[1], w[2], w[3]]);
        }
        let (mut a, mut b, mut c, mut d) = (h[0], h[1], h[2], h[3]);
        for i in 0..64 {
            let (f, g) = match i / 16 {
                0 => ((b & c) | (!b & d), i),
                1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                2 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let f = f.wrapping_add(a).wrapping_add(MD5_K[i]).wrapping_add(m[g]);
            a = d;
            d = c;
            c = b;
            b = b.wrapping_add(f.rotate_left(MD5_S[(i / 16) * 4 + i % 4]));
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
    }
    let mut out = [0u8; 16];
    for (i, v) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
    }
    out
}

// ---------------------------------------------------------------------------------------------
// SHA-256, SHA-384 y SHA-512 (FIPS 180-4), para AES-256
// ---------------------------------------------------------------------------------------------

/// Constantes de SHA-512. Las de SHA-256 son sus 32 bits altos (salen de las mismas raices).
const SHA_K: [u64; 80] = [
    0x428a2f98d728ae22, 0x7137449123ef65cd, 0xb5c0fbcfec4d3b2f, 0xe9b5dba58189dbbc, 0x3956c25bf348b538, 0x59f111f1b605d019, 0x923f82a4af194f9b, 0xab1c5ed5da6d8118,
    0xd807aa98a3030242, 0x12835b0145706fbe, 0x243185be4ee4b28c, 0x550c7dc3d5ffb4e2, 0x72be5d74f27b896f, 0x80deb1fe3b1696b1, 0x9bdc06a725c71235, 0xc19bf174cf692694,
    0xe49b69c19ef14ad2, 0xefbe4786384f25e3, 0x0fc19dc68b8cd5b5, 0x240ca1cc77ac9c65, 0x2de92c6f592b0275, 0x4a7484aa6ea6e483, 0x5cb0a9dcbd41fbd4, 0x76f988da831153b5,
    0x983e5152ee66dfab, 0xa831c66d2db43210, 0xb00327c898fb213f, 0xbf597fc7beef0ee4, 0xc6e00bf33da88fc2, 0xd5a79147930aa725, 0x06ca6351e003826f, 0x142929670a0e6e70,
    0x27b70a8546d22ffc, 0x2e1b21385c26c926, 0x4d2c6dfc5ac42aed, 0x53380d139d95b3df, 0x650a73548baf63de, 0x766a0abb3c77b2a8, 0x81c2c92e47edaee6, 0x92722c851482353b,
    0xa2bfe8a14cf10364, 0xa81a664bbc423001, 0xc24b8b70d0f89791, 0xc76c51a30654be30, 0xd192e819d6ef5218, 0xd69906245565a910, 0xf40e35855771202a, 0x106aa07032bbd1b8,
    0x19a4c116b8d2d0c8, 0x1e376c085141ab53, 0x2748774cdf8eeb99, 0x34b0bcb5e19b48a8, 0x391c0cb3c5c95a63, 0x4ed8aa4ae3418acb, 0x5b9cca4f7763e373, 0x682e6ff3d6b2b8a3,
    0x748f82ee5defb2fc, 0x78a5636f43172f60, 0x84c87814a1f0ab72, 0x8cc702081a6439ec, 0x90befffa23631e28, 0xa4506cebde82bde9, 0xbef9a3f7b2c67915, 0xc67178f2e372532b,
    0xca273eceea26619c, 0xd186b8c721c0c207, 0xeada7dd6cde0eb1e, 0xf57d4f7fee6ed178, 0x06f067aa72176fba, 0x0a637dc5a2c898a6, 0x113f9804bef90dae, 0x1b710b35131c471b,
    0x28db77f523047d84, 0x32caab7b40c72493, 0x3c9ebe0a15c9bebc, 0x431d67c49c100d4c, 0x4cc5d4becb3e42b6, 0x597f299cfc657e2a, 0x5fcb6fab3ad6faec, 0x6c44198c4a475817,
];

/// Valores iniciales de SHA-512 (los de SHA-256 son sus 32 bits altos) y, despues, los de SHA-384.
const SHA_IV: [u64; 16] = [
    0x6a09e667f3bcc908, 0xbb67ae8584caa73b, 0x3c6ef372fe94f82b, 0xa54ff53a5f1d36f1, 0x510e527fade682d1, 0x9b05688c2b3e6c1f, 0x1f83d9abfb41bd6b, 0x5be0cd19137e2179,
    0xcbbb9d5dc1059ed8, 0x629a292a367cd507, 0x9159015a3070dd17, 0x152fecd8f70e5939, 0x67332667ffc00b31, 0x8eb44a8768581511, 0xdb0c2e0d64f98fa7, 0x47b5481dbefa4fa4,
];

/// Mensaje con el relleno de SHA-2: 0x80, ceros y el largo en bits (en `len_bytes` bytes).
fn sha_pad(data: &[u8], block: usize, len_bytes: usize) -> Vec<u8> {
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % block != block - len_bytes {
        msg.push(0);
    }
    msg.extend_from_slice(&((data.len() as u128) * 8).to_be_bytes()[16 - len_bytes..]);
    msg
}

pub fn sha256(data: &[u8]) -> [u8; 32] {
    let mut h: [u32; 8] = core::array::from_fn(|i| (SHA_IV[i] >> 32) as u32);
    for block in sha_pad(data, 64, 8).chunks_exact(64) {
        let mut w = [0u32; 64];
        for (i, b) in block.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes([b[0], b[1], b[2], b[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let mut v = h;
        for (i, wi) in w.iter().enumerate() {
            let [a, b, c, d, e, f, g, x] = v;
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let t1 = x.wrapping_add(s1).wrapping_add((e & f) ^ (!e & g)).wrapping_add((SHA_K[i] >> 32) as u32).wrapping_add(*wi);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let t2 = s0.wrapping_add((a & b) ^ (a & c) ^ (b & c));
            v = [t1.wrapping_add(t2), a, b, c, d.wrapping_add(t1), e, f, g];
        }
        for (x, y) in h.iter_mut().zip(v) {
            *x = x.wrapping_add(y);
        }
    }
    let mut out = [0u8; 32];
    for (i, x) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&x.to_be_bytes());
    }
    out
}

/// SHA-512 con los valores iniciales dados; SHA-384 es lo mismo con otros valores, truncado.
fn sha512_with(data: &[u8], iv: &[u64]) -> [u8; 64] {
    let mut h: [u64; 8] = core::array::from_fn(|i| iv[i]);
    for block in sha_pad(data, 128, 16).chunks_exact(128) {
        let mut w = [0u64; 80];
        for (i, b) in block.chunks_exact(8).enumerate() {
            w[i] = u64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]);
        }
        for i in 16..80 {
            let s0 = w[i - 15].rotate_right(1) ^ w[i - 15].rotate_right(8) ^ (w[i - 15] >> 7);
            let s1 = w[i - 2].rotate_right(19) ^ w[i - 2].rotate_right(61) ^ (w[i - 2] >> 6);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let mut v = h;
        for (i, wi) in w.iter().enumerate() {
            let [a, b, c, d, e, f, g, x] = v;
            let s1 = e.rotate_right(14) ^ e.rotate_right(18) ^ e.rotate_right(41);
            let t1 = x.wrapping_add(s1).wrapping_add((e & f) ^ (!e & g)).wrapping_add(SHA_K[i]).wrapping_add(*wi);
            let s0 = a.rotate_right(28) ^ a.rotate_right(34) ^ a.rotate_right(39);
            let t2 = s0.wrapping_add((a & b) ^ (a & c) ^ (b & c));
            v = [t1.wrapping_add(t2), a, b, c, d.wrapping_add(t1), e, f, g];
        }
        for (x, y) in h.iter_mut().zip(v) {
            *x = x.wrapping_add(y);
        }
    }
    let mut out = [0u8; 64];
    for (i, x) in h.iter().enumerate() {
        out[i * 8..i * 8 + 8].copy_from_slice(&x.to_be_bytes());
    }
    out
}

pub fn sha512(data: &[u8]) -> [u8; 64] {
    sha512_with(data, &SHA_IV[..8])
}

pub fn sha384(data: &[u8]) -> [u8; 48] {
    let h = sha512_with(data, &SHA_IV[8..]);
    core::array::from_fn(|i| h[i])
}

// ---------------------------------------------------------------------------------------------
// RC4
// ---------------------------------------------------------------------------------------------

pub fn rc4(key: &[u8], data: &[u8]) -> Vec<u8> {
    if key.is_empty() {
        return data.to_vec();
    }
    let mut s: [u8; 256] = [0; 256];
    for (i, v) in s.iter_mut().enumerate() {
        *v = i as u8;
    }
    let mut j = 0u8;
    for i in 0..256 {
        j = j.wrapping_add(s[i]).wrapping_add(key[i % key.len()]);
        s.swap(i, j as usize);
    }
    let (mut i, mut j) = (0u8, 0u8);
    data.iter()
        .map(|b| {
            i = i.wrapping_add(1);
            j = j.wrapping_add(s[i as usize]);
            s.swap(i as usize, j as usize);
            b ^ s[s[i as usize].wrapping_add(s[j as usize]) as usize]
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------
// AES-128 y AES-256 (FIPS-197), modo CBC
// ---------------------------------------------------------------------------------------------

/// Cajas S de AES y la tabla de descifrado por columnas, calculadas una sola vez con la
/// aritmetica del campo GF(2^8), sin tablas escritas a mano.
struct Tables {
    sbox: [u8; 256],
    inv: [u8; 256],
    /// Sustitucion inversa y mezcla inversa de columnas de un byte de la fila 0: la columna
    /// (14, 9, 13, 11) * inv[x], con la fila r en los bits 8r. Las otras filas son rotaciones.
    td: [u32; 256],
}

fn tables() -> &'static Tables {
    static T: std::sync::OnceLock<Tables> = std::sync::OnceLock::new();
    T.get_or_init(|| {
        let mut sbox = [0u8; 256];
        let (mut p, mut q) = (1u8, 1u8);
        loop {
            p ^= xtime(p);
            q ^= q << 1;
            q ^= q << 2;
            q ^= q << 4;
            if q & 0x80 != 0 {
                q ^= 0x09;
            }
            sbox[p as usize] = q ^ q.rotate_left(1) ^ q.rotate_left(2) ^ q.rotate_left(3) ^ q.rotate_left(4) ^ 0x63;
            if p == 1 {
                break;
            }
        }
        sbox[0] = 0x63;
        let mut inv = [0u8; 256];
        for (i, v) in sbox.iter().enumerate() {
            inv[*v as usize] = i as u8;
        }
        let td = core::array::from_fn(|x| u32::from_le_bytes([0x0E, 0x09, 0x0D, 0x0B].map(|m| gmul(inv[x], m))));
        Tables { sbox, inv, td }
    })
}

fn xtime(a: u8) -> u8 {
    (a << 1) ^ if a & 0x80 != 0 { 0x1B } else { 0 }
}

fn gmul(mut a: u8, mut b: u8) -> u8 {
    let mut r = 0u8;
    while b != 0 {
        if b & 1 != 0 {
            r ^= a;
        }
        a = xtime(a);
        b >>= 1;
    }
    r
}

fn xor16(a: &mut [u8; 16], b: &[u8; 16]) {
    a.iter_mut().zip(b).for_each(|(x, y)| *x ^= y);
}

/// Claves de ronda de AES: 10 rondas con clave de 128 bits, 14 con 256.
struct Aes {
    keys: Vec<[u8; 16]>,
    /// Las mismas por columnas para el descifrado equivalente (FIPS-197, 5.3.5): las
    /// intermedias pasan por la mezcla inversa de columnas.
    dkeys: Vec<[u32; 4]>,
    t: &'static Tables,
}

/// Byte de la fila `r` de una columna.
fn row(w: u32, r: usize) -> usize {
    ((w >> (8 * r)) & 0xFF) as usize
}

impl Aes {
    fn new(key: &[u8]) -> Option<Aes> {
        if key.len() != 16 && key.len() != 32 {
            return None;
        }
        let t = tables();
        let nk = key.len() / 4;
        let mut w: Vec<[u8; 4]> = key.chunks_exact(4).map(|c| [c[0], c[1], c[2], c[3]]).collect();
        let mut rcon = 1u8;
        for i in nk..4 * (nk + 7) {
            let mut x = w[i - 1];
            if i % nk == 0 {
                x = [x[1], x[2], x[3], x[0]].map(|b| t.sbox[b as usize]);
                x[0] ^= rcon;
                rcon = xtime(rcon);
            } else if nk > 6 && i % nk == 4 {
                x = x.map(|b| t.sbox[b as usize]);
            }
            let p = w[i - nk];
            w.push([p[0] ^ x[0], p[1] ^ x[1], p[2] ^ x[2], p[3] ^ x[3]]);
        }
        let keys: Vec<[u8; 16]> = w.chunks_exact(4).map(|c| core::array::from_fn(|j| c[j / 4][j % 4])).collect();
        let n = keys.len() - 1;
        let dkeys = w
            .iter()
            .enumerate()
            .map(|(i, k)| {
                let col = u32::from_le_bytes(*k);
                // inv[sbox[x]] = x: la tabla aplicada a sbox[x] es solo la mezcla inversa.
                if i < 4 || i >= 4 * n {
                    col
                } else {
                    (0..4).fold(0, |acc, r| acc ^ t.td[t.sbox[row(col, r)] as usize].rotate_left(8 * r as u32))
                }
            })
            .collect::<Vec<u32>>()
            .chunks_exact(4)
            .map(|c| [c[0], c[1], c[2], c[3]])
            .collect();
        Some(Aes { keys, dkeys, t })
    }

    fn decrypt(&self, block: &mut [u8; 16]) {
        let (t, n) = (self.t, self.dkeys.len() - 1);
        let mut s: [u32; 4] = core::array::from_fn(|c| u32::from_le_bytes([block[c * 4], block[c * 4 + 1], block[c * 4 + 2], block[c * 4 + 3]]) ^ self.dkeys[n][c]);
        // La fila r de la columna c viene de la columna c - r (desplazamiento inverso de filas).
        for round in (1..n).rev() {
            s = core::array::from_fn(|c| (0..4).fold(self.dkeys[round][c], |acc, r| acc ^ t.td[row(s[(c + 4 - r) % 4], r)].rotate_left(8 * r as u32)));
        }
        for c in 0..4 {
            for r in 0..4 {
                block[c * 4 + r] = t.inv[row(s[(c + 4 - r) % 4], r)] ^ row(self.dkeys[0][c], r) as u8;
            }
        }
    }

    /// Cifrado de un bloque: solo lo usa la derivacion de la clave de la revision 6.
    fn encrypt(&self, block: &mut [u8; 16]) {
        let (t, n) = (self.t, self.keys.len() - 1);
        xor16(block, &self.keys[0]);
        for (round, key) in self.keys.iter().enumerate().skip(1) {
            let s = *block;
            for c in 0..4 {
                for r in 0..4 {
                    block[c * 4 + r] = t.sbox[s[((c + r) % 4) * 4 + r] as usize];
                }
            }
            if round < n {
                for c in 0..4 {
                    let col = [block[c * 4], block[c * 4 + 1], block[c * 4 + 2], block[c * 4 + 3]];
                    for r in 0..4 {
                        let (a, b) = (col[r], col[(r + 1) % 4]);
                        block[c * 4 + r] = xtime(a) ^ xtime(b) ^ b ^ col[(r + 2) % 4] ^ col[(r + 3) % 4];
                    }
                }
            }
            xor16(block, key);
        }
    }

    /// CBC sin relleno sobre los bloques completos de `data`.
    fn cbc(&self, iv: &[u8; 16], data: &[u8], encrypt: bool) -> Vec<u8> {
        let mut prev = *iv;
        let mut out = Vec::with_capacity(data.len());
        for chunk in data.chunks_exact(16) {
            let mut block: [u8; 16] = core::array::from_fn(|i| chunk[i]);
            if encrypt {
                xor16(&mut block, &prev);
                self.encrypt(&mut block);
                prev = block;
            } else {
                self.decrypt(&mut block);
                xor16(&mut block, &prev);
                prev = core::array::from_fn(|i| chunk[i]);
            }
            out.extend_from_slice(&block);
        }
        out
    }

    /// Descifra datos con el vector inicial en los primeros 16 bytes y quita el relleno final.
    fn decrypt_padded(&self, data: &[u8]) -> Vec<u8> {
        if data.len() < 32 {
            return Vec::new();
        }
        let mut out = self.cbc(&core::array::from_fn(|i| data[i]), &data[16..], false);
        // Relleno PKCS#7.
        if let Some(&n) = out.last() {
            if (1..=16).contains(&n) && out.len() >= n as usize {
                out.truncate(out.len() - n as usize);
            }
        }
        out
    }
}

/// Descifra AES-CBC con clave de 128 o 256 bits: los primeros 16 bytes son el vector inicial;
/// quita el relleno final.
pub fn aes_cbc_decrypt(key: &[u8], data: &[u8]) -> Vec<u8> {
    Aes::new(key).map(|a| a.decrypt_padded(data)).unwrap_or_default()
}

/// Descifra AES-128-CBC: los primeros 16 bytes son el vector inicial; quita el relleno final.
/// Se mantiene por compatibilidad: es `aes_cbc_decrypt` limitado a claves de 16 bytes.
pub fn aes128_cbc_decrypt(key: &[u8], data: &[u8]) -> Vec<u8> {
    if key.len() != 16 {
        return Vec::new();
    }
    aes_cbc_decrypt(key, data)
}

/// Hash de una contrasena para AES-256: SHA-256 en la revision 5 y el algoritmo 2.B de ISO
/// 32000-2 en la 6. `udata` va vacio para la contrasena de usuario.
fn hash_aes256(password: &[u8], salt: &[u8], udata: &[u8], r: i64) -> [u8; 32] {
    let join = |k: &[u8]| [password, k, udata].concat();
    let mut k = sha256(&[password, salt, udata].concat()).to_vec();
    if r >= 6 {
        let mut round = 0u32;
        loop {
            let k1 = join(&k).repeat(64);
            let aes = Aes::new(&k[..16]).expect("clave de 16 bytes");
            let e = aes.cbc(&core::array::from_fn(|i| k[16 + i]), &k1, true);
            // Los 16 primeros bytes como numero, modulo 3 (256 = 1 mod 3: basta sumarlos).
            k = match e[..16].iter().map(|&b| b as u32).sum::<u32>() % 3 {
                0 => sha256(&e).to_vec(),
                1 => sha384(&e).to_vec(),
                _ => sha512(&e).to_vec(),
            };
            round += 1;
            if round >= 64 && e.last().map_or(0, |&b| b as u32) + 32 <= round {
                break;
            }
        }
    }
    core::array::from_fn(|i| k[i])
}

// ---------------------------------------------------------------------------------------------
// Manejador de seguridad estandar
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Method {
    None,
    Rc4,
    Aes,
    /// AES-256 (version 5): la clave del archivo se usa tal cual, sin derivar una por objeto.
    Aes256,
}

pub struct Crypt {
    key: Vec<u8>,
    /// Claves de ronda de AES-256 (version 5), preparadas una vez para todo el documento.
    aes256: Option<Aes>,
    pub strings: Method,
    pub streams: Method,
}

/// Datos del diccionario `/Encrypt` ya extraidos.
pub struct Params<'a> {
    pub filter: &'a str,
    pub v: i64,
    pub r: i64,
    pub length: i64,
    pub o: &'a [u8],
    pub u: &'a [u8],
    /// Clave del archivo cifrada con la contrasena de usuario (solo version 5).
    pub ue: &'a [u8],
    pub p: i64,
    pub id: &'a [u8],
    pub encrypt_metadata: bool,
    /// Metodos de los filtros de cifrado para cadenas y flujos (versiones 4 y 5).
    pub strings: Option<&'a str>,
    pub streams: Option<&'a str>,
}

const PIDE_CONTRASENA: &str = "El PDF pide una contrasena para abrirlo. Quitele la contrasena y vuelva a intentar.";
// Mensajes fijos: no repiten nada que venga del archivo (una web podria mostrarlos como HTML).
const METODO_DESCONOCIDO: &str = "El PDF usa un metodo de cifrado desconocido y no se puede leer.";
const INCOHERENTE: &str = "El PDF tiene una informacion de cifrado incoherente y no se puede leer.";
const ILEGIBLE: &str = "El PDF esta cifrado pero su informacion de cifrado no se pudo leer.";

impl Crypt {
    /// Prepara el descifrado con la contrasena de usuario vacia.
    ///
    /// Con AES-256 no se comprueba `/Perms` (la copia cifrada de los permisos, que solo sirve
    /// para detectar que alguien cambio `/P`) ni se prueba la contrasena de propietario: la clave
    /// del archivo sale de `/UE` con la contrasena de usuario, y si esta no es vacia el PDF se
    /// rechaza igual que con RC4 y AES-128.
    pub fn new(p: &Params) -> Result<Crypt, String> {
        if p.filter != "Standard" {
            return Err("El PDF usa un sistema de proteccion no estandar (certificados o DRM) y no se puede leer.".into());
        }
        let method = |cfm: Option<&str>| match cfm {
            Some("V2") => Ok(Method::Rc4),
            Some("AESV2") => Ok(Method::Aes),
            Some("AESV3") => Ok(Method::Aes256),
            Some("None") | Some("Identity") | None => Ok(Method::None),
            Some(_) => Err(METODO_DESCONOCIDO.to_string()),
        };
        let (strings, streams, bits) = match p.v {
            1 => (Method::Rc4, Method::Rc4, 40),
            2 | 3 => (Method::Rc4, Method::Rc4, p.length.clamp(40, 128)),
            4 => (method(p.strings)?, method(p.streams)?, 128),
            5 => (method(p.strings)?, method(p.streams)?, 256),
            _ => return Err("El PDF usa una version de cifrado desconocida y no se puede leer.".into()),
        };
        // AES-256 va solo con la version 5, y la version 5 solo con AES-256 (o sin cifrar): una
        // mezcla daria basura o un documento vacio.
        if [strings, streams].iter().any(|&m| (m == Method::Aes256) != (p.v == 5) && m != Method::None) {
            return Err(INCOHERENTE.into());
        }
        if p.v == 5 {
            if p.r != 5 && p.r != 6 {
                return Err(INCOHERENTE.into());
            }
            // Revisiones 5 y 6 (algoritmo 2.A): la entrada /U trae el hash de la contrasena de
            // usuario (32 bytes), su sal de validacion (8) y la sal de la clave (8). Con la
            // contrasena vacia, el hash con la segunda sal descifra la clave del archivo de /UE.
            if p.u.len() < 48 || p.ue.len() < 32 {
                return Err(ILEGIBLE.into());
            }
            if hash_aes256(b"", &p.u[32..40], b"", p.r) != p.u[..32] {
                return Err(PIDE_CONTRASENA.into());
            }
            let k = hash_aes256(b"", &p.u[40..48], b"", p.r);
            let key = Aes::new(&k).map(|a| a.cbc(&[0; 16], &p.ue[..32], false)).unwrap_or_default();
            return Ok(Crypt { aes256: Aes::new(&key), key, strings, streams });
        }
        let n = if p.r <= 2 { 5 } else { (bits / 8) as usize };
        // Clave a partir de la contrasena de usuario vacia (algoritmo 2 de la especificacion).
        let mut input = PAD.to_vec();
        input.extend_from_slice(&p.o[..p.o.len().min(32)]);
        input.extend_from_slice(&(p.p as u32).to_le_bytes());
        input.extend_from_slice(p.id);
        if p.r >= 4 && !p.encrypt_metadata {
            input.extend_from_slice(&[0xFF; 4]);
        }
        let mut hash = md5(&input);
        if p.r >= 3 {
            for _ in 0..50 {
                hash = md5(&hash[..n]);
            }
        }
        let key = hash[..n].to_vec();

        // Comprobacion: con esa clave debe reproducirse la entrada /U.
        let ok = if p.r <= 2 {
            rc4(&key, &PAD) == p.u[..p.u.len().min(32)]
        } else {
            let mut seed = PAD.to_vec();
            seed.extend_from_slice(p.id);
            let mut x = rc4(&key, &md5(&seed));
            for i in 1..=19u8 {
                let k: Vec<u8> = key.iter().map(|b| b ^ i).collect();
                x = rc4(&k, &x);
            }
            p.u.len() >= 16 && x[..16] == p.u[..16]
        };
        if !ok {
            return Err(PIDE_CONTRASENA.into());
        }
        Ok(Crypt { key, aes256: None, strings, streams })
    }

    /// Descifra los datos del objeto `num gen` (algoritmo 1; con AES-256, la clave del archivo).
    pub fn decrypt(&self, method: Method, num: u32, gen: u16, data: &[u8]) -> Vec<u8> {
        match method {
            Method::None => return data.to_vec(),
            Method::Aes256 => return self.aes256.as_ref().map(|a| a.decrypt_padded(data)).unwrap_or_default(),
            _ => {}
        }
        let mut k = self.key.clone();
        k.extend_from_slice(&num.to_le_bytes()[..3]);
        k.extend_from_slice(&gen.to_le_bytes());
        if method == Method::Aes {
            k.extend_from_slice(b"sAlT");
        }
        let hash = md5(&k);
        let key = &hash[..(self.key.len() + 5).min(16)];
        match method {
            Method::Aes => aes_cbc_decrypt(key, data),
            _ => rc4(key, data),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{:02x}", x)).collect()
    }

    fn unhex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    #[test]
    fn md5_conocidos() {
        assert_eq!(hex(&md5(b"")), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(hex(&md5(b"abc")), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(hex(&md5(&[b'a'; 1000])), "cabe45dcc9ae5b66ba86600cca6b8ba8");
    }

    #[test]
    fn sha2_conocidos() {
        // Vectores de FIPS 180-4 ("abc", vacio y el mensaje de dos bloques).
        assert_eq!(hex(&sha256(b"abc")), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(hex(&sha256(b"")), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        assert_eq!(hex(&sha256(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq")), "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1");
        assert_eq!(
            hex(&sha384(b"abc")),
            "cb00753f45a35e8bb5a03d699ac65007272c32ab0eded1631a8b605a43ff5bed8086072ba1e7cc2358baeca134c825a7"
        );
        assert_eq!(
            hex(&sha512(b"abc")),
            "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f"
        );
        assert_eq!(
            hex(&sha512(&[b'a'; 200])),
            "4b11459c33f52a22ee8236782714c150a3b2c60994e9acee17fe68947a3e6789f31e7668394592da7bef827cddca88c4e6f86e4df7ed1ae6cba71f3e98faee9f"
        );
    }

    #[test]
    fn rc4_conocido() {
        assert_eq!(hex(&rc4(b"Key", b"Plaintext")), "bbf316e8d940af0ad3");
    }

    #[test]
    fn aes_conocido() {
        // Vectores de FIPS-197 (apendice C): texto 00112233..ff, claves 00..0f y 00..1f.
        let plain: [u8; 16] = core::array::from_fn(|i| i as u8 * 0x11);
        for (key_len, cipher) in [(16, "69c4e0d86a7b0430d8cdb78070b4c55a"), (32, "8ea2b7ca516745bfeafc49904b496089")] {
            let key: Vec<u8> = (0..key_len as u8).collect();
            let aes = Aes::new(&key).unwrap();
            let mut block = plain;
            aes.encrypt(&mut block);
            assert_eq!(hex(&block), cipher);
            // Vector inicial de ceros + el bloque cifrado + un segundo bloque para el relleno.
            let mut data = vec![0u8; 16];
            data.extend_from_slice(&unhex(cipher));
            data.extend_from_slice(&unhex(cipher));
            assert_eq!(&aes_cbc_decrypt(&key, &data)[..16], &plain[..]);
        }
    }

    /// Entradas /U y /UE de un PDF cifrado con AES-256 (revision 6) por MuPDF 1.27, con
    /// contrasena de usuario vacia, y un flujo cifrado del mismo archivo.
    const MUPDF_U: &str = "c6f9281661d22f41ce04bdb27d878c8fd429df135620f52eab2735acf77386784c3fd22e154616646ddc39ee0d29eb1a";
    const MUPDF_UE: &str = "15d6dda906d208ab784597a9f91daafc3e3142f21a8aa7eed5b76d6ceb13d808";
    const MUPDF_STREAM: &str = concat!(
        "ad83871ac56b570da8cc4f947c75895c0e8d9166991c24df9c54400e6cbf81410b47f0885ead08ddd1a2bf4113e4c757",
        "11a17d0c99eb7b1f1ccc47525436e72819cf51aaa56ee4e90c72e4077ae1d1abff76ee7434d94b4f42811a86b948b1a0"
    );
    const MUPDF_PLAIN: &[u8] = b"\nq\nBT\n1 0 0 1 10 50 Tm\n/helv 12 Tf [<486f6c61204145532d323536>]TJ\nET\nQ\n";

    #[test]
    fn aes256_revision_6_de_mupdf() {
        let (u, ue) = (unhex(MUPDF_U), unhex(MUPDF_UE));
        let params = Params {
            filter: "Standard",
            v: 5,
            r: 6,
            length: 256,
            o: &[],
            u: &u,
            ue: &ue,
            p: -4,
            id: &[],
            encrypt_metadata: true,
            strings: Some("AESV3"),
            streams: Some("AESV3"),
        };
        let crypt = Crypt::new(&params).unwrap();
        assert_eq!(crypt.decrypt(crypt.streams, 6, 0, &unhex(MUPDF_STREAM)), MUPDF_PLAIN);
        // Con /U alterado, la contrasena vacia ya no sirve.
        let mut otro = u.clone();
        otro[0] ^= 1;
        let params = Params { u: &otro, ..params };
        assert_eq!(Crypt::new(&params).err().as_deref(), Some(PIDE_CONTRASENA));
    }

    /// PDF cifrado con AES-256 (revision 5) armado aqui: a diferencia del de MuPDF, el
    /// diccionario /Encrypt es un objeto aparte y el titulo va en una cadena cifrada.
    fn pdf_revision_5(alterar_u: bool) -> Vec<u8> {
        let file_key: Vec<u8> = (0..32u8).map(|i| i.wrapping_mul(7) ^ 0x5A).collect();
        let (vsalt, ksalt) = (*b"validar!", *b"clave!!!");
        let mut u = [&hash_aes256(b"", &vsalt, b"", 5)[..], &vsalt, &ksalt].concat();
        if alterar_u {
            u[0] ^= 1;
        }
        let ue = Aes::new(&hash_aes256(b"", &ksalt, b"", 5)).unwrap().cbc(&[0; 16], &file_key, true);
        let enc = |data: &[u8]| {
            let n = 16 - data.len() % 16;
            let padded = [data, &vec![n as u8; n]].concat();
            [&[9u8; 16][..], &Aes::new(&file_key).unwrap().cbc(&[9; 16], &padded, true)].concat()
        };
        let h = |b: &[u8]| format!("<{}>", hex(b));
        let content = enc(b"BT /F1 12 Tf 10 50 Td (Hola revision 5) Tj ET");
        let objs = [
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 100] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".to_vec(),
            [format!("<< /Length {} >>\nstream\n", content.len()).as_bytes(), &content, b"\nendstream"].concat(),
            b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec(),
            format!("<< /Title {} >>", h(&enc(b"Titulo cifrado"))).into_bytes(),
            format!(
                "<< /Filter /Standard /V 5 /R 5 /Length 256 /P -4 /O {} /U {} /OE {} /UE {} /CF << /StdCF << /CFM /AESV3 /Length 32 >> >> /StmF /StdCF /StrF /StdCF >>",
                h(&[0; 48]),
                h(&u),
                h(&[0; 32]),
                h(&ue)
            )
            .into_bytes(),
        ];
        let mut pdf = b"%PDF-1.7\n".to_vec();
        let mut offs = Vec::new();
        for (i, o) in objs.iter().enumerate() {
            offs.push(pdf.len());
            pdf.extend(format!("{} 0 obj\n", i + 1).bytes());
            pdf.extend(o);
            pdf.extend(b"\nendobj\n");
        }
        let x = pdf.len();
        pdf.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).bytes());
        for o in offs {
            pdf.extend(format!("{o:010} 00000 n \n").bytes());
        }
        pdf.extend(format!("trailer\n<< /Size {} /Root 1 0 R /Info 6 0 R /Encrypt 7 0 R >>\nstartxref\n{x}\n%%EOF\n", objs.len() + 1).bytes());
        pdf
    }

    fn mupdf_params<'a>(u: &'a [u8], ue: &'a [u8]) -> Params<'a> {
        Params {
            filter: "Standard",
            v: 5,
            r: 6,
            length: 256,
            o: &[],
            u,
            ue,
            p: -4,
            id: &[],
            encrypt_metadata: true,
            strings: Some("AESV3"),
            streams: Some("AESV3"),
        }
    }

    #[test]
    fn metodo_desconocido_con_mensaje_fijo() {
        let (u, ue) = (unhex(MUPDF_U), unhex(MUPDF_UE));
        let raro = "<img src=x onerror=alert(1)>".repeat(1000);
        let p = Params { streams: Some(&raro), ..mupdf_params(&u, &ue) };
        assert_eq!(Crypt::new(&p).err().as_deref(), Some(METODO_DESCONOCIDO));
        let p = Params { v: 99, ..mupdf_params(&u, &ue) };
        assert!(!Crypt::new(&p).err().unwrap().contains("99"));
    }

    #[test]
    fn combinaciones_incoherentes() {
        let (u, ue) = (unhex(MUPDF_U), unhex(MUPDF_UE));
        let base = mupdf_params(&u, &ue);
        let casos = [
            // Version 4 con AES-256, en cadenas o en flujos.
            Params { v: 4, r: 4, ..mupdf_params(&u, &ue) },
            Params { v: 4, r: 4, strings: Some("AESV2"), ..mupdf_params(&u, &ue) },
            // Version 5 con RC4 o AES-128.
            Params { streams: Some("V2"), ..mupdf_params(&u, &ue) },
            Params { strings: Some("AESV2"), ..mupdf_params(&u, &ue) },
            // Version 5 con una revision que no es 5 ni 6.
            Params { r: 4, ..mupdf_params(&u, &ue) },
        ];
        for p in &casos {
            assert_eq!(Crypt::new(p).err().as_deref(), Some(INCOHERENTE), "V {} R {} {:?}/{:?}", p.v, p.r, p.strings, p.streams);
        }
        // Version 5 con un filtro sin cifrar (Identity) sigue valiendo.
        assert!(Crypt::new(&Params { strings: Some("Identity"), ..base }).is_ok());
    }

    #[test]
    fn u_y_ue_cortos() {
        let (u, ue) = (unhex(MUPDF_U), unhex(MUPDF_UE));
        assert_eq!(Crypt::new(&mupdf_params(&u[..47], &ue)).err().as_deref(), Some(ILEGIBLE));
        assert_eq!(Crypt::new(&mupdf_params(&u, &ue[..31])).err().as_deref(), Some(ILEGIBLE));
        assert_eq!(Crypt::new(&mupdf_params(&[], &[])).err().as_deref(), Some(ILEGIBLE));
    }

    #[test]
    fn datos_cortos_o_mal_rellenados() {
        let (u, ue) = (unhex(MUPDF_U), unhex(MUPDF_UE));
        let crypt = Crypt::new(&mupdf_params(&u, &ue)).unwrap();
        let stream = unhex(MUPDF_STREAM);
        // Menos de dos bloques (vector inicial y un bloque de datos): nada que descifrar.
        for n in [0, 1, 16, 31] {
            assert!(crypt.decrypt(crypt.streams, 6, 0, &stream[..n]).is_empty(), "{n} bytes");
        }
        // Un final incompleto se ignora; el resto se descifra igual.
        let cortado = crypt.decrypt(crypt.streams, 6, 0, &stream[..stream.len() - 5]);
        assert!(cortado.len() >= 48 && cortado.len() <= stream.len() - 32);
        assert!(MUPDF_PLAIN.starts_with(&cortado));
        // Relleno invalido (0 o mayor que 16): se entrega el bloque sin recortar, sin fallar.
        let key = [7u8; 32];
        let aes = Aes::new(&key).unwrap();
        for ultimo in [0u8, 17, 255] {
            let mut plano = [b'x'; 32];
            plano[31] = ultimo;
            let datos = [&[0u8; 16][..], &aes.cbc(&[0; 16], &plano, true)].concat();
            assert_eq!(aes_cbc_decrypt(&key, &datos), plano, "relleno {ultimo}");
        }
        // Clave de largo invalido: vacio.
        assert!(aes_cbc_decrypt(&[1; 20], &stream).is_empty());
    }

    #[test]
    fn aes128_se_mantiene() {
        let key: Vec<u8> = (0..16).collect();
        let aes = Aes::new(&key).unwrap();
        let datos = [&[3u8; 16][..], &aes.cbc(&[3; 16], &[[b'a'; 12].as_slice(), &[4; 4]].concat(), true)].concat();
        assert_eq!(aes128_cbc_decrypt(&key, &datos), b"aaaaaaaaaaaa");
        assert!(aes128_cbc_decrypt(&[0; 32], &datos).is_empty(), "solo claves de 16 bytes");
    }

    /// Prueba el cableado (diccionario /Encrypt indirecto, cadenas cifradas): el archivo se arma
    /// con las funciones de este modulo. La conformidad con otra implementacion la prueba
    /// `tests/cifrado.rs` con un PDF hecho por pypdf.
    #[test]
    fn pdf_aes256_revision_5() {
        let doc = crate::pdf::read(&pdf_revision_5(false)).unwrap();
        let json = crate::doc::to_json(&doc, false);
        assert!(json.contains("\"title\":\"Titulo cifrado\""), "{json}");
        assert!(json.contains("\"text\":\"Hola revision 5\""), "{json}");
        assert!(json.contains("\"encrypted\":true"), "{json}");
        assert_eq!(crate::pdf::read(&pdf_revision_5(true)).err().as_deref(), Some(PIDE_CONTRASENA));
    }
}
