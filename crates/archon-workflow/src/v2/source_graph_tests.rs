use crate::{WorkflowV2HostCall, WorkflowV2HostOptions};

use super::*;

// _a and _b hold only `#[test]` functions, so they are declared but not
// imported -- a glob of them brings in nothing and rustc warns when you ask
// for one. _c also defines the shared execution/universe builders the other
// two call, so its glob stays.
#[path = "source_graph_tests_a.rs"]
mod source_graph_tests_a;
#[path = "source_graph_tests_b.rs"]
mod source_graph_tests_b;
#[path = "source_graph_tests_c.rs"]
mod source_graph_tests_c;
use source_graph_tests_c::*;
