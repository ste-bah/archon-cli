//! Coverage for the v3 authoring pre-flight: the mandatory map→reduce review
//! contract, its remediation exemption, and the reducer-bound accounting.
//!
//! Split three ways only to hold the 500-line source ceiling; `_a` owns the
//! shared plan builders the other two use.

use super::*;

#[path = "v3_author_checks_tests_a.rs"]
mod v3_author_checks_tests_a;
use v3_author_checks_tests_a::*;
#[path = "v3_author_checks_tests_b.rs"]
mod v3_author_checks_tests_b;
use v3_author_checks_tests_b::*;
#[path = "v3_author_checks_tests_c.rs"]
mod v3_author_checks_tests_c;
