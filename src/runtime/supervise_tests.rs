//! The backoff curve's arithmetic: doubling from `backoff` and clamped at
//! `cap`, with the shift bounded so absurd attempt numbers cannot overflow.

use super::SupervisePolicy;

#[test]
fn delays_double_and_cap() {
    let p = SupervisePolicy {
        backoff_ms: 100,
        cap_ms: 1000,
        max_restarts: 5,
        window_ms: 60_000,
    };
    assert_eq!(p.delay_ms(1), 100);
    assert_eq!(p.delay_ms(2), 200);
    assert_eq!(p.delay_ms(3), 400);
    assert_eq!(p.delay_ms(4), 800);
    assert_eq!(p.delay_ms(5), 1000);
    assert_eq!(
        p.delay_ms(60),
        1000,
        "huge attempts stay capped, no overflow"
    );
}
