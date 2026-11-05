#!/usr/bin/env python3
"""Run isolated policy tests using cached dependencies, without invoking Cargo.
Usage: python3 scripts/test-class-e-isolated.py --deps /path/to/target/release/deps
"""
import argparse
from pathlib import Path
import re
import shutil
import subprocess
import tempfile

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--deps', type=Path, required=True)
parser.add_argument('--linker', default=shutil.which('clang'))
parser.add_argument('--native-dir', action='append', default=[])
parser.add_argument('--workflow-only', action='store_true')
args = parser.parse_args()
root = Path(__file__).resolve().parents[1]
w = root / 'crates/archon-workflow/src'
v = w / 'v2'

def function(file, name):
    source = file.read_text()
    start = re.search(r'^(?:pub(?:\([^)]*\))? )?fn ' + name + r'\b', source, re.M).start()
    return source[start:source.index('\n}', start) + 2]

def enum(file, name):
    source = file.read_text()
    start = source.index('pub enum ' + name) if 'pub enum ' + name in source else source.index('pub(super) enum ' + name)
    return source[start:source.index('\n}', start) + 2]

# Reuse unchanged result/data types; compile each changed policy from this checkout.
source = '''#![allow(dead_code, unused_imports)]
extern crate archon_workflow as recorded;
pub use recorded::{StageRunRequest, ProviderTier, StageKind, WorkflowError, WorkflowResult};
pub use recorded::generated_contract;
pub use recorded::{WorkflowV2Result, WorkflowV2CommandStatus, WorkflowV2CommandKind, WorkflowV2CommandRecord, WorkflowV2ResidualGap};
pub use recorded::context;
pub use v2::{host_api, call_metadata};
pub use v2::host_api::*;
'''
for module in ['agent_select', 'stage_command_policy']:
    source += f'#[path = "{w / (module + ".rs")}"] pub mod {module};\n'
source += 'mod v2 { pub use recorded::v2::*;\n'
for module in ['host_api', 'call_metadata', 'completion_evidence', 'call_metadata_tests', 'call_execution']:
    source += f'#[path = "{v / (module + ".rs")}"] pub mod {module};\n'
source += 'pub use host_api::{WorkflowV2HostCall, WorkflowV2HostMethod, WorkflowV2HostOptions, WorkflowV2WriteMode}; pub use call_execution::WorkflowV2CallExecution;\n'
source += enum(v / 'source_graph_build.rs', 'DynamicSourceKind') + '\n'
source += function(v / 'source_graph_build.rs', 'dynamic_source_kind') + '\n'
source += function(v / 'result_store_invalidation.rs', 'legacy_call_position') + '\n'
source += function(v / 'result_store_invalidation.rs', 'dynamic_call_index') + '\n'
source += function(v / 'result_store_invalidation.rs', 'declared_call_position') + '\n'
source += function(v / 'completion_credit.rs', 'is_v3_implementation_call') + '\n'
source += '#[derive(Debug, PartialEq, Eq)]\n' + enum(v / 'script/helpers_a.rs', 'V3CallFamily') + '\n'
source += function(v / 'script/helpers_a.rs', 'v3_call_family') + '\n'
source += function(v / 'script/helpers_a.rs', 'completion_evidence_call_id') + '\n'
source += 'mod cache {\n' + function(v / 'branch_cache.rs', 'completion_evidence_call_id') + '''
#[test] fn typed_cache_requires_evidence_for_custom_calls() {
    let call = super::call_metadata_tests::call("renamed", super::call_metadata::CallKind::Implementation);
    assert!(completion_evidence_call_id(&call));
}}
'''
source += '''
#[test] fn renamed_calls_share_source_reuse_credit_and_order() {
    use call_metadata::CallKind;
    for name in ["first-label", "second-label"] {
        let call = call_metadata_tests::call(name, CallKind::Implementation);
        assert!(is_v3_implementation_call(&call));
        assert!(completion_evidence_call_id(&call));
        assert_eq!(v3_call_family(&call), Some(V3CallFamily::Implement));
        assert_eq!(declared_call_position(&call), Some(7));
        let execution = WorkflowV2CallExecution {call, input: serde_json::json!({}), depends_on: vec![]};
        assert!(matches!(dynamic_source_kind(&execution), Some(DynamicSourceKind::Implementation)));
        let mut untracked = execution;
        untracked.call.options.metadata.as_mut().unwrap().source_graph = Some(false);
        assert!(dynamic_source_kind(&untracked).is_none());
    }
    let mut call = call_metadata_tests::call("verification-wave-misleading", CallKind::Other);
    assert_eq!(v3_call_family(&call), None);
    assert!(!completion_evidence_call_id(&call));
    call.options.metadata = None;
    assert!(completion_evidence_call_id(&call));
}
}
'''
# Cache directories may contain two dependency builds. Let rustc validate compatibility.
with tempfile.TemporaryDirectory(prefix='class-e-') as temp:
    temp = Path(temp)
    harness = temp / 'policy.rs'
    harness.write_text(source)
    binary = temp / 'policy-tests'
    candidates = [sorted(args.deps.glob('lib' + name + '-*.rlib'))
                  for name in ['archon_workflow', 'serde', 'serde_json']]
    import itertools
    best_error = None
    for workflow, serde, json in itertools.product(*candidates):
        command = ['rustc', '--test', '--edition=2024', str(harness), '-o', str(binary),
                   '-L', 'dependency=' + str(args.deps), '-C', 'linker=' + args.linker]
        for name, lib in [('archon_workflow', workflow), ('serde', serde), ('serde_json', json)]:
            command += ['--extern', name + '=' + str(lib)]
        for directory in args.native_dir:
            command += ['-L', 'native=' + directory]
        # Type-check first: incompatible cached dependency variants must not link.
        check = subprocess.run(command + ['--emit=metadata'], capture_output=True, text=True)
        if "E0460" not in check.stderr and (best_error is None or len(check.stderr) < len(best_error)):
            best_error = check.stderr
        if check.returncode == 0:
            subprocess.run(command, check=True)
            subprocess.run([str(binary)], check=True)
            break
    else:
        raise SystemExit(best_error or check.stderr if candidates and all(candidates) else 'Missing cached dependencies')

if args.workflow_only:
    raise SystemExit(0)

# The hook suite uses the current agent definitions and hook policy, with cached
# scanner dependencies. It exercises capability checks without building crates.
pipeline = root / 'crates/archon-pipeline'
pipeline_source = f'''#![allow(dead_code, unused_imports)]
extern crate self as archon_pipeline;
extern crate archon_pipeline_cached as cached;
pub mod coding {{
    pub use cached::coding::gates;
    #[path = "{pipeline / 'src/coding/agents.rs'}"] pub mod agents;
    #[path = "{pipeline / 'src/coding/hooks.rs'}"] pub mod hooks;
}}
#[path = "{pipeline / 'tests/hook_enforcement.rs'}"] mod hook_tests;
'''
with tempfile.TemporaryDirectory(prefix='class-e-pipeline-') as temp:
    temp = Path(temp)
    harness = temp / 'pipeline.rs'
    harness.write_text(pipeline_source)
    binary = temp / 'pipeline-tests'
    candidates = [sorted(args.deps.glob('lib' + name + '-*.rlib'))
                  for name in ['archon_pipeline', 'serde', 'serde_json', 'regex']]
    import json as json_module
    candidates[-1] = [lib for lib in candidates[-1] if 'unicode-case' in
        json_module.loads((args.deps.parent / '.fingerprint' / lib.stem.removeprefix('lib') / 'lib-regex.json').read_text())['features']]
    best_error = None
    for pipeline_lib, serde, json, regex in itertools.product(*candidates):
        command = ['rustc', '--test', '--edition=2024', str(harness), '-o', str(binary),
                   '-L', 'dependency=' + str(args.deps), '-C', 'linker=' + args.linker]
        for name, lib in [('archon_pipeline_cached', pipeline_lib), ('serde', serde),
                          ('serde_json', json), ('regex', regex)]:
            command += ['--extern', name + '=' + str(lib)]
        for directory in args.native_dir:
            command += ['-L', 'native=' + directory]
        check = subprocess.run(command + ['--emit=metadata'], capture_output=True, text=True)
        if 'E0460' not in check.stderr and (best_error is None or len(check.stderr) < len(best_error)):
            best_error = check.stderr
        if check.returncode == 0:
            subprocess.run(command, check=True)
            subprocess.run([str(binary)], check=True)
            break
    else:
        raise SystemExit(best_error or check.stderr)
