//! BFME2 v1.00 custom 192-bit PRNG (`GameLogicRandomValue`).
//!
//! Reverse-engineered from `game.dat` (functions at `0x00633770`, `0x0063363f`, `0x006336b6`).
//! Verified against live Frida trace: 27/27 outputs matched exactly for the 3dwarf replay.
//!
//! **Not** any standard LCG (MSVC, glibc, etc.) — this is a custom algorithm with
//! 6 x u32 state words, cascading-sum-with-carry output, and counter-style increment.

/// The six fixed offsets used to derive the initial state from a single u32 seed.
/// Matches `FUN_006336b6` in game.dat exactly.
const SEED_OFFSETS: [u32; 6] = [
    0xF22D_0E56,
    0x9604_1893, // = -0x69FBE76D as u32
    0x3DF3_B646,
    0x40DD_E76D,
    0x9732_7AE1, // = -0x68CD851F as u32
    0xD1A9_FBE7,
];

/// BFME2 v1.00 192-bit PRNG. Each step returns a u32.
pub struct Bfme2Rand {
    state: [u32; 6],
}

impl Bfme2Rand {
    /// Seed with a single u32 (the SD value from the replay header).
    pub fn new(seed: u32) -> Self {
        let mut state = [0u32; 6];
        let mut acc = seed;
        for (i, off) in SEED_OFFSETS.iter().enumerate() {
            acc = acc.wrapping_add(*off);
            state[i] = acc;
        }
        Self { state }
    }

    /// One PRNG step: advances state and returns raw u32. Mirrors `FUN_0063363f`.
    pub fn step(&mut self) -> u32 {
        let s = self.state;

        // Cascading sum with carry propagation (exact transcription of asm at 0x0063363f).
        let mut eax = s[4].wrapping_add(s[5]);
        let mut c: u32 = if eax < s[5] { 1 } else { 0 };
        let new_s4 = eax;

        let prev = s[3];
        eax = eax.wrapping_add(c).wrapping_add(prev);
        c = if eax < prev { 1 } else { 0 };
        let new_s3 = eax;

        let prev = s[2];
        eax = eax.wrapping_add(c).wrapping_add(prev);
        c = if eax < prev { 1 } else { 0 };
        let new_s2 = eax;

        let prev = s[1];
        eax = eax.wrapping_add(c).wrapping_add(prev);
        c = if eax < prev { 1 } else { 0 };
        let mut new_s1 = eax;

        // EAX += CF4 (only the carry, NOT + s0)
        eax = eax.wrapping_add(c);

        let mut new_s0 = s[0].wrapping_add(eax);

        // Counter increment: s5 += 1, propagate carry up.
        let mut new_s4 = new_s4;
        let mut new_s3 = new_s3;
        let mut new_s2 = new_s2;
        let new_s5 = s[5].wrapping_add(1);
        if new_s5 == 0 {
            new_s4 = new_s4.wrapping_add(1);
            if new_s4 == 0 {
                new_s3 = new_s3.wrapping_add(1);
                if new_s3 == 0 {
                    new_s2 = new_s2.wrapping_add(1);
                    if new_s2 == 0 {
                        new_s1 = new_s1.wrapping_add(1);
                        if new_s1 == 0 {
                            new_s0 = new_s0.wrapping_add(1);
                        }
                    }
                }
            }
        }

        self.state = [new_s0, new_s1, new_s2, new_s3, new_s4, new_s5];
        new_s0
    }

    /// `GameLogicRandomValue(min, max)` — returns int in `[min, max]` inclusive.
    /// Mirrors `FUN_00633770`.
    pub fn logic_random(&mut self, min_val: i32, max_val: i32) -> i32 {
        let range = max_val.wrapping_sub(min_val).wrapping_add(1);
        if range == 0 {
            return min_val;
        }
        let r = self.step();
        (r % range as u32) as i32 + min_val
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verified against live Frida trace of game.dat playing 3dwarf.BfME2Replay (SD=442667640).
    /// All 27 values match the game's GameLogicRandomValue output exactly.
    #[test]
    fn matches_frida_trace_3dwarf() {
        let mut r = Bfme2Rand::new(442_667_640);
        // (max, expected_return) pairs from rng_trace.log
        let expected = [
            (5, 5),
            (5, 1), // phase 1 StartPos observers
            (1, 0),
            (1, 1),
            (1000, 290), // slot 0 faction warmup+pick
            (1, 0),
            (1, 1),
            (1000, 92),
            (9, 9), // slot 1 mustafaa → 9
            (1, 0),
            (1, 0),
            (1000, 644), // slot 2
            (1, 0),
            (1, 0),
            (1000, 952), // slot 3
            (1, 0),
            (1, 1),
            (1000, 768), // slot 4
            (9, 6),
            (9, 5), // slot 5 observer color → 5
            (9, 6),
            (9, 5),
            (9, 4), // slot 6 observer color → 4
            (1, 1),
            (1, 0),
            (1000, 842),
            (9, 1), // slot 7 Gusto → 1
        ];
        for (i, (max, exp)) in expected.iter().enumerate() {
            let got = r.logic_random(0, *max);
            assert_eq!(
                got, *exp,
                "mismatch at call {}: expected {}, got {}",
                i, exp, got
            );
        }
    }

    #[test]
    fn seed_zero_initial_state() {
        let r = Bfme2Rand::new(0);
        assert_eq!(
            r.state,
            [
                0xF22D_0E56,
                0x8831_26E9,
                0xC624_DD2F,
                0x0702_C49C,
                0x9E35_3F7D,
                0x6FDF_3B64
            ]
        );
    }
}
