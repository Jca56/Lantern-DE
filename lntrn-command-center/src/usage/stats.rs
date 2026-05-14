//! Plain-data containers shared between the scanner worker and the
//! rendering view.

#[derive(Debug, Clone, Default)]
pub struct UsageStats {
    pub turns: u64,
    pub sessions: u64,

    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_5m_tokens: u64,
    pub cache_1h_tokens: u64,
    pub cache_read_tokens: u64,

    pub web_search_requests: u64,
    pub web_fetch_requests: u64,

    pub cost_usd: f64,

    pub first_ts: Option<String>,
    pub last_ts: Option<String>,

    pub by_project: Vec<ProjectBucket>,
    pub by_model: Vec<ProjectBucket>,
    pub by_day: Vec<DayBucket>,
}

impl UsageStats {
    pub fn total_tokens(&self) -> u64 {
        self.input_tokens
            + self.output_tokens
            + self.cache_5m_tokens
            + self.cache_1h_tokens
            + self.cache_read_tokens
    }

    /// cache_read / (cache_read + input + cache_writes). Cache hits
    /// are the cheap path so a high ratio means we're saving money.
    pub fn cache_hit_ratio(&self) -> f32 {
        let total_input = self.input_tokens
            + self.cache_5m_tokens
            + self.cache_1h_tokens
            + self.cache_read_tokens;
        if total_input == 0 {
            return 0.0;
        }
        self.cache_read_tokens as f32 / total_input as f32
    }

    /// Sum over the trailing `n` days in `by_day`. Caller passes an
    /// ISO date prefix (YYYY-MM-DD) for "today" so we can compare
    /// lexicographically.
    pub fn since(&self, cutoff_day: &str) -> (u64, u64, f64) {
        let mut turns = 0u64;
        let mut tokens = 0u64;
        let mut cost = 0.0;
        for d in &self.by_day {
            if d.day.as_str() >= cutoff_day {
                turns += d.turns;
                tokens += d.tokens;
                cost += d.cost_usd;
            }
        }
        (turns, tokens, cost)
    }
}

#[derive(Debug, Clone)]
pub struct ProjectBucket {
    pub label: String,
    pub turns: u64,
    pub tokens: u64,
    pub cost_usd: f64,
}

#[derive(Debug, Clone)]
pub struct DayBucket {
    pub day: String,
    pub turns: u64,
    pub tokens: u64,
    pub cost_usd: f64,
}
