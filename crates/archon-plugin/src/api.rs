//! Plugin API — tools_from_plugin_instance adapter factory.

use std::sync::{Arc, Mutex};

use crate::adapter_tool::PluginToolAdapter;
use crate::host::WasmPluginHost;
use crate::instance::PluginInstance;

/// Create one `Box<dyn Tool>` per tool registered by `instance`.
///
/// Each adapter wraps the live WASM host so calls are dispatched into the guest.
/// Returns an empty vec if the instance registered no tools.
pub fn tools_from_plugin_instance(
    plugin_id: &str,
    instance: &PluginInstance,
    host: Arc<Mutex<WasmPluginHost>>,
) -> Vec<Box<dyn archon_tools::tool::Tool>> {
    instance
        .registered_tools()
        .iter()
        .map(|t| {
            let adapter = PluginToolAdapter::new_with_host(
                plugin_id.to_string(),
                t.name.clone(),
                String::new(),
                t.schema_json.clone(),
                Arc::clone(&host),
            );
            Box::new(adapter) as Box<dyn archon_tools::tool::Tool>
        })
        .collect()
}
