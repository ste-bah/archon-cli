//! Structured trace-window representations for world-model training.

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

use crate::embedding::{EmbeddingRequest, WorldEmbeddingAdapter};
use crate::features::{GraphContextFeatures, graph_context_for_row};
use crate::schema::{ScalarFeatures, WorldActionKind, WorldLabelSet, WorldTraceRow};

include!("representation/00_core.rs");
include!("representation/01_tests.rs");
