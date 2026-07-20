use crate::models::{DocumentStatus, OcrStatus};

pub(crate) fn status_str(s: &DocumentStatus) -> &'static str {
    match s {
        DocumentStatus::Discovered => "discovered",
        DocumentStatus::Ingesting => "ingesting",
        DocumentStatus::Ingested => "ingested",
        DocumentStatus::Processing => "processing",
        DocumentStatus::Processed => "processed",
        DocumentStatus::Failed => "failed",
    }
}

pub(crate) fn parse_status(s: &str) -> DocumentStatus {
    match s {
        "discovered" => DocumentStatus::Discovered,
        "ingesting" => DocumentStatus::Ingesting,
        "ingested" => DocumentStatus::Ingested,
        "processing" => DocumentStatus::Processing,
        "processed" => DocumentStatus::Processed,
        "failed" => DocumentStatus::Failed,
        _ => DocumentStatus::Discovered,
    }
}

pub(crate) fn ocr_status_str(s: &OcrStatus) -> &'static str {
    match s {
        OcrStatus::Pending => "pending",
        OcrStatus::Running => "running",
        OcrStatus::Completed => "completed",
        OcrStatus::Failed => "failed",
    }
}

pub(crate) fn parse_ocr_status(s: &str) -> OcrStatus {
    match s {
        "pending" => OcrStatus::Pending,
        "running" => OcrStatus::Running,
        "completed" => OcrStatus::Completed,
        "failed" => OcrStatus::Failed,
        _ => OcrStatus::Pending,
    }
}

pub(crate) fn edge_type_str(t: &crate::models::ProvenanceEdgeType) -> &'static str {
    match t {
        crate::models::ProvenanceEdgeType::DerivedFrom => "DerivedFrom",
        crate::models::ProvenanceEdgeType::Contains => "Contains",
        crate::models::ProvenanceEdgeType::ExtractedFrom => "ExtractedFrom",
        crate::models::ProvenanceEdgeType::Describes => "Describes",
        crate::models::ProvenanceEdgeType::Cites => "Cites",
    }
}

pub(crate) fn parse_edge_type(s: &str) -> crate::models::ProvenanceEdgeType {
    match s {
        "DerivedFrom" => crate::models::ProvenanceEdgeType::DerivedFrom,
        "Contains" => crate::models::ProvenanceEdgeType::Contains,
        "ExtractedFrom" => crate::models::ProvenanceEdgeType::ExtractedFrom,
        "Describes" => crate::models::ProvenanceEdgeType::Describes,
        "Cites" => crate::models::ProvenanceEdgeType::Cites,
        _ => crate::models::ProvenanceEdgeType::DerivedFrom,
    }
}

pub(crate) fn optional_str(s: &str) -> Option<String> {
    if s.is_empty() {
        None
    } else {
        Some(s.to_string())
    }
}
