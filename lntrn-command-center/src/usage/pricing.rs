//! Public Anthropic API pricing per million tokens. We only use this
//! to compute "what would this have cost on the API?" — purely a
//! display number, since Claude Code on Max is a flat subscription.

#[derive(Debug, Clone, Copy)]
pub struct Price {
    pub input: f64,
    pub output: f64,
    pub cache_5m: f64,
    pub cache_1h: f64,
    pub cache_read: f64,
}

impl Price {
    pub fn cost(&self, input: u64, output: u64, cache_5m: u64, cache_1h: u64, cache_read: u64) -> f64 {
        let m = 1_000_000.0;
        (input as f64 * self.input
            + output as f64 * self.output
            + cache_5m as f64 * self.cache_5m
            + cache_1h as f64 * self.cache_1h
            + cache_read as f64 * self.cache_read)
            / m
    }
}

const OPUS: Price = Price {
    input: 15.0,
    output: 75.0,
    cache_5m: 18.75,
    cache_1h: 30.0,
    cache_read: 1.50,
};
const SONNET: Price = Price {
    input: 3.0,
    output: 15.0,
    cache_5m: 3.75,
    cache_1h: 6.0,
    cache_read: 0.30,
};
const HAIKU: Price = Price {
    input: 1.0,
    output: 5.0,
    cache_5m: 1.25,
    cache_1h: 2.0,
    cache_read: 0.10,
};

pub fn price_for(model: &str) -> Price {
    if model.starts_with("claude-opus") {
        OPUS
    } else if model.starts_with("claude-sonnet") {
        SONNET
    } else if model.starts_with("claude-haiku") {
        HAIKU
    } else {
        // Unknown model — default to Sonnet so totals don't explode.
        SONNET
    }
}

/// Friendly short name for display ("claude-opus-4-7" → "Opus 4.7").
pub fn short_name(model: &str) -> String {
    let mut tier = "?";
    let mut rest = "";
    if let Some(r) = model.strip_prefix("claude-opus-") {
        tier = "Opus";
        rest = r;
    } else if let Some(r) = model.strip_prefix("claude-sonnet-") {
        tier = "Sonnet";
        rest = r;
    } else if let Some(r) = model.strip_prefix("claude-haiku-") {
        tier = "Haiku";
        rest = r;
    }
    if rest.is_empty() {
        return model.to_string();
    }
    // "4-7" → "4.7", "4-7-20251001" → "4.7"
    let mut parts = rest.split('-');
    let major = parts.next().unwrap_or("");
    let minor = parts.next().unwrap_or("");
    if minor.is_empty() {
        format!("{} {}", tier, major)
    } else {
        format!("{} {}.{}", tier, major, minor)
    }
}
