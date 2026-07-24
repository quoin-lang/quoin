//! `Random` — the seedable PRNG for simulations, property tests, and
//! deterministic bots: the same seed answers the same stream on every platform
//! and every Quoin version. The generator is xoshiro256** with the seed
//! expanded through SplitMix64, and that pair is the documented stability
//! CONTRACT, not an implementation detail — replay harnesses and
//! "print the seed on failure" test patterns depend on the stream never
//! changing under them. Not for secrets: `[Crypto]Random` answers OS CSPRNG
//! bytes and is deliberately not seedable; this class is the other half of
//! that sentence.

use crate::arg;
use crate::error::QuoinError;
use crate::runtime::list::NativeListState;
use crate::value::{AnyCollect, NativeClassBuilder, Value};
use crate::vm::VmState;

use gc_arena::Mutation;
use gc_arena::collect::Trace;
use std::any::Any;

pub struct NativeRandom {
    state: [u64; 4],
    /// The seed this generator was built from. `Random.new` remembers the one
    /// it drew so a failing run can print it and be replayed via `Random.seed:`.
    seed: i64,
}

impl std::fmt::Debug for NativeRandom {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Random{{seed:{}}}", self.seed)
    }
}

impl AnyCollect for NativeRandom {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    fn trace_gc<'gc>(&self, _cc: &mut dyn Trace<'gc>) {} // no Gc fields
}

/// SplitMix64 (Steele/Lea/Flood) — the seed expander xoshiro's authors
/// prescribe: consecutive outputs fill the state so even seeds 0/1/2… land on
/// well-mixed, distinct streams.
fn splitmix64(x: &mut u64) -> u64 {
    *x = x.wrapping_add(0x9E3779B97F4A7C15);
    let mut z = *x;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
    z ^ (z >> 31)
}

impl NativeRandom {
    pub fn from_seed(seed: i64) -> Self {
        let mut x = seed as u64;
        let mut state = [0u64; 4];
        for s in &mut state {
            *s = splitmix64(&mut x);
        }
        // xoshiro's one forbidden state. SplitMix64 can't practically produce
        // it (probability 2^-256), but the guard costs nothing.
        if state == [0; 4] {
            state[0] = 1;
        }
        NativeRandom { state, seed }
    }

    /// xoshiro256** (Blackman/Vigna) — reference algorithm, verbatim.
    pub fn next_u64(&mut self) -> u64 {
        let s = &mut self.state;
        let result = s[1].wrapping_mul(5).rotate_left(7).wrapping_mul(9);
        let t = s[1] << 17;
        s[2] ^= s[0];
        s[3] ^= s[1];
        s[1] ^= s[2];
        s[0] ^= s[3];
        s[2] ^= t;
        s[3] = s[3].rotate_left(45);
        result
    }

    /// Unbiased integer in `[0, n)`: reject the sliver at the top of the u64
    /// range that would make `% n` favor small results.
    fn bounded(&mut self, n: u64) -> u64 {
        let threshold = n.wrapping_neg() % n;
        loop {
            let x = self.next_u64();
            if x >= threshold {
                return x % n;
            }
        }
    }

    /// The next Double in `[0, 1)`: the top 53 bits, evenly spaced — every
    /// value is exactly representable, 0.0 possible, 1.0 never.
    fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
    }
}

fn make_random<'gc>(vm: &VmState<'gc>, mc: &Mutation<'gc>, seed: i64) -> Value<'gc> {
    let class = vm.get_or_create_builtin_class(mc, "Random");
    vm.new_native_state(mc, class, NativeRandom::from_seed(seed))
}

/// Read a List argument's elements (a copy — the borrow must not be held
/// while the receiver's state is borrowed mutably for the draws).
fn list_arg<'gc>(val: Value<'gc>) -> Result<Vec<Value<'gc>>, QuoinError> {
    val.with_native_state::<NativeListState, _, _>(|l| l.get_vec().to_vec())
        .map_err(QuoinError::Other)
}

pub fn build_random_class() -> NativeClassBuilder {
    NativeClassBuilder::new("Random", Some("Object"))
        .class_doc(
            "The seedable pseudo-random generator — for simulations, property tests, \
             and anything that must REPLAY: the same seed answers the same stream on \
             every platform and every Quoin version (xoshiro256** seeded via \
             SplitMix64 — the algorithm is part of the contract). Draws are per \
             instance; there is no hidden global generator. Not for secrets: \
             `[Crypto]Random` answers OS CSPRNG bytes and is deliberately not \
             seedable.\n\n\
             ```\n\
             var r = Random.seed:2026\n\
             r.int:100                       \"* -> 9\n\
             r.int:100                       \"* -> 12\n\
             (Random.seed:2026).int:100      \"* -> 9\n\
             ```",
        )
        .typed_class_method("seed:", &["Integer"], |vm, mc, _receiver, args| {
            let seed = arg!(args, Int, 0);
            Ok(make_random(vm, mc, seed))
        })
        .returns("Random")
        .doc(
            "A deterministic generator: equal seeds answer equal streams, forever.\n\n\
             ```\n\
             (Random.seed:7).int:1000     \"* -> 994\n\
             (Random.seed:7).int:1000     \"* -> 994\n\
             ```",
        )
        .class_method("new", |vm, mc, _receiver, _args| {
            let mut buf = [0u8; 8];
            getrandom::fill(&mut buf)
                .map_err(|e| QuoinError::Other(format!("OS entropy failed: {e}")))?;
            // Mask to non-negative so printed seeds read clean; 63 bits of
            // entropy is plenty for a simulation seed.
            let seed = (u64::from_le_bytes(buf) & (i64::MAX as u64)) as i64;
            Ok(make_random(vm, mc, seed))
        })
        .returns("Random")
        .doc(
            "A generator seeded from OS entropy — for when any stream will do. It \
             remembers the seed it drew (`seed`), so a failing simulation can print \
             it and be replayed exactly with `Random.seed:`.\n\n\
             ```\n\
             var r = Random.new\n\
             ((Random.seed:r.seed).int:100) == (r.int:100)     \"* -> true\n\
             ```",
        )
        .instance_method("seed", |vm, mc, receiver, _args| {
            receiver
                .with_native_state::<NativeRandom, _, _>(|r| r.seed)
                .map_err(QuoinError::Other)
                .map(|s| vm.new_int(mc, s))
        })
        .returns("Integer")
        .doc(
            "The seed this generator was built from — `Random.seed:` it to replay \
             the stream from the start.",
        )
        .instance_method("next", |vm, mc, receiver, _args| {
            receiver
                .with_native_state_mut::<NativeRandom, _, _>(mc, |r| r.next_f64())
                .map_err(QuoinError::Other)
                .map(|f| vm.new_double(mc, f))
        })
        .returns("Double")
        .doc(
            "The next Double in [0, 1) — the stream's top 53 bits, evenly spaced \
             (0.0 possible, 1.0 never).\n\n\
             ```\n\
             (Random.seed:1).next < 1     \"* -> true\n\
             ```",
        )
        .typed_instance_method("int:", &["Integer"], |vm, mc, receiver, args| {
            let n = arg!(args, Int, 0);
            if n <= 0 {
                return Err(QuoinError::ValueError(format!(
                    "int: answers in [0, n) and needs a bound >= 1, got {n}"
                )));
            }
            let v = receiver
                .with_native_state_mut::<NativeRandom, _, _>(mc, |r| r.bounded(n as u64))
                .map_err(QuoinError::Other)?;
            Ok(vm.new_int(mc, v as i64))
        })
        .returns("Integer")
        .doc(
            "An unbiased Integer in [0, n) — end-exclusive, like ranges. The bound \
             must be >= 1 (a ValueError otherwise).\n\n\
             ```\n\
             (Random.seed:3).int:6     \"* -> 2\n\
             (Random.seed:3).int:1     \"* -> 0\n\
             ```",
        )
        .typed_instance_method("pick:", &["List"], |_vm, mc, receiver, args| {
            let elems = list_arg(args[0])?;
            if elems.is_empty() {
                return Err(QuoinError::IndexError {
                    index: 0,
                    len: 0,
                    msg: "pick: there is nothing to pick from an empty List".to_string(),
                });
            }
            let i = receiver
                .with_native_state_mut::<NativeRandom, _, _>(mc, |r| r.bounded(elems.len() as u64))
                .map_err(QuoinError::Other)? as usize;
            Ok(elems[i])
        })
        .returns("Object")
        .doc(
            "A uniformly random element of the List (an IndexError when it is \
             empty). Consumes one draw.\n\n\
             ```\n\
             (Random.seed:5).pick:#('a' 'b' 'c')     \"* -> 'c'\n\
             ```",
        )
        .typed_instance_method("shuffle:", &["List"], |vm, mc, receiver, args| {
            let mut elems = list_arg(args[0])?;
            receiver
                .with_native_state_mut::<NativeRandom, _, _>(mc, |r| {
                    // Fisher-Yates, high index down — the draw count (len - 1)
                    // and order are part of the stream contract.
                    for i in (1..elems.len()).rev() {
                        let j = r.bounded(i as u64 + 1) as usize;
                        elems.swap(i, j);
                    }
                })
                .map_err(QuoinError::Other)?;
            Ok(vm.new_list(mc, elems))
        })
        .returns("List")
        .doc(
            "A NEW List with the elements in uniformly random order (Fisher-Yates); \
             the argument is untouched.\n\n\
             ```\n\
             (Random.seed:9).shuffle:#(1 2 3 4 5)     \"* -> #(4 5 3 2 1)\n\
             ```",
        )
        .typed_instance_method("bytes:", &["Integer"], |vm, mc, receiver, args| {
            let n = arg!(args, Int, 0);
            let n = usize::try_from(n).map_err(|_| {
                QuoinError::ValueError(format!("bytes: needs a count >= 0, got {n}"))
            })?;
            let buf = receiver
                .with_native_state_mut::<NativeRandom, _, _>(mc, |r| {
                    let mut out = Vec::with_capacity(n.next_multiple_of(8));
                    while out.len() < n {
                        out.extend_from_slice(&r.next_u64().to_le_bytes());
                    }
                    out.truncate(n);
                    out
                })
                .map_err(QuoinError::Other)?;
            Ok(vm.new_bytes(mc, buf))
        })
        .returns("Bytes")
        .doc(
            "N deterministic bytes — the u64 stream in little-endian chunks (the \
             layout is part of the contract). Reproducible test fixtures; for keys, \
             tokens, and salts use `[Crypto]Random.bytes:` instead.\n\n\
             ```\n\
             ((Random.seed:11).bytes:4).toHex     \"* -> 'dfa73969'\n\
             ```",
        )
        .instance_method("s", |vm, mc, receiver, _args| {
            let seed = receiver
                .with_native_state::<NativeRandom, _, _>(|r| r.seed)
                .map_err(QuoinError::Other)?;
            Ok(vm.new_string(mc, format!("Random(seed: {seed})")))
        })
        .doc("A short description naming the seed.")
}

#[cfg(test)]
#[path = "random_tests.rs"]
mod random_tests;
