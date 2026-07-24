//! Pins the `Random` stream to reference values generated from an INDEPENDENT
//! transcription of the published algorithms (Vigna's splitmix64.c +
//! xoshiro256starstar.c — scripted in Python, not derived from this Rust).
//! The stream is a documented stability contract: same seed, same stream,
//! every platform, every Quoin version. If one of these pins ever fails, the
//! contract broke — fix the code, never the pin.

use super::*;

#[test]
fn raw_stream_matches_reference_seed_0() {
    let mut r = NativeRandom::from_seed(0);
    assert_eq!(r.next_u64(), 11091344671253066420);
    assert_eq!(r.next_u64(), 13793997310169335082);
    assert_eq!(r.next_u64(), 1900383378846508768);
    assert_eq!(r.next_u64(), 7684712102626143532);
}

#[test]
fn raw_stream_matches_reference_seed_42() {
    let mut r = NativeRandom::from_seed(42);
    assert_eq!(r.next_u64(), 1546998764402558742);
    assert_eq!(r.next_u64(), 6990951692964543102);
    assert_eq!(r.next_u64(), 12544586762248559009);
    assert_eq!(r.next_u64(), 17057574109182124193);
}

#[test]
fn negative_seeds_are_their_own_streams() {
    // Seeded as the u64 bit pattern — a negative seed is legal and distinct.
    let mut r = NativeRandom::from_seed(-1);
    assert_eq!(r.next_u64(), 10328197420357168392);
    assert_eq!(r.next_u64(), 14156678507024973869);
}

#[test]
fn bounded_matches_reference_and_stays_in_range() {
    let mut r = NativeRandom::from_seed(42);
    let draws: Vec<u64> = (0..6).map(|_| r.bounded(100)).collect();
    assert_eq!(draws, [42, 2, 9, 93, 76, 84]);

    let mut r = NativeRandom::from_seed(3);
    for _ in 0..10_000 {
        assert!(r.bounded(7) < 7);
    }
    // n = 1 has one possible answer and must not spin in the rejection loop.
    assert_eq!(NativeRandom::from_seed(3).bounded(1), 0);
}

#[test]
fn next_f64_is_half_open_unit_and_matches_reference() {
    let mut r = NativeRandom::from_seed(1);
    let a = r.next_f64();
    assert!((a - 0.7029218331588505).abs() < 1e-15);
    for _ in 0..10_000 {
        let v = r.next_f64();
        assert!((0.0..1.0).contains(&v));
    }
}

#[test]
fn equal_seeds_equal_streams() {
    let mut a = NativeRandom::from_seed(123456789);
    let mut b = NativeRandom::from_seed(123456789);
    for _ in 0..100 {
        assert_eq!(a.next_u64(), b.next_u64());
    }
}
