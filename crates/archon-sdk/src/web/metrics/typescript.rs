use ts_rs::{Config as TsConfig, TS};

use super::{
    MetricEventPreview, MetricStoreHealth, MetricValue, MetricsSummary,
    ProviderRuntimeEventPreview, ProviderRuntimeMetric,
};

pub(super) fn generated_typescript() -> String {
    let cfg = TsConfig::default().with_large_int("number");
    [
        exported(MetricsSummary::decl(&cfg)),
        exported(MetricStoreHealth::decl(&cfg)),
        exported(MetricValue::decl(&cfg)),
        exported(MetricEventPreview::decl(&cfg)),
        exported(ProviderRuntimeMetric::decl(&cfg)),
        exported(ProviderRuntimeEventPreview::decl(&cfg)),
    ]
    .join("\n\n")
        + "\n"
}

fn exported(decl: String) -> String {
    format!("export {decl}")
}
