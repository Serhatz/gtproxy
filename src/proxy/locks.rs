//! Lock arithmetic for `/cd` and `/balance`.
//!
//! 1 Diamond Lock = 100 World Locks, 1 Blue Gem Lock = 100 Diamond Locks.
//! "Using" a lock from the inventory (`ItemActivateRequest`) converts it:
//! 100 WL -> 1 DL, 1 DL -> 100 WL, 1 BGL -> 100 DL. One slot holds at most 200.

pub const WL: u32 = 242;
pub const DL: u32 = 1796;
pub const BGL: u32 = 7188;

const STACK: u32 = 200;

pub fn total_wl(wl: u32, dl: u32, bgl: u32) -> u32 {
    wl + dl * 100 + bgl * 10_000
}

#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    /// Use the lock once (convert it).
    Convert(u32),
    Drop { item_id: u32, amount: u32 },
}

/// Plans dropping exactly `amount` World Locks' worth from the given stacks,
/// breaking or merging locks where the change does not fit.
pub fn plan_custom_drop(amount: u32, mut wl: u32, mut dl: u32, mut bgl: u32) -> Result<Vec<Step>, String> {
    if amount == 0 {
        return Err("amount must be at least 1".into());
    }
    let have = total_wl(wl, dl, bgl);
    if have < amount {
        return Err(format!("not enough locks: you have {have} WL worth"));
    }
    let mut steps = Vec::new();
    for _ in 0..64 {
        let drop_bgl = (amount / 10_000).min(bgl);
        let rest = amount - drop_bgl * 10_000;
        let drop_dl = (rest / 100).min(dl);
        let drop_wl = rest - drop_dl * 100;

        if drop_wl <= wl {
            for (item_id, n) in [(BGL, drop_bgl), (DL, drop_dl), (WL, drop_wl)] {
                if n > 0 {
                    steps.push(Step::Drop { item_id, amount: n });
                }
            }
            return Ok(steps);
        }
        // Too few World Locks for the change: break a bigger lock.
        if dl > drop_dl && wl + 100 <= STACK {
            steps.push(Step::Convert(DL));
            dl -= 1;
            wl += 100;
        } else if bgl > drop_bgl && dl + 100 <= STACK {
            steps.push(Step::Convert(BGL));
            bgl -= 1;
            dl += 100;
        } else {
            return Err("cannot make change without overflowing a 200 stack; drop some locks first".into());
        }
    }
    Err("could not plan the drop".into())
}

/// Most Diamond Locks `/dd` drops at once: 200 Blue Gem Locks' worth.
pub const MAX_DL_DROP: u32 = 20_000;

/// Plans `/dd n` (`n` Diamond Locks' worth):
/// - enough DL: drop `n` DL (at most a stack);
/// - otherwise hundreds go as Blue Gem Locks and the rest as DL;
/// - DL still short: break Blue Gem Locks into DL first (1 BGL -> 100 DL). If
///   that would overflow the 200 stack, the DL we have are dropped first.
pub fn plan_dl_drop(n: u32, dl: u32, bgl: u32) -> Result<Vec<Step>, String> {
    if n == 0 || n > MAX_DL_DROP {
        return Err(format!("amount must be 1-{MAX_DL_DROP}"));
    }
    if dl * 100 + bgl * 10_000 < n * 100 {
        return Err(format!("not enough: you have {dl} DL and {bgl} BGL"));
    }
    if n <= dl && n <= STACK {
        return Ok(vec![Step::Drop { item_id: DL, amount: n }]);
    }
    let mut steps = Vec::new();
    let drop_bgl = (n / 100).min(bgl);
    if drop_bgl > 0 {
        steps.push(Step::Drop { item_id: BGL, amount: drop_bgl });
    }
    let rest = n - drop_bgl * 100;
    if rest <= dl {
        if rest > 0 {
            steps.push(Step::Drop { item_id: DL, amount: rest });
        }
        return Ok(steps);
    }
    // Break Blue Gem Locks for the missing DL.
    let short = rest - dl;
    let breaks = short.div_ceil(100);
    if bgl - drop_bgl < breaks {
        return Err(format!("not enough: you have {dl} DL and {bgl} BGL"));
    }
    if dl + breaks * 100 > STACK {
        // Make room in the DL slot first.
        if dl > 0 {
            steps.push(Step::Drop { item_id: DL, amount: dl });
        }
        steps.extend((0..breaks).map(|_| Step::Convert(BGL)));
        steps.push(Step::Drop { item_id: DL, amount: rest - dl });
    } else {
        steps.extend((0..breaks).map(|_| Step::Convert(BGL)));
        steps.push(Step::Drop { item_id: DL, amount: rest });
    }
    Ok(steps)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dd_uses_dl_when_there_are_enough() {
        assert_eq!(plan_dl_drop(30, 50, 3), Ok(vec![Step::Drop { item_id: DL, amount: 30 }]));
    }

    #[test]
    fn dd_breaks_a_bgl_when_dl_are_short() {
        assert_eq!(
            plan_dl_drop(30, 0, 2),
            Ok(vec![Step::Convert(BGL), Step::Drop { item_id: DL, amount: 30 }])
        );
        assert_eq!(
            plan_dl_drop(30, 10, 1),
            Ok(vec![Step::Convert(BGL), Step::Drop { item_id: DL, amount: 30 }])
        );
    }

    #[test]
    fn dd_hundreds_go_as_bgl() {
        assert_eq!(plan_dl_drop(100, 0, 3), Ok(vec![Step::Drop { item_id: BGL, amount: 1 }]));
        assert_eq!(plan_dl_drop(200, 0, 3), Ok(vec![Step::Drop { item_id: BGL, amount: 2 }]));
        assert_eq!(
            plan_dl_drop(250, 50, 3),
            Ok(vec![Step::Drop { item_id: BGL, amount: 2 }, Step::Drop { item_id: DL, amount: 50 }])
        );
    }

    #[test]
    fn dd_prefers_a_whole_bgl_over_breaking_one() {
        // 150 DL, 180 wanted: 1 BGL + 80 DL, nothing to break.
        assert_eq!(
            plan_dl_drop(180, 150, 1),
            Ok(vec![Step::Drop { item_id: BGL, amount: 1 }, Step::Drop { item_id: DL, amount: 80 }])
        );
    }

    #[test]
    fn dd_refuses_when_short() {
        assert!(plan_dl_drop(30, 10, 0).is_err());
        assert!(plan_dl_drop(0, 10, 0).is_err());
        assert!(plan_dl_drop(300, 0, 2).is_err());
    }

    #[test]
    fn exact_change() {
        assert_eq!(
            plan_custom_drop(250, 60, 5, 0),
            Ok(vec![Step::Drop { item_id: DL, amount: 2 }, Step::Drop { item_id: WL, amount: 50 }])
        );
    }

    #[test]
    fn breaks_a_diamond_lock() {
        assert_eq!(
            plan_custom_drop(150, 40, 2, 0),
            Ok(vec![
                Step::Convert(DL),
                Step::Drop { item_id: DL, amount: 1 },
                Step::Drop { item_id: WL, amount: 50 }
            ])
        );
    }

    #[test]
    fn breaks_bgl_then_dl() {
        assert_eq!(
            plan_custom_drop(150, 40, 0, 1),
            Ok(vec![
                Step::Convert(BGL),
                Step::Convert(DL),
                Step::Drop { item_id: DL, amount: 1 },
                Step::Drop { item_id: WL, amount: 50 }
            ])
        );
    }

    #[test]
    fn refuses_when_short() {
        assert!(plan_custom_drop(500, 10, 2, 0).is_err());
        assert!(plan_custom_drop(0, 10, 2, 0).is_err());
    }

    #[test]
    fn totals() {
        assert_eq!(total_wl(23, 4, 1), 10_423);
    }
}
