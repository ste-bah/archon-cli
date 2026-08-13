//! Per-model token prices, and the cache tiers derived from them.
//!
//! Figures from Anthropic's published rate card (`platform.claude.com/docs/en/
//! about-claude/pricing`) and DeepSeek's. They are USD per million tokens.
//!
//! The cache tiers are expressed as **multipliers of base input** rather than as
//! four more absolute numbers, because that is how the vendors publish them and
//! because it is the shape that does not rot: a price change moves one figure
//! instead of five, and the ratios are what the caching decision actually turns
//! on. For Claude they are fixed across every model — 1.25x to write, 2x for the
//! one-hour tier, 0.1x to read — which is the arithmetic behind the rule that a
//! five-minute checkpoint pays for itself after one read and a one-hour
//! checkpoint after two. A checkpoint that is never read back costs *more* than
//! not caching at all, and that is the failure this whole area exists to make
//! visible.
//!
//! Not modelled, and deliberately: the Batch API's 50% discount, fast mode's
//! premium on Opus 5/4.8, and the 1.1x `inference_geo: "us"` multiplier. Each
//! needs a request-level flag the cost call sites do not carry. The regional
//! Bedrock premium *is* modelled, because it is inferable from the model id.

/// Prices for one model, per million tokens.
///
/// Also the shape of a `[context.model_pricing]` entry. The multipliers default
/// to Claude's published ratios, so an operator adding a model released after
/// the binary writes two numbers rather than five.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Pricing {
    pub input_per_mtok: f64,
    pub output_per_mtok: f64,
    /// Cache read (hit), as a multiple of `input_per_mtok`.
    #[serde(default = "default_cache_read_multiplier")]
    pub cache_read_multiplier: f64,
    /// Cache write at the default five-minute retention.
    #[serde(default = "default_cache_write_multiplier")]
    pub cache_write_multiplier: f64,
    /// Cache write at the one-hour retention.
    #[serde(default = "default_cache_write_1h_multiplier")]
    pub cache_write_1h_multiplier: f64,
}

fn default_cache_read_multiplier() -> f64 {
    0.1
}

fn default_cache_write_multiplier() -> f64 {
    1.25
}

fn default_cache_write_1h_multiplier() -> f64 {
    2.0
}

impl Pricing {
    pub fn cache_read_per_mtok(self) -> f64 {
        self.input_per_mtok * self.cache_read_multiplier
    }

    pub fn cache_write_per_mtok(self, ttl_1h: bool) -> f64 {
        let multiplier = if ttl_1h {
            self.cache_write_1h_multiplier
        } else {
            self.cache_write_multiplier
        };
        self.input_per_mtok * multiplier
    }

    /// The same prices scaled by a platform multiplier.
    pub fn scaled(self, factor: f64) -> Self {
        Self {
            input_per_mtok: self.input_per_mtok * factor,
            output_per_mtok: self.output_per_mtok * factor,
            ..self
        }
    }
}

/// Claude's cache tiers, identical on every model Anthropic publishes.
const fn claude(input_per_mtok: f64, output_per_mtok: f64) -> Pricing {
    Pricing {
        input_per_mtok,
        output_per_mtok,
        cache_read_multiplier: 0.1,
        cache_write_multiplier: 1.25,
        cache_write_1h_multiplier: 2.0,
    }
}

/// DeepSeek prices cache hits far below Claude's tenth, and does not bill a
/// separate write tier — caching is automatic and a write is just input.
const fn deepseek(input_per_mtok: f64, output_per_mtok: f64, read_multiplier: f64) -> Pricing {
    Pricing {
        input_per_mtok,
        output_per_mtok,
        cache_read_multiplier: read_multiplier,
        cache_write_multiplier: 1.0,
        cache_write_1h_multiplier: 1.0,
    }
}

/// What an unrecognised model is costed at.
///
/// Sonnet-class rates with Claude's cache tiers. It is an estimate and named as
/// one — but an estimate that prices cache reads at a tenth and cache writes
/// above par is the right *shape*, which is what the previous fallback got
/// wrong: it billed reads at zero and writes at par, so a deployment writing
/// checkpoints it never read back showed a falling cost while the invoice rose.
pub const UNKNOWN_MODEL_ESTIMATE: Pricing = claude(3.0, 15.0);

/// Model-id substring -> prices. Longest match wins, so `claude-opus-4-5` is not
/// shadowed by a hypothetical `claude-opus-4` entry.
const BUILT_IN: &[(&str, Pricing)] = &[
    ("claude-fable-5", claude(10.0, 50.0)),
    ("claude-mythos-5", claude(10.0, 50.0)),
    ("claude-opus-5", claude(5.0, 25.0)),
    ("claude-opus-4-8", claude(5.0, 25.0)),
    ("claude-opus-4-7", claude(5.0, 25.0)),
    ("claude-opus-4-6", claude(5.0, 25.0)),
    ("claude-opus-4-5", claude(5.0, 25.0)),
    ("claude-opus-4-1", claude(15.0, 75.0)),
    ("claude-opus-4", claude(15.0, 75.0)),
    ("claude-sonnet-5", claude(2.0, 10.0)),
    ("claude-sonnet-4-6", claude(3.0, 15.0)),
    ("claude-sonnet-4-5", claude(3.0, 15.0)),
    ("claude-sonnet-4", claude(3.0, 15.0)),
    ("claude-3-7-sonnet", claude(3.0, 15.0)),
    ("claude-3-sonnet", claude(3.0, 15.0)),
    ("claude-haiku-4-5", claude(1.0, 5.0)),
    ("claude-haiku-3-5", claude(0.8, 4.0)),
    ("claude-3-5-haiku", claude(0.8, 4.0)),
    ("claude-3-haiku", claude(0.25, 1.25)),
    (
        "deepseek-v4-pro",
        deepseek(0.435, 0.87, 0.008_333_333_333_333_333),
    ),
    ("deepseek-v4-flash", deepseek(0.14, 0.28, 0.02)),
    ("deepseek-chat", deepseek(0.14, 0.28, 0.02)),
    ("deepseek-reasoner", deepseek(0.14, 0.28, 0.02)),
];

/// Look a model up in the built-in table.
///
/// Substring rather than exact match, because the same model arrives spelled
/// four ways: `claude-sonnet-4-6`, `anthropic.claude-sonnet-4-6`,
/// `eu.anthropic.claude-sonnet-4-6`, and a full Bedrock ARN. Longest match wins
/// so the more specific entry always beats the more general one.
pub fn lookup(model: &str) -> Option<Pricing> {
    let normalized = model.trim().to_ascii_lowercase();
    BUILT_IN
        .iter()
        .filter(|(marker, _)| normalized.contains(marker))
        .max_by_key(|(marker, _)| marker.len())
        .map(|(_, pricing)| *pricing)
}

/// The premium a partner platform adds over the first-party rate.
///
/// Bedrock and Google Cloud both charge 10% more on a *regional* or
/// multi-region endpoint than on a global one, for Sonnet 4.5, Haiku 4.5, Opus
/// 4.5 and everything since. The endpoint type is spelled into the model id —
/// `eu.anthropic.…` against `global.anthropic.…` — so it costs nothing to be
/// right about, and getting it wrong understates a European deployment's bill by
/// a tenth on every single token.
pub fn platform_multiplier(model: &str) -> f64 {
    let normalized = model.trim().to_ascii_lowercase();
    // A full ARN carries the inference-profile id after the last `/`.
    let id = normalized.rsplit('/').next().unwrap_or(&normalized);

    const REGIONAL_PREFIXES: &[&str] = &["us.", "eu.", "apac.", "ap.", "jp.", "au.", "ca."];
    if REGIONAL_PREFIXES
        .iter()
        .any(|prefix| id.starts_with(prefix))
    {
        1.1
    } else {
        1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_longest_marker_wins() {
        assert_eq!(lookup("claude-opus-4-5").unwrap().input_per_mtok, 5.0);
        assert_eq!(lookup("claude-opus-4-1").unwrap().input_per_mtok, 15.0);
        assert_eq!(lookup("claude-sonnet-5").unwrap().input_per_mtok, 2.0);
        assert_eq!(lookup("claude-sonnet-4-6").unwrap().input_per_mtok, 3.0);
    }

    /// The spelling a Bedrock deployment actually sends. Missing it is how a
    /// model ends up costed at the fallback estimate without anyone noticing.
    #[test]
    fn decorated_bedrock_and_vertex_ids_resolve() {
        for id in [
            "anthropic.claude-sonnet-4-6",
            "eu.anthropic.claude-sonnet-4-6",
            "arn:aws:bedrock:eu-west-2:1234:inference-profile/eu.anthropic.claude-sonnet-4-6",
            "claude-sonnet-4-6@20260115",
        ] {
            assert_eq!(lookup(id).unwrap().input_per_mtok, 3.0, "{id}");
        }
    }

    #[test]
    fn the_cache_tiers_follow_the_published_multipliers() {
        let opus5 = lookup("claude-opus-5").unwrap();

        assert_eq!(opus5.cache_read_per_mtok(), 0.5);
        assert_eq!(opus5.cache_write_per_mtok(false), 6.25);
        assert_eq!(opus5.cache_write_per_mtok(true), 10.0);
    }

    /// The figure that decides whether a checkpoint is worth writing: a write
    /// costs more than plain input, so one that is never read back loses money.
    #[test]
    fn a_cache_write_costs_more_than_the_input_it_replaces() {
        let pricing = lookup("claude-sonnet-4-6").unwrap();

        assert!(pricing.cache_write_per_mtok(false) > pricing.input_per_mtok);
        assert!(pricing.cache_write_per_mtok(true) > pricing.cache_write_per_mtok(false));
        assert!(pricing.cache_read_per_mtok() < pricing.input_per_mtok);
    }

    #[test]
    fn a_regional_endpoint_carries_its_ten_percent_premium() {
        assert_eq!(platform_multiplier("eu.anthropic.claude-opus-5"), 1.1);
        assert_eq!(platform_multiplier("us.anthropic.claude-opus-5"), 1.1);
        assert_eq!(platform_multiplier("global.anthropic.claude-opus-5"), 1.0);
        assert_eq!(platform_multiplier("claude-opus-5"), 1.0);
        assert_eq!(
            platform_multiplier(
                "arn:aws:bedrock:eu-west-2:1234:inference-profile/eu.anthropic.claude-opus-5"
            ),
            1.1
        );
    }

    /// The live figure from `aws bedrock list-foundation-model-agreement-offers`
    /// for Opus 5 in eu-west-2, which is what this pairing has to reproduce.
    #[test]
    fn eu_west_2_opus_5_matches_the_live_bedrock_rate_card() {
        let id = "eu.anthropic.claude-opus-5";
        let pricing = lookup(id).unwrap().scaled(platform_multiplier(id));

        assert!((pricing.input_per_mtok - 5.5).abs() < 1e-9);
        assert!((pricing.output_per_mtok - 27.5).abs() < 1e-9);
        assert!((pricing.cache_read_per_mtok() - 0.55).abs() < 1e-9);
        assert!((pricing.cache_write_per_mtok(false) - 6.875).abs() < 1e-9);
        assert!((pricing.cache_write_per_mtok(true) - 11.0).abs() < 1e-9);
    }
}
