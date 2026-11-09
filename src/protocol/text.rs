/// `key|value` lines used by text messages and server_data.php.
///
/// Keeps line order and duplicate keys so a modified packet differs from the
/// original only in the fields that were changed. Chat input arrives as
/// `action|input\n|text|hello`; the leading `|` is tolerated.
#[derive(Debug, Clone, Default)]
pub struct TextPacket {
    /// `None` value = a bare line without `|` (e.g. `RTENDMARKERBS1001`), kept verbatim.
    lines: Vec<(String, Option<String>)>,
}

impl TextPacket {
    pub fn parse(text: &str) -> Self {
        let lines = text
            .split('\n')
            .filter(|l| !l.is_empty())
            .map(|l| {
                let l = l.strip_prefix('|').unwrap_or(l);
                match l.split_once('|') {
                    Some((k, v)) => (k.to_string(), Some(v.to_string())),
                    None => (l.to_string(), None),
                }
            })
            .collect();
        Self { lines }
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.lines
            .iter()
            .find(|(k, _)| k == key)
            .and_then(|(_, v)| v.as_deref())
    }

    /// Replaces the first occurrence of `key`, or appends it.
    pub fn set(&mut self, key: &str, value: impl Into<String>) {
        let value = value.into();
        match self.lines.iter_mut().find(|(k, _)| k == key) {
            Some((_, v)) => *v = Some(value),
            None => self.lines.push((key.to_string(), Some(value))),
        }
    }

    pub fn lines(&self) -> &[(String, Option<String>)] {
        &self.lines
    }

    pub fn render(&self) -> String {
        let mut out = String::new();
        for (k, v) in &self.lines {
            out.push_str(k);
            if let Some(v) = v {
                out.push('|');
                out.push_str(v);
            }
            out.push('\n');
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_input() {
        let t = TextPacket::parse("action|input\n|text|/warp start");
        assert_eq!(t.get("action"), Some("input"));
        assert_eq!(t.get("text"), Some("/warp start"));
    }

    #[test]
    fn server_data_rewrite() {
        let mut t = TextPacket::parse("server|1.2.3.4\nport|17000\ntype|1\nmeta|abc\nRTENDMARKERBS1001");
        t.set("server", "127.0.0.1");
        t.set("port", "17091");
        assert_eq!(
            t.render(),
            "server|127.0.0.1\nport|17091\ntype|1\nmeta|abc\nRTENDMARKERBS1001\n"
        );
    }
}
