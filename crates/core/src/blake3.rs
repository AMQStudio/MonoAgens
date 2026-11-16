//! Pure-Rust BLAKE3-256 implementation with zero external dependencies.
//!
//! Used by `Core::open` and `Core::take_snapshot` to verify `blake3` snapshot
//! checksums inside `monoagens-core` while preserving the `protocol + serde`
//! dependency boundary. Output matches the official BLAKE3 specification bit-for-bit.

const OUT_LEN: usize = 32;
const BLOCK_LEN: usize = 64;
const CHUNK_LEN: usize = 1024;

const CHUNK_START: u32 = 1 << 0;
const CHUNK_END: u32 = 1 << 1;
const PARENT: u32 = 1 << 2;
const ROOT: u32 = 1 << 3;

const IV: [u32; 8] = [
    0x6A09_E667,
    0xBB67_AE85,
    0x3C6E_F372,
    0xA54F_F53A,
    0x510E_527F,
    0x9B05_688C,
    0x1F83_D9AB,
    0x5BE0_CD19,
];

const MSG_PERMUTATION: [usize; 16] = [2, 6, 3, 10, 7, 0, 4, 13, 1, 11, 12, 5, 9, 14, 15, 8];

#[inline]
fn g(state: &mut [u32; 16], a: usize, b: usize, c: usize, d: usize, mx: u32, my: u32) {
    state[a] = state[a].wrapping_add(state[b]).wrapping_add(mx);
    state[d] = (state[d] ^ state[a]).rotate_right(16);
    state[c] = state[c].wrapping_add(state[d]);
    state[b] = (state[b] ^ state[c]).rotate_right(12);
    state[a] = state[a].wrapping_add(state[b]).wrapping_add(my);
    state[d] = (state[d] ^ state[a]).rotate_right(8);
    state[c] = state[c].wrapping_add(state[d]);
    state[b] = (state[b] ^ state[c]).rotate_right(7);
}

fn round(state: &mut [u32; 16], m: &[u32; 16]) {
    g(state, 0, 4, 8, 12, m[0], m[1]);
    g(state, 1, 5, 9, 13, m[2], m[3]);
    g(state, 2, 6, 10, 14, m[4], m[5]);
    g(state, 3, 7, 11, 15, m[6], m[7]);
    g(state, 0, 5, 10, 15, m[8], m[9]);
    g(state, 1, 6, 11, 12, m[10], m[11]);
    g(state, 2, 7, 8, 13, m[12], m[13]);
    g(state, 3, 4, 9, 14, m[14], m[15]);
}

fn permute(m: &mut [u32; 16]) {
    let mut permuted = [0u32; 16];
    for (i, &src) in MSG_PERMUTATION.iter().enumerate() {
        permuted[i] = m[src];
    }
    *m = permuted;
}

fn compress(
    chaining_value: &[u32; 8],
    block_words: &[u32; 16],
    counter: u64,
    block_len: u32,
    flags: u32,
) -> [u32; 16] {
    let mut state = [
        chaining_value[0],
        chaining_value[1],
        chaining_value[2],
        chaining_value[3],
        chaining_value[4],
        chaining_value[5],
        chaining_value[6],
        chaining_value[7],
        IV[0],
        IV[1],
        IV[2],
        IV[3],
        counter as u32,
        (counter >> 32) as u32,
        block_len,
        flags,
    ];
    let mut block = *block_words;

    round(&mut state, &block);
    permute(&mut block);
    round(&mut state, &block);
    permute(&mut block);
    round(&mut state, &block);
    permute(&mut block);
    round(&mut state, &block);
    permute(&mut block);
    round(&mut state, &block);
    permute(&mut block);
    round(&mut state, &block);
    permute(&mut block);
    round(&mut state, &block);

    for i in 0..8 {
        state[i] ^= state[i + 8];
        state[i + 8] ^= chaining_value[i];
    }
    state
}

fn words_from_le_bytes_64(bytes: &[u8; BLOCK_LEN]) -> [u32; 16] {
    let mut out = [0u32; 16];
    for (i, chunk) in bytes.as_chunks::<4>().0.iter().enumerate() {
        out[i] = u32::from_le_bytes(*chunk);
    }
    out
}

#[derive(Clone, Copy)]
struct Output {
    input_chaining_value: [u32; 8],
    block_words: [u32; 16],
    counter: u64,
    block_len: u32,
    flags: u32,
}

impl Output {
    fn chaining_value(&self) -> [u32; 8] {
        let compressed = compress(
            &self.input_chaining_value,
            &self.block_words,
            self.counter,
            self.block_len,
            self.flags,
        );
        let mut out = [0u32; 8];
        out.copy_from_slice(&compressed[..8]);
        out
    }

    fn root_hash_bytes(&self) -> [u8; OUT_LEN] {
        let compressed = compress(
            &self.input_chaining_value,
            &self.block_words,
            0,
            self.block_len,
            self.flags | ROOT,
        );
        let mut out = [0u8; OUT_LEN];
        for (i, &word) in compressed[..8].iter().enumerate() {
            out[i * 4..(i + 1) * 4].copy_from_slice(&word.to_le_bytes());
        }
        out
    }
}

struct ChunkState {
    chaining_value: [u32; 8],
    chunk_counter: u64,
    block: [u8; BLOCK_LEN],
    block_len: u8,
    blocks_compressed: u8,
}

impl ChunkState {
    fn new(chunk_counter: u64) -> Self {
        Self {
            chaining_value: IV,
            chunk_counter,
            block: [0u8; BLOCK_LEN],
            block_len: 0,
            blocks_compressed: 0,
        }
    }

    fn len(&self) -> usize {
        BLOCK_LEN * usize::from(self.blocks_compressed) + usize::from(self.block_len)
    }

    fn start_flag(&self) -> u32 {
        if self.blocks_compressed == 0 {
            CHUNK_START
        } else {
            0
        }
    }

    fn update(&mut self, mut input: &[u8]) {
        while !input.is_empty() {
            if usize::from(self.block_len) == BLOCK_LEN {
                let block_words = words_from_le_bytes_64(&self.block);
                let compressed = compress(
                    &self.chaining_value,
                    &block_words,
                    self.chunk_counter,
                    BLOCK_LEN as u32,
                    self.start_flag(),
                );
                self.chaining_value.copy_from_slice(&compressed[..8]);
                self.blocks_compressed += 1;
                self.block = [0u8; BLOCK_LEN];
                self.block_len = 0;
            }

            let want = BLOCK_LEN - usize::from(self.block_len);
            let take = want.min(input.len());
            let start = usize::from(self.block_len);
            self.block[start..start + take].copy_from_slice(&input[..take]);
            self.block_len += take as u8;
            input = &input[take..];
        }
    }

    fn output(&self) -> Output {
        let block_words = words_from_le_bytes_64(&self.block);
        Output {
            input_chaining_value: self.chaining_value,
            block_words,
            counter: self.chunk_counter,
            block_len: u32::from(self.block_len),
            flags: self.start_flag() | CHUNK_END,
        }
    }
}

fn parent_output(left_cv: [u32; 8], right_cv: [u32; 8]) -> Output {
    let mut block_words = [0u32; 16];
    block_words[..8].copy_from_slice(&left_cv);
    block_words[8..].copy_from_slice(&right_cv);
    Output {
        input_chaining_value: IV,
        block_words,
        counter: 0,
        block_len: BLOCK_LEN as u32,
        flags: PARENT,
    }
}

/// Computes the standard 32-byte BLAKE3-256 digest of `input`.
#[must_use]
pub fn hash(mut input: &[u8]) -> [u8; OUT_LEN] {
    let mut chunk_state = ChunkState::new(0);
    let mut cv_stack: Vec<[u32; 8]> = Vec::new();

    while !input.is_empty() {
        if chunk_state.len() == CHUNK_LEN {
            let chunk_cv = chunk_state.output().chaining_value();
            let total_chunks = chunk_state.chunk_counter + 1;
            push_cv(&mut cv_stack, chunk_cv, total_chunks);
            chunk_state = ChunkState::new(total_chunks);
        }

        let want = CHUNK_LEN - chunk_state.len();
        let take = want.min(input.len());
        chunk_state.update(&input[..take]);
        input = &input[take..];
    }

    let mut output = chunk_state.output();
    while let Some(left_cv) = cv_stack.pop() {
        output = parent_output(left_cv, output.chaining_value());
    }
    output.root_hash_bytes()
}

fn push_cv(cv_stack: &mut Vec<[u32; 8]>, mut new_cv: [u32; 8], mut total_chunks: u64) {
    while total_chunks & 1 == 0 {
        if let Some(left_cv) = cv_stack.pop() {
            new_cv = parent_output(left_cv, new_cv).chaining_value();
        }
        total_chunks >>= 1;
    }
    cv_stack.push(new_cv);
}

/// Formats a 32-byte BLAKE3 digest as lowercase hexadecimal.
#[must_use]
pub fn to_hex(digest: &[u8; OUT_LEN]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(OUT_LEN * 2);
    for &b in digest {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{hash, to_hex};

    #[test]
    fn blake3_matches_official_test_vectors() {
        // Official BLAKE3 vector for empty input ""
        assert_eq!(
            to_hex(&hash(b"")),
            "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"
        );

        // Official BLAKE3 vector for "abc"
        // Wait: let's verify known official BLAKE3 vector for sequential pattern 0..251
        // Official BLAKE3 spec test vector: input len 3 = [0, 1, 2]
        assert_eq!(
            to_hex(&hash(&[0, 1, 2])),
            "e1be4d7a8ab5560aa4199eea339849ba8e293d55ca0a81006726d184519e647f"
        );

        // Multi-chunk input (length 1025 = [i % 251]) from official BLAKE3 test vectors
        let vec_1025: Vec<u8> = (0..1025).map(|i| (i % 251) as u8).collect();
        assert_eq!(
            to_hex(&hash(&vec_1025)),
            "d00278ae47eb27b34faecf67b4fe263f82d5412916c1ffd97c8cb7fb814b8444"
        );
    }
}
