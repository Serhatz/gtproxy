//! Chat helpers (`/me`, `/formalize`, `/calc`), `/timer` and the ping alert.

use std::time::{Duration, Instant};

use super::commands::{say, warn};
use super::effect::Effect;
use super::interceptor::Interceptor;

const PING_ALERT_COOLDOWN: Duration = Duration::from_secs(30);
const TIMER_REFRESH: Duration = Duration::from_secs(1);

pub fn me(ic: &mut Interceptor, _: &[&str]) {
    ic.features.me = !ic.features.me;
    let state = if ic.features.me { "`2on``" } else { "`4off``" };
    say(ic, format!("/me mode {state}"));
}

pub fn formalize(ic: &mut Interceptor, _: &[&str]) {
    ic.features.formalize = !ic.features.formalize;
    let state = if ic.features.formalize { "`2on``" } else { "`4off``" };
    say(ic, format!("formalize {state}"));
}

/// Rewrites a chat line you typed (not commands). `None` = unchanged.
pub fn rewrite_chat(ic: &Interceptor, line: &str) -> Option<String> {
    if line.starts_with('/') || line.trim().is_empty() || !(ic.features.me || ic.features.formalize) {
        return None;
    }
    let mut out = line.trim().to_string();
    if ic.features.formalize {
        out = formal(&out);
    }
    if ic.features.me {
        out = format!("/me {out}");
    }
    (out != line).then_some(out)
}

/// Capital first letter, ends with punctuation: "hello there" -> "Hello there."
fn formal(s: &str) -> String {
    let mut chars = s.chars();
    let mut out: String = match chars.next() {
        Some(c) => c.to_uppercase().chain(chars).collect(),
        None => return String::new(),
    };
    if !out.ends_with(['.', '!', '?']) {
        out.push('.');
    }
    out
}

// ── /calc ────────────────────────────────────────────────────────────────────

pub fn calc(ic: &mut Interceptor, args: &[&str]) {
    let expr = args.join("");
    if expr.is_empty() {
        warn(ic, format!("usage: {}calc 25+50*2", ic.cfg.command.prefix));
        return;
    }
    match eval(&expr) {
        Some(v) => say(ic, format!("{expr} = `w{}``", fmt_number(v))),
        None => warn(ic, "could not calculate that"),
    }
}

fn fmt_number(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 { format!("{}", v as i64) } else { format!("{v:.4}").trim_end_matches('0').to_string() }
}

/// `+ - * / % ( )` and decimals, normal precedence.
fn eval(s: &str) -> Option<f64> {
    let tokens: Vec<char> = s.chars().filter(|c| !c.is_whitespace()).collect();
    let mut p = Parser { t: &tokens, i: 0 };
    let v = p.sum()?;
    (p.i == tokens.len() && v.is_finite()).then_some(v)
}

struct Parser<'a> {
    t: &'a [char],
    i: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<char> {
        self.t.get(self.i).copied()
    }

    fn sum(&mut self) -> Option<f64> {
        let mut v = self.product()?;
        while let Some(op @ ('+' | '-')) = self.peek() {
            self.i += 1;
            let r = self.product()?;
            v = if op == '+' { v + r } else { v - r };
        }
        Some(v)
    }

    fn product(&mut self) -> Option<f64> {
        let mut v = self.atom()?;
        while let Some(op @ ('*' | 'x' | '/' | '%')) = self.peek() {
            self.i += 1;
            let r = self.atom()?;
            v = match op {
                '/' if r == 0.0 => return None,
                '/' => v / r,
                '%' => v % r,
                _ => v * r,
            };
        }
        Some(v)
    }

    fn atom(&mut self) -> Option<f64> {
        match self.peek()? {
            '-' => {
                self.i += 1;
                Some(-self.atom()?)
            }
            '(' => {
                self.i += 1;
                let v = self.sum()?;
                (self.peek()? == ')').then(|| self.i += 1)?;
                Some(v)
            }
            _ => {
                let start = self.i;
                while self.peek().is_some_and(|c| c.is_ascii_digit() || c == '.') {
                    self.i += 1;
                }
                self.t[start..self.i].iter().collect::<String>().parse().ok()
            }
        }
    }
}

// ── /timer ───────────────────────────────────────────────────────────────────

pub fn timer(ic: &mut Interceptor, _: &[&str]) {
    match ic.features.timer.take() {
        Some(started) => {
            ic.features.timer_shown = None;
            say(ic, format!("timer stopped at `w{}``", hms(started.elapsed())));
        }
        None => {
            ic.features.timer = Some(Instant::now());
            say(ic, "timer started (type /timer again to stop)");
        }
    }
}

fn hms(d: Duration) -> String {
    let s = d.as_secs();
    format!("{:02}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
}

/// Called every bridge loop: timer display and ping alert.
pub fn tick(ic: &mut Interceptor, now: Instant) {
    if let Some(started) = ic.features.timer {
        if ic.features.timer_shown.is_none_or(|t| now >= t + TIMER_REFRESH) {
            ic.features.timer_shown = Some(now);
            ic.emit(Effect::overlay(format!("`w{}``", hms(now - started))));
        }
    }
    if let (Some(limit), Some(rtt)) = (ic.features.ping_alert_ms, ic.link.server_rtt) {
        let ms = rtt.as_millis() as u32;
        if ms > limit && ic.features.last_ping_alert.is_none_or(|t| now >= t + PING_ALERT_COOLDOWN) {
            ic.features.last_ping_alert = Some(now);
            warn(ic, format!("High ping: {ms}ms"));
        }
    }
}

// ── Ping alert ───────────────────────────────────────────────────────────────

pub fn ping_alert(ic: &mut Interceptor, args: &[&str]) {
    match args.first() {
        Some(&"off") => {
            ic.features.ping_alert_ms = None;
            say(ic, "ping alert `4off``");
        }
        Some(v) => match v.parse::<u32>() {
            Ok(ms) if ms >= 50 => {
                ic.features.ping_alert_ms = Some(ms);
                say(ic, format!("ping alert above `w{ms}ms``"));
            }
            _ => warn(ic, format!("usage: {}pingalert <ms, 50+>|off", ic.cfg.command.prefix)),
        },
        None => {
            let now = ic.link.server_rtt.map_or("-".to_string(), |d| format!("{}ms", d.as_millis()));
            let limit = ic.features.ping_alert_ms.map_or("off".to_string(), |ms| format!("{ms}ms"));
            say(ic, format!("ping now {now}, alert {limit} ({}pingalert <ms>|off)", ic.cfg.command.prefix));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calculator() {
        assert_eq!(eval("25+50"), Some(75.0));
        assert_eq!(eval("2+3*4"), Some(14.0));
        assert_eq!(eval("(2+3)*4"), Some(20.0));
        assert_eq!(eval("10/4"), Some(2.5));
        assert_eq!(eval("-3+5"), Some(2.0));
        assert_eq!(eval("1/0"), None);
        assert_eq!(eval("2+"), None);
        assert_eq!(fmt_number(2.5), "2.5");
        assert_eq!(fmt_number(75.0), "75");
    }

    #[test]
    fn formal_text() {
        assert_eq!(formal("hello there"), "Hello there.");
        assert_eq!(formal("really?"), "Really?");
    }

    #[test]
    fn rewrites_only_chat() {
        let mut ic = Interceptor::new(Default::default());
        ic.features.me = true;
        ic.features.formalize = true;
        assert_eq!(rewrite_chat(&ic, "hi all").as_deref(), Some("/me Hi all."));
        assert_eq!(rewrite_chat(&ic, "/warp start"), None);
    }

    #[test]
    fn clock() {
        assert_eq!(hms(Duration::from_secs(3725)), "01:02:05");
    }
}
