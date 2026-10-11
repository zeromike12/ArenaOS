//! ADR-0110: narrow kernel CSPRNG seeded only through the rngd seed cap.
//!
//! This is intentionally not a general entropy API. The production rngd
//! receives exactly one WRITE-only `KernelEntropySeed` cap, submits 32 bytes
//! obtained from virtio-rng after DRIVER_OK, and no application can read this
//! state. ChaCha20 is used as a stream generator; bounded rejection sampling
//! provides unbiased placement slots without a modulo-only shortcut.

use crate::sync::{SyncCell, without_interrupts};

const CHACHA_CONSTANTS: [u32; 4] = [
    0x6170_7865, // "expa"
    0x3320_646e, // "nd 3"
    0x7962_2d32, // "2-by"
    0x6b20_6574, // "te k"
];

#[derive(Clone, Copy)]
struct Generator {
    seeded: bool,
    key: [u32; 8],
    next_counter: u64,
    block: [u8; 64],
    offset: usize,
}

impl Generator {
    const EMPTY: Self = Self {
        seeded: false,
        key: [0; 8],
        next_counter: 0,
        block: [0; 64],
        offset: 64,
    };

    fn reseed(&mut self, seed: &[u8; 32]) -> bool {
        let mut key_bytes = *seed;
        if self.seeded {
            let mut mixing = [0u8; 32];
            if !self.fill(&mut mixing) {
                return false;
            }
            for (new, prior) in key_bytes.iter_mut().zip(mixing) {
                *new ^= prior;
            }
            mixing.fill(0);
        }
        for (index, word) in self.key.iter_mut().enumerate() {
            let at = index * 4;
            *word = u32::from_le_bytes([
                key_bytes[at],
                key_bytes[at + 1],
                key_bytes[at + 2],
                key_bytes[at + 3],
            ]);
        }
        key_bytes.fill(0);
        self.seeded = true;
        self.next_counter = 0;
        self.block.fill(0);
        self.offset = 64;
        true
    }

    fn fill(&mut self, out: &mut [u8]) -> bool {
        if !self.seeded {
            return false;
        }
        for byte in out {
            if self.offset == self.block.len() {
                if self.next_counter > u32::MAX as u64 {
                    return false;
                }
                let words = chacha20_block(self.key, self.next_counter as u32, [0; 3]);
                for (index, word) in words.iter().enumerate() {
                    self.block[index * 4..index * 4 + 4].copy_from_slice(&word.to_le_bytes());
                }
                self.next_counter += 1;
                self.offset = 0;
            }
            *byte = self.block[self.offset];
            self.block[self.offset] = 0;
            self.offset += 1;
        }
        true
    }
}

static GENERATOR: SyncCell<Generator> = SyncCell::new(Generator::EMPTY);

/// Install or mix a fresh 256-bit seed. Called only after the caller's exact
/// seed capability and buffer have been validated by the syscall boundary.
pub fn seed(bytes: &[u8; 32]) -> bool {
    without_interrupts(|| unsafe { (*GENERATOR.get()).reseed(bytes) })
}

pub fn is_ready() -> bool {
    without_interrupts(|| unsafe {
        let generator = &*GENERATOR.get();
        generator.seeded && generator.next_counter <= u32::MAX as u64
    })
}

fn random_u64(generator: &mut Generator) -> Option<u64> {
    let mut bytes = [0u8; 8];
    if !generator.fill(&mut bytes) {
        return None;
    }
    Some(u64::from_le_bytes(bytes))
}

/// Return an unbiased integer in `0..upper`. Refuses unseeded/exhausted state
/// and has a fixed retry bound so a corrupted generator cannot hang a spawn.
pub fn uniform_below(upper: u64) -> Option<u64> {
    if upper == 0 {
        return None;
    }
    without_interrupts(|| unsafe {
        let generator = &mut *GENERATOR.get();
        let threshold = upper.wrapping_neg() % upper;
        for _ in 0..16 {
            let value = random_u64(generator)?;
            if value >= threshold {
                return Some(value % upper);
            }
        }
        None
    })
}

fn quarter_round(state: &mut [u32; 16], a: usize, b: usize, c: usize, d: usize) {
    state[a] = state[a].wrapping_add(state[b]);
    state[d] = (state[d] ^ state[a]).rotate_left(16);
    state[c] = state[c].wrapping_add(state[d]);
    state[b] = (state[b] ^ state[c]).rotate_left(12);
    state[a] = state[a].wrapping_add(state[b]);
    state[d] = (state[d] ^ state[a]).rotate_left(8);
    state[c] = state[c].wrapping_add(state[d]);
    state[b] = (state[b] ^ state[c]).rotate_left(7);
}

fn chacha20_block(key: [u32; 8], counter: u32, nonce: [u32; 3]) -> [u32; 16] {
    let initial = [
        CHACHA_CONSTANTS[0],
        CHACHA_CONSTANTS[1],
        CHACHA_CONSTANTS[2],
        CHACHA_CONSTANTS[3],
        key[0],
        key[1],
        key[2],
        key[3],
        key[4],
        key[5],
        key[6],
        key[7],
        counter,
        nonce[0],
        nonce[1],
        nonce[2],
    ];
    let mut working = initial;
    for _ in 0..10 {
        quarter_round(&mut working, 0, 4, 8, 12);
        quarter_round(&mut working, 1, 5, 9, 13);
        quarter_round(&mut working, 2, 6, 10, 14);
        quarter_round(&mut working, 3, 7, 11, 15);
        quarter_round(&mut working, 0, 5, 10, 15);
        quarter_round(&mut working, 1, 6, 11, 12);
        quarter_round(&mut working, 2, 7, 8, 13);
        quarter_round(&mut working, 3, 4, 9, 14);
    }
    for (word, original) in working.iter_mut().zip(initial) {
        *word = word.wrapping_add(original);
    }
    working
}

/// RFC 8439 block-function known-answer check used by the boot qualification.
pub fn self_test() -> bool {
    let mut key = [0u32; 8];
    for (index, word) in key.iter_mut().enumerate() {
        let at = index * 4;
        *word = u32::from_le_bytes([at as u8, at as u8 + 1, at as u8 + 2, at as u8 + 3]);
    }
    let block = chacha20_block(key, 1, [0x0900_0000, 0x4a00_0000, 0]);
    let mut first = [0u8; 16];
    for (index, word) in block[..4].iter().enumerate() {
        first[index * 4..index * 4 + 4].copy_from_slice(&word.to_le_bytes());
    }
    first
        == [
            0x10, 0xf1, 0xe7, 0xe4, 0xd1, 0x3b, 0x59, 0x15, 0x50, 0x0f, 0xdd, 0x1f, 0xa3, 0x20,
            0x71, 0xc4,
        ]
}
