//! `/spinstats`: what the roulette wheels in this session have shown.

use std::collections::VecDeque;

use super::dialog::{Dialog, DialogReturn};
use super::interceptor::Interceptor;
use super::roulette::{Spin, SpinMode};
use super::tools::show;

const RECENT: usize = 15;
const ROULETTE_ICON: u32 = 758;

#[derive(Debug, Default, Clone)]
pub struct SpinStats {
    pub total: u32,
    pub red: u32,
    pub black: u32,
    pub green: u32,
    /// Most recent first: (number, colour code).
    recent: VecDeque<(u8, char)>,
    qq: [u32; 10],
    reme: [u32; 10],
}

impl SpinStats {
    pub fn record(&mut self, spin: &Spin) {
        self.total += 1;
        match spin.colour {
            '4' => self.red += 1,
            '2' => self.green += 1,
            _ => self.black += 1,
        }
        self.recent.push_front((spin.number, spin.colour));
        self.recent.truncate(RECENT);
        self.qq[SpinMode::Qq.value(spin.number).unwrap() as usize] += 1;
        self.reme[SpinMode::Reme.value(spin.number).unwrap() as usize] += 1;
    }

    pub fn recent(&self) -> Vec<(u8, char)> {
        self.recent.iter().copied().collect()
    }

    fn average(counts: &[u32; 10]) -> f64 {
        let n: u32 = counts.iter().sum();
        if n == 0 {
            return 0.0;
        }
        counts.iter().enumerate().map(|(v, c)| v as f64 * *c as f64).sum::<f64>() / n as f64
    }
}

fn pct(part: u32, total: u32) -> String {
    if total == 0 { "-".into() } else { format!("{:.0}%", part as f64 * 100.0 / total as f64) }
}

pub fn spinstats(ic: &mut Interceptor, args: &[&str]) {
    if args.first() == Some(&"reset") {
        ic.roulette_stats = SpinStats::default();
        return;
    }
    let s = ic.roulette_stats.clone();
    let recent: Vec<String> = s.recent.iter().map(|(n, c)| format!("`{c}{n}``")).collect();
    let d = Dialog::new("spinstats")
        .title("Spin statistics", ROULETTE_ICON)
        .small("Counted from the [REAL] spins seen since the proxy started.")
        .spacer()
        .field("Spins", &s.total.to_string())
        .text(&format!(
            "`4Red`` {} ({}) | `bBlack`` {} ({}) | `2Green`` {} ({})",
            s.red,
            pct(s.red, s.total),
            s.black,
            pct(s.black, s.total),
            s.green,
            pct(s.green, s.total)
        ))
        .spacer()
        .heading("Last spins")
        .text(&if recent.is_empty() { "-".to_string() } else { recent.join(" ") })
        .spacer()
        .heading("Games")
        .field("QQ average", &format!("{:.2}", SpinStats::average(&s.qq)))
        .field("QQ 0", &format!("{} ({})", s.qq[0], pct(s.qq[0], s.total)))
        .field("Reme average", &format!("{:.2}", SpinStats::average(&s.reme)))
        .field("Reme 0", &format!("{} ({})", s.reme[0], pct(s.reme[0], s.total)))
        .spacer()
        .button("reset", "`4Reset``");
    show(ic, d, "Close", "");
}

pub fn on_dialog(ic: &mut Interceptor, ret: &DialogReturn) -> bool {
    if ret.name != "spinstats" {
        return false;
    }
    if ret.button == Some("reset") {
        ic.roulette_stats = SpinStats::default();
        spinstats(ic, &[]);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts() {
        let mut s = SpinStats::default();
        for (n, c) in [(0, '2'), (23, '4'), (29, 'b'), (19, '4')] {
            s.record(&Spin { player: "x".into(), number: n, colour: c });
        }
        assert_eq!((s.total, s.red, s.black, s.green), (4, 2, 1, 1));
        assert_eq!(s.recent.front(), Some(&(19, '4')));
        assert_eq!(s.reme[0], 2); // 0 and 19
        assert_eq!(s.qq[3], 1);
    }
}
