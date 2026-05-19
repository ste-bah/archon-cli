//! Embedding adapter interface for world-model state/action text.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Result, bail};
use chrono::{DateTime, Utc};
use regex::Regex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

include!("embedding/00_core.rs");
include!("embedding/01_cache.rs");
include!("embedding/02_tests.rs");
